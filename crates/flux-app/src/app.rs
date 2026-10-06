//! Application : état de l'éditeur et logique. Les panneaux et le canevas sont dans
//! les sous-modules `panels` et `canvas`.

mod canvas;
mod palette;
mod panels;
mod sim;
#[cfg(test)]
mod tests;

use crate::lang::{self, Lang, tr, tr_name};
use crate::render::FieldRenderer;
use crate::theme as t;
use crate::ui::{self, Icon};
use crate::units::{self, Units};
use crate::visuals::Visuals;
use eframe::egui::{self, Key, Modifiers, Pos2, Rect, vec2};
use eframe::egui_wgpu;
use flux_core::DVec2;
use flux_core::material::MagClass;
use flux_core::raster::rasterize;
use flux_core::scene::Scene;
use flux_core::shape::{PathNode, Sdf, Shape, flatten_path};
use flux_solver::{Cpu64Reference, DEMAG_TOLERANCE, FieldSolver, Newton, Planar2DGpu, SolveStatus, Wrench, demagnetize, forces};
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Clone, Copy, PartialEq)]
enum Tool {
    Select,
    Rect,
    Circle,
    Ellipse,
    Ring,
    Polygon,
    Bezier,
    Magnet,
    Coil,
    Probe,
    Cut,
    Seed,
    Sprinkle,
    Brush,
    Heat,
    Cold,
}

/// Outil, nom court, raccourci, infobulle, icône (chemin SVG sur une grille de 18).
const TOOLS: [(Tool, &str, Key, &str, &str); 16] = [
    (Tool::Select, "Sélection", Key::V, "Sélectionner et déplacer (V)", "M4 2.5l10 5.5-4.5 1.3L7.5 14 4 2.5Z"),
    (Tool::Rect, "Rect.", Key::R, "Dessiner un bloc du matériau courant (R)", "M3 4.5h12v9H3z"),
    (Tool::Circle, "Disque", Key::E, "Dessiner un disque du matériau courant (E)", "M9 3a6 6 0 1 0 0 12A6 6 0 0 0 9 3Z"),
    (Tool::Ellipse, "Ellipse", Key::O, "Dessiner une ellipse du matériau courant (O)", "M9 5a7 4 0 1 0 0 8A7 4 0 0 0 9 5Z"),
    (
        Tool::Ring,
        "Anneau",
        Key::A,
        "Dessiner un anneau du matériau courant (A)",
        "M9 3a6 6 0 1 0 0 12A6 6 0 0 0 9 3ZM9 6.5a2.5 2.5 0 1 0 0 5A2.5 2.5 0 0 0 9 6.5Z",
    ),
    (
        Tool::Polygon,
        "Polygone",
        Key::P,
        "Polygone (P) — clic : sommet · double-clic ou Entrée : fermer · Retour arrière : annuler un sommet",
        "M9 2.5 15.5 7.5 13 15H5L2.5 7.5Z",
    ),
    (
        Tool::Bezier,
        "Courbe",
        Key::B,
        "Courbe de Bézier (B) — clic : sommet · glisser : tangente · double-clic ou Entrée : fermer",
        "M3 14C3 6 15 12 15 4M2 13h2v2H2zM14 3h2v2h-2z",
    ),
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
    (
        Tool::Brush,
        "Pinceau",
        Key::N,
        "Peindre l’aimantation d’un aimant : elle suit le geste (N)",
        "M10 9.5 15 3.5M8.5 9.5l2.5 2c-.5 2-2.5 3.5-7 3.5 1.5-1 1.8-2.2 2.2-3.5.4-1.3 1.2-2 2.3-2Z",
    ),
    (
        Tool::Heat,
        "Chauffer",
        Key::T,
        "Pistolet chauffant : maintenir sur un objet (T)",
        "M9 2.5c.5 2.5 3.5 4 3.5 7.5a3.5 3.5 0 0 1-7 0c0-1.5.7-2.5 1.5-3.2.2 1.2.8 1.7 1.3 1.7C8.6 6.5 8.2 4.5 9 2.5Z",
    ),
    (
        Tool::Cold,
        "Refroidir",
        Key::Y,
        "Bombe de froid : maintenir sur un objet (Y)",
        "M9 2.5v13M3.4 5.8l11.2 6.4M14.6 5.8 3.4 12.2M7.5 3.5 9 5l1.5-1.5M7.5 14.5 9 13l1.5 1.5",
    ),
];

