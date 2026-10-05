//! Application : état de l'éditeur et logique. Les panneaux et le canevas sont dans
//! les sous-modules `panels` et `canvas`.

mod canvas;
mod panels;

use crate::render::FieldRenderer;
use crate::theme as t;
use crate::ui::Icon;
use crate::visuals::Visuals;
use eframe::egui::{self, Key, Modifiers, Pos2, Rect, vec2};
use eframe::egui_wgpu;
use flux_core::DVec2;
use flux_core::material::MagClass;
use flux_core::raster::rasterize;
use flux_core::scene::Scene;
use flux_core::shape::{Sdf, Shape};
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
    Seed,
    Sprinkle,
}

/// Outil, nom court, raccourci, infobulle, icône (chemin SVG sur une grille de 18).
const TOOLS: [(Tool, &str, Key, &str, &str); 9] = [
    (Tool::Select, "Sélection", Key::V, "Sélectionner et déplacer (V)", "M4 2.5l10 5.5-4.5 1.3L7.5 14 4 2.5Z"),
    (Tool::Rect, "Rect.", Key::R, "Dessiner un bloc du matériau courant (R)", "M3 4.5h12v9H3z"),
    (Tool::Circle, "Disque", Key::E, "Dessiner un disque du matériau courant (E)", "M9 3a6 6 0 1 0 0 12A6 6 0 0 0 9 3Z"),
    (
        Tool::Magnet,
        "Aimant",
        Key::M,
        "Dessiner un aimant (M)",
        "M3.5 3h3.5v6.5a2 2 0 0 0 4 0V3h3.5v6.5a5.5 5.5 0 0 1-11 0V3ZM3.5 6h3.5M11 6h3.5",
    ),
    (
        Tool::Coil,
        "Bobine",
        Key::C,
        "Glisser : bobine (aller + retour) · clic : fil (C)",
        "M3 9c0-2 1.5-3 3-3s1.5 6 3 6 1.5-6 3-6 3 1 3 3M2 9h1M15 9h1",
    ),
    (Tool::Probe, "Sonde", Key::H, "Épingler une sonde (H)", "M9 2v4M9 12v4M2 9h4M12 9h4M9 7a2 2 0 1 0 0 4 2 2 0 0 0 0-4Z"),
    (Tool::Cut, "Coupe", Key::L, "Tracer une ligne de coupe (L)", "M3 15 15 3M2 13l3 3M13 2l3 3"),
    (
        Tool::Seed,
        "Graine",
        Key::G,
        "Tracer la ligne de champ passant par un point (G)",
        "M9 15V9M9 9c0-3 2-5 5-5 0 3-2 5-5 5ZM9 11c0-2-1.5-3.5-4-3.5 0 2 1.5 3.5 4 3.5Z",
    ),
    (
        Tool::Sprinkle,
        "Limaille",
        Key::S,
        "Saupoudrer de la limaille de fer (S)",
        "M3 5l2-1M8 3.5l2.5.5M13 4l2 1.5M4 9.5l2.5-.5M10 9l2.5.5M3.5 14l2-1M8 13.5l2.5.5M13 13l2 1.5",
    ),
];

enum Drag {
    Move { id: u32, offset: DVec2 },
    Create(DVec2),
    Cut(DVec2),
    Pan,
    Sprinkle,
}

#[derive(Clone, Copy, PartialEq)]
enum Compare {
    Off,
    Split,
    Diff,
}

/// État figé auquel le champ courant est comparé.
struct Reference {
    n: usize,
    size: f64,
    scene: Scene,
}

/// Transformation monde (m, y vers le haut) ↔ écran (points, y vers le bas).
#[derive(Clone, Copy)]
pub(crate) struct View {
    pub center: DVec2,
    /// Mètres par point d'écran.
    pub scale: f64,
    pub rect: Rect,
}

impl View {
    pub fn to_world(self, p: Pos2) -> DVec2 {
        let d = p - self.rect.center();
        self.center + DVec2::new(d.x as f64, -d.y as f64) * self.scale
    }

    pub fn to_screen(self, w: DVec2) -> Pos2 {
        let d = (w - self.center) / self.scale;
        self.rect.center() + vec2(d.x as f32, -d.y as f32)
    }
}

