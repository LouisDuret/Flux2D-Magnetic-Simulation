//! Tests de bout en bout des gestes du canevas : l'interface tourne sans fenêtre ni GPU
//! et reçoit des événements de souris et de clavier simulés.

use super::*;
use eframe::egui::{Event, PointerButton, RawInput, pos2};
use flux_core::scene::{Body, Link, MechView};
use flux_core::shape::{BoolOp, Shape};
use std::f64::consts::FRAC_PI_2;

struct Rig {
    ctx: egui::Context,
    app: App,
    time: f64,
    pos: Pos2,
    modifiers: Modifiers,
}

impl Rig {
    fn new(scene: Scene) -> Rig {
        let ctx = egui::Context::default();
        let mut app = App::with(&ctx, None, Prefs::default());
        app.load(scene, None);
        let mut rig = Rig { ctx, app, time: 0.0, pos: Pos2::ZERO, modifiers: Modifiers::NONE };
        // Deux images pour connaître la taille du canevas, puis cadrage de la scène.
        rig.idle(2);
        rig.app.frame_all();
        rig.idle(2);
        rig
    }

    fn frame(&mut self, mut events: Vec<Event>) {
        self.time += 1.0 / 60.0;
        events.insert(0, Event::ModifiersChanged(self.modifiers));
        let screen_rect = Some(Rect::from_min_size(Pos2::ZERO, vec2(1440.0, 880.0)));
        let input = RawInput { screen_rect, time: Some(self.time), events, ..Default::default() };
        let _ = self.ctx.run_ui(input, |ui| self.app.frame(ui));
    }

    fn idle(&mut self, frames: usize) {
        (0..frames).for_each(|_| self.frame(Vec::new()));
    }

    fn move_to(&mut self, p: Pos2) {
        self.pos = p;
        self.frame(vec![Event::PointerMoved(p)]);
    }

    fn button(&mut self, pressed: bool) {
        self.frame(vec![Event::PointerButton { pos: self.pos, button: PointerButton::Primary, pressed, modifiers: self.modifiers }]);
    }

    fn click(&mut self, p: Pos2) {
        self.move_to(p);
        self.button(true);
        self.button(false);
    }

    /// Appuie en `from`, glisse jusqu'à `to` en plusieurs images, sans relâcher.
    fn drag_to(&mut self, from: Pos2, to: Pos2) {
        self.move_to(from);
        self.button(true);
        (1..=8).for_each(|k| self.move_to(from + (to - from) * (k as f32 / 8.0)));
    }

    fn drag(&mut self, from: Pos2, to: Pos2) {
        self.drag_to(from, to);
        self.button(false);
        self.idle(1);
    }

    fn key(&mut self, key: Key) {
        self.frame(vec![Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: self.modifiers }]);
    }

    /// Position à l'écran d'un point du monde donné en millimètres.
    fn at(&self, x_mm: f64, y_mm: f64) -> Pos2 {
        let view = View { center: self.app.view_center, scale: self.app.view_scale, rect: self.app.plot };
        view.to_screen(DVec2::new(x_mm, y_mm) * 1e-3)
    }

    fn px(&self, mm: f64) -> f32 {
        (mm * 1e-3 / self.app.view_scale) as f32
    }
}

/// Scène de démonstration : aimant 20 × 40 mm en (−25, 0), plaque de fer 16 × 60 mm en (20, 0).
fn demo() -> Rig {
    Rig::new(Scene::demo())
}

