//! Panneaux : barre supérieure, rail d'outils, bibliothèque, inspecteur, graphe de coupe, barre d'état.

use super::{App, Compare, TOOLS, fmt_b, fmt_force};
use crate::theme::{self as t, mono, mono_bold, sans, sans_bold};
use crate::ui::{self, PAD, fr};
use eframe::egui::{self, Align, Align2, Color32, Layout, Rect, Sense, Stroke, Ui, Vec2, pos2, vec2};
use flux_core::ABSOLUTE_ZERO_C;
use flux_core::material::MagClass;
use flux_core::scene::Scene;
use flux_core::shape::Shape;

const DEG: f64 = 180.0 / std::f64::consts::PI;

/// Prépare un panneau : lignes jointives. Renvoie son rectangle.
fn begin(ui: &mut Ui) -> Rect {
    ui.spacing_mut().item_spacing = Vec2::ZERO;
    ui.max_rect()
}

/// Nom court d'un matériau pour l'arbre de scène : « Acier doux (S235) » → « S235 ».
fn short_name(name: &str) -> String {
    let short = name.split_once('(').and_then(|(_, rest)| rest.split_once(')')).map_or(name, |(inner, _)| inner);
    if short.chars().count() > 14 { short.chars().take(13).chain(['…']).collect() } else { short.to_owned() }
}

impl App {
    pub(super) fn header(&mut self, ui: &mut Ui) {
        let r = begin(ui);
        let p = ui.painter().clone();
        let line = Stroke::new(1.0, t::LINE);
        p.hline(r.x_range(), r.bottom() - 0.5, line);
        let logo = Rect::from_min_size(r.min, vec2(56.0, r.height() - 1.0));
        ui::grad_h(&p, logo);
        self.icons.logo.paint(&p, Rect::from_center_size(logo.center(), Vec2::splat(20.0)), 1.6, t::ON_ACCENT);

        // Nom de la scène, modifiable en place.
        let name_w = ui::text_width(&p, &self.scene.name, sans_bold(13.0)).clamp(40.0, 320.0) + 6.0;
        let title = Rect::from_min_size(pos2(logo.right() + 16.0, r.top()), vec2(name_w, r.height()));
        let mut child = ui::place(ui, title, Layout::left_to_right(Align::Center));
        child.add(
            egui::TextEdit::singleline(&mut self.scene.name)
                .frame(egui::Frame::NONE)
                .font(sans_bold(13.0))
                .text_color(t::TEXT_HI)
                .desired_width(name_w)
                .margin(egui::Margin::ZERO),
        );
        self.icons.chevron.paint(&p, Rect::from_center_size(pos2(title.right() + 15.0, r.center().y), Vec2::splat(10.0)), 1.3, t::DIM);
        let mut x = title.right() + 36.0;
        p.vline(x, r.y_range(), line);

        for (i, label) in ["Nouveau", "Exemples", "Ouvrir…", "Enregistrer", "Enregistrer sous…"].into_iter().enumerate() {
            let w = ui::text_width(&p, label, sans(12.5)) + 28.0;
            let c = Rect::from_min_size(pos2(x, r.top()), vec2(w, r.height() - 1.0));
            x += w;
            p.vline(x, r.y_range(), line);
            let resp = ui::cell(ui, c, ("nav", i), label, sans(12.5), t::TEXT_MID);
            match i {
                0 if resp.clicked() => self.load(Scene::default(), None),
                1 => {
                    egui::Popup::menu(&resp).show(|ui| {
                        if ui.button("Aimant + plaque de fer").clicked() {
                            self.load(Scene::demo(), None);
                        }
                        if ui.button("Supraconducteur et diamagnétique").clicked() {
                            self.load(Scene::meissner_demo(), None);
                        }
                    });
                }
                2 if resp.clicked() => self.open(),
                3 if resp.clicked() => self.save(false),
                4 if resp.clicked() => self.save(true),
                _ => {}
            }
        }
        for (i, enabled) in [!self.undo.is_empty(), !self.redo.is_empty()].into_iter().enumerate() {
            let c = Rect::from_min_size(pos2(x, r.top()), vec2(40.0, r.height() - 1.0));
            x += 40.0;
            p.vline(x, r.y_range(), line);
            let tip = if i == 0 { "Annuler (Ctrl Z)" } else { "Rétablir (Ctrl Maj Z)" };
            let resp = ui.interact(c, ui.id().with(("history", i)), Sense::click()).on_hover_text(tip);
            if enabled && resp.hovered() {
                p.rect_filled(c, 0.0, t::tint());
            }
            let icon = if i == 0 { &self.icons.undo } else { &self.icons.redo };
            icon.paint(&p, Rect::from_center_size(c.center(), Vec2::splat(15.0)), 1.4, if enabled { t::TEXT_MID } else { t::DISABLED });
            if enabled && resp.clicked() {
                if i == 0 { self.do_undo() } else { self.do_redo() }
            }
        }
    }