struct Icons {
    tools: Vec<Icon>,
    logo: Icon,
    chevron: Icon,
    undo: Icon,
    redo: Icon,
    search: Icon,
    zoom_out: Icon,
    zoom_in: Icon,
    frame: Icon,
    arrow: Icon,
}

impl Icons {
    fn new() -> Icons {
        Icons {
            tools: TOOLS.iter().map(|tool| Icon::parse(18.0, tool.4)).collect(),
            logo: Icon::parse(16.0, "M3 2.5h3.5v6a1.5 1.5 0 0 0 3 0v-6H13v6a5 5 0 0 1-10 0v-6ZM3 5h3.5M9.5 5H13"),
            chevron: Icon::parse(10.0, "M2.5 3.5 5 6l2.5-2.5"),
            undo: Icon::parse(16.0, "M5.5 4 2.5 7l3 3M3 7h7a3.5 3.5 0 0 1 0 7H8"),
            redo: Icon::parse(16.0, "M10.5 4l3 3-3 3M13 7H6a3.5 3.5 0 0 0 0 7h2"),
            search: Icon::parse(14.0, "M1.8 6a4.2 4.2 0 1 0 8.4 0a4.2 4.2 0 1 0-8.4 0M9.2 9.2l3 3"),
            zoom_out: Icon::parse(14.0, "M3 7h8"),
            zoom_in: Icon::parse(14.0, "M3 7h8M7 3v8"),
            frame: Icon::parse(16.0, "M2 5.5V2h3.5M10.5 2H14v3.5M14 10.5V14h-3.5M5.5 14H2v-3.5"),
            arrow: Icon::parse(12.0, "M2 6h8M6.5 2.5 10 6l-3.5 3.5"),
        }
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
    /// Taille du canevas à la dernière image, pour le cadrage.
    plot_size: egui::Vec2,
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
    show_lic: bool,
    lic_animate: bool,
    show_filings: bool,
    show_compass: bool,
    show_particles: bool,
    visuals: Visuals,
    /// Incrémenté à chaque nouveau champ calculé.
    field_version: u64,
    reference: Option<Reference>,
    compare: Compare,
    /// Position de la séparation avant/après, en fraction de la largeur du canevas.
    split: f64,
    n_lines: f64,
    show_ui: bool,
    icons: Icons,
}

fn fmt_b(tesla: f64) -> String {
    let a = tesla.abs();
    if a >= 1.0 {
        format!("{} T", crate::ui::fr(tesla, 3))
    } else if a >= 1e-3 {
        format!("{} mT", crate::ui::fr(tesla * 1e3, 2))
    } else {
        format!("{} µT", crate::ui::fr(tesla * 1e6, 2))
    }
}

/// Force : notation décimale, ou scientifique pour les très petites valeurs.
fn fmt_force(v: f64) -> String {
    if v == 0.0 || v.abs() >= 1e-2 { crate::ui::fr(v, 3) } else { format!("{v:.2e}") }
}

/// Plus petit pas « rond » (1, 2 ou 5 × 10ⁿ) au moins égal à `min`.
fn nice_step(min: f64) -> f64 {
    let p = 10f64.powf(min.log10().floor());
    [1.0, 2.0, 5.0, 10.0].into_iter().map(|m| m * p).find(|s| *s >= min).unwrap_or(10.0 * p)
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> App {
        t::apply(&cc.egui_ctx);
        let render_state = cc.wgpu_render_state.clone();
        let mut use_gpu = false;
        let mut solver: Box<dyn FieldSolver> = Box::new(Cpu64Reference::with_tolerance(1e-6));
        if let Some(rs) = &render_state {
            rs.renderer.write().callback_resources.insert(FieldRenderer::new(&rs.device, rs.target_format));
            solver = Box::new(Planar2DGpu::new(rs.device.clone(), rs.queue.clone()));
            use_gpu = true;
        }
        // Arguments : une scène à ouvrir, et `--modes=125` pour choisir les modes affichés au démarrage.
        let (mut path, mut modes) = (None, None);
        for arg in std::env::args_os().skip(1) {
            match arg.to_string_lossy().strip_prefix("--modes=") {
                Some(m) => modes = Some(m.to_owned()),
                None => path = Some(std::path::PathBuf::from(arg)),
            }
        }
        let mode = |digit: char, default: bool| modes.as_ref().map_or(default, |m| m.contains(digit));
        let scene = path.as_ref().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|s| Scene::from_ron(&s).ok());
        let mut app = App {
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
            plot_size: vec2(813.0, 792.0),
            tool: Tool::Select,
            drag: None,
            selected: None,
            cursor: None,
            lib_material: "Acier doux (S235)".into(),
            lib_filter: None,
            search: String::new(),
            show_lines: mode('1', true),
            show_map: mode('2', true),
            show_vectors: mode('3', false),
            show_lic: mode('4', false),
            lic_animate: true,
            show_filings: mode('5', false),
            show_compass: mode('6', false),
            show_particles: mode('7', false),
            visuals: Visuals::default(),
            field_version: 0,
            reference: None,
            compare: Compare::Off,
            split: 0.5,
            n_lines: 48.0,
            show_ui: true,
            icons: Icons::new(),
        };
        app.frame_all();
        app
    }