#[test]
fn rotation_handle_rotates_and_snaps() {
    let mut rig = demo();
    rig.click(rig.at(-25.0, 12.0));
    assert_eq!(rig.app.selected, Some(1));

    // La poignée est à 30 points au-dessus du bord supérieur de l'aimant.
    let handle = rig.at(-25.0, 20.0) - vec2(0.0, 30.0);
    rig.drag(handle, rig.at(25.0, 0.0));
    let angle = rig.app.scene.get(1).unwrap().angle;
    assert!((angle + FRAC_PI_2).abs() < 0.01, "angle = {angle}");
    assert_eq!(rig.app.undo.len(), 1, "un geste continu ne crée qu'une entrée d'historique");

    // Tourné de −90°, l'aimant a son « haut » vers la droite. Avec Maj, l'angle suit des pas de 15°.
    let handle = rig.at(-25.0 + 20.0, 0.0) + vec2(30.0, 0.0);
    rig.modifiers = Modifiers::SHIFT;
    rig.drag(handle, rig.at(-25.0 + 40.0, 10.0));
    let degrees = rig.app.scene.get(1).unwrap().angle.to_degrees();
    assert!((degrees + 75.0).abs() < 1e-9, "angle = {degrees}°");
}

#[test]
fn magnetization_handle_turns_the_arrow() {
    let mut rig = demo();
    rig.click(rig.at(-25.0, 12.0));
    // La poignée est 7 points au-delà de la pointe de la flèche, longue de 42 % de la demi-largeur.
    let len = (rig.px(10.0) * 0.42).clamp(6.0, 46.0);
    let tip = rig.at(-25.0, 0.0) + vec2(len + 7.0, 0.0);
    rig.drag(tip, rig.at(-25.0, 30.0));
    let o = rig.app.scene.get(1).unwrap();
    assert!((o.mag_dir() - DVec2::Y).length() < 0.01, "aimantation = {:?}", o.mag_dir());
    assert_eq!(o.angle, 0.0, "l'objet lui-même ne tourne pas");
    assert_eq!(o.pos.truncate(), DVec2::new(-0.025, 0.0), "ni ne se déplace");
}

#[test]
fn click_on_symbol_flips_current() {
    let mut scene = Scene::default();
    let wire = scene.add("Fil", Shape::Circle { r: 0.005 }, DVec2::ZERO, "Cuivre (bobinage)");
    let o = scene.get_mut(wire).unwrap();
    (o.turns, o.current) = (1.0, 10.0);
    let mut rig = Rig::new(scene);
    assert!(rig.px(5.0) > 40.0);

    // Hors du symbole, le clic ne fait que sélectionner.
    rig.click(rig.at(0.0, 0.0) + vec2(25.0, 0.0));
    assert_eq!((rig.app.selected, rig.app.scene.get(wire).unwrap().current), (Some(wire), 10.0));
    rig.idle(30);
    rig.click(rig.at(0.0, 0.0) + vec2(3.0, -2.0));
    assert_eq!(rig.app.scene.get(wire).unwrap().current, -10.0);
    // Glisser le fil par son symbole le déplace sans inverser le courant.
    rig.idle(30);
    rig.drag(rig.at(0.0, 0.0), rig.at(2.0, 1.0));
    let o = rig.app.scene.get(wire).unwrap();
    assert_eq!(o.current, -10.0);
    assert!((o.pos.truncate() - DVec2::new(0.002, 0.001)).length() < 1e-4);
}

#[test]
fn material_dragged_from_library_onto_object() {
    let mut rig = demo();
    rig.app.search = "Bismuth".into();
    rig.idle(2);
    // Une seule ligne reste dans la bibliothèque : on la cherche en descendant la colonne.
    let mut grabbed = false;
    for y in (150..400).step_by(6) {
        rig.drag_to(pos2(150.0, y as f32), pos2(190.0, y as f32 + 4.0));
        if rig.app.drag_material.is_some() {
            grabbed = true;
            break;
        }
        rig.button(false);
    }
    assert!(grabbed, "aucune ligne de matériau n'a pu être saisie");
    assert_eq!(rig.app.drag_material.as_deref(), Some("Bismuth"));

    let plate = rig.at(20.0, 10.0);
    (1..=8).for_each(|k| rig.move_to(pos2(190.0, 200.0) + (plate - pos2(190.0, 200.0)) * (k as f32 / 8.0)));
    assert_eq!(rig.app.drop_target, Some(2), "la plaque survolée doit montrer l'aperçu");
    assert_eq!(rig.app.scene.get(2).unwrap().material, "Fer pur (Armco)", "rien n'est appliqué avant de relâcher");
    rig.button(false);
    rig.idle(1);
    assert_eq!(rig.app.scene.get(2).unwrap().material, "Bismuth");
    assert_eq!((rig.app.selected, rig.app.drag_material.as_deref()), (Some(2), None));
    assert_eq!(rig.app.undo.len(), 1);

    // Relâché dans le vide, un matériau glissé ne change rien.
    rig.drag_to(rig.at(0.0, 70.0), rig.at(0.0, 60.0));
    rig.app.drag_material = Some("Aluminium".into());
    rig.idle(1);
    rig.button(false);
    rig.idle(1);
    assert_eq!(rig.app.scene.objects.iter().filter(|o| o.material == "Aluminium").count(), 0);
}