    pub(super) fn rail(&mut self, ui: &mut Ui) {
        let r = begin(ui);
        let p = ui.painter().clone();
        for (i, tool) in TOOLS.iter().enumerate() {
            let c = Rect::from_min_size(pos2(r.left(), r.top() + 54.0 * i as f32), vec2(r.width() - 1.0, 54.0));
            let resp = ui.interact(c, ui.id().with(("tool", i)), Sense::click()).on_hover_text(tool.3);
            let on = self.tool == tool.0;
            if on || resp.hovered() {
                p.rect_filled(c, 0.0, if on { t::tint() } else { t::tint().gamma_multiply(0.5) });
            }
            if on {
                ui::grad_v(&p, Rect::from_min_size(c.min, vec2(2.0, c.height())));
            }
            p.hline(c.x_range(), c.bottom() - 0.5, Stroke::new(1.0, t::LINE_SOFT));
            let icon = Rect::from_min_size(pos2(c.center().x - 9.0, c.top() + 10.0), Vec2::splat(18.0));
            self.icons.tools[i].paint(&p, icon, 1.4, if on { t::ACCENT } else { t::TEXT_MID });
            let color = if on { t::ACCENT_TEXT } else { t::DIM };
            p.text(pos2(c.center().x, c.top() + 39.0), Align2::CENTER_CENTER, tool.1, sans(9.5), color);
            if resp.clicked() {
                self.tool = tool.0;
            }
        }
        p.vline(r.right() - 0.5, r.y_range(), Stroke::new(1.0, t::LINE));
    }

