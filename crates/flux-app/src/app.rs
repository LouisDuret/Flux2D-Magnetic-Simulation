//! Application : état de l'éditeur, panneaux et canevas.

use crate::render::{FLAG_LINES, FLAG_MAP, FLAG_SRGB, FieldCallback, FieldRenderer, Uniforms};
use crate::theme;
use eframe::egui::{self, Align2, Color32, FontId, Key, Modifiers, PointerButton, Pos2, Rect, Sense, Stroke, vec2};
use eframe::egui_wgpu;
use flux_core::material::MagClass;
use flux_core::raster::rasterize;
use flux_core::scene::{Object, Scene};
use flux_core::shape::{Sdf, Shape};
use flux_core::{ABSOLUTE_ZERO_C, DVec2};
use flux_solver::{Cpu64Reference, FieldSolver, Planar2DGpu, SolveStatus, Wrench, forces};
use std::time::Duration;

#[derive(Clone, Copy, PartialEq)]
enum Tool {
    Select,
    Rect,
    Circle,
    Magnet,
    Coil,
    Probe,
    Cut,
}

const TOOLS: [(Tool, &str, Key, &str); 7] = [
    (Tool::Select, "Sélection", Key::V, "Sélectionner et déplacer (V)"),
    (Tool::Rect, "Rectangle", Key::R, "Dessiner un bloc du matériau courant (R)"),
    (Tool::Circle, "Disque", Key::E, "Dessiner un disque du matériau courant (E)"),
    (Tool::Magnet, "Aimant", Key::M, "Dessiner un aimant (M)"),
    (Tool::Coil, "Bobine", Key::C, "Glisser : bobine (aller + retour) · clic : fil (C)"),
    (Tool::Probe, "Sonde", Key::H, "Épingler une sonde (H)"),
    (Tool::Cut, "Coupe", Key::L, "Tracer une ligne de coupe (L)"),
];

enum Drag {
    Move { id: u32, offset: DVec2 },
    Create(DVec2),
    Cut(DVec2),
    Pan,
}

/// Transformation monde (m, y vers le haut) ↔ écran (points, y vers le bas).
#[derive(Clone, Copy)]
struct View {
    center: DVec2,
    scale: f64,
    rect: Rect,
}

impl View {
    fn to_world(self, p: Pos2) -> DVec2 {
        let d = p - self.rect.center();
        self.center + DVec2::new(d.x as f64, -d.y as f64) * self.scale
    }

    fn to_screen(self, w: DVec2) -> Pos2 {
        let d = (w - self.center) / self.scale;
        self.rect.center() + vec2(d.x as f32, -d.y as f32)
    }
}

pub struct App {
    scene: Scene,
    undo: Vec<Scene>,
    redo: Vec<Scene>,
    /// Une modification continue (glisser) est en cours : elle ne crée qu'une entrée d'historique.
    edit_open: bool,
    /// La scène vient d'être remplacée par l'historique : ne pas l'y réenregistrer.
    skip_history: bool,
    path: Option<std::path::PathBuf>,
    message: String,

    render_state: Option<egui_wgpu::RenderState>,
    solver: Box<dyn FieldSolver>,
    use_gpu: bool,
    grid_n: usize,
    solved_n: usize,
    dirty: bool,
    pending: bool,
    stats_stale: bool,
    status: SolveStatus,
    wrenches: Vec<Wrench>,
    b_max: f64,
    a_span: f64,

    view_center: DVec2,
    view_scale: f64,
    tool: Tool,
    drag: Option<Drag>,
    selected: Option<u32>,
    cursor: Option<DVec2>,
    lib_material: String,
    lib_filter: Option<MagClass>,
    search: String,
    show_lines: bool,
    show_map: bool,
    show_vectors: bool,
    n_lines: f64,
    show_ui: bool,
}

fn fmt_b(t: f64) -> String {
    let a = t.abs();
    if a >= 1.0 {
        format!("{t:.3} T")
    } else if a >= 1e-3 {
        format!("{:.2} mT", t * 1e3)
    } else {
        format!("{:.2} µT", t * 1e6)
    }
}

/// Champ numérique à glissement, affiché dans une unité multiple de l'unité SI.
fn drag_value(ui: &mut egui::Ui, v: &mut f64, factor: f64, speed: f64, suffix: &str) -> egui::Response {
    let mut shown = *v * factor;
    let r = ui.add(egui::DragValue::new(&mut shown).speed(speed).suffix(suffix));
    if r.changed() {
        *v = shown / factor;
    }
    r
}

/// Température en °C, bornée au zéro absolu, avec son équivalent en kelvins.
fn temperature_value(ui: &mut egui::Ui, t: &mut f64) {
    ui.horizontal(|ui| {
        ui.add(egui::DragValue::new(t).speed(1.0).range(ABSOLUTE_ZERO_C..=5000.0).suffix(" °C"));
        ui.weak(format!("{:.2} K", *t - ABSOLUTE_ZERO_C));
    });
}

/// Force : notation décimale, ou scientifique pour les très petites valeurs.
fn fmt_force(v: f64) -> String {
    if v == 0.0 || v.abs() >= 1e-2 { format!("{v:9.3}") } else { format!("{v:9.2e}") }
}