#[test]
fn polygon_tool_closes_with_enter_or_first_vertex() {
    let mut rig = demo();
    rig.key(Key::P);
    assert!(rig.app.tool == Tool::Polygon);
    for (x, y) in [(-10.0, 40.0), (10.0, 40.0), (10.0, 50.0), (-10.0, 50.0)] {
        rig.click(rig.at(x, y));
    }
    assert_eq!(rig.app.draft.len(), 4);
    // Retour arrière retire le dernier sommet sans toucher aux objets.
    rig.key(Key::Backspace);
    assert_eq!((rig.app.draft.len(), rig.app.scene.objects.len()), (3, 2));
    rig.click(rig.at(-10.0, 50.0));
    rig.key(Key::Enter);
    let o = rig.app.scene.objects.last().unwrap();
    assert!(matches!(&o.shape, Shape::Polygon { pts } if pts.len() == 4), "{:?}", o.shape);
    assert!((o.shape.area() / 2e-4 - 1.0).abs() < 0.02, "aire = {}", o.shape.area());
    assert!((o.pos.truncate() - DVec2::new(0.0, 0.045)).length() < 3e-4);
    assert_eq!(o.material, "Acier doux (S235)");
    assert!(rig.app.tool == Tool::Select && rig.app.selected == Some(o.id) && rig.app.draft.is_empty());

    // Un clic sur le premier sommet ferme aussi le tracé.
    rig.key(Key::P);
    for (x, y) in [(-10.0, -40.0), (10.0, -40.0), (0.0, -55.0), (-10.0, -40.0)] {
        rig.click(rig.at(x, y));
    }
    assert_eq!(rig.app.scene.objects.len(), 4);
    let o = rig.app.scene.objects.last().unwrap();
    assert!(matches!(&o.shape, Shape::Polygon { pts } if pts.len() == 3), "{:?}", o.shape);
    assert!((o.shape.area() / 1.5e-4 - 1.0).abs() < 0.02);

    // Échap abandonne le tracé en cours.
    rig.key(Key::P);
    rig.click(rig.at(40.0, 40.0));
    rig.key(Key::Escape);
    rig.idle(1);
    assert!(rig.app.draft.is_empty() && rig.app.scene.objects.len() == 4);
}

#[test]
fn bezier_tool_pulls_tangents() {
    let mut rig = demo();
    rig.key(Key::B);
    // Quatre sommets en losange, chacun avec une tangente tirée dans le sens du parcours.
    let r = 12.0;
    for (x, y, tx, ty) in [(r, 0.0, 0.0, 1.0), (0.0, r, -1.0, 0.0), (-r, 0.0, 0.0, -1.0), (0.0, -r, 1.0, 0.0)] {
        let k = 0.5523 * r;
        rig.drag(rig.at(x, 45.0 + y), rig.at(x + k * tx, 45.0 + y + k * ty));
    }
    assert_eq!(rig.app.draft.len(), 4);
    assert!(rig.app.draft.iter().all(|node| node.handle.length() > 5e-3));
    rig.key(Key::Enter);
    let o = rig.app.scene.objects.last().unwrap();
    // Les tangentes font du losange un disque de 12 mm de rayon.
    let disc = std::f64::consts::PI * 0.012 * 0.012;
    assert!(matches!(&o.shape, Shape::Polygon { pts } if pts.len() == 64), "{:?}", o.shape);
    assert!((o.shape.area() / disc - 1.0).abs() < 0.03, "aire = {} au lieu de {disc}", o.shape.area());
}