enum Drag {
    Move {
        id: u32,
        offset: DVec2,
    },
    Rotate {
        id: u32,
    },
    Magnetize {
        id: u32,
    },
    Create(DVec2),
    Cut(DVec2),
    /// Tracé d'un polygone ou d'une courbe : les clics posent des sommets.
    Draft,
    Pan,
    Sprinkle,
    /// Pinceau d'aimantation sur un aimant.
    Paint {
        id: u32,
    },
    /// Pistolet chauffant ou bombe de froid tenus sur le canevas.
    Blow,
}

/// Élément du canevas sous le curseur qui réagit au clic ou au glisser.
#[derive(Clone, Copy, PartialEq)]
enum Hot {
    Rotate,
    Magnetize,
    /// Symbole ⊙/⊗ d'un conducteur : un clic inverse le courant.
    Current(u32),
}

#[derive(Clone, Copy, PartialEq)]
enum Compare {
    Off,
    Split,
    Diff,
}

#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
enum Side {
    Left,
    Right,
    Top,
    Bottom,
}

impl Side {
    fn opposite(self) -> Side {
        match self {
            Side::Left => Side::Right,
            Side::Right => Side::Left,
            Side::Top => Side::Bottom,
            Side::Bottom => Side::Top,
        }
    }
}

/// Panneaux repliables et ancrables.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Panel {
    Library,
    Inspector,
    Graph,
}

/// Ancrage d'un panneau : déplié ou replié, et bord de la fenêtre où il se trouve.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
struct Dock {
    open: bool,
    side: Side,
}

/// Préférences conservées d'un lancement à l'autre.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Prefs {
    lang: Lang,
    units: Units,
    library: Dock,
    inspector: Dock,
    graph: Dock,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            lang: Lang::Fr,
            units: Units::Si,
            library: Dock { open: true, side: Side::Left },
            inspector: Dock { open: true, side: Side::Right },
            graph: Dock { open: true, side: Side::Bottom },
        }
    }
}

impl Prefs {
    fn dock(&mut self, panel: Panel) -> &mut Dock {
        match panel {
            Panel::Library => &mut self.library,
            Panel::Inspector => &mut self.inspector,
            Panel::Graph => &mut self.graph,
        }
    }
}

/// Largeur d'un panneau latéral replié.
const COLLAPSED: f32 = 28.0;

/// Scènes d'exemple livrées avec l'application.
const EXAMPLES: [&str; 7] = [
    "Aimant + plaque de fer",
    "Supraconducteur et diamagnétique",
    "Plaque attirée sur une table",
    "Tôle saturée",
    "Réseau de Halbach",
    "Aimant surchauffé",
    "Lévitation du graphite",
];

/// Scène vide, nommée dans la langue courante.
fn blank() -> Scene {
    let mut scene = Scene::default();
    scene.name = tr("Scène sans titre").to_owned();
    scene
}