/// Pastille de couleur d'une classe magnétique.
fn color_dot(ui: &mut egui::Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.0, color);
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> App {
        theme::apply(&cc.egui_ctx);
        let render_state = cc.wgpu_render_state.clone();
        let mut use_gpu = false;
        let mut solver: Box<dyn FieldSolver> = Box::new(Cpu64Reference::with_tolerance(1e-6));
        if let Some(rs) = &render_state {
            rs.renderer.write().callback_resources.insert(FieldRenderer::new(&rs.device, rs.target_format));
            solver = Box::new(Planar2DGpu::new(rs.device.clone(), rs.queue.clone()));
            use_gpu = true;
        }
        // Une scène passée en argument s'ouvre au démarrage.
        let path = std::env::args_os().nth(1).map(std::path::PathBuf::from);
        let scene = path.as_ref().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|s| Scene::from_ron(&s).ok());
        App {
            path: path.filter(|_| scene.is_some()),
            scene: scene.unwrap_or_else(Scene::demo),
            undo: Vec::new(),
            redo: Vec::new(),
            edit_open: false,
            skip_history: false,
            message: String::new(),
            render_state,
            solver,
            use_gpu,
            grid_n: if use_gpu { 1024 } else { 512 },
            solved_n: 0,
            dirty: true,
            pending: false,
            stats_stale: true,
            status: SolveStatus::default(),
            wrenches: Vec::new(),
            b_max: 1.0,
            a_span: 0.0,
            view_center: DVec2::ZERO,
            view_scale: 1.6e-4,
            tool: Tool::Select,
            drag: None,
            selected: None,
            cursor: None,
            lib_material: "Acier doux (S235)".into(),
            lib_filter: None,
            search: String::new(),
            show_lines: true,
            show_map: true,
            show_vectors: false,
            n_lines: 48.0,
            show_ui: true,
        }
    }

    fn set_solver(&mut self, gpu: bool) {
        self.solver = match (&self.render_state, gpu) {
            (Some(rs), true) => Box::new(Planar2DGpu::new(rs.device.clone(), rs.queue.clone())),
            _ => Box::new(Cpu64Reference::with_tolerance(1e-6)),
        };
        self.use_gpu = gpu && self.render_state.is_some();
        self.dirty = true;
    }

    /// Rastérise si la scène a changé, puis itère dans le budget de l'image.
    fn step_solver(&mut self, ctx: &egui::Context) {
        let dragging = self.edit_open || matches!(self.drag, Some(Drag::Move { .. }));
        // Résolution progressive : le solveur CPU calcule en 256² pendant le geste.
        let n = if dragging && !self.use_gpu { self.grid_n.min(256) } else { self.grid_n };
        if self.dirty || n != self.solved_n {
            self.solver.upload(&rasterize(&self.scene, n));
            (self.solved_n, self.dirty, self.pending) = (n, false, true);
        }
        if self.pending {
            self.status = self.solver.solve(Duration::from_millis(if self.use_gpu { 6 } else { 12 }));
            self.stats_stale = true;
            let field = self.solver.field();
            if let Some(rs) = &self.render_state
                && let Some(r) = rs.renderer.write().callback_resources.get_mut::<FieldRenderer>()
            {
                r.upload(&rs.device, &rs.queue, field);
            }
            if self.status.converged {
                self.wrenches = forces(field, &self.scene);
                self.pending = false;
            } else {
                ctx.request_repaint();
            }
        }
        // L'échelle de couleurs et le pas des lignes restent fixes pendant un geste.
        if self.stats_stale && (!dragging || self.a_span == 0.0) {
            let field = self.solver.field();
            let (lo, hi) = field.a_range();
            self.a_span = (hi - lo).max(0.0);
            self.b_max = field.b_max().max(1e-9);
            self.stats_stale = false;
        }
    }

    fn do_undo(&mut self) {
        self.skip_history = true;
        if let Some(s) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.scene, s));
        }
    }

    fn do_redo(&mut self) {
        self.skip_history = true;
        if let Some(s) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.scene, s));
        }
    }

    fn load(&mut self, scene: Scene, path: Option<std::path::PathBuf>) {
        (self.scene, self.path, self.selected) = (scene, path, None);
        self.undo.clear();
        self.redo.clear();
        self.skip_history = true;
        self.frame_all();
    }

    fn open(&mut self) {
        let Some(path) = rfd::FileDialog::new().add_filter("Scène Flux2D", &["flux"]).pick_file() else { return };
        match std::fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|s| Scene::from_ron(&s)) {
            Ok(scene) => self.load(scene, Some(path)),
            Err(e) => self.message = format!("Ouverture impossible : {e}"),
        }
    }

    fn save(&mut self, ask: bool) {
        if ask || self.path.is_none() {
            let name = format!("{}.flux", self.scene.name);
            let Some(p) = rfd::FileDialog::new().add_filter("Scène Flux2D", &["flux"]).set_file_name(name).save_file() else {
                return;
            };
            self.path = Some(p);
        }
        let path = self.path.clone().unwrap();
        let result = self.scene.to_ron().map_err(|e| e.to_string()).and_then(|s| std::fs::write(&path, s).map_err(|e| e.to_string()));
        self.message = match result {
            Ok(()) => format!("Enregistré : {}", path.display()),
            Err(e) => format!("Enregistrement impossible : {e}"),
        };
    }

    fn export_cut(&mut self) {
        let Some([p, q]) = self.scene.cut_line else { return };
        let Some(path) = rfd::FileDialog::new().add_filter("CSV", &["csv"]).set_file_name("coupe.csv").save_file() else {
            return;
        };
        let mut csv = String::from("s_mm;B_T;Bx_T;By_T\n");
        for k in 0..=200 {
            let t = k as f64 / 200.0;
            if let Some(s) = self.solver.field().sample(p.lerp(q, t)) {
                csv += &format!("{:.4};{:.6e};{:.6e};{:.6e}\n", t * (q - p).length() * 1e3, s.b.length(), s.b.x, s.b.y);
            }
        }
        if let Err(e) = std::fs::write(&path, csv) {
            self.message = format!("Export impossible : {e}");
        }
    }

    /// Cadre tous les objets (ou le domaine si la scène est vide).
    fn frame_all(&mut self) {
        let (mut lo, mut hi) = (DVec2::splat(f64::INFINITY), DVec2::splat(f64::NEG_INFINITY));
        for o in &self.scene.objects {
            let r = DVec2::splat(o.shape.bounding_radius());
            (lo, hi) = (lo.min(o.pos.truncate() - r), hi.max(o.pos.truncate() + r));
        }
        if lo.x > hi.x {
            (lo, hi) = (DVec2::splat(-self.scene.size / 4.0), DVec2::splat(self.scene.size / 4.0));
        }
        self.view_center = (lo + hi) / 2.0;
        self.view_scale = ((hi - lo).max_element() * 2.2 / 800.0).max(1e-6);
    }

    /// Crée l'objet de l'outil courant entre deux points du canevas.
    fn create(&mut self, a: DVec2, b: DVec2) {
        let (c, d) = ((a + b) / 2.0, (b - a).abs());
        let tiny = d.max_element() < 4.0 * self.view_scale;
        let class = self.scene.material(&self.lib_material).map(|m| m.class);
        let lib = self.lib_material.clone();
        let rect = |w: f64, h: f64| if tiny { Shape::Rect { w, h } } else { Shape::Rect { w: d.x.max(1e-3), h: d.y.max(1e-3) } };
        let id = match self.tool {
            Tool::Rect => self.scene.add("Bloc", rect(0.02, 0.02), c, &lib),
            Tool::Circle => {
                let r = if tiny { 0.01 } else { (b - a).length().max(1e-3) };
                self.scene.add("Disque", Shape::Circle { r }, a, &lib)
            }
            Tool::Magnet => {
                let mat = if class == Some(MagClass::Magnet) { lib.as_str() } else { "NdFeB N42" };
                self.scene.add("Aimant", rect(0.01, 0.03), c, mat)
            }
            Tool::Coil if tiny => {
                let id = self.scene.add("Fil", Shape::Circle { r: 0.002 }, a, "Cuivre (bobinage)");
                let o = self.scene.get_mut(id).unwrap();
                (o.turns, o.current) = (1.0, 10.0);
                id
            }
            Tool::Coil => {
                // Coupe d'une bobine : deux conducteurs parcourus en sens opposés.
                let cw = (d.x * 0.2).max(1e-3);
                let mut id = 0;
                for sign in [-1.0, 1.0] {
                    let pos = c + DVec2::new(sign * (d.x - cw) / 2.0, 0.0);
                    id = self.scene.add("Bobine", Shape::Rect { w: cw, h: d.y.max(1e-3) }, pos, "Cuivre (bobinage)");
                    let o = self.scene.get_mut(id).unwrap();
                    (o.turns, o.current) = (100.0, sign);
                }
                id
            }
            _ => return,
        };
        self.selected = Some(id);
        self.tool = Tool::Select;
    }

    fn shortcuts(&mut self, ui: &egui::Ui) {
        if ui.ctx().egui_wants_keyboard_input() {
            return;
        }
        let (undo, redo, dup, save) = ui.input_mut(|i| {
            (
                i.consume_key(Modifiers::COMMAND, Key::Z),
                i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z),
                i.consume_key(Modifiers::COMMAND, Key::D),
                i.consume_key(Modifiers::COMMAND, Key::S),
            )
        });
        if redo {
            self.do_redo();
        } else if undo {
            self.do_undo();
        }
        if dup && let Some(id) = self.selected {
            self.selected = self.scene.duplicate(id, DVec2::splat(0.005));
        }
        if save {
            self.save(false);
        }
        ui.input(|i| {
            if i.modifiers.any() {
                return;
            }
            for (tool, _, key, _) in TOOLS {
                if i.key_pressed(key) {
                    self.tool = tool;
                }
            }
            for (key, flag) in [(Key::Num1, &mut self.show_lines), (Key::Num2, &mut self.show_map), (Key::Num3, &mut self.show_vectors)] {
                if i.key_pressed(key) {
                    *flag = !*flag;
                }
            }
            if i.key_pressed(Key::Tab) {
                self.show_ui = !self.show_ui;
            }
            if i.key_pressed(Key::F) {
                self.frame_all();
            }
            if i.key_pressed(Key::Escape) {
                (self.tool, self.selected) = (Tool::Select, None);
            }
            if (i.key_pressed(Key::Delete) || i.key_pressed(Key::Backspace))
                && let Some(id) = self.selected.take()
            {
                self.scene.objects.retain(|o| o.id != id);
            }
        });
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.scene.name).desired_width(180.0));
            ui.separator();
            if ui.button("Nouveau").clicked() {
                self.load(Scene::default(), None);
            }
            ui.menu_button("Exemples", |ui| {
                if ui.button("Aimant + plaque de fer").clicked() {
                    self.load(Scene::demo(), None);
                }
                if ui.button("Supraconducteur et diamagnétique").clicked() {
                    self.load(Scene::meissner_demo(), None);
                }
            });
            if ui.button("Ouvrir…").clicked() {
                self.open();
            }
            if ui.button("Enregistrer").on_hover_text("Ctrl S").clicked() {
                self.save(false);
            }
            if ui.button("Sous…").clicked() {
                self.save(true);
            }
            ui.separator();
            if ui.add_enabled(!self.undo.is_empty(), egui::Button::new("Annuler")).on_hover_text("Annuler (Ctrl Z)").clicked() {
                self.do_undo();
            }
            if ui.add_enabled(!self.redo.is_empty(), egui::Button::new("Rétablir")).on_hover_text("Rétablir (Ctrl Maj Z)").clicked() {
                self.do_redo();
            }
            ui.separator();
            ui.toggle_value(&mut self.show_lines, "1 Lignes");
            ui.toggle_value(&mut self.show_map, "2 Carte");
            ui.toggle_value(&mut self.show_vectors, "3 Vecteurs");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.colored_label(theme::ACCENT, format!("2D plan · profondeur {:.0} mm", self.scene.depth * 1e3))
                    .on_hover_text("Objets extrudés à l'infini selon z : les forces sont calculées par mètre de profondeur.");
            });
        });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let s = &self.status;
            let state = if self.pending { "calcul…" } else { "convergé" };
            ui.colored_label(if self.pending { theme::WARN } else { theme::FORCE }, state);
            ui.label(format!(
                "{} · {}² · {} it. · résidu {:.1e} · {:.1} ms",
                self.solver.name(),
                self.solved_n,
                s.iterations,
                s.residual,
                s.elapsed.as_secs_f64() * 1e3
            ));
            ui.separator();
            if let Some(p) = self.cursor {
                let mut text = format!("x {:.1} mm  y {:.1} mm", p.x * 1e3, p.y * 1e3);
                if let Some(f) = self.solver.field().sample(p.extend(0.0)) {
                    text += &format!("   |B| {}   Bx {}   By {}   A {:.3e} T·m", fmt_b(f.b.length()), fmt_b(f.b.x), fmt_b(f.b.y), f.a);
                }
                ui.monospace(text);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(format!("{:.0} img/s", 1.0 / ui.input(|i| i.stable_dt).max(1e-4)));
                ui.colored_label(theme::WARN, self.message.as_str());
            });
        });
    }

    fn left_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            for (tool, label, _, tip) in TOOLS {
                if ui.selectable_label(self.tool == tool, label).on_hover_text(tip).clicked() {
                    self.tool = tool;
                }
            }
        });
        ui.separator();
        ui.strong("Bibliothèque");
        ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("Rechercher…"));
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(&mut self.lib_filter, None, "Tous");
            for class in MagClass::ALL {
                ui.selectable_value(&mut self.lib_filter, Some(class), class.label());
            }
        });
        let needle = self.search.to_lowercase();
        let mut picked = None;
        egui::ScrollArea::vertical().id_salt("lib").max_height(ui.available_height() * 0.55).show(ui, |ui| {
            for m in &self.scene.materials {
                if self.lib_filter.is_some_and(|c| c != m.class) || !m.name.to_lowercase().contains(&needle) {
                    continue;
                }
                let key = match m.class {
                    MagClass::Magnet => format!("Br {:.2} T", m.br),
                    MagClass::Ferro => format!("μr {:.0}", m.mu_r),
                    MagClass::Superconductor => format!("Tc {:.0} K", m.t_critical - ABSOLUTE_ZERO_C),
                    _ => format!("χ {:+.1e}", m.chi),
                };
                ui.horizontal(|ui| {
                    color_dot(ui, theme::class_color(m.class));
                    if ui
                        .selectable_label(self.lib_material == m.name, m.name.as_str())
                        .on_hover_text(format!("{key} · ρ {:.0} kg/m³", m.density))
                        .clicked()
                    {
                        picked = Some(m.name.clone());
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.weak(key));
                });
            }
        });
        // Choisir un matériau l'applique aussi à la sélection.
        if let Some(name) = picked {
            if let Some(o) = self.selected.and_then(|id| self.scene.get_mut(id)) {
                o.material = name.clone();
            }
            self.lib_material = name;
        }
        ui.separator();
        ui.strong("Scène");
        egui::ScrollArea::vertical().id_salt("tree").show(ui, |ui| {
            for o in &self.scene.objects {
                let color = self.scene.material(&o.material).map_or(theme::TEXT_DIM, |m| theme::class_color(m.class));
                ui.horizontal(|ui| {
                    color_dot(ui, color);
                    if ui.selectable_label(self.selected == Some(o.id), o.name.as_str()).clicked() {
                        self.selected = Some(o.id);
                    }
                });
            }
        });
    }

    fn inspector(&mut self, ui: &mut egui::Ui) {
        let names: Vec<String> = self.scene.materials.iter().map(|m| m.name.clone()).collect();
        let (depth, ambient) = (self.scene.depth, self.scene.ambient);
        let wrench = self.selected.and_then(|id| self.wrenches.iter().find(|w| w.id == id).copied());
        let Some(id) = self.selected.filter(|&id| self.scene.get(id).is_some()) else {
            ui.strong("Scène");
            egui::Grid::new("scene").num_columns(2).show(ui, |ui| {
                ui.label("Profondeur");
                drag_value(ui, &mut self.scene.depth, 1e3, 0.1, " mm");
                ui.end_row();
                ui.label("Domaine");
                drag_value(ui, &mut self.scene.size, 1e3, 1.0, " mm");
                ui.end_row();
                ui.label("T ambiante");
                temperature_value(ui, &mut self.scene.ambient);
                ui.end_row();
            });
            self.scene.depth = self.scene.depth.max(1e-4);
            self.scene.size = self.scene.size.clamp(0.01, 10.0);
            ui.separator();
            ui.strong("Calcul");
            let mut gpu = self.use_gpu;
            ui.horizontal(|ui| {
                ui.add_enabled_ui(self.render_state.is_some(), |ui| ui.selectable_value(&mut gpu, true, "GPU f32"));
                ui.selectable_value(&mut gpu, false, "CPU f64");
            });
            if gpu != self.use_gpu {
                self.set_solver(gpu);
            }
            ui.horizontal(|ui| {
                ui.label("Grille");
                for n in [256, 512, 1024, 2048] {
                    ui.selectable_value(&mut self.grid_n, n, format!("{n}²"));
                }
            });
            ui.label(format!("Pas : {:.3} mm", self.scene.size / self.grid_n as f64 * 1e3));
            ui.add(egui::Slider::new(&mut self.n_lines, 8.0..=160.0).text("lignes"));
            ui.separator();
            ui.weak("Sélectionnez un objet pour l'inspecter, ou choisissez un outil et dessinez sur le canevas.");
            return;
        };
        let mat = self.scene.get(id).and_then(|o| self.scene.material(&o.material)).cloned();
        let o = self.scene.get_mut(id).unwrap();
        ui.text_edit_singleline(&mut o.name);
        egui::ComboBox::from_id_salt("material").width(ui.available_width() - 8.0).selected_text(o.material.clone()).show_ui(ui, |ui| {
            for name in &names {
                ui.selectable_value(&mut o.material, name.clone(), name.as_str());
            }
        });
        ui.separator();
        ui.strong("Géométrie");
        egui::Grid::new("geometry").num_columns(2).show(ui, |ui| {
            ui.label("Position");
            ui.horizontal(|ui| {
                drag_value(ui, &mut o.pos.x, 1e3, 0.1, " mm");
                drag_value(ui, &mut o.pos.y, 1e3, 0.1, " mm");
            });
            ui.end_row();
            ui.label("Rotation");
            drag_value(ui, &mut o.angle, 180.0 / std::f64::consts::PI, 1.0, "°");
            ui.end_row();
            match &mut o.shape {
                Shape::Rect { w, h } => {
                    ui.label("Taille");
                    ui.horizontal(|ui| {
                        drag_value(ui, w, 1e3, 0.1, " mm");
                        drag_value(ui, h, 1e3, 0.1, " mm");
                    });
                    (*w, *h) = (w.max(2e-4), h.max(2e-4));
                }
                Shape::Circle { r } => {
                    ui.label("Rayon");
                    drag_value(ui, r, 1e3, 0.1, " mm");
                    *r = r.max(2e-4);
                }
                Shape::Ring { r_in, r_out } => {
                    ui.label("Rayons");
                    ui.horizontal(|ui| {
                        drag_value(ui, r_in, 1e3, 0.1, " mm");
                        drag_value(ui, r_out, 1e3, 0.1, " mm");
                    });
                    *r_in = r_in.max(1e-4);
                    *r_out = r_out.max(*r_in + 2e-4);
                }
                Shape::Polygon { pts } => {
                    ui.label("Polygone");
                    ui.label(format!("{} sommets", pts.len()));
                }
            }
            ui.end_row();
            ui.label("Température");
            temperature_value(ui, &mut o.temperature);
            ui.end_row();
        });
        let Some(mat) = mat else { return };
        let area = o.shape.area();
        ui.label(format!("Masse : {:.1} g  ({:.3} kg/m)", mat.density * area * depth * 1e3, mat.density * area));
        ui.separator();
        match mat.class {
            MagClass::Magnet => {
                ui.strong("Aimantation");
                ui.horizontal(|ui| {
                    ui.label("Angle");
                    drag_value(ui, &mut o.mag_angle, 180.0 / std::f64::consts::PI, 1.0, "°");
                    if ui.button("Inverser").clicked() {
                        o.mag_angle += std::f64::consts::PI;
                    }
                });
                ui.label(format!("Br(20 °C) = {:.3} T  →  Br({:.0} °C) = {:.3} T", mat.br, o.temperature, mat.br_at(o.temperature)));
                ui.label(format!("μrec = {:.2} · Tc = {:.0} °C", mat.mu_r, mat.t_curie));
            }
            MagClass::Ferro => {
                ui.strong("Ferromagnétique doux");
                ui.label(format!("μr = {:.0} (linéaire) · Tc = {:.0} °C", mat.mu_r_solver(o.temperature), mat.t_curie));
            }
            MagClass::Conductor => {
                ui.strong("Courant");
                egui::Grid::new("current").num_columns(2).show(ui, |ui| {
                    ui.label("Spires");
                    ui.add(egui::DragValue::new(&mut o.turns).speed(1.0).range(0.0..=1e6));
                    ui.end_row();
                    ui.label("Courant");
                    ui.add(egui::DragValue::new(&mut o.current).speed(0.1).suffix(" A"));
                    ui.end_row();
                });
                if ui.button(if o.current >= 0.0 { "Sortant — inverser" } else { "Entrant — inverser" }).clicked() {
                    o.current = -o.current;
                }
                let j = o.amp_turns().abs() / area * 1e-6;
                let color = if j > 5.0 { theme::WARN } else { theme::TEXT_DIM };
                ui.colored_label(color, format!("{:.0} A·tours · J = {j:.2} A/mm²", o.amp_turns()));
            }
            MagClass::Para | MagClass::Dia => {
                ui.strong(if mat.class == MagClass::Para { "Paramagnétique" } else { "Diamagnétique" });
                let chi = mat.chi_at(o.temperature);
                ui.label(format!("χ = {chi:+.2e}"));
                ui.weak(format!(
                    "Modifie le champ d'environ {:.4} % (χ/2) : invisible sur les lignes de champ. \
                     L'objet subit en revanche une force (densité de Kelvin), affichée ci-dessous.",
                    chi.abs() * 50.0
                ));
            }
            MagClass::Superconductor => {
                ui.strong("Supraconducteur");
                ui.label(format!("Tc = {:.1} K ({:.1} °C)", mat.t_critical - ABSOLUTE_ZERO_C, mat.t_critical));
                if mat.is_superconducting(o.temperature) {
                    ui.colored_label(theme::ACCENT, "État Meissner : le champ est expulsé (χ = −1).");
                } else {
                    ui.colored_label(theme::WARN, "État normal : refroidir sous Tc pour expulser le champ.");
                }
                ui.horizontal_wrapped(|ui| {
                    for (label, t) in [("Azote liquide", -196.0), ("Hélium liquide", -269.0), ("Ambiante", ambient)] {
                        if ui.button(label).on_hover_text(format!("{t:.0} °C")).clicked() {
                            o.temperature = t;
                        }
                    }
                });
            }
        }
        if let Some(w) = wrench {
            ui.separator();
            ui.strong("Force magnétique");
            let f = w.force.truncate();
            ui.monospace(format!("|F| {} N/m  {} N", fmt_force(f.length()), fmt_force(f.length() * depth)));
            ui.monospace(format!("Fx  {} N/m  {} N", fmt_force(f.x), fmt_force(f.x * depth)));
            ui.monospace(format!("Fy  {} N/m  {} N", fmt_force(f.y), fmt_force(f.y * depth)));
            ui.monospace(format!("τ   {:9.4} N·m/m", w.torque));
            let weight = mat.density * area * 9.81;
            ui.label(format!("F / (m·g) = {}", fmt_force(f.length() / weight).trim()));
            if w.resolution_limited {
                ui.colored_label(theme::WARN, "⚠ Entrefer plus fin que la grille : valeur limitée par la résolution.");
            }
        }
    }

    fn bottom_panel(&mut self, ui: &mut egui::Ui) {
        let Some([p, q]) = self.scene.cut_line else { return };
        let field = self.solver.field();
        let samples: Vec<f64> = (0..=240).map(|k| field.sample(p.lerp(q, k as f64 / 240.0)).map_or(0.0, |s| s.b.length())).collect();
        let max = samples.iter().copied().fold(1e-12, f64::max);
        let (mut clear, mut export) = (false, false);
        ui.horizontal(|ui| {
            ui.strong("Ligne de coupe — |B|");
            ui.label(format!("longueur {:.1} mm · max {}", (q - p).length() * 1e3, fmt_b(max)));
            export = ui.button("Exporter CSV").clicked();
            clear = ui.button("Effacer").clicked();
        });
        let (resp, painter) = ui.allocate_painter(vec2(ui.available_width(), 110.0), Sense::hover());
        let r = resp.rect.shrink(4.0);
        painter.rect_filled(resp.rect, 4.0, theme::BG);
        let pts: Vec<Pos2> = samples
            .iter()
            .enumerate()
            .map(|(k, &b)| egui::pos2(r.left() + r.width() * k as f32 / 240.0, r.bottom() - r.height() * (b / max) as f32))
            .collect();
        painter.add(egui::Shape::line(pts, Stroke::new(1.5, theme::ACCENT)));
        painter.text(r.left_top(), Align2::LEFT_TOP, fmt_b(max), FontId::monospace(10.0), theme::TEXT_DIM);
        painter.text(r.left_bottom(), Align2::LEFT_BOTTOM, "0", FontId::monospace(10.0), theme::TEXT_DIM);
        if export {
            self.export_cut();
        }
        if clear {
            self.scene.cut_line = None;
        }
    }

    fn canvas(&mut self, ui: &mut egui::Ui) {
        let (resp, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        let mut view = View { center: self.view_center, scale: self.view_scale, rect: resp.rect };

        // Zoom centré sur le curseur.
        if let Some(p) = resp.hover_pos() {
            let zoom = ui.input(|i| i.zoom_delta() as f64 * (i.smooth_scroll_delta.y as f64 * 0.004).exp());
            if zoom != 1.0 {
                let before = view.to_world(p);
                view.scale = (view.scale / zoom).clamp(1e-6, 5e-3);
                view.center += before - view.to_world(p);
            }
        }
        let pos = resp.interact_pointer_pos().or(resp.hover_pos());
        let cur = pos.map(|p| view.to_world(p));
        self.cursor = resp.hover_pos().map(|p| view.to_world(p));

        if resp.drag_started_by(PointerButton::Primary) {
            let start = ui.input(|i| i.pointer.press_origin()).map(|p| view.to_world(p)).or(cur).unwrap_or_default();
            self.drag = Some(match self.tool {
                Tool::Select => match self.scene.pick(start.extend(0.0)) {
                    Some(mut id) => {
                        if ui.input(|i| i.modifiers.alt) {
                            id = self.scene.duplicate(id, DVec2::ZERO).unwrap_or(id);
                        }
                        self.selected = Some(id);
                        Drag::Move { id, offset: self.scene.get(id).unwrap().pos.truncate() - start }
                    }
                    None => Drag::Pan,
                },
                Tool::Probe => Drag::Pan,
                Tool::Cut => Drag::Cut(start),
                _ => Drag::Create(start),
            });
        }
        let panning =
            resp.dragged_by(PointerButton::Middle) || (resp.dragged_by(PointerButton::Primary) && matches!(self.drag, Some(Drag::Pan)));
        if panning {
            let d = resp.drag_delta();
            view.center -= DVec2::new(d.x as f64, -d.y as f64) * view.scale;
        }
        if resp.dragged_by(PointerButton::Primary)
            && let Some(cur) = cur
        {
            match self.drag {
                Some(Drag::Move { id, offset }) => {
                    if let Some(o) = self.scene.get_mut(id) {
                        o.pos = (cur + offset).extend(0.0);
                    }
                }
                Some(Drag::Cut(start)) => self.scene.cut_line = Some([start.extend(0.0), cur.extend(0.0)]),
                _ => {}
            }
        }
        if resp.drag_stopped() {
            if let (Some(Drag::Create(start)), Some(cur)) = (self.drag.take(), cur) {
                self.create(start, cur);
            }
        } else if resp.clicked()
            && let Some(cur) = cur
        {
            match self.tool {
                Tool::Select => self.selected = self.scene.pick(cur.extend(0.0)),
                Tool::Probe => self.scene.probes.push(cur.extend(0.0)),
                Tool::Cut => {}
                _ => self.create(cur, cur),
            }
        }
        (self.view_center, self.view_scale) = (view.center, view.scale);

        // Champ : rendu GPU derrière les objets.
        let field = self.solver.field();
        let lines = self.show_lines && self.a_span > 0.0;
        if self.render_state.is_some() {
            let srgb = self.render_state.as_ref().is_some_and(|rs| rs.target_format.is_srgb());
            let half = resp.rect.size() * 0.5 * view.scale as f32;
            painter.add(egui_wgpu::Callback::new_paint_callback(
                resp.rect,
                FieldCallback(Uniforms {
                    center: view.center.as_vec2().to_array(),
                    half: [half.x, half.y],
                    size: field.size as f32,
                    n: field.n as u32,
                    delta_a: (self.a_span / self.n_lines) as f32,
                    b_max: self.b_max as f32,
                    flags: (lines as u32 * FLAG_LINES) | (self.show_map as u32 * FLAG_MAP) | (srgb as u32 * FLAG_SRGB),
                    line_px: 0.9 * ui.ctx().pixels_per_point(),
                    pad: [0.0; 2],
                }),
            ));
        }
        if self.show_vectors {
            let step = 34.0;
            let (nx, ny) = ((resp.rect.width() / step) as i32, (resp.rect.height() / step) as i32);
            for j in 0..=ny {
                for i in 0..=nx {
                    let p = resp.rect.min + vec2(i as f32 + 0.5, j as f32 + 0.5) * step;
                    let Some(s) = field.sample(view.to_world(p).extend(0.0)) else { continue };
                    // Longueur logarithmique sur trois décades.
                    let len = (1.0 + (s.b.length() / self.b_max).max(1e-12).log10() / 3.0).clamp(0.0, 1.0) as f32;
                    let dir = s.b.truncate().normalize_or_zero();
                    let v = vec2(dir.x as f32, -dir.y as f32) * len * step * 0.8;
                    if len > 0.05 {
                        painter.arrow(p - v * 0.5, v, Stroke::new(1.0, theme::TEXT.gamma_multiply(0.75)));
                    }
                }
            }
        }

        for o in &self.scene.objects {
            self.draw_object(&painter, view, o);
        }
        let f_max = self.wrenches.iter().map(|w| w.force.length()).fold(1e-12, f64::max);
        for w in &self.wrenches {
            let Some(o) = self.scene.get(w.id) else { continue };
            let f = w.force.truncate();
            if f.length() < 1e-12 || (f.length() < 1e-3 * f_max && self.selected != Some(w.id)) {
                continue;
            }
            let len = 20.0 + 50.0 * (f.length() / f_max).sqrt();
            let v = vec2(f.x as f32, -f.y as f32) / f.length() as f32 * len as f32;
            let origin = view.to_screen(o.pos.truncate());
            painter.arrow(origin, v, Stroke::new(2.0, theme::FORCE));
            if self.selected == Some(w.id) {
                painter.text(
                    origin + v,
                    Align2::LEFT_BOTTOM,
                    format!(" {} N/m", fmt_force(f.length()).trim()),
                    FontId::monospace(12.0),
                    theme::FORCE,
                );
            }
        }
        if let Some([p, q]) = self.scene.cut_line {
            painter.line_segment([view.to_screen(p.truncate()), view.to_screen(q.truncate())], Stroke::new(1.5, theme::ACCENT));
        }
        for p in &self.scene.probes {
            let s = view.to_screen(p.truncate());
            painter.circle(s, 4.0, theme::BG, Stroke::new(1.5, theme::ACCENT));
            if let Some(f) = field.sample(*p) {
                painter.text(s + vec2(7.0, 0.0), Align2::LEFT_CENTER, fmt_b(f.b.length()), FontId::monospace(12.0), theme::ACCENT);
            }
        }
        if let (Some(Drag::Create(start)), Some(cur)) = (&self.drag, cur) {
            let stroke = Stroke::new(1.0, theme::ACCENT);
            if self.tool == Tool::Circle {
                painter.circle_stroke(view.to_screen(*start), ((cur - *start).length() / view.scale) as f32, stroke);
            } else {
                let r = Rect::from_two_pos(view.to_screen(*start), view.to_screen(cur));
                painter.rect_stroke(r, 0.0, stroke, egui::StrokeKind::Middle);
            }
        }
        if self.scene.objects.is_empty() {
            let hint = "Canevas vide — choisissez un outil : M aimant · R bloc · E disque · C bobine\nMolette : zoom · clic milieu : déplacer la vue · F : cadrer";
            painter.text(resp.rect.center(), Align2::CENTER_CENTER, hint, FontId::proportional(15.0), theme::TEXT_DIM);
        }
    }

    fn draw_object(&self, painter: &egui::Painter, view: View, o: &Object) {
        let Some(mat) = self.scene.material(&o.material) else { return };
        let selected = self.selected == Some(o.id);
        let color = theme::class_color(mat.class);
        let fill = if mat.class == MagClass::Magnet { Color32::from_black_alpha(90) } else { color.gamma_multiply(0.22) };
        let stroke = if selected { Stroke::new(2.0, theme::ACCENT) } else { Stroke::new(1.2, color) };
        let center = o.pos.truncate();
        let tf = |l: DVec2| view.to_screen(center + DVec2::from_angle(o.angle).rotate(l));
        let px = |m: f64| (m / view.scale) as f32;
        match &o.shape {
            Shape::Rect { w, h } => {
                let pts = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)].map(|(sx, sy)| tf(DVec2::new(sx * w / 2.0, sy * h / 2.0)));
                painter.add(egui::Shape::convex_polygon(pts.to_vec(), fill, stroke));
            }
            Shape::Circle { r } => {
                painter.circle(tf(DVec2::ZERO), px(*r), fill, stroke);
            }
            Shape::Ring { r_in, r_out } => {
                painter.circle_stroke(tf(DVec2::ZERO), px(*r_out), stroke);
                painter.circle_stroke(tf(DVec2::ZERO), px(*r_in), stroke);
            }
            Shape::Polygon { pts } => {
                painter.add(egui::Shape::closed_line(pts.iter().map(|p| tf(*p)).collect(), stroke));
            }
        }
        let c = view.to_screen(center);
        if mat.class == MagClass::Magnet {
            // Flèche d'aimantation du pôle Sud vers le pôle Nord, toujours étiquetés.
            let d = o.mag_dir();
            let half = match &o.shape {
                Shape::Rect { w, h } => {
                    let l = DVec2::from_angle(o.mag_angle);
                    0.5 / (l.x.abs() / w).max(l.y.abs() / h)
                }
                s => s.bounding_radius(),
            };
            let v = vec2(d.x as f32, -d.y as f32) * px(half * 0.6);
            painter.arrow(c - v, v * 2.0, Stroke::new(1.5, Color32::WHITE));
            for (pos, letter, color) in [(c + v * 1.4, "N", theme::NORTH), (c - v * 1.4, "S", theme::SOUTH)] {
                painter.circle_filled(pos, 8.0, theme::BG);
                painter.text(pos, Align2::CENTER_CENTER, letter, FontId::proportional(12.0), color);
            }
        } else if o.amp_turns() != 0.0 {
            // ⊙ courant sortant, ⊗ courant entrant.
            let s = Stroke::new(1.5, Color32::WHITE);
            painter.circle_stroke(c, 6.0, s);
            if o.amp_turns() > 0.0 {
                painter.circle_filled(c, 1.8, Color32::WHITE);
            } else {
                painter.line_segment([c + vec2(-4.0, -4.0), c + vec2(4.0, 4.0)], s);
                painter.line_segment([c + vec2(-4.0, 4.0), c + vec2(4.0, -4.0)], s);
            }
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.step_solver(&ctx);
        let before = self.scene.clone();
        self.shortcuts(ui);
        if self.show_ui {
            egui::Panel::top("top").show(ui, |ui| self.top_bar(ui));
            egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
            egui::Panel::left("left").default_size(250.0).show(ui, |ui| self.left_panel(ui));
            egui::Panel::right("inspector").default_size(290.0).show(ui, |ui| self.inspector(ui));
            if self.scene.cut_line.is_some() {
                egui::Panel::bottom("graphs").show(ui, |ui| self.bottom_panel(ui));
            }
        }
        egui::CentralPanel::default().frame(egui::Frame::new().fill(theme::BG)).show(ui, |ui| self.canvas(ui));

        self.scene.clamp_temperatures();

        // Historique : un geste continu ne crée qu'une entrée.
        let pointer_down = ctx.input(|i| i.pointer.any_down());
        if self.scene != before {
            if !self.edit_open && !self.skip_history {
                self.undo.push(before);
                self.redo.clear();
            }
            self.edit_open = pointer_down;
            self.dirty = true;
            ctx.request_repaint();
        } else if !pointer_down {
            if self.edit_open {
                ctx.request_repaint();
            }
            self.edit_open = false;
        }
        self.skip_history = false;
    }
}