#[test]
fn ellipse_and_ring_tools() {
    let mut rig = demo();
    rig.key(Key::O);
    rig.drag(rig.at(-20.0, 40.0), rig.at(20.0, 60.0));
    let o = rig.app.scene.objects.last().unwrap();
    let Shape::Ellipse { rx, ry } = o.shape else { panic!("{:?}", o.shape) };
    assert!((rx - 0.020).abs() < 3e-4 && (ry - 0.010).abs() < 3e-4, "{rx} × {ry}");
    assert!((o.pos.truncate() - DVec2::new(0.0, 0.050)).length() < 3e-4);

    rig.key(Key::A);
    rig.drag(rig.at(0.0, -50.0), rig.at(10.0, -50.0));
    let o = rig.app.scene.objects.last().unwrap();
    let Shape::Ring { r_in, r_out } = o.shape else { panic!("{:?}", o.shape) };
    assert!((r_out - 0.010).abs() < 3e-4 && (r_in - 0.006).abs() < 3e-4, "{r_in} – {r_out}");
    // Le trou de l'anneau ne se sélectionne pas.
    rig.click(rig.at(0.0, -50.0));
    assert_eq!(rig.app.selected, None);
}

#[test]
fn shift_click_picks_boolean_operand() {
    let mut rig = demo();
    rig.click(rig.at(-25.0, 12.0));
    rig.modifiers = Modifiers::SHIFT;
    rig.click(rig.at(20.0, 20.0));
    assert_eq!((rig.app.selected, rig.app.operand), (Some(1), Some((1, 2))));
    rig.idle(2);
    assert!(!rig.app.reveal_boolean, "l'inspecteur doit avoir fait défiler jusqu'à la section");

    // L'opérande est oublié dès que la sélection change.
    rig.modifiers = Modifiers::NONE;
    rig.click(rig.at(20.0, 20.0));
    rig.idle(1);
    assert_eq!((rig.app.selected, rig.app.operand), (Some(2), None));

    // Une union de deux objets disjoints donne deux morceaux du matériau du premier.
    assert_eq!(rig.app.scene.boolean(1, 2, BoolOp::Union), Some(1));
    rig.idle(3);
    assert_eq!(rig.app.scene.objects.len(), 2);
    assert!(rig.app.scene.objects.iter().all(|o| o.material == "NdFeB N42"));
}

#[test]
fn double_click_on_magnet_focuses_angle_field() {
    let mut rig = demo();
    rig.click(rig.at(-25.0, 12.0));
    rig.click(rig.at(-25.0, 12.0));
    rig.idle(2);
    assert!(rig.ctx.egui_wants_keyboard_input(), "le champ Angle de l'inspecteur doit avoir le focus");
    // Sur la plaque de fer, le double-clic ne fait que sélectionner.
    let mut rig = demo();
    rig.click(rig.at(20.0, 20.0));
    rig.click(rig.at(20.0, 20.0));
    rig.idle(2);
    assert!(!rig.ctx.egui_wants_keyboard_input());
}

impl Rig {
    fn with_prefs(scene: Scene, prefs: Prefs) -> Rig {
        let mut rig = Rig::new(scene);
        rig.app.prefs = prefs;
        rig.idle(2);
        rig.app.frame_all();
        rig.idle(2);
        rig
    }

    fn text(&mut self, text: &str) {
        self.frame(vec![Event::Text(text.to_owned())]);
    }

    /// Ouvre la palette de commandes avec Ctrl K.
    fn open_palette(&mut self) {
        self.modifiers = Modifiers::COMMAND | Modifiers::CTRL;
        self.key(Key::K);
        self.modifiers = Modifiers::NONE;
        self.idle(2);
    }
}