/// Scène d'exemple, son nom et ceux de ses objets dans la langue courante.
fn example(index: usize) -> Scene {
    let mut scene = match index {
        0 => Scene::demo(),
        1 => Scene::meissner_demo(),
        2 => Scene::friction_demo(),
        3 => Scene::saturation_demo(),
        4 => Scene::halbach_demo(),
        5 => Scene::overheated_demo(),
        _ => Scene::levitation_demo(),
    };
    scene.name = tr(&scene.name).to_owned();
    scene.objects.iter_mut().for_each(|o| o.name = tr_name(&o.name));
    scene
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
    eye: Icon,
    eye_off: Icon,
    lock: Icon,
    unlock: Icon,
    /// Chevrons vers la gauche, la droite, le haut et le bas.
    chevrons: [Icon; 4],
    swap: Icon,
    play: Icon,
    pause: Icon,
    step: Icon,
    rewind: Icon,
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
            eye: Icon::parse(
                14.0,
                "M1.5 7C3 4.3 5 3 7 3s4 1.3 5.5 4C11 9.7 9 11 7 11S3 9.7 1.5 7ZM5.2 7a1.8 1.8 0 1 0 3.6 0a1.8 1.8 0 1 0-3.6 0",
            ),
            eye_off: Icon::parse(14.0, "M1.5 7C3 4.3 5 3 7 3s4 1.3 5.5 4C11 9.7 9 11 7 11S3 9.7 1.5 7ZM2.5 2.5l9 9"),
            lock: Icon::parse(14.0, "M3.5 6.5h7v5h-7zM5 6.5V4.5a2 2 0 0 1 4 0v2"),
            unlock: Icon::parse(14.0, "M3.5 6.5h7v5h-7zM5 6.5V4.5a2 2 0 0 1 3.7-1"),
            chevrons: ["M8.5 3.5 5 7l3.5 3.5", "M5.5 3.5 9 7l-3.5 3.5", "M3.5 8.5 7 5l3.5 3.5", "M3.5 5.5 7 9l3.5-3.5"]
                .map(|d| Icon::parse(14.0, d)),
            swap: Icon::parse(14.0, "M2 4.5h9M8.5 2 11 4.5 8.5 7M12 9.5H3M5.5 7 3 9.5 5.5 12"),
            play: Icon::parse(16.0, "M5 3v10l8-5z"),
            pause: Icon::parse(16.0, "M5.5 3v10M10.5 3v10"),
            step: Icon::parse(16.0, "M4 3v10l7-5zM12.5 3v10"),
            rewind: Icon::parse(16.0, "M12 3v10L5 8zM3.5 3v10"),
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
    /// Résolution en cours : scène rastérisée et itérations de Newton des matériaux saturables.
    newton: Option<Newton>,
    sim: sim::Sim,
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
    /// Rectangle du canevas à la dernière image, pour le cadrage.
    plot: Rect,
    tool: Tool,
    drag: Option<Drag>,
    selected: Option<u32>,
    /// Second objet d'une opération booléenne : (objet sélectionné, opérande).
    operand: Option<(u32, u32)>,
    /// Chemin en cours de tracé (outils Polygone et Courbe), en repère monde.
    draft: Vec<PathNode>,
    hot: Option<Hot>,
    /// Déplacement lissé du pinceau d'aimantation (m) : il donne la direction peinte.
    stroke: DVec2,
    /// Matériau en cours de glisser-déposer depuis la bibliothèque, et objet survolé.
    drag_material: Option<String>,
    drop_target: Option<u32>,
    /// Donner le focus au champ « Angle » de l'inspecteur (double-clic sur un aimant).
    focus_angle: bool,
    /// Faire défiler l'inspecteur jusqu'aux opérations booléennes.
    reveal_boolean: bool,
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
    prefs: Prefs,
    /// Panneau dont l'en-tête est en train d'être glissé vers un autre bord.
    docking: Option<Panel>,
    palette: Option<palette::Palette>,
    icons: Icons,
}