    pub(super) fn library(&mut self, ui: &mut Ui) {
        let r = begin(ui);
        let p = ui.painter().clone();
        let needle = self.search.to_lowercase();
        let shown: Vec<usize> = (0..self.scene.materials.len())
            .filter(|&k| {
                let m = &self.scene.materials[k];
                self.lib_filter.is_none_or(|c| c == m.class) && m.name.to_lowercase().contains(&needle)
            })
            .collect();

        let head = ui::row(ui, 36.0, t::LINE);
        ui::tracked(&p, pos2(head.left() + PAD, head.center().y), Align2::LEFT_CENTER, "BIBLIOTHÈQUE", mono_bold(10.5), t::TEXT_HI);
        p.text(pos2(head.right() - PAD, head.center().y), Align2::RIGHT_CENTER, shown.len().to_string(), mono(10.5), t::DIM);

        let search = ui::row(ui, 36.0, t::LINE);
        self.icons.search.paint(
            &p,
            Rect::from_center_size(pos2(search.left() + PAD + 6.5, search.center().y), Vec2::splat(13.0)),
            1.3,
            t::DIM,
        );
        let field = Rect::from_min_max(pos2(search.left() + PAD + 21.0, search.top()), pos2(search.right() - PAD, search.bottom()));
        ui::place(ui, field, Layout::left_to_right(Align::Center)).add(
            egui::TextEdit::singleline(&mut self.search)
                .frame(egui::Frame::NONE)
                .hint_text("Rechercher un matériau")
                .font(sans(12.0))
                .desired_width(field.width())
                .margin(egui::Margin::ZERO),
        );

        // Familles : onglets soulignés, avec retour à la ligne.
        let chips: Vec<(Option<MagClass>, &str)> =
            std::iter::once((None, "Tous")).chain(MagClass::ALL.map(|c| (Some(c), c.label()))).collect();
        let (mut x, mut y) = (PAD, 8.0);
        let mut cells = Vec::new();
        for (class, name) in &chips {
            let font = if self.lib_filter == *class { sans_bold(11.5) } else { sans(11.5) };
            let w = ui::text_width(&p, name, font);
            if x + w > r.width() - PAD {
                (x, y) = (PAD, y + 26.0);
            }
            cells.push((x, y, w));
            x += w + 12.0;
        }
        let area = ui::row(ui, y + 30.0, t::LINE);
        for ((class, name), (x, y, w)) in chips.iter().zip(cells) {
            let c = Rect::from_min_size(area.min + vec2(x, y), vec2(w, 24.0));
            let resp = ui.interact(c.expand2(vec2(5.0, 0.0)), ui.id().with(("chip", name)), Sense::click());
            let on = self.lib_filter == *class;
            let color = if on || resp.hovered() { t::TEXT_HI } else { t::DIM };
            p.text(c.left_center(), Align2::LEFT_CENTER, name, if on { sans_bold(11.5) } else { sans(11.5) }, color);
            if on {
                ui::grad_h(&p, Rect::from_min_max(pos2(c.left(), c.bottom() - 3.0), pos2(c.right(), c.bottom() - 1.0)));
            }
            if resp.clicked() {
                self.lib_filter = *class;
            }
        }

        let columns = ui::row(ui, 26.0, t::LINE);
        ui::tracked(&p, pos2(columns.left() + PAD, columns.center().y), Align2::LEFT_CENTER, "MATÉRIAU", mono(10.0), t::DIM);
        ui::tracked(&p, pos2(columns.right() - PAD, columns.center().y), Align2::RIGHT_CENTER, "VALEUR", mono(10.0), t::DIM);

        // Liste des matériaux : elle occupe la place laissée par l'arbre de scène.
        let scene_rows = self.scene.objects.len().min(6) as f32;
        let list_h = (ui.available_height() - 32.0 - 30.0 * scene_rows).max(54.0);
        let list = Rect::from_min_size(ui.cursor().min, vec2(r.width(), list_h));
        let mut picked = None;
        ui.allocate_ui(list.size(), |ui| {
            egui::ScrollArea::vertical().id_salt("materials").auto_shrink(false).show(ui, |ui| {
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                for &k in &shown {
                    let m = &self.scene.materials[k];
                    let value = match m.class {
                        MagClass::Magnet => format!("Br {:.2} T", m.br),
                        MagClass::Ferro => format!("μr {:.0}", m.mu_r),
                        MagClass::Superconductor => format!("Tc {:.0} K", m.t_critical - ABSOLUTE_ZERO_C),
                        _ => format!("χ {:+.1e}", m.chi),
                    };
                    if ui::list_row(ui, 27.0, ("material", k), self.lib_material == m.name, t::class_color(m.class), &m.name, &value)
                        .clicked()
                    {
                        picked = Some(m.name.clone());
                    }
                }
                ui.add_space(30.0);
            });
        });
        ui::fade_down(&p, Rect::from_min_max(pos2(list.left(), list.bottom() - 36.0), list.right_bottom()), t::PANEL);
        // Choisir un matériau l'applique aussi à la sélection.
        if let Some(name) = picked {
            if let Some(o) = self.selected.and_then(|id| self.scene.get_mut(id)) {
                o.material = name.clone();
            }
            self.lib_material = name;
        }

        let head = ui::section(ui, "", "SCÈNE");
        p.text(pos2(head.right() - PAD, head.center().y), Align2::RIGHT_CENTER, self.scene.objects.len().to_string(), mono(10.5), t::DIM);
        egui::ScrollArea::vertical().id_salt("objects").auto_shrink(false).show(ui, |ui| {
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            for o in &self.scene.objects {
                let color = self.scene.material(&o.material).map_or(t::DIM, |m| t::class_color(m.class));
                if ui::list_row(ui, 30.0, ("object", o.id), self.selected == Some(o.id), color, &o.name, &short_name(&o.material)).clicked()
                {
                    self.selected = Some(o.id);
                }
            }
        });
        p.vline(r.right() - 0.5, r.y_range(), Stroke::new(1.0, t::LINE));
    }