#[test]
fn command_palette_filters_and_runs() {
    let mut rig = demo();
    rig.open_palette();
    assert!(rig.app.palette.is_some());
    rig.text("ellip");
    rig.idle(1);
    rig.key(Key::Enter);
    assert!(rig.app.tool == Tool::Ellipse && rig.app.palette.is_none());

    // La recherche ignore les accents et accepte plusieurs mots ; les flèches changent de ligne.
    rig.open_palette();
    rig.text("deplier");
    rig.idle(1);
    rig.key(Key::ArrowDown);
    rig.key(Key::Enter);
    assert!(rig.app.prefs.library.open && !rig.app.prefs.inspector.open, "la deuxième commande replie l'inspecteur");

    rig.open_palette();
    rig.key(Key::Escape);
    assert!(rig.app.palette.is_none());
    // Pendant que la palette est ouverte, les raccourcis d'outils sont suspendus.
    rig.open_palette();
    rig.text("p");
    rig.idle(1);
    assert!(rig.app.tool == Tool::Ellipse);
}

#[test]
fn hidden_objects_leave_the_scene_and_locked_ones_stay_put() {
    let mut rig = demo();
    rig.app.scene.objects[1].visible = false;
    rig.idle(40);
    assert!(!rig.app.pending, "le champ doit avoir été recalculé sans la plaque");
    assert!(rig.app.wrenches.iter().all(|w| w.id != 2));
    rig.click(rig.at(20.0, 20.0));
    assert_eq!(rig.app.selected, None, "un objet masqué ne se sélectionne pas sur le canevas");

    rig.app.scene.objects[0].locked = true;
    rig.drag(rig.at(-25.0, 12.0), rig.at(10.0, 40.0));
    let o = rig.app.scene.get(1).unwrap();
    assert_eq!((rig.app.selected, o.pos.truncate()), (Some(1), DVec2::new(-0.025, 0.0)));
    rig.key(Key::Delete);
    assert!(rig.app.scene.get(1).is_some() && !rig.app.message.is_empty());
    rig.app.scene.objects[0].locked = false;
    rig.key(Key::Delete);
    assert!(rig.app.scene.get(1).is_none());
}

#[test]
fn panels_fold_and_dock() {
    let mut rig = demo();
    let plot = rig.app.plot;
    assert_eq!((plot.left(), plot.right()), (56.0 + 248.0 + 20.0, 1440.0 - 300.0));

    // Bouton de repli, à droite de l'en-tête de la bibliothèque, puis clic sur la bande repliée.
    rig.click(pos2(56.0 + 248.0 - 17.0, 58.0));
    rig.idle(2);
    assert!(!rig.app.prefs.library.open);
    assert_eq!(rig.app.plot.left(), 56.0 + COLLAPSED + 20.0);
    rig.click(pos2(56.0 + 14.0, 300.0));
    rig.idle(2);
    assert!(rig.app.prefs.library.open);

    // Bouton d'ancrage : la bibliothèque passe à droite, à l'extérieur de l'inspecteur.
    rig.click(pos2(56.0 + 248.0 - 43.0, 58.0));
    rig.idle(2);
    assert_eq!(rig.app.prefs.library.side, Side::Right);
    assert_eq!((rig.app.plot.left(), rig.app.plot.right()), (56.0 + 20.0, 1440.0 - 300.0 - 248.0));

    // Glisser son en-tête vers la moitié gauche de la fenêtre la ramène à gauche.
    rig.drag_to(pos2(1440.0 - 248.0 + 60.0, 58.0), pos2(300.0, 400.0));
    assert_eq!(rig.app.docking, Some(Panel::Library));
    rig.button(false);
    rig.idle(2);
    assert_eq!((rig.app.prefs.library.side, rig.app.docking), (Side::Left, None));
    assert_eq!(rig.app.plot.left(), 56.0 + 248.0 + 20.0);
}