/// Plus petit pas « rond » (1, 2 ou 5 × 10ⁿ) au moins égal à `min`.
fn nice_step(min: f64) -> f64 {
    let p = 10f64.powf(min.log10().floor());
    [1.0, 2.0, 5.0, 10.0].into_iter().map(|m| m * p).find(|s| *s >= min).unwrap_or(10.0 * p)
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> App {
        let prefs = cc.storage.and_then(|storage| eframe::get_value(storage, "prefs")).unwrap_or_default();
        App::with(&cc.egui_ctx, cc.wgpu_render_state.clone(), prefs)
    }

    /// Sans état de rendu, le calcul et l'affichage du champ se passent du GPU.
    fn with(ctx: &egui::Context, render_state: Option<egui_wgpu::RenderState>, prefs: Prefs) -> App {
        t::apply(ctx);
        lang::set(prefs.lang);
        units::set(prefs.units);
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
            scene: scene.unwrap_or_else(|| example(0)),
            undo: Vec::new(),
            redo: Vec::new(),
            edit_open: false,
            skip_history: false,
            message: String::new(),
            render_state,
            solver,
            newton: None,
            sim: sim::Sim::default(),
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
            plot: Rect::from_min_size(Pos2::ZERO, vec2(813.0, 792.0)),
            tool: Tool::Select,
            drag: None,
            selected: None,
            operand: None,
            draft: Vec::new(),
            hot: None,
            stroke: DVec2::ZERO,
            drag_material: None,
            drop_target: None,
            focus_angle: false,
            reveal_boolean: false,
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
            prefs,
            docking: None,
            palette: None,
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
        self.newton = None;
        self.dirty = true;
    }

    /// Rastérise si la scène a changé, puis itère dans le budget de l'image.
    fn step_solver(&mut self, ctx: &egui::Context) {
        let dragging = self.edit_open
            || matches!(self.drag, Some(Drag::Move { .. } | Drag::Rotate { .. } | Drag::Magnetize { .. } | Drag::Paint { .. }));
        // Résolution progressive : le solveur CPU calcule en 256² pendant le geste.
        let n = if dragging && !self.use_gpu { self.grid_n.min(256) } else { self.grid_n };
        if self.dirty || n != self.solved_n || self.newton.is_none() {
            self.newton = Some(Newton::start(self.solver.as_mut(), rasterize(&self.scene, n)));
            (self.solved_n, self.dirty, self.pending) = (n, false, true);
        }
        // Pendant la lecture, la mécanique a besoin des forces à chaque image.
        let playing = self.sim.playing;
        if self.pending
            && let Some(newton) = &mut self.newton
        {
            let budget = if self.use_gpu { 6 } else { 12 } * if playing { 2 } else { 1 };
            self.status = newton.advance(self.solver.as_mut(), Duration::from_millis(budget));
            self.stats_stale = true;
            self.field_version += 1;
            let field = self.solver.field();
            if let Some(rs) = &self.render_state
                && let Some(r) = rs.renderer.write().callback_resources.get_mut::<FieldRenderer>()
            {
                r.upload(&rs.device, &rs.queue, field);
            }
            if self.status.converged || playing {
                self.wrenches = forces(field, &self.scene);
            }
            if self.status.converged {
                self.pending = false;
                // Désaimantation irréversible : une fois le champ convergé, jamais pendant les
                // itérations. Si un aimant vient de perdre de sa rémanence, le champ est à refaire.
                if demagnetize(field, &mut self.scene) >= DEMAG_TOLERANCE {
                    self.dirty = true;
                    ctx.request_repaint();
                }
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
        self.sim.reset();
        self.skip_history = true;
        if let Some(s) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.scene, s));
        }
    }

    fn do_redo(&mut self) {
        self.sim.reset();
        self.skip_history = true;
        if let Some(s) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.scene, s));
        }
    }

    fn load(&mut self, scene: Scene, path: Option<std::path::PathBuf>) {
        (self.scene, self.path, self.selected) = (scene, path, None);
        self.sim.reset();
        self.draft.clear();
        self.undo.clear();
        self.redo.clear();
        self.visuals.filings.clear();
        self.skip_history = true;
        self.frame_all();
    }

    fn open(&mut self) {
        let Some(path) = rfd::FileDialog::new().add_filter(tr("Scène Flux2D"), &["flux"]).pick_file() else { return };
        match std::fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|s| Scene::from_ron(&s)) {
            Ok(scene) => self.load(scene, Some(path)),
            Err(e) => self.message = format!("{} {e}", tr("Ouverture impossible :")),
        }
    }

    fn save(&mut self, ask: bool) {
        if ask || self.path.is_none() {
            let name = format!("{}.flux", self.scene.name);
            let Some(p) = rfd::FileDialog::new().add_filter(tr("Scène Flux2D"), &["flux"]).set_file_name(name).save_file() else {
                return;
            };
            self.path = Some(p);
        }
        let path = self.path.clone().unwrap();
        let result = self.scene.to_ron().map_err(|e| e.to_string()).and_then(|s| std::fs::write(&path, s).map_err(|e| e.to_string()));
        self.message = match result {
            Ok(()) => format!("{} {}", tr("Enregistré :"), path.display()),
            Err(e) => format!("{} {e}", tr("Enregistrement impossible :")),
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
            self.message = format!("{} {e}", tr("Export impossible :"));
        }
    }

    /// Écrit un script Lua qui reproduit la scène dans FEMM, le logiciel de référence.
    fn export_femm(&mut self) {
        let name = format!("{}.lua", self.scene.name);
        let Some(path) = rfd::FileDialog::new().add_filter("FEMM Lua", &["lua"]).set_file_name(name).save_file() else { return };
        let results = path.with_extension("txt");
        let lua = flux_core::femm::lua_script(&self.scene, &results.display().to_string(), false);
        self.message = match std::fs::write(&path, lua) {
            Ok(()) => format!("{} {}", tr("Script FEMM écrit :"), path.display()),
            Err(e) => format!("{} {e}", tr("Export impossible :")),
        };
    }

    /// Remplace la courbe B(H) d'un matériau de la scène par une table lue dans un fichier.
    fn import_bh(&mut self, material: &str) {
        let Some(path) = rfd::FileDialog::new().add_filter(tr("Table B(H)"), &["csv", "txt"]).pick_file() else { return };
        let curve = std::fs::read_to_string(&path).ok().and_then(|text| flux_core::material::BhCurve::from_text(&text));
        match (curve, self.scene.materials.iter_mut().find(|m| m.name == material)) {
            (Some(curve), Some(m)) => {
                self.message = format!("{} {}", curve.points.len(), tr("points B(H) importés."));
                m.bh = Some(curve);
            }
            _ => self.message = tr("Table B(H) illisible : il faut au moins deux lignes « H ; B » croissantes.").into(),
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
        let fit = (hi - lo) * 2.2 / DVec2::new(self.plot.width() as f64, self.plot.height() as f64);
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
            Tool::Rect => self.scene.add(tr("Bloc"), rect(0.02, 0.02), c, &lib),
            Tool::Circle => {
                let r = if tiny { 0.01 } else { (b - a).length().max(1e-3) };
                self.scene.add(tr("Disque"), Shape::Circle { r }, a, &lib)
            }
            Tool::Ellipse => {
                let (rx, ry) = if tiny { (0.015, 0.009) } else { (d.x.max(1e-3) / 2.0, d.y.max(1e-3) / 2.0) };
                self.scene.add(tr("Ellipse"), Shape::Ellipse { rx, ry }, c, &lib)
            }
            Tool::Ring => {
                let r_out = if tiny { 0.012 } else { (b - a).length().max(1e-3) };
                self.scene.add(tr("Anneau"), Shape::Ring { r_in: 0.6 * r_out, r_out }, a, &lib)
            }
            Tool::Magnet => {
                let mat = if class == Some(MagClass::Magnet) { lib.as_str() } else { "NdFeB N42" };
                self.scene.add(tr("Aimant"), rect(0.01, 0.03), c, mat)
            }
            Tool::Coil if tiny => {
                let id = self.scene.add(tr("Fil"), Shape::Circle { r: 0.002 }, a, "Cuivre (bobinage)");
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
                    id = self.scene.add(tr("Bobine"), Shape::Rect { w: cw, h: d.y.max(1e-3) }, pos, "Cuivre (bobinage)");
                    let o = self.scene.get_mut(id).unwrap();
                    // Bobinage : le cuivre occupe environ 60 % de la section.
                    (o.turns, o.current, o.fill) = (100.0, sign, 0.6);
                }
                id
            }
            _ => return,
        };
        self.selected = Some(id);
        self.tool = Tool::Select;
    }

    /// Ferme le chemin en cours de tracé et en fait un objet du matériau courant.
    fn finish_draft(&mut self) {
        // Les sommets confondus (double-clic de fermeture) sont fusionnés.
        let min = 3.0 * self.view_scale;
        let mut nodes = std::mem::take(&mut self.draft);
        nodes.dedup_by(|b, a| (b.p - a.p).length() < min);
        if nodes.len() > 1 && (nodes[0].p - nodes[nodes.len() - 1].p).length() < min {
            nodes.pop();
        }
        let name = tr(if self.tool == Tool::Bezier { "Forme" } else { "Polygone" });
        let lib = self.lib_material.clone();
        match self.scene.add_contours(name, vec![flatten_path(&nodes, true)], &lib) {
            Some(id) => (self.selected, self.tool) = (Some(id), Tool::Select),
            None => self.message = tr("Tracé trop petit : il faut au moins trois sommets non alignés.").into(),
        }
    }

    /// Supprime l'objet sélectionné, sauf s'il est verrouillé.
    fn delete_selected(&mut self) {
        let Some(id) = self.selected else { return };
        if self.scene.get(id).is_some_and(|o| o.locked) {
            self.message = tr("Objet verrouillé : suppression ignorée.").into();
        } else {
            self.scene.objects.retain(|o| o.id != id);
            self.selected = None;
        }
    }

    fn shortcuts(&mut self, ui: &egui::Ui) {
        // La palette de commandes s'ouvre et se ferme même pendant une saisie.
        if ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::K)) {
            self.palette = if self.palette.is_some() { None } else { Some(palette::Palette::default()) };
        }
        if self.palette.is_some() || ui.ctx().egui_wants_keyboard_input() {
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
        let (play, step) =
            ui.input(|i| (!i.modifiers.any() && i.key_pressed(Key::Space), !i.modifiers.any() && i.key_pressed(Key::Period)));
        if play {
            self.toggle_play();
        }
        if step {
            self.step_simulation();
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
            if i.key_pressed(Key::Enter) && self.draft.len() >= 3 {
                self.finish_draft();
            }
            if i.key_pressed(Key::Delete) || i.key_pressed(Key::Backspace) {
                // Pendant un tracé, la touche retire le dernier sommet au lieu de supprimer l'objet.
                if self.draft.pop().is_none() {
                    self.delete_selected();
                }
            }
        });
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.frame(ui);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, "prefs", &self.prefs);
    }
}