    pub(super) fn inspector(&mut self, ui: &mut Ui) {
        let r = begin(ui);
        let p = ui.painter().clone();
        egui::ScrollArea::vertical().id_salt("inspector").auto_shrink(false).show(ui, |ui| {
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            match self.selected.filter(|&id| self.scene.get(id).is_some()) {
                Some(id) => self.inspect_object(ui, id),
                None => self.inspect_scene(ui),
            }
        });
        p.vline(r.left() + 0.5, r.y_range(), Stroke::new(1.0, t::LINE));
    }

    fn inspect_scene(&mut self, ui: &mut Ui) {
        ui::section(ui, "01", "SCÈNE");
        ui::kv_edit(ui, "Profondeur", "mm", |ui| ui::number(ui, &mut self.scene.depth, 1e3, 0.1, 1));
        ui::kv_edit(ui, "Domaine", "mm", |ui| ui::number(ui, &mut self.scene.size, 1e3, 1.0, 0));
        let kelvin = format!("°C · {} K", fr(self.scene.ambient - ABSOLUTE_ZERO_C, 2));
        ui::kv_edit(ui, "T ambiante", &kelvin, |ui| ui::number(ui, &mut self.scene.ambient, 1.0, 0.5, 0));
        self.scene.depth = self.scene.depth.max(1e-4);
        self.scene.size = self.scene.size.clamp(0.01, 10.0);

        ui::section(ui, "02", "CALCUL");
        let mut gpu = self.use_gpu;
        if ui::segmented(ui, "Moteur", "engine", &mut gpu, &[(true, "GPU f32"), (false, "CPU f64")]) {
            self.set_solver(gpu);
        }
        ui::segmented(ui, "Grille", "grid", &mut self.grid_n, &[(256, "256²"), (512, "512²"), (1024, "1024²"), (2048, "2048²")]);
        ui::kv(ui, "Pas", &fr(self.scene.size / self.grid_n as f64 * 1e3, 3), "mm");
        let shown = format!("{:.0}", self.n_lines);
        ui::slider(ui, "Lignes", 44.0, &mut self.n_lines, 8.0..=160.0, Some(shown), true);
        self.n_lines = self.n_lines.round();

        ui::section(ui, "03", "AFFICHAGE");
        ui::switch(ui, "Animer la LIC", &mut self.lic_animate);
        let clear = [("Effacer les graines", !self.scene.seeds.is_empty()), ("Balayer la limaille", !self.visuals.filings.is_empty())];
        match ui::button_row(ui, "clear", &clear) {
            Some(0) => self.scene.seeds.clear(),
            Some(_) => self.visuals.filings.clear(),
            None => {}
        }

        ui::section(ui, "04", "COMPARAISON");
        if ui::accent_button(ui, "Figer l’état actuel comme référence", &self.icons.arrow) {
            self.freeze_reference();
        }
        let has_reference = self.reference.is_some();
        let options = [(Compare::Off, "Aucune"), (Compare::Split, "Avant | après"), (Compare::Diff, "Différence")];
        ui::tabs(ui, "compare", &mut self.compare, &options, has_reference);
        ui::slider(ui, "Séparation", 72.0, &mut self.split, 0.05..=0.95, None, has_reference && self.compare == Compare::Split);

        ui.add_space((ui.available_height() - 66.0).max(0.0));
        ui::note(ui, "Sélectionnez un objet pour l’inspecter, ou choisissez un outil et dessinez sur le canevas.", t::ACCENT);
    }