#[test]
fn expression_typed_in_a_numeric_field() {
    let mut rig = demo();
    rig.click(rig.at(-25.0, 12.0));
    rig.click(rig.at(-25.0, 12.0));
    rig.idle(2);
    // Le champ « Angle » a le focus : on remplace son contenu par une expression avec unités.
    rig.modifiers = Modifiers::COMMAND | Modifiers::CTRL;
    rig.key(Key::A);
    rig.modifiers = Modifiers::NONE;
    rig.text("30° + 0,5 * 120 deg");
    rig.key(Key::Enter);
    rig.idle(1);
    let degrees = rig.app.scene.get(1).unwrap().mag_angle.to_degrees();
    assert!((degrees - 90.0).abs() < 1e-9, "angle = {degrees}°");
}

/// Parcourt les écrans en anglais : aucun texte de l'interface ne doit manquer dans la table.
#[test]
fn english_interface_is_fully_translated() {
    let mut scene = Scene::demo();
    let wire = scene.add("Fil", Shape::Circle { r: 0.004 }, DVec2::new(0.0, 0.06), "Cuivre (bobinage)");
    let o = scene.get_mut(wire).unwrap();
    (o.turns, o.current) = (10.0, 2.0);
    let bismuth = scene.add("Bloc", Shape::Rect { w: 0.01, h: 0.01 }, DVec2::new(0.0, -0.06), "Bismuth");
    let supra = scene.add("Disque", Shape::Circle { r: 0.006 }, DVec2::new(0.06, 0.06), "YBCO");
    scene.get_mut(supra).unwrap().temperature = -196.0;
    let region = scene.add_contours("Forme", vec![Shape::Rect { w: 0.02, h: 0.01 }.contours().remove(0)], "Aluminium").unwrap();
    scene.cut_line = Some([flux_core::DVec3::new(-0.05, 0.03, 0.0), flux_core::DVec3::new(0.05, 0.03, 0.0)]);
    scene.objects[1].locked = true;

    let mut rig = Rig::with_prefs(scene, Prefs { lang: Lang::En, units: Units::Cgs, ..Prefs::default() });
    rig.idle(40);
    assert_eq!((tr("Aimant"), example(0).name.as_str()), ("Magnet", "Magnet + iron plate"));
    assert_eq!(example(0).objects[0].name, "Magnet 1");
    for id in [1, 2, wire, bismuth, supra, region] {
        rig.app.selected = Some(id);
        rig.app.operand = Some((id, if id == 1 { 2 } else { 1 }));
        rig.move_to(rig.at(0.0, 0.0));
        rig.idle(2);
    }
    // Mécanique : chaque liaison dans l'inspecteur, puis lecture dans les deux vues.
    let spring = Link::Spring { anchor: DVec2::new(0.02, 0.05), stiffness: 50.0, damping: 0.05, length: 0.03 };
    for (k, link) in [Link::Free, Link::Pivot { anchor: DVec2::ZERO }, Link::Slider { angle: 0.3 }, spring].into_iter().enumerate() {
        rig.app.scene.get_mut(2).unwrap().body = Body { mobile: true, mu_s: if k == 0 { 0.0 } else { 0.123 }, mu_k: 0.1, link };
        rig.app.selected = Some(2);
        rig.idle(2);
    }
    rig.key(Key::Space);
    rig.idle(3);
    rig.app.scene.mechanics.view = MechView::Side;
    rig.idle(2);
    rig.key(Key::Space);
    rig.app.selected = None;
    rig.idle(2);
    rig.app.rewind();
    rig.app.scene.get_mut(2).unwrap().body.mobile = false;
    rig.key(Key::Space);
    rig.idle(1);
    rig.app.load(example(3), None);
    rig.app.selected = Some(2);
    rig.idle(6);
    rig.app.load(example(0), None);
    rig.idle(2);
    rig.app.scene.objects[0].visible = false;
    let _ = supra;
    rig.app.freeze_reference();
    rig.idle(3);
    rig.app.selected = None;
    for tool in TOOLS {
        rig.app.tool = tool.0;
        rig.move_to(pos2(28.0, 100.0));
        rig.idle(1);
    }
    rig.app.prefs.library.open = false;
    rig.app.prefs.inspector.open = false;
    rig.app.prefs.graph.side = Side::Top;
    rig.idle(2);
    rig.open_palette();
    rig.text("zzz");
    rig.idle(2);
    rig.key(Key::Escape);
    rig.app.load(blank(), None);
    rig.idle(2);
    rig.app.tool = Tool::Polygon;
    rig.click(rig.at(0.0, 0.0));
    rig.app.finish_draft();
    rig.app.selected = Some(1);
    rig.app.delete_selected();
    assert_eq!(crate::lang::take_missing(), Vec::<String>::new());
    // En anglais, les nombres prennent un point décimal ; en CGS, l'induction est en gauss.
    assert_eq!((crate::ui::fr(1.5, 1), units::b(0.5)), ("1.5".into(), "5.000 kG".into()));
}