impl App {
    /// Une image : calcul, panneaux, canevas, puis historique.
    fn frame(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        lang::set(self.prefs.lang);
        units::set(self.prefs.units);
        self.step_solver(&ctx);
        // La mécanique déplace les objets avant la photo de la scène : ses pas n'entrent pas
        // un à un dans l'historique.
        self.simulate(&ctx);
        let before = self.scene.clone();
        self.shortcuts(ui);
        let bare = |fill| egui::Frame::new().fill(fill);
        if self.show_ui {
            let panel = |p: egui::Panel, size: f32| p.exact_size(size).resizable(false).show_separator_line(false).frame(bare(t::PANEL));
            panel(egui::Panel::top("header"), 40.0).show(ui, |ui| self.header(ui));
            panel(egui::Panel::bottom("footer"), 28.0).show(ui, |ui| self.footer(ui));
            panel(egui::Panel::left("rail"), 56.0).show(ui, |ui| self.rail(ui));
            // Chaque panneau latéral s'ancre à gauche ou à droite, et se replie en une bande étroite.
            for which in [Panel::Library, Panel::Inspector] {
                let dock = *self.prefs.dock(which);
                let left = dock.side == Side::Left;
                let id = egui::Id::new(("dock", which as u8, left));
                let width = match (dock.open, which) {
                    (false, _) => COLLAPSED,
                    (true, Panel::Library) => 248.0,
                    (true, _) => 300.0,
                };
                panel(if left { egui::Panel::left(id) } else { egui::Panel::right(id) }, width).show(ui, |ui| match (dock.open, which) {
                    (false, _) => self.collapsed(ui, which),
                    (true, Panel::Library) => self.library(ui),
                    (true, _) => self.inspector(ui),
                });
            }
            if self.scene.cut_line.is_some() {
                let dock = self.prefs.graph;
                let slot = if dock.side == Side::Top { egui::Panel::top("graph-top") } else { egui::Panel::bottom("graph") };
                panel(slot, if dock.open { 150.0 } else { 32.0 }).show(ui, |ui| self.graph(ui));
            }
        }
        egui::CentralPanel::default().frame(bare(t::BG)).show(ui, |ui| self.canvas(ui));
        if self.show_ui {
            self.dock_drop(&ctx);
        }
        self.show_palette(&ctx);
        self.scene.clamp_temperatures();
        if !ctx.input(|i| i.pointer.any_down()) {
            self.drag_material = None;
        }
        // L'opérande d'une opération booléenne ne vaut que pour l'objet sélectionné.
        self.operand = self.operand.filter(|&(a, b)| self.selected == Some(a) && self.scene.get(b).is_some());

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

    /// Glisser l'en-tête d'un panneau : le bord visé s'illumine, relâcher y ancre le panneau.
    fn dock_drop(&mut self, ctx: &egui::Context) {
        let Some(which) = self.docking else { return };
        let (screen, at) = (ctx.content_rect(), ctx.pointer_latest_pos());
        let Some(at) = at else { return self.docking = None };
        // Zone utile : sous la barre supérieure, au-dessus de la barre d'état, à droite du rail d'outils.
        let area = Rect::from_min_max(screen.min + vec2(56.0, 40.0), screen.max - vec2(0.0, 28.0));
        let (side, zone) = match which {
            Panel::Graph if at.y < area.center().y => (Side::Top, area.with_max_y(area.top() + 150.0)),
            Panel::Graph => (Side::Bottom, area.with_min_y(area.bottom() - 150.0)),
            _ => {
                let width = if which == Panel::Library { 248.0 } else { 300.0 };
                if at.x < area.center().x {
                    (Side::Left, area.with_max_x(area.left() + width))
                } else {
                    (Side::Right, area.with_min_x(area.right() - width))
                }
            }
        };
        if ctx.input(|i| i.pointer.any_down()) {
            ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
            let p = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("docking")));
            p.rect_filled(zone, 0.0, t::tint());
            p.rect_stroke(zone, 0.0, egui::Stroke::new(1.0, t::ACCENT), egui::StrokeKind::Inside);
            ui::tag(&p, zone.left_top() + vec2(12.0, 30.0), tr("ANCRER ICI"), t::ACCENT, t::TEXT_HI);
        } else {
            self.prefs.dock(which).side = side;
            self.docking = None;
        }
    }
}