    fn inspect_object(&mut self, ui: &mut Ui, id: u32) {
        let names: Vec<String> = self.scene.materials.iter().map(|m| m.name.clone()).collect();
        let (depth, ambient) = (self.scene.depth, self.scene.ambient);
        let wrench = self.wrenches.iter().find(|w| w.id == id).copied();
        let mat = self.scene.get(id).and_then(|o| self.scene.material(&o.material)).cloned();
        let Some(o) = self.scene.get_mut(id) else { return };

        ui::section(ui, "01", "OBJET");
        ui::kv_edit(ui, "Nom", "", |ui| {
            let edit = egui::TextEdit::singleline(&mut o.name).frame(egui::Frame::NONE).font(mono(12.0)).text_color(t::TEXT_HI);
            ui.add(edit.desired_width(170.0).horizontal_align(Align::RIGHT).margin(egui::Margin::ZERO))
        });
        ui::kv_edit(ui, "Matériau", "", |ui| {
            let shown = egui::RichText::new(o.material.clone()).font(mono(11.5)).color(t::TEXT_HI);
            egui::ComboBox::from_id_salt("material").width(184.0).selected_text(shown).show_ui(ui, |ui| {
                for name in &names {
                    ui.selectable_value(&mut o.material, name.clone(), name.as_str());
                }
            });
        });
        let kelvin = format!("°C · {} K", fr(o.temperature - ABSOLUTE_ZERO_C, 2));
        ui::kv_edit(ui, "Température", &kelvin, |ui| ui::number(ui, &mut o.temperature, 1.0, 1.0, 0));

        ui::section(ui, "02", "GÉOMÉTRIE");
        ui::kv_edit(ui, "Position X", "mm", |ui| ui::number(ui, &mut o.pos.x, 1e3, 0.1, 1));
        ui::kv_edit(ui, "Position Y", "mm", |ui| ui::number(ui, &mut o.pos.y, 1e3, 0.1, 1));
        ui::kv_edit(ui, "Rotation", "°", |ui| ui::number(ui, &mut o.angle, DEG, 1.0, 0));
        match &mut o.shape {
            Shape::Rect { w, h } => {
                ui::kv_edit(ui, "Largeur", "mm", |ui| ui::number(ui, w, 1e3, 0.1, 1));
                ui::kv_edit(ui, "Hauteur", "mm", |ui| ui::number(ui, h, 1e3, 0.1, 1));
                (*w, *h) = (w.max(2e-4), h.max(2e-4));
            }
            Shape::Circle { r } => {
                ui::kv_edit(ui, "Rayon", "mm", |ui| ui::number(ui, r, 1e3, 0.1, 1));
                *r = r.max(2e-4);
            }
            Shape::Ring { r_in, r_out } => {
                ui::kv_edit(ui, "Rayon intérieur", "mm", |ui| ui::number(ui, r_in, 1e3, 0.1, 1));
                ui::kv_edit(ui, "Rayon extérieur", "mm", |ui| ui::number(ui, r_out, 1e3, 0.1, 1));
                *r_in = r_in.max(1e-4);
                *r_out = r_out.max(*r_in + 2e-4);
            }
            Shape::Polygon { pts } => ui::kv(ui, "Sommets", &pts.len().to_string(), ""),
        }
        let Some(mat) = mat else { return };
        let area = o.shape.area();
        ui::kv(ui, "Masse", &fr(mat.density * area * depth * 1e3, 1), "g");

        let title = match mat.class {
            MagClass::Magnet => "AIMANTATION",
            MagClass::Ferro => "FERROMAGNÉTIQUE",
            MagClass::Para => "PARAMAGNÉTIQUE",
            MagClass::Dia => "DIAMAGNÉTIQUE",
            MagClass::Conductor => "COURANT",
            MagClass::Superconductor => "SUPRACONDUCTEUR",
        };
        ui::section(ui, "03", title);
        match mat.class {
            MagClass::Magnet => {
                ui::kv_edit(ui, "Angle", "°", |ui| ui::number(ui, &mut o.mag_angle, DEG, 1.0, 0));
                if ui::button_row(ui, "flip", &[("Inverser les pôles", true)]).is_some() {
                    o.mag_angle += std::f64::consts::PI;
                }
                ui::kv(ui, "Br à 20 °C", &fr(mat.br, 3), "T");
                ui::kv(ui, "Br à la température de l’objet", &fr(mat.br_at(o.temperature), 3), "T");
                ui::kv(ui, "μrec", &fr(mat.mu_r, 2), "");
                ui::kv(ui, "Tc", &fr(mat.t_curie, 0), "°C");
            }
            MagClass::Ferro => {
                ui::kv(ui, "μr (linéaire)", &fr(mat.mu_r_solver(o.temperature), 0), "");
                ui::kv(ui, "Tc", &fr(mat.t_curie, 0), "°C");
            }
            MagClass::Conductor => {
                ui::kv_edit(ui, "Spires", "", |ui| ui::number(ui, &mut o.turns, 1.0, 1.0, 0));
                o.turns = o.turns.max(0.0);
                ui::kv_edit(ui, "Courant", "A", |ui| ui::number(ui, &mut o.current, 1.0, 0.1, 2));
                let sense = if o.current >= 0.0 { "Sortant — inverser le sens" } else { "Entrant — inverser le sens" };
                if ui::button_row(ui, "flip", &[(sense, true)]).is_some() {
                    o.current = -o.current;
                }
                ui::kv(ui, "Ampères-tours", &fr(o.amp_turns(), 0), "A");
                let j = o.amp_turns().abs() / area * 1e-6;
                // Au-delà d'environ 5 A/mm² en continu, le bobinage chauffe.
                ui::kv_colored(ui, "Densité J", &fr(j, 2), "A/mm²", if j > 5.0 { t::WARN } else { t::TEXT_HI });
            }
            MagClass::Para | MagClass::Dia => {
                let chi = mat.chi_at(o.temperature);
                ui::kv(ui, "χ", &format!("{chi:+.2e}"), "");
                let text = format!(
                    "Modifie le champ d’environ {} % (χ/2) : invisible sur les lignes de champ. \
                     L’objet subit en revanche une force (densité de Kelvin), donnée ci-dessous.",
                    fr(chi.abs() * 50.0, 4)
                );
                ui::note(ui, &text, t::FAINT);
            }
            MagClass::Superconductor => {
                ui::kv(ui, "Tc", &fr(mat.t_critical - ABSOLUTE_ZERO_C, 1), "K");
                let cold = mat.is_superconducting(o.temperature);
                ui::kv_colored(ui, "État", if cold { "Meissner" } else { "normal" }, "", if cold { t::ACCENT_TEXT } else { t::WARN });
                match ui::button_row(ui, "cool", &[("Azote liquide", true), ("Hélium liquide", true), ("Ambiante", true)]) {
                    Some(0) => o.temperature = -196.0,
                    Some(1) => o.temperature = -269.0,
                    Some(_) => o.temperature = ambient,
                    None => {}
                }
                let text = if cold { "Le champ est expulsé (χ = −1)." } else { "Refroidir sous Tc pour expulser le champ." };
                ui::note(ui, text, if cold { t::ACCENT } else { t::WARN });
            }
        }

        if let Some(w) = wrench {
            ui::section(ui, "04", "FORCE");
            let f = w.force.truncate();
            ui::kv_colored(ui, "|F|", &fmt_force(f.length()), "N/m", t::FORCE_TEXT);
            ui::kv(ui, "Fx", &fmt_force(f.x), "N/m");
            ui::kv(ui, "Fy", &fmt_force(f.y), "N/m");
            ui::kv(ui, "|F| réelle", &fmt_force(f.length() * depth), "N");
            ui::kv(ui, "Couple", &fmt_force(w.torque), "N·m/m");
            ui::kv(ui, "F / (m·g)", &fmt_force(f.length() / (mat.density * area * 9.81)), "");
            if w.resolution_limited {
                ui::note(ui, "Entrefer plus fin que la grille : valeur limitée par la résolution.", t::WARN);
            }
        }
    }