/// Avance jusqu'à ce que le champ soit convergé.
fn settle(rig: &mut Rig) {
    for _ in 0..400 {
        rig.idle(1);
        if !rig.app.pending {
            return;
        }
    }
    panic!("le champ ne converge pas");
}

/// Espace lance la simulation : la plaque glisse vers l'aimant. Pause la fige, « Revenir » la
/// replace, et l'historique ne garde qu'une entrée pour toute la simulation.
#[test]
fn space_plays_and_the_plate_slides_to_the_magnet() {
    let mut rig = Rig::new(Scene::friction_demo());
    rig.app.grid_n = 256;
    settle(&mut rig);
    let start = rig.app.scene.get(2).unwrap().pos.x;
    assert!(rig.app.wrenches[1].force.x < 0.0, "la plaque est attirée vers l'aimant");
    // Au ralenti, pour mettre en pause avant que la plaque n'atteigne l'aimant.
    rig.app.sim.speed = 0.1;
    rig.key(Key::Space);
    assert!(rig.app.sim.playing);
    rig.idle(8);
    let x = rig.app.scene.get(2).unwrap().pos.x;
    assert!(x < start - 2e-4, "la plaque devrait glisser vers l'aimant : {start} → {x}");
    assert!(rig.app.sim.time() > 0.0 && rig.app.sim.started());
    assert_eq!(rig.app.undo.len(), 1);

    rig.key(Key::Space);
    assert!(!rig.app.sim.playing);
    let paused = rig.app.scene.get(2).unwrap().pos.x;
    rig.idle(5);
    assert_eq!(rig.app.scene.get(2).unwrap().pos.x, paused);

    // Un pas à la fois avec la touche point.
    rig.key(Key::Period);
    rig.idle(1);
    assert!(rig.app.scene.get(2).unwrap().pos.x < paused && !rig.app.sim.playing);

    rig.app.rewind();
    assert_eq!(rig.app.scene.get(2).unwrap().pos.x, start);
    assert!(!rig.app.sim.started());

    // Laissée assez longtemps, la plaque vient se coller à l'aimant sans le traverser.
    rig.app.scene.get_mut(2).unwrap().body.mu_k = 0.02;
    rig.app.sim.speed = 1.0;
    rig.key(Key::Space);
    for _ in 0..3000 {
        rig.idle(1);
        if rig.app.scene.get(2).unwrap().pos.x < -0.0065 {
            break;
        }
    }
    rig.idle(30);
    let x = rig.app.scene.get(2).unwrap().pos.x;
    // Bord droit de l'aimant en −15 mm, demi-largeur de la plaque 8 mm.
    assert!((x + 0.007).abs() < 6e-4, "la plaque devrait toucher l'aimant : x = {x}");
}