    /// Les sept représentations du champ, dans l'ordre de leurs raccourcis 1 à 7.
    fn modes(&mut self) -> [(&'static str, &mut bool); 7] {
        [
            ("Lignes", &mut self.show_lines),
            ("Carte", &mut self.show_map),
            ("Vecteurs", &mut self.show_vectors),
            ("LIC", &mut self.show_lic),
            ("Limaille", &mut self.show_filings),
            ("Boussoles", &mut self.show_compass),
            ("Particules", &mut self.show_particles),
        ]
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
            self.field_version += 1;
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

    /// Fige le champ et la scène actuels comme référence de la comparaison avant/après.
    fn freeze_reference(&mut self) {
        let field = self.solver.field();
        if let Some(rs) = &self.render_state
            && let Some(r) = rs.renderer.write().callback_resources.get_mut::<FieldRenderer>()
        {
            r.set_reference(&rs.device, &rs.queue, Some(&field.a));
        }
        self.reference = Some(Reference { n: field.n, size: field.size, scene: self.scene.clone() });
        if self.compare == Compare::Off {
            self.compare = Compare::Split;
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
        self.visuals.filings.clear();
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
            let s = k as f64 / 200.0;
            if let Some(f) = self.solver.field().sample(p.lerp(q, s)) {
                csv += &format!("{:.4};{:.6e};{:.6e};{:.6e}\n", s * (q - p).length() * 1e3, f.b.length(), f.b.x, f.b.y);
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
        let fit = (hi - lo) * 2.2 / DVec2::new(self.plot_size.x as f64, self.plot_size.y as f64);
        self.view_scale = fit.max_element().max(1e-6);
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
            for tool in TOOLS {
                if i.key_pressed(tool.2) {
                    self.tool = tool.0;
                }
            }
            let keys = [Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7];
            for (key, (_, flag)) in keys.into_iter().zip(self.modes()) {
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
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.step_solver(&ctx);
        let before = self.scene.clone();
        self.shortcuts(ui);
        let bare = |fill| egui::Frame::new().fill(fill);
        if self.show_ui {
            let panel = |p: egui::Panel, size: f32| p.exact_size(size).resizable(false).show_separator_line(false).frame(bare(t::PANEL));
            panel(egui::Panel::top("header"), 40.0).show(ui, |ui| self.header(ui));
            panel(egui::Panel::bottom("footer"), 28.0).show(ui, |ui| self.footer(ui));
            panel(egui::Panel::left("rail"), 56.0).show(ui, |ui| self.rail(ui));
            panel(egui::Panel::left("library"), 248.0).show(ui, |ui| self.library(ui));
            panel(egui::Panel::right("inspector"), 300.0).show(ui, |ui| self.inspector(ui));
            if self.scene.cut_line.is_some() {
                panel(egui::Panel::bottom("graph"), 150.0).show(ui, |ui| self.graph(ui));
            }
        }
        egui::CentralPanel::default().frame(bare(t::BG)).show(ui, |ui| self.canvas(ui));
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