    pub(super) fn graph(&mut self, ui: &mut Ui) {
        let r = begin(ui);
        let p = ui.painter().clone();
        let Some([a, b]) = self.scene.cut_line else { return };
        let field = self.solver.field();
        let samples: Vec<f64> = (0..=240).map(|k| field.sample(a.lerp(b, k as f64 / 240.0)).map_or(0.0, |s| s.b.length())).collect();
        let max = samples.iter().copied().fold(1e-12, f64::max);

        let head = ui::section(ui, "", "COUPE · |B|");
        let info = format!("longueur {} mm · max {}", fr((b - a).length() * 1e3, 1), fmt_b(max));
        p.text(pos2(head.left() + 130.0, head.center().y), Align2::LEFT_CENTER, info, mono(10.5), t::DIM);
        let mut action = None;
        for (i, label) in ["Effacer", "Exporter CSV"].into_iter().enumerate() {
            let c = Rect::from_min_size(pos2(head.right() - 110.0 * (i + 1) as f32, head.top()), vec2(110.0, head.height() - 1.0));
            p.vline(c.left(), head.y_range(), Stroke::new(1.0, t::LINE));
            if ui::cell(ui, c, ("cut", i), label, sans(12.0), t::TEXT_MID).clicked() {
                action = Some(i);
            }
        }
        let plot = Rect::from_min_max(pos2(r.left() + PAD, head.bottom() + 12.0), pos2(r.right() - PAD, r.bottom() - 12.0));
        p.hline(plot.x_range(), plot.bottom(), Stroke::new(1.0, t::LINE_CTRL));
        let points = samples
            .iter()
            .enumerate()
            .map(|(k, &v)| pos2(plot.left() + plot.width() * k as f32 / 240.0, plot.bottom() - plot.height() * (v / max) as f32));
        p.add(egui::Shape::line(points.collect(), Stroke::new(1.5, t::ACCENT)));
        p.text(plot.left_top(), Align2::LEFT_TOP, fmt_b(max), mono(10.0), t::DIM);
        match action {
            Some(0) => self.scene.cut_line = None,
            Some(_) => self.export_cut(),
            None => {}
        }
    }