/// Critère de sortie de la phase 2 : une bille posée près d'un aimant démarre quand la force
/// calculée dépasse μs·m·g, à 5 % près.
#[test]
fn ball_starts_at_the_computed_threshold() {
    let mut scene = Scene::default();
    scene.add("Aimant", Shape::Rect { w: 0.02, h: 0.04 }, DVec2::new(-0.025, 0.0), "NdFeB N42");
    let ball = scene.add("Bille", Shape::Circle { r: 0.006 }, DVec2::new(0.045, 0.0), "Acier doux (S235)");
    let mut rig = Rig::new(scene);
    rig.app.grid_n = 256;
    settle(&mut rig);
    let o = rig.app.scene.get(ball).unwrap();
    let weight = rig.app.scene.material(&o.material).unwrap().density * o.shape.area() * 9.81;
    let pull = rig.app.wrenches.iter().find(|w| w.id == ball).unwrap().force.length();
    let critical = pull / weight;
    println!("F = {pull:.2} N/m, poids = {weight:.2} N/m, μs critique = {critical:.3}");
    assert!(critical > 0.05 && critical < 2.0);

    for (factor, starts) in [(1.05, false), (0.95, true)] {
        rig.app.rewind();
        rig.app.sim.playing = false;
        rig.app.scene.get_mut(ball).unwrap().body = Body { mobile: true, mu_s: factor * critical, mu_k: 0.5 * critical, link: Link::Free };
        settle(&mut rig);
        rig.key(Key::Space);
        rig.idle(20);
        let moved = 0.045 - rig.app.scene.get(ball).unwrap().pos.x;
        assert_eq!(moved > 1e-5, starts, "μs = {factor}·μs critique : déplacement {moved} m");
        rig.key(Key::Space);
    }
}

/// Vue de côté : une bille lâchée tombe sur une plaque fixe et s'y pose.
#[test]
fn side_view_ball_falls_onto_a_fixed_plate() {
    let mut scene = Scene::default();
    scene.add("Plaque", Shape::Rect { w: 0.08, h: 0.01 }, DVec2::new(0.0, -0.02), "Aluminium");
    let ball = scene.add("Bille", Shape::Circle { r: 0.005 }, DVec2::new(0.0, 0.02), "Acier doux (S235)");
    scene.get_mut(ball).unwrap().body.mobile = true;
    let mut rig = Rig::new(scene);
    rig.app.grid_n = 256;
    // La vue se choisit depuis la palette de commandes.
    rig.open_palette();
    rig.text("vue de cote");
    rig.key(Key::Enter);
    assert_eq!(rig.app.scene.mechanics.view, MechView::Side);
    settle(&mut rig);
    rig.key(Key::Space);
    for _ in 0..600 {
        rig.idle(1);
        let world = rig.app.sim.world().unwrap();
        if rig.app.sim.time() > 0.3 && !world.moving() {
            break;
        }
    }
    // Dessus de la plaque en −15 mm, rayon de la bille 5 mm.
    let y = rig.app.scene.get(ball).unwrap().pos.y;
    assert!((y + 0.010).abs() < 4e-4, "la bille devrait reposer sur la plaque : y = {y}");
}

/// Une tôle mince devant un aimant puissant sature : Newton itère, et l'inspecteur le montre.
#[test]
fn saturated_sheet_converges_in_the_app() {
    let mut rig = Rig::new(Scene::saturation_demo());
    rig.app.selected = Some(2);
    settle(&mut rig);
    let newton = rig.app.newton.as_ref().unwrap();
    assert!(newton.raster.nonlinear.is_some() && newton.iterations >= 2, "{}", newton.iterations);
    let inside = (-30..=30).map(|k| rig.app.solver.field().sample(flux_core::DVec3::new(k as f64 * 1e-3, 0.0, 0.0)).unwrap().b.length());
    let peak = inside.fold(0.0, f64::max);
    assert!(peak > 1.5 && peak < 2.5, "induction dans la tôle : {peak} T");

    // Déplacer l'aimant relance le calcul depuis le champ précédent : peu d'itérations suffisent.
    rig.app.scene.get_mut(1).unwrap().pos.y -= 0.002;
    settle(&mut rig);
    assert!(rig.app.newton.as_ref().unwrap().iterations <= 8);
}