    pub(super) fn footer(&mut self, ui: &mut Ui) {
        let r = begin(ui);
        let p = ui.painter().clone();
        let line = Stroke::new(1.0, t::LINE);
        p.hline(r.x_range(), r.top() + 0.5, line);
        let (top, cy) = (r.top() + 1.0, r.center().y + 0.5);

        // État du solveur, sur fond d'accent une fois convergé.
        let state = if self.pending { "CALCUL…" } else { "CONVERGÉ" };
        let width = ui::text_width(&p, state, mono_bold(10.5)) + 1.26 * state.chars().count() as f32 + 24.0;
        let block = Rect::from_min_max(pos2(r.left(), top), pos2(r.left() + width, r.bottom()));
        if self.pending {
            p.rect_filled(block, 0.0, t::tint());
        } else {
            ui::grad_h(&p, block);
        }
        let color = if self.pending { t::ACCENT_TEXT } else { t::ON_ACCENT };
        ui::tracked(&p, pos2(block.left() + 12.0, cy), Align2::LEFT_CENTER, state, mono_bold(10.5), color);

        let mut x = block.right();
        let mut segment = |text: String, color: Color32| {
            x += 12.0;
            x = p.text(pos2(x, cy), Align2::LEFT_CENTER, text, mono(10.5), color).right() + 12.0;
            p.vline(x, top..=r.bottom(), line);
        };
        let s = self.status;
        segment(self.solver.name().to_owned(), t::LABEL);
        segment(format!("{}²", self.solved_n), t::LABEL);
        segment(format!("{} it.", s.iterations), t::LABEL);
        segment(format!("résidu {:.1e}", s.residual), t::LABEL);
        segment(format!("{:.1} ms", s.elapsed.as_secs_f64() * 1e3), t::LABEL);
        if let Some(c) = self.cursor {
            segment(format!("x {} · y {} mm", fr(c.x * 1e3, 1), fr(c.y * 1e3, 1)), t::LABEL);
            if let Some(f) = self.solver.field().sample(c.extend(0.0)) {
                segment(format!("|B| {}", fmt_b(f.b.length())), t::TEXT_HI);
                segment(format!("Bx {} · By {}", fmt_b(f.b.x), fmt_b(f.b.y)), t::LABEL);
            }
        }
        if !self.message.is_empty() {
            segment(self.message.clone(), t::WARN);
        }

        let mut x = r.right();
        let mut segment = |parts: &[(String, Color32)]| {
            x -= 12.0;
            for (text, color) in parts.iter().rev() {
                x = p.text(pos2(x, cy), Align2::RIGHT_CENTER, text, mono(10.5), *color).left();
            }
            x -= 12.0;
            p.vline(x, top..=r.bottom(), line);
        };
        let fps = 1.0 / ui.input(|i| i.stable_dt).max(1e-4);
        segment(&[(format!("{fps:.0} IMG/S"), t::FORCE)]);
        segment(&[("2D PLAN".into(), t::TEXT), (format!(" · PROFONDEUR {} MM", fr(self.scene.depth * 1e3, 0)), t::LABEL)]);
    }
}
