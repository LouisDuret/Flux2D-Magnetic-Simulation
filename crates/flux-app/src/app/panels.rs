//! Panneaux : barre supérieure, rail d'outils, bibliothèque, inspecteur, graphe de coupe, barre d'état.

use super::{App, Compare, EXAMPLES, Panel, Side, TOOLS, blank, example, palette};
use crate::expr::Quantity;
use crate::lang::{Lang, tr};
use crate::theme::{self as t, mono, mono_bold, sans, sans_bold};
use crate::ui::{self, PAD, fr};
use crate::units::{self, Units};
use eframe::egui::{self, Align, Align2, Color32, CursorIcon, Layout, Rect, Sense, Stroke, Ui, Vec2, pos2, vec2};
use flux_core::magnet::MagPattern;
use flux_core::material::{KNEE, MagClass, Material, NuTable};
use flux_core::scene::{Link, MechView, Object, SURFACES};
use flux_core::shape::{BoolOp, Sdf, Shape};
use flux_core::{ABSOLUTE_ZERO_C, DVec2, DVec3, MU0};
use flux_solver::Field;

/// Prépare un panneau : lignes jointives. Renvoie son rectangle.
fn begin(ui: &mut Ui) -> Rect {
    ui.spacing_mut().item_spacing = Vec2::ZERO;
    ui.max_rect()
}

/// Nom court d'un matériau pour l'arbre de scène : « Acier doux (S235) » → « S235 ».
fn short_name(name: &str) -> String {
    let short = name.split_once('(').and_then(|(_, rest)| rest.split_once(')')).map_or(name, |(inner, _)| inner);
    if short.chars().count() > 12 { short.chars().take(11).chain(['…']).collect() } else { short.to_owned() }
}

/// Induction moyenne à l'intérieur d'un objet (T), sur une grille de points ; `None` si aucun
/// point ne tombe dedans.
fn mean_b(field: &Field, o: &Object) -> Option<f64> {
    let reach = o.shape.bounding_radius();
    let (mut sum, mut count) = (0.0, 0);
    for j in 0..17 {
        for i in 0..17 {
            let p = o.pos + DVec3::new(i as f64 / 8.0 - 1.0, j as f64 / 8.0 - 1.0, 0.0) * reach;
            if o.distance(p) < -field.h()
                && let Some(s) = field.sample(p)
            {
                sum += s.b.length();
                count += 1;
            }
        }
    }
    (count > 0).then(|| sum / count as f64)
}

/// Plus fort champ inverse subi par un aimant le long de son aimantation (A/m), relevé sur une
/// grille de points intérieurs ; `None` si aucun point ne tombe dedans. Avec `ideal`, la
/// désaimantation déjà subie est ignorée.
fn reverse_field(field: &Field, o: &Object, mat: &Material, ideal: bool) -> Option<f64> {
    let (lo, hi) = o.shape.bounds();
    let (magnetization, br) = (o.magnetization(), mat.br_at(o.temperature));
    let mut worst: Option<f64> = None;
    for k in 0..13 * 13 {
        let local = lo + (hi - lo) * DVec2::new(((k % 13) as f64 + 0.5) / 13.0, ((k / 13) as f64 + 0.5) / 13.0);
        if o.shape.distance(local.extend(0.0)) > -field.h() {
            continue;
        }
        let Some(sample) = field.sample(o.to_world(local).extend(0.0)) else { continue };
        let left = if ideal { 1.0 } else { o.remanence_left(local) };
        let h = (sample.b.truncate().dot(magnetization.dir(local)) - left * br) / (MU0 * mat.mu_r);
        worst = Some(worst.map_or(-h, |w| w.max(-h)));
    }
    worst
}

/// Courbe B(H) d'un matériau saturable, H en échelle logarithmique, avec le point de
/// fonctionnement de l'objet.
fn bh_plot(ui: &mut Ui, table: &NuTable, js: f64, at: Option<f64>) {
    let r = ui::row(ui, 138.0, t::LINE_SOFT);
    let p = ui.painter().clone();
    let plot = Rect::from_min_max(pos2(r.left() + PAD, r.top() + 30.0), pos2(r.right() - PAD, r.bottom() - 24.0));
    let b_top = 1.12 * js;
    let (h_lo, h_hi) = (table.h(0.02 * js).max(1e-3), table.h(b_top));
    let place = |b: f64| {
        let x = ((table.h(b).max(h_lo) / h_lo).ln() / (h_hi / h_lo).ln()).clamp(0.0, 1.0) as f32;
        pos2(plot.left() + plot.width() * x, plot.bottom() - plot.height() * (b / b_top).min(1.0) as f32)
    };
    ui::tracked(&p, pos2(plot.left(), r.top() + 15.0), Align2::LEFT_CENTER, "B(H)", mono_bold(10.0), t::TEXT_HI);
    let (top, unit) = units::remanence(b_top);
    p.text(pos2(plot.right(), r.top() + 15.0), Align2::RIGHT_CENTER, format!("{} {unit}", fr(top, 2)), mono(10.0), t::DIM);
    p.hline(plot.x_range(), plot.bottom(), Stroke::new(1.0, t::LINE_CTRL));
    p.vline(plot.left(), plot.y_range(), Stroke::new(1.0, t::LINE_CTRL));
    // Niveau de la polarisation à saturation.
    let y = plot.bottom() - plot.height() * (js / b_top) as f32;
    p.add(egui::Shape::dashed_line(&[pos2(plot.left(), y), pos2(plot.right(), y)], Stroke::new(1.0, t::TICK_MAJOR), 3.0, 3.0));
    p.text(pos2(plot.right(), y + 9.0), Align2::RIGHT_CENTER, "Js", mono(10.0), t::FAINT);
    let curve: Vec<egui::Pos2> = (1..=96).map(|k| place(b_top * k as f64 / 96.0)).collect();
    p.add(egui::Shape::line(curve, Stroke::new(1.5, t::ACCENT)));
    if let Some(b) = at {
        let mark = Rect::from_center_size(place(b.min(b_top)), Vec2::splat(7.0));
        p.rect_filled(mark, 0.0, t::BG);
        p.rect_stroke(mark, 0.0, Stroke::new(1.5, t::TEXT_HI), egui::StrokeKind::Middle);
    }
    let amps =
        |h: f64| if h >= 1000.0 { format!("{} kA/m", fr(h / 1000.0, 0)) } else { format!("{} A/m", fr(h, if h < 10.0 { 1 } else { 0 })) };
    p.text(pos2(plot.left(), r.bottom() - 12.0), Align2::LEFT_CENTER, amps(h_lo), mono(10.0), t::DIM);
    p.text(pos2(plot.center().x, r.bottom() - 12.0), Align2::CENTER_CENTER, "H · log", mono(10.0), t::FAINT);
    p.text(pos2(plot.right(), r.bottom() - 12.0), Align2::RIGHT_CENTER, amps(h_hi), mono(10.0), t::DIM);
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
            let w = ui::text_width(&p, tr(label), sans(12.5)) + 28.0;
            let c = Rect::from_min_size(pos2(x, r.top()), vec2(w, r.height() - 1.0));
            x += w;
            p.vline(x, r.y_range(), line);
            let resp = ui::cell(ui, c, ("nav", i), tr(label), sans(12.5), t::TEXT_MID);
            match i {
                0 if resp.clicked() => self.load(blank(), None),
                1 => {
                    egui::Popup::menu(&resp).show(|ui| {
                        for (index, name) in EXAMPLES.into_iter().enumerate() {
                            if ui.button(tr(name)).clicked() {
                                self.load(example(index), None);
                            }
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
            let resp = ui.interact(c, ui.id().with(("history", i)), Sense::click()).on_hover_text(tr(tip));
            if enabled && resp.hovered() {
                p.rect_filled(c, 0.0, t::tint());
            }
            let icon = if i == 0 { &self.icons.undo } else { &self.icons.redo };
            icon.paint(&p, Rect::from_center_size(c.center(), Vec2::splat(15.0)), 1.4, if enabled { t::TEXT_MID } else { t::DISABLED });
            if enabled && resp.clicked() {
                if i == 0 { self.do_undo() } else { self.do_redo() }
            }
        }

        // Simulation : lecture ou pause, pas à pas, retour à l'état initial, temps simulé.
        let cy = r.center().y;
        let (playing, started) = (self.sim.playing, self.sim.started());
        let controls = [
            (if playing { &self.icons.pause } else { &self.icons.play }, if playing { "Pause (Espace)" } else { "Lecture (Espace)" }, true),
            (&self.icons.step, "Avancer d’un pas (.)", true),
            (&self.icons.rewind, "Revenir à l’état initial", started),
        ];
        let mut action = None;
        for (i, (icon, tip, enabled)) in controls.into_iter().enumerate() {
            let c = Rect::from_min_size(pos2(x, r.top()), vec2(40.0, r.height() - 1.0));
            x += 40.0;
            p.vline(x, r.y_range(), line);
            let resp = ui.interact(c, ui.id().with(("sim", i)), Sense::click()).on_hover_text(tr(tip));
            let on = i == 0 && playing;
            if on || (enabled && resp.hovered()) {
                p.rect_filled(c, 0.0, t::tint());
            }
            if on {
                ui::grad_h(&p, Rect::from_min_max(pos2(c.left(), c.bottom() - 2.0), c.max));
            }
            let color = match (enabled, on) {
                (false, _) => t::DISABLED,
                (true, true) => t::ACCENT,
                (true, false) => t::TEXT_MID,
            };
            icon.paint(&p, Rect::from_center_size(c.center(), Vec2::splat(15.0)), 1.4, color);
            if enabled && resp.clicked() {
                action = Some(i);
            }
        }
        match action {
            Some(0) => self.toggle_play(),
            Some(1) => self.step_simulation(),
            Some(_) => self.rewind(),
            None => {}
        }
        if started {
            let clock = format!("t {} s", fr(self.sim.time(), 2));
            p.text(pos2(x + 12.0, cy), Align2::LEFT_CENTER, clock, mono(11.0), if playing { t::ACCENT_TEXT } else { t::LABEL });
        }

        // À droite : langue, unités d'affichage, palette de commandes.
        let (_, left) = ui::segmented_box(ui, r.right() - PAD, cy, "lang", &mut self.prefs.lang, &[(Lang::Fr, "FR"), (Lang::En, "EN")]);
        let (_, left) = ui::segmented_box(ui, left - 10.0, cy, "units", &mut self.prefs.units, &[(Units::Si, "SI"), (Units::Cgs, "CGS")]);
        let right = left - PAD;
        p.vline(right, r.y_range(), line);
        let label = tr("Commandes");
        let label_w = ui::text_width(&p, label, sans(12.5));
        let width = 14.0 + 13.0 + 8.0 + label_w + 10.0 + ui::text_width(&p, "Ctrl K", mono(10.5)) + 14.0;
        let c = Rect::from_min_max(pos2(right - width, r.top()), pos2(right, r.bottom() - 1.0));
        p.vline(c.left(), r.y_range(), line);
        let resp = ui.interact(c, ui.id().with("palette"), Sense::click()).on_hover_text(tr("Palette de commandes (Ctrl K)"));
        if resp.hovered() {
            p.rect_filled(c, 0.0, t::tint());
        }
        self.icons.search.paint(&p, Rect::from_center_size(pos2(c.left() + 20.5, cy), Vec2::splat(13.0)), 1.3, t::DIM);
        let text = p.text(
            pos2(c.left() + 35.0, cy),
            Align2::LEFT_CENTER,
            label,
            sans(12.5),
            if resp.hovered() { t::TEXT_HI } else { t::TEXT_MID },
        );
        p.text(pos2(text.right() + 10.0, cy), Align2::LEFT_CENTER, "Ctrl K", mono(10.5), t::FAINT);
        if resp.clicked() {
            self.palette = Some(palette::Palette::default());
        }
    }

    pub(super) fn rail(&mut self, ui: &mut Ui) {
        let r = begin(ui);
        let p = ui.painter().clone();
        // Les cases se tassent si la fenêtre est trop basse, et perdent alors leur libellé.
        let step = (r.height() / TOOLS.len() as f32).min(54.0);
        let labels = step >= 44.0;
        for (i, tool) in TOOLS.iter().enumerate() {
            let c = Rect::from_min_size(pos2(r.left(), r.top() + step * i as f32), vec2(r.width() - 1.0, step));
            let resp = ui.interact(c, ui.id().with(("tool", i)), Sense::click()).on_hover_text(tr(tool.3));
            let on = self.tool == tool.0;
            if on || resp.hovered() {
                p.rect_filled(c, 0.0, if on { t::tint() } else { t::tint().gamma_multiply(0.5) });
            }
            if on {
                ui::grad_v(&p, Rect::from_min_size(c.min, vec2(2.0, c.height())));
            }
            p.hline(c.x_range(), c.bottom() - 0.5, Stroke::new(1.0, t::LINE_SOFT));
            let top = c.top() + (step - if labels { 34.0 } else { 18.0 }) / 2.0;
            let icon = Rect::from_min_size(pos2(c.center().x - 9.0, top), Vec2::splat(18.0));
            self.icons.tools[i].paint(&p, icon, 1.4, if on { t::ACCENT } else { t::TEXT_MID });
            if labels {
                let color = if on { t::ACCENT_TEXT } else { t::DIM };
                p.text(pos2(c.center().x, top + 29.0), Align2::CENTER_CENTER, tr(tool.1), sans(9.5), color);
            }
            if resp.clicked() {
                self.tool = tool.0;
            }
        }
        p.vline(r.right() - 0.5, r.y_range(), Stroke::new(1.0, t::LINE));
    }

    /// Boutons d'ancrage et de repli à droite de l'en-tête d'un panneau. L'en-tête lui-même
    /// se glisse vers un autre bord de la fenêtre. Renvoie l'abscisse gauche des boutons.
    fn dock_controls(&mut self, ui: &Ui, head: Rect, which: Panel) -> f32 {
        let dock = *self.prefs.dock(which);
        let cell = |k: f32| Rect::from_min_size(pos2(head.right() - 4.0 - 26.0 * k, head.top()), vec2(26.0, head.height() - 1.0));
        let grip = ui.interact(head.with_max_x(cell(2.0).left()), ui.id().with(("grip", which as u8)), Sense::drag());
        if grip.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::Grab);
        }
        if grip.drag_started() {
            self.docking = Some(which);
        }
        // Le chevron pointe vers le bord où le panneau se replie, ou à l'opposé pour le déplier.
        let toward = if dock.open { dock.side } else { dock.side.opposite() };
        let chevron =
            &self.icons.chevrons[[Side::Left, Side::Right, Side::Top, Side::Bottom].iter().position(|s| *s == toward).unwrap_or(0)];
        let tip = if dock.open { "Replier le panneau" } else { "Déplier le panneau" };
        if ui::icon_button(ui, cell(1.0), ("fold", which as u8), chevron, t::DIM, tip).clicked() {
            self.prefs.dock(which).open = !dock.open;
        }
        if ui::icon_button(ui, cell(2.0), ("swap", which as u8), &self.icons.swap, t::DIM, "Ancrer de l'autre côté (ou glisser l'en-tête)")
            .clicked()
        {
            self.prefs.dock(which).side = dock.side.opposite();
        }
        cell(2.0).left()
    }

    /// Filet du panneau du côté du canevas.
    fn panel_edge(&mut self, ui: &Ui, r: Rect, which: Panel) {
        let x = if self.prefs.dock(which).side == Side::Left { r.right() - 0.5 } else { r.left() + 0.5 };
        ui.painter().vline(x, r.y_range(), Stroke::new(1.0, t::LINE));
    }

    /// Panneau latéral replié : une bande étroite portant son titre, qui se déplie d'un clic.
    pub(super) fn collapsed(&mut self, ui: &mut Ui, which: Panel) {
        let r = begin(ui);
        let p = ui.painter().clone();
        let resp = ui.interact(r, ui.id().with(("unfold", which as u8)), Sense::click()).on_hover_text(tr("Déplier le panneau"));
        if resp.hovered() {
            p.rect_filled(r, 0.0, t::tint().gamma_multiply(0.5));
        }
        let color = if resp.hovered() { t::TEXT_HI } else { t::DIM };
        // Le chevron pointe vers le canevas, là où le panneau se dépliera.
        let chevron = &self.icons.chevrons[if self.prefs.dock(which).side == Side::Left { 1 } else { 0 }];
        chevron.paint(&p, Rect::from_center_size(pos2(r.center().x, r.top() + 18.0), Vec2::splat(14.0)), 1.3, color);
        let title = tr(if which == Panel::Library { "BIBLIOTHÈQUE" } else { "INSPECTEUR" });
        let length = ui::text_width(&p, title, mono_bold(10.5)) + 1.26 * title.chars().count() as f32;
        ui::vertical_title(&p, r.center().x, r.top() + 40.0 + length, title, color);
        self.panel_edge(ui, r, which);
        if resp.clicked() {
            self.prefs.dock(which).open = true;
        }
    }

    pub(super) fn library(&mut self, ui: &mut Ui) {
        let r = begin(ui);
        let p = ui.painter().clone();
        let needle = self.search.to_lowercase();
        let shown: Vec<usize> = (0..self.scene.materials.len())
            .filter(|&k| {
                let m = &self.scene.materials[k];
                self.lib_filter.is_none_or(|c| c == m.class) && tr(&m.name).to_lowercase().contains(&needle)
            })
            .collect();

        let head = ui::row(ui, 36.0, t::LINE);
        ui::tracked(&p, pos2(head.left() + PAD, head.center().y), Align2::LEFT_CENTER, tr("BIBLIOTHÈQUE"), mono_bold(10.5), t::TEXT_HI);
        let controls = self.dock_controls(ui, head, Panel::Library);
        p.text(pos2(controls - 8.0, head.center().y), Align2::RIGHT_CENTER, shown.len().to_string(), mono(10.5), t::DIM);

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
                .hint_text(tr("Rechercher un matériau"))
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
            let w = ui::text_width(&p, tr(name), font);
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
            p.text(c.left_center(), Align2::LEFT_CENTER, tr(name), if on { sans_bold(11.5) } else { sans(11.5) }, color);
            if on {
                ui::grad_h(&p, Rect::from_min_max(pos2(c.left(), c.bottom() - 3.0), pos2(c.right(), c.bottom() - 1.0)));
            }
            if resp.clicked() {
                self.lib_filter = *class;
            }
        }

        let columns = ui::row(ui, 26.0, t::LINE);
        ui::tracked(&p, pos2(columns.left() + PAD, columns.center().y), Align2::LEFT_CENTER, tr("MATÉRIAU"), mono(10.0), t::DIM);
        ui::tracked(&p, pos2(columns.right() - PAD, columns.center().y), Align2::RIGHT_CENTER, tr("VALEUR"), mono(10.0), t::DIM);

        // Liste des matériaux : elle occupe la place laissée par l'arbre de scène.
        let scene_rows = self.scene.objects.len().min(6) as f32;
        let list_h = (ui.available_height() - 32.0 - 30.0 * scene_rows).max(54.0);
        let list = Rect::from_min_size(ui.cursor().min, vec2(r.width(), list_h));
        let (mut picked, mut dragged) = (None, None);
        ui.allocate_ui(list.size(), |ui| {
            egui::ScrollArea::vertical().id_salt("materials").auto_shrink(false).show(ui, |ui| {
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                for &k in &shown {
                    let m = &self.scene.materials[k];
                    let value = match m.class {
                        MagClass::Magnet => {
                            let (br, unit) = units::remanence(m.br);
                            format!("Br {br:.2} {unit}")
                        }
                        MagClass::Ferro => format!("μr {:.0}", m.mu_r),
                        MagClass::Superconductor => format!("Tc {:.0} K", m.t_critical - ABSOLUTE_ZERO_C),
                        _ => format!("χ {:+.1e}", units::susceptibility(m.chi)),
                    };
                    let row =
                        ui::list_row(ui, 27.0, ("material", k), self.lib_material == m.name, t::class_color(m.class), tr(&m.name), &value);
                    if row.clicked() {
                        picked = Some(m.name.clone());
                    }
                    if row.drag_started() {
                        dragged = Some(m.name.clone());
                    }
                }
                ui.add_space(30.0);
            });
        });
        ui::fade_down(&p, Rect::from_min_max(pos2(list.left(), list.bottom() - 36.0), list.right_bottom()), t::PANEL);
        // Un matériau se glisse sur un objet du canevas ; son étiquette suit le curseur.
        if dragged.is_some() {
            self.drag_material = dragged;
        }
        if let Some(name) = &self.drag_material
            && let Some(at) = ui.ctx().pointer_latest_pos()
        {
            ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
            let color = self.scene.material(name).map_or(t::DIM, |m| t::class_color(m.class));
            let top = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("drag-material")));
            ui::tag(&top, at + vec2(14.0, 30.0), tr(name), color, t::TEXT_HI);
        }
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
            for k in 0..self.scene.objects.len() {
                self.object_row(ui, k);
            }
        });
        self.panel_edge(ui, r, Panel::Library);
    }

    /// Ligne de l'arbre de scène : pastille, nom, matériau, puis visibilité et verrouillage.
    fn object_row(&mut self, ui: &mut Ui, index: usize) {
        let o = &self.scene.objects[index];
        let (id, visible, locked, selected) = (o.id, o.visible, o.locked, self.selected == Some(o.id));
        let color = self.scene.material(&o.material).map_or(t::DIM, |m| t::class_color(m.class));
        let r = ui::row(ui, 30.0, t::LINE_SOFT);
        let resp = ui.interact(r, ui.id().with(("object", id)), Sense::click());
        let hovered = ui.rect_contains_pointer(r);
        let p = ui.painter().clone();
        if selected || hovered {
            p.rect_filled(r.shrink2(vec2(0.0, 0.5)), 0.0, if selected { t::tint() } else { t::tint().gamma_multiply(0.5) });
        }
        if selected {
            ui::grad_v(&p, Rect::from_min_size(r.min, vec2(2.0, r.height())));
        }
        let y = r.center().y;
        // Un objet masqué s'estompe dans la liste.
        let fade = |c: Color32| if visible { c } else { c.gamma_multiply(0.45) };
        p.rect_filled(Rect::from_center_size(pos2(r.left() + PAD + 3.0, y), vec2(6.0, 6.0)), 0.0, fade(color));
        let buttons = r.right() - 6.0 - 2.0 * 22.0;
        let material = short_name(tr(&o.material));
        let value = p.text(
            pos2(buttons - 6.0, y),
            Align2::RIGHT_CENTER,
            material,
            mono(11.0),
            fade(if selected { t::ACCENT_TEXT } else { t::DIM }),
        );
        let clip = Rect::from_min_max(pos2(r.left() + PAD + 16.0, r.top()), pos2(value.left() - 8.0, r.bottom()));
        let name_color = fade(if selected { t::TEXT_HI } else { t::TEXT_MID });
        p.with_clip_rect(clip).text(clip.left_center(), Align2::LEFT_CENTER, &o.name, sans(12.0), name_color);
        if resp.clicked() {
            self.selected = Some(id);
        }
        // Les deux boutons n'apparaissent qu'au survol, ou quand l'objet est masqué ou verrouillé.
        let cell = |k: f32| Rect::from_min_size(pos2(buttons + 22.0 * k, r.top()), vec2(22.0, r.height() - 1.0));
        if hovered || !visible {
            let (icon, tip) =
                if visible { (&self.icons.eye, "Masquer : l'objet est retiré du calcul") } else { (&self.icons.eye_off, "Afficher") };
            if ui::icon_button(ui, cell(0.0), ("eye", id), icon, if visible { t::FAINT } else { t::ACCENT }, tip).clicked() {
                self.scene.objects[index].visible = !visible;
            }
        }
        if hovered || locked {
            let (icon, tip) = if locked {
                (&self.icons.lock, "Déverrouiller")
            } else {
                (&self.icons.unlock, "Verrouiller : ni déplacement, ni rotation, ni suppression sur le canevas")
            };
            if ui::icon_button(ui, cell(1.0), ("lock", id), icon, if locked { t::ACCENT } else { t::FAINT }, tip).clicked() {
                self.scene.objects[index].locked = !locked;
            }
        }
    }

    pub(super) fn inspector(&mut self, ui: &mut Ui) {
        let r = begin(ui);
        egui::ScrollArea::vertical().id_salt("inspector").auto_shrink(false).show(ui, |ui| {
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            match self.selected.filter(|&id| self.scene.get(id).is_some()) {
                Some(id) => self.inspect_object(ui, id),
                None => self.inspect_scene(ui),
            }
        });
        self.panel_edge(ui, r, Panel::Inspector);
    }

    fn inspect_scene(&mut self, ui: &mut Ui) {
        let head = ui::section(ui, "01", "SCÈNE");
        self.dock_controls(ui, head, Panel::Inspector);
        ui::kv_edit(ui, "Profondeur", "mm", |ui| ui::number(ui, &mut self.scene.depth, Quantity::Length, 0.1, 1));
        ui::kv_edit(ui, "Domaine", "mm", |ui| ui::number(ui, &mut self.scene.size, Quantity::Length, 1.0, 0));
        let kelvin = format!("°C · {} K", fr(self.scene.ambient - ABSOLUTE_ZERO_C, 2));
        ui::kv_edit(ui, "T ambiante", &kelvin, |ui| ui::number(ui, &mut self.scene.ambient, Quantity::Temperature, 0.5, 0));
        self.scene.depth = self.scene.depth.max(1e-4);
        self.scene.size = self.scene.size.clamp(0.01, 10.0);

        ui::section(ui, "02", "MÉCANIQUE");
        let views = [(MechView::Top, tr("Dessus")), (MechView::Side, tr("Côté"))];
        ui::segmented(ui, "Vue", "mech-view", &mut self.scene.mechanics.view, &views);
        ui::kv_edit(ui, "Pesanteur", "m/s²", |ui| ui::number(ui, &mut self.scene.mechanics.gravity, Quantity::Count, 0.01, 2));
        self.scene.mechanics.gravity = self.scene.mechanics.gravity.clamp(0.0, 100.0);
        let speeds = [(0.1, format!("×{}", fr(0.1, 1))), (0.25, format!("×{}", fr(0.25, 2))), (1.0, "×1".to_owned())];
        ui::segmented(ui, "Vitesse", "sim-speed", &mut self.sim.speed, &speeds.each_ref().map(|(v, name)| (*v, name.as_str())));
        let play = if self.sim.playing { "Pause" } else { "Lecture" };
        match ui::button_row(ui, "sim", &[(play, true), ("Un pas", true), ("Revenir", self.sim.started())]) {
            Some(0) => self.toggle_play(),
            Some(1) => self.step_simulation(),
            Some(_) => self.rewind(),
            None => {}
        }
        let text = match self.scene.mechanics.view {
            MechView::Top => "Vue de dessus : les objets mobiles reposent sur une table et ne démarrent que si la force dépasse μs·m·g.",
            MechView::Side => "Vue de côté : la pesanteur agit vers le bas ; les objets fixes et le bord du domaine servent d’appuis.",
        };
        ui::note(ui, tr(text), t::FAINT);

        ui::section(ui, "03", "THERMIQUE");
        ui::switch(ui, "Bilan thermique", &mut self.scene.thermal.enabled);
        ui::kv_edit(ui, "Convection h", "W/(m²·K)", |ui| ui::number(ui, &mut self.scene.thermal.convection, Quantity::Count, 0.5, 1));
        self.scene.thermal.convection = self.scene.thermal.convection.clamp(0.0, 1e4);
        ui::segmented(ui, "Accélération", "thermal-speed", &mut self.scene.thermal.speed, &[(1.0, "×1"), (10.0, "×10"), (100.0, "×100")]);
        ui::note(
            ui,
            tr(
                "Pendant la lecture, chaque objet échange de la chaleur avec l’air (convection et rayonnement) et les bobines chauffent par effet Joule. Un objet à « température imposée » garde la sienne.",
            ),
            t::FAINT,
        );

        ui::section(ui, "04", "CALCUL");
        let mut gpu = self.use_gpu;
        if ui::segmented(ui, "Moteur", "engine", &mut gpu, &[(true, "GPU f32"), (false, "CPU f64")]) {
            self.set_solver(gpu);
        }
        ui::segmented(ui, "Grille", "grid", &mut self.grid_n, &[(256, "256²"), (512, "512²"), (1024, "1024²"), (2048, "2048²")]);
        ui::kv(ui, "Pas", &fr(self.scene.size / self.grid_n as f64 * 1e3, 3), "mm");
        let shown = format!("{:.0}", self.n_lines);
        ui::slider(ui, "Lignes", 44.0, &mut self.n_lines, 8.0..=160.0, Some(shown), true);
        self.n_lines = self.n_lines.round();
        ui::switch(ui, "Désaimantation des aimants", &mut self.scene.demagnetization);

        ui::section(ui, "05", "AFFICHAGE");
        ui::switch(ui, "Animer la LIC", &mut self.lic_animate);
        let clear = [("Effacer les graines", !self.scene.seeds.is_empty()), ("Balayer la limaille", !self.visuals.filings.is_empty())];
        match ui::button_row(ui, "clear", &clear) {
            Some(0) => self.scene.seeds.clear(),
            Some(_) => self.visuals.filings.clear(),
            None => {}
        }

        ui::section(ui, "06", "COMPARAISON");
        if ui::accent_button(ui, "Figer l’état actuel comme référence", &self.icons.arrow) {
            self.freeze_reference();
        }
        let has_reference = self.reference.is_some();
        let options = [(Compare::Off, tr("Aucune")), (Compare::Split, tr("Avant | après")), (Compare::Diff, tr("Différence"))];
        ui::tabs(ui, "compare", &mut self.compare, &options, has_reference);
        ui::slider(ui, "Séparation", 72.0, &mut self.split, 0.05..=0.95, None, has_reference && self.compare == Compare::Split);

        ui.add_space((ui.available_height() - 66.0).max(0.0));
        ui::note(ui, tr("Sélectionnez un objet pour l’inspecter, ou choisissez un outil et dessinez sur le canevas."), t::ACCENT);
    }

    fn inspect_object(&mut self, ui: &mut Ui, id: u32) {
        let names: Vec<String> = self.scene.materials.iter().map(|m| m.name.clone()).collect();
        let (depth, ambient) = (self.scene.depth, self.scene.ambient);
        let (view, gravity) = (self.scene.mechanics.view, self.scene.mechanics.gravity);
        let inside = self.scene.get(id).and_then(|o| mean_b(self.solver.field(), o));
        let motion = self.sim.world().and_then(|w| w.motion(id));
        let wrench = self.wrenches.iter().find(|w| w.id == id).copied();
        let mat = self.scene.get(id).and_then(|o| self.scene.material(&o.material)).cloned();
        let (thermal, demagnetization) = (self.scene.thermal, self.scene.demagnetization);
        let (tau, resistance) = self.scene.get(id).map_or((0.0, None), |o| (self.scene.time_constant(o), self.scene.resistance(o)));
        let reverse = self.scene.get(id).zip(mat.as_ref()).and_then(|(o, m)| reverse_field(self.solver.field(), o, m, !demagnetization));
        let mut import_bh = false;
        let focus_angle = std::mem::take(&mut self.focus_angle);
        let mut count = 0;
        let mut next = move || {
            count += 1;
            format!("{count:02}")
        };

        let head = ui::section(ui, &next(), "OBJET");
        self.dock_controls(ui, head, Panel::Inspector);
        let Some(o) = self.scene.get_mut(id) else { return };
        ui::kv_edit(ui, "Nom", "", |ui| {
            let edit = egui::TextEdit::singleline(&mut o.name).frame(egui::Frame::NONE).font(mono(12.0)).text_color(t::TEXT_HI);
            ui.add(edit.desired_width(170.0).horizontal_align(Align::RIGHT).margin(egui::Margin::ZERO))
        });
        ui::kv_edit(ui, "Matériau", "", |ui| {
            let shown = egui::RichText::new(tr(&o.material)).font(mono(11.5)).color(t::TEXT_HI);
            egui::ComboBox::from_id_salt("material").width(184.0).selected_text(shown).show_ui(ui, |ui| {
                for name in &names {
                    ui.selectable_value(&mut o.material, name.clone(), tr(name));
                }
            });
        });
        let kelvin = format!("°C · {} K", fr(o.temperature - ABSOLUTE_ZERO_C, 2));
        ui::kv_edit(ui, "Température", &kelvin, |ui| ui::number(ui, &mut o.temperature, Quantity::Temperature, 1.0, 0));
        ui::switch(ui, "Température imposée", &mut o.thermostat);
        if thermal.enabled && !o.thermostat && tau > 0.0 {
            ui::kv(ui, "Constante de temps thermique", &fr(tau / thermal.speed, 0), "s");
        }
        if !o.visible {
            ui::note(ui, tr("Objet masqué : il est retiré du calcul."), t::WARN);
        }
        if o.locked {
            ui::note(ui, tr("Objet verrouillé : il ne peut être ni déplacé, ni tourné, ni supprimé sur le canevas."), t::FAINT);
        }

        ui::section(ui, &next(), "GÉOMÉTRIE");
        ui::kv_edit(ui, "Position X", "mm", |ui| ui::number(ui, &mut o.pos.x, Quantity::Length, 0.1, 1));
        ui::kv_edit(ui, "Position Y", "mm", |ui| ui::number(ui, &mut o.pos.y, Quantity::Length, 0.1, 1));
        ui::kv_edit(ui, "Rotation", "°", |ui| ui::number(ui, &mut o.angle, Quantity::Angle, 1.0, 0));
        let length = |ui: &mut Ui, label: &str, v: &mut f64| {
            ui::kv_edit(ui, label, "mm", |ui| ui::number(ui, v, Quantity::Length, 0.1, 1));
            *v = v.max(2e-4);
        };
        match &mut o.shape {
            Shape::Rect { w, h } => {
                length(ui, "Largeur", w);
                length(ui, "Hauteur", h);
            }
            Shape::Coil { w, h, thick } => {
                length(ui, "Largeur", w);
                length(ui, "Hauteur", h);
                length(ui, "Épaisseur du bobinage", thick);
                // Les deux sections ne se recouvrent pas.
                *thick = thick.min(*w / 2.0);
            }
            Shape::Circle { r } => length(ui, "Rayon", r),
            Shape::Ellipse { rx, ry } => {
                length(ui, "Demi-axe X", rx);
                length(ui, "Demi-axe Y", ry);
            }
            Shape::Ring { r_in, r_out } => {
                length(ui, "Rayon intérieur", r_in);
                length(ui, "Rayon extérieur", r_out);
                *r_out = r_out.max(*r_in + 2e-4);
            }
            Shape::Polygon { pts } => ui::kv(ui, "Sommets", &pts.len().to_string(), ""),
            Shape::Region { contours } => {
                ui::kv(ui, "Contours", &contours.len().to_string(), "");
                ui::kv(ui, "Sommets", &contours.iter().map(Vec::len).sum::<usize>().to_string(), "");
            }
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
        ui::section(ui, &next(), title);
        match mat.class {
            MagClass::Magnet => {
                // Motif d'aimantation : les formes rondes suivent l'angle polaire, les autres leur longueur.
                let round = o.magnetization().is_round();
                let label = |pattern: &MagPattern| match pattern {
                    MagPattern::Uniform if round => "Diamétrale",
                    MagPattern::Uniform => "Uniforme",
                    MagPattern::Radial => "Radiale",
                    MagPattern::Multipole { .. } => "Multipolaire",
                    MagPattern::Halbach { .. } => "Halbach",
                    MagPattern::Painted => "Peinte",
                };
                let halbach = MagPattern::Halbach { pairs: if round { 1 } else { 2 }, flip: false };
                let kinds = [MagPattern::Uniform, MagPattern::Radial, MagPattern::Multipole { pairs: 2 }, halbach, MagPattern::Painted];
                ui::kv_edit(ui, "Motif", "", |ui| {
                    let shown = egui::RichText::new(tr(label(&o.pattern))).font(mono(11.5)).color(t::TEXT_HI);
                    egui::ComboBox::from_id_salt("pattern").width(184.0).selected_text(shown).show_ui(ui, |ui| {
                        for kind in kinds {
                            let on = std::mem::discriminant(&kind) == std::mem::discriminant(&o.pattern);
                            if ui.selectable_label(on, tr(label(&kind))).clicked() && !on {
                                if kind == MagPattern::Painted {
                                    // Le motif peint part des directions du motif qu'il remplace.
                                    o.painted();
                                } else {
                                    (o.pattern, o.paint) = (kind, None);
                                }
                            }
                        }
                    });
                });
                if o.pattern != MagPattern::Painted {
                    let angle = ui::kv_edit(ui, "Angle", "°", |ui| ui::number(ui, &mut o.mag_angle, Quantity::Angle, 1.0, 0));
                    if focus_angle {
                        angle.request_focus();
                    }
                }
                if let MagPattern::Multipole { pairs } | MagPattern::Halbach { pairs, .. } = &mut o.pattern {
                    let mut value = *pairs as f64;
                    let shown = pairs.to_string();
                    ui::slider(ui, "Paires de pôles", 110.0, &mut value, 1.0..=12.0, Some(shown), true);
                    *pairs = value.round().clamp(1.0, 12.0) as u32;
                }
                if let MagPattern::Halbach { flip, .. } = &mut o.pattern {
                    let sides = if round { [tr("Intérieur"), tr("Extérieur")] } else { [tr("Dessus"), tr("Dessous")] };
                    ui::segmented(ui, "Côté fort", "halbach-side", flip, &[(false, sides[0]), (true, sides[1])]);
                }
                if o.pattern == MagPattern::Painted {
                    ui::note(ui, tr("Glissez le pinceau (N) sur l’aimant : l’aimantation suit le geste."), t::ACCENT);
                }
                if ui::button_row(ui, "flip", &[("Inverser les pôles", true)]).is_some() {
                    match (o.pattern, &mut o.paint) {
                        (MagPattern::Painted, Some(lattice)) => lattice.values.iter_mut().for_each(|a| *a += std::f32::consts::PI),
                        _ => o.mag_angle += std::f64::consts::PI,
                    }
                }
                for (label, tesla) in [("Br à 20 °C", mat.br), ("Br à la température de l’objet", mat.br_at(o.temperature))] {
                    let (br, unit) = units::remanence(tesla);
                    ui::kv(ui, label, &fr(br, 3), unit);
                }
                ui::kv(ui, "μrec", &fr(mat.mu_r, 2), "");
                if mat.hcj > 0.0 {
                    let (value, unit) = units::coercivity(mat.hcj_at(o.temperature));
                    ui::kv(ui, "HcJ à la température de l’objet", &value, unit);
                }
                if mat.t_max > 0.0 {
                    let color = if o.temperature > mat.t_max { t::WARN } else { t::TEXT_HI };
                    ui::kv_colored(ui, "T max d’emploi", &fr(mat.t_max, 0), "°C", color);
                }
                ui::kv(ui, "Tc", &fr(mat.t_curie, 0), "°C");
                // Tenue à la désaimantation : champ inverse rapporté au coude de la courbe.
                if mat.hcj > 0.0 && demagnetization {
                    if let Some(h) = reverse {
                        let ratio = (h / (KNEE * mat.hcj_at(o.temperature).max(1.0))).max(0.0);
                        ui::gauge(ui, "Champ inverse / coude", ratio, 1.5, &format!("{} %", fr(100.0 * ratio, 0)), t::WARN);
                    }
                    let left = o.mean_remanence_left();
                    ui::kv_colored(ui, "Rémanence restante", &fr(100.0 * left, 1), "%", if left < 0.995 { t::WARN } else { t::TEXT_HI });
                    if o.demag.is_some() {
                        if ui::button_row(ui, "remagnetize", &[("Ré-aimanter", true)]).is_some() {
                            o.demag = None;
                        }
                        let text = "Désaimantation irréversible : le champ inverse a dépassé le coude de la courbe. L’aimant ne retrouve sa rémanence qu’une fois ré-aimanté.";
                        ui::note(ui, tr(text), t::WARN);
                    }
                } else if mat.hcj > 0.0 {
                    ui::note(ui, tr("Désaimantation ignorée : réglage « Désaimantation des aimants » de la scène."), t::FAINT);
                }
            }
            MagClass::Ferro => {
                match mat.curve_at(o.temperature) {
                    Some(curve) => {
                        let (table, js) = (curve.table(), curve.js());
                        ui::kv(ui, "μr initiale", &fr(curve.mu_r_initial(), 0), "");
                        let (value, unit) = units::remanence(js);
                        ui::kv(ui, "Js (saturation)", &fr(value, 2), unit);
                        let factor = mat.js_factor(o.temperature);
                        if factor != 1.0 {
                            // Loi de Kuz'min : la saturation baisse à l'approche de Tc.
                            ui::kv(ui, "Js(T) / Js de référence", &fr(100.0 * factor, 1), "%");
                        }
                        if let Some(b) = inside {
                            ui::kv(ui, "B dans l’objet", &units::b(b), "");
                            ui::kv(ui, "μr effective", &fr(1.0 / table.eval(b).0, 0), "");
                            ui::gauge(ui, "Saturation B / Js", b / js, 1.25, &format!("{} %", fr(100.0 * b / js, 0)), t::WARN);
                        }
                        bh_plot(ui, &table, js, inside);
                    }
                    None if mat.is_ferromagnetic(o.temperature) => ui::kv(ui, "μr (linéaire)", &fr(mat.mu_r_solver(o.temperature), 0), ""),
                    None => {
                        ui::kv_colored(ui, "État", tr("paramagnétique"), "", t::WARN);
                        ui::kv(ui, "χ (Curie–Weiss)", &format!("{:+.2e}", units::susceptibility(mat.chi_at(o.temperature))), "");
                        let text = "Au-dessus de sa température de Curie, le matériau n’est plus que paramagnétique : χ = C / (T − Tc).";
                        ui::note(ui, tr(text), t::WARN);
                    }
                }
                ui::kv(ui, "Tc", &fr(mat.t_curie, 0), "°C");
                if ui::button_row(ui, "bh", &[("Importer une courbe B(H)…", true)]).is_some() {
                    import_bh = true;
                }
            }
            MagClass::Conductor => {
                ui::kv_edit(ui, "Spires", "", |ui| ui::number(ui, &mut o.turns, Quantity::Count, 1.0, 0));
                o.turns = o.turns.max(0.0);
                ui::kv_edit(ui, "Courant", "A", |ui| ui::number(ui, &mut o.current, Quantity::Current, 0.1, 2));
                let sense = if o.current >= 0.0 { "Sortant — inverser le sens" } else { "Entrant — inverser le sens" };
                if ui::button_row(ui, "flip", &[(sense, true)]).is_some() {
                    o.current = -o.current;
                }
                ui::kv(ui, "Ampères-tours", &fr(o.amp_turns(), 0), "A");
                // Dans une bobine, chaque section porte tous les ampères-tours.
                let j = o.amp_turns().abs() * o.shape.passes() / area * 1e-6;
                // Au-delà d'environ 5 A/mm² en continu, le bobinage chauffe.
                ui::kv_colored(ui, "Densité J", &fr(j, 2), "A/mm²", if j > 5.0 { t::WARN } else { t::TEXT_HI });
                if o.turns > 1.0 {
                    ui::kv_edit(ui, "Remplissage", "", |ui| ui::number(ui, &mut o.fill, Quantity::Count, 0.01, 2));
                    o.fill = o.fill.clamp(0.05, 1.0);
                }
                if let Some(r) = resistance {
                    // ρ(T) = ρ20·[1 + 0,00393·(T − 20)] : la bobine résiste davantage en chauffant.
                    ui::kv(ui, "Résistivité", &fr(mat.resistivity_at(o.temperature) * 1e9, 2), "nΩ·m");
                    ui::kv(ui, "Résistance", &units::number(r * 1e3), "mΩ");
                    ui::kv(ui, "Tension", &units::number(r * o.current.abs()), "V");
                    ui::kv(ui, "Puissance Joule", &units::number(r * o.current * o.current), "W");
                }
            }
            MagClass::Para | MagClass::Dia => {
                let chi = mat.chi_at(o.temperature);
                match mat.weak_chi(o.temperature) {
                    Some([perp, par]) if perp != par => {
                        ui::kv(ui, "χ en travers des feuillets", &format!("{:+.2e}", units::susceptibility(perp)), "");
                        ui::kv(ui, "χ le long des feuillets", &format!("{:+.2e}", units::susceptibility(par)), "");
                        ui::note(ui, tr("Matériau anisotrope : ses feuillets suivent la largeur de l’objet (axe X local)."), t::FAINT);
                    }
                    _ => ui::kv(ui, "χ", &format!("{:+.2e}", units::susceptibility(chi)), ""),
                }
                let text = format!(
                    "{} {} % {}",
                    tr("Modifie le champ d’environ"),
                    fr(chi.abs() * 50.0, 4),
                    tr(
                        "(χ/2) : invisible sur les lignes de champ. L’objet subit en revanche une force (densité de Kelvin), donnée ci-dessous."
                    )
                );
                ui::note(ui, &text, t::FAINT);
            }
            MagClass::Superconductor => {
                ui::kv(ui, "Tc", &fr(mat.t_critical - ABSOLUTE_ZERO_C, 1), "K");
                let cold = mat.is_superconducting(o.temperature);
                ui::kv_colored(ui, "État", tr(if cold { "Meissner" } else { "normal" }), "", if cold { t::ACCENT_TEXT } else { t::WARN });
                match ui::button_row(ui, "cool", &[("Azote liquide", true), ("Hélium liquide", true), ("Ambiante", true)]) {
                    Some(0) => o.temperature = -196.0,
                    Some(1) => o.temperature = -269.0,
                    Some(_) => o.temperature = ambient,
                    None => {}
                }
                let text = if cold { "Le champ est expulsé (χ = −1)." } else { "Refroidir sous Tc pour expulser le champ." };
                ui::note(ui, tr(text), if cold { t::ACCENT } else { t::WARN });
            }
        }
        let locked = o.locked;

        // Mécanique : fixe ou mobile, frottement, liaison avec le support.
        ui::section(ui, &next(), "MÉCANIQUE");
        ui::segmented(ui, "État", "mobile", &mut o.body.mobile, &[(false, tr("Fixe")), (true, tr("Mobile"))]);
        if o.body.mobile {
            let preset = SURFACES.iter().find(|s| (s.1, s.2) == (o.body.mu_s, o.body.mu_k)).map_or("Personnalisé", |s| s.0);
            ui::kv_edit(ui, "Surface", "", |ui| {
                let shown = egui::RichText::new(tr(preset)).font(mono(11.5)).color(t::TEXT_HI);
                egui::ComboBox::from_id_salt("surface").width(184.0).selected_text(shown).show_ui(ui, |ui| {
                    for (name, mu_s, mu_k) in SURFACES {
                        if ui.selectable_label(name == preset, tr(name)).clicked() {
                            (o.body.mu_s, o.body.mu_k) = (mu_s, mu_k);
                        }
                    }
                });
            });
            ui::kv_edit(ui, "μs (statique)", "", |ui| ui::number(ui, &mut o.body.mu_s, Quantity::Count, 0.005, 3));
            ui::kv_edit(ui, "μk (dynamique)", "", |ui| ui::number(ui, &mut o.body.mu_k, Quantity::Count, 0.005, 3));
            o.body.mu_s = o.body.mu_s.clamp(0.0, 5.0);
            o.body.mu_k = o.body.mu_k.clamp(0.0, 5.0);

            let center = o.pos.truncate();
            let kinds = [Link::Free, Link::Pivot { anchor: DVec2::ZERO }, Link::Slider { angle: 0.0 }];
            let spring = Link::Spring { anchor: center + DVec2::new(0.0, 0.03), stiffness: 50.0, damping: 0.05, length: 0.03 };
            let rope = Link::Rope { anchor: center + DVec2::new(0.0, 0.05), length: 0.05 };
            let name = |link: &Link| match link {
                Link::Free => "Libre",
                Link::Pivot { .. } => "Pivot",
                Link::Slider { .. } => "Glissière",
                Link::Spring { .. } => "Ressort",
                Link::Rope { .. } => "Fil (pendule)",
            };
            ui::kv_edit(ui, "Liaison", "", |ui| {
                let shown = egui::RichText::new(tr(name(&o.body.link))).font(mono(11.5)).color(t::TEXT_HI);
                egui::ComboBox::from_id_salt("link").width(184.0).selected_text(shown).show_ui(ui, |ui| {
                    for kind in kinds.into_iter().chain([spring, rope]) {
                        let on = std::mem::discriminant(&kind) == std::mem::discriminant(&o.body.link);
                        if ui.selectable_label(on, tr(name(&kind))).clicked() && !on {
                            o.body.link = kind;
                        }
                    }
                });
            });
            let length = |ui: &mut Ui, label: &str, v: &mut f64| {
                ui::kv_edit(ui, label, "mm", |ui| ui::number(ui, v, Quantity::Length, 0.1, 1));
            };
            match &mut o.body.link {
                Link::Free => {}
                Link::Pivot { anchor } => {
                    length(ui, "Pivot X (local)", &mut anchor.x);
                    length(ui, "Pivot Y (local)", &mut anchor.y);
                }
                Link::Slider { angle } => {
                    ui::kv_edit(ui, "Axe", "°", |ui| ui::number(ui, angle, Quantity::Angle, 1.0, 0));
                }
                Link::Spring { anchor, stiffness, damping, length: rest } => {
                    length(ui, "Ancrage X", &mut anchor.x);
                    length(ui, "Ancrage Y", &mut anchor.y);
                    ui::kv_edit(ui, "Raideur", "N/m", |ui| ui::number(ui, stiffness, Quantity::Count, 1.0, 1));
                    ui::kv_edit(ui, "Amortissement", "N·s/m", |ui| ui::number(ui, damping, Quantity::Count, 0.01, 3));
                    length(ui, "Longueur au repos", rest);
                    (*stiffness, *damping, *rest) = (stiffness.max(0.0), damping.max(0.0), rest.max(0.0));
                }
                Link::Rope { anchor, length: rest } => {
                    length(ui, "Ancrage X", &mut anchor.x);
                    length(ui, "Ancrage Y", &mut anchor.y);
                    length(ui, "Longueur du fil", rest);
                    *rest = rest.max(1e-4);
                }
            }
            if let Some(m) = motion {
                ui::kv(ui, "Vitesse", &fr(m.velocity.length() * 1e3, 1), "mm/s");
                let state = if m.sliding { "en mouvement" } else { "immobile" };
                ui::kv_colored(ui, "Mouvement", tr(state), "", if m.sliding { t::ACCENT_TEXT } else { t::TEXT_HI });
            }
        }
        let body = o.body;

        if let Some(w) = wrench {
            ui::section(ui, &next(), "FORCE");
            let f = w.force.truncate();
            let (value, unit) = units::force_per_length(f.length());
            ui::kv_colored(ui, "|F|", &value, unit, t::FORCE_TEXT);
            for (label, component) in [("Fx", f.x), ("Fy", f.y)] {
                let (value, unit) = units::force_per_length(component);
                ui::kv(ui, label, &value, unit);
            }
            let (value, unit) = units::force(f.length() * depth);
            ui::kv(ui, "|F| réelle", &value, unit);
            let (value, unit) = units::torque_per_length(w.torque);
            ui::kv(ui, "Couple", &value, unit);
            let weight = mat.density * area * gravity;
            if weight > 0.0 {
                ui::kv(ui, "F / (m·g)", &units::number(f.length() / weight), "");
            }
            // Jauge de glissement : sur la table, l'objet démarre quand F dépasse μs·m·g.
            if body.mobile && view == MechView::Top && weight > 0.0 && !matches!(body.link, Link::Pivot { .. }) {
                let pull = match body.link {
                    Link::Slider { angle } => f.dot(DVec2::from_angle(angle)).abs(),
                    _ => f.length(),
                };
                if body.mu_s > 0.0 {
                    let ratio = pull / (body.mu_s * weight);
                    ui::gauge(ui, "Glissement F / (μs·m·g)", ratio, 2.0, &units::number(ratio), t::ACCENT_TEXT);
                } else {
                    ui::note(ui, tr("Sans frottement statique : la moindre force met l’objet en mouvement."), t::FAINT);
                }
            }
            if w.resolution_limited {
                ui::note(ui, tr("Entrefer plus fin que la grille : valeur limitée par la résolution."), t::WARN);
            }
        }

        // Opérations booléennes avec un second objet, désigné ici ou par Maj + clic sur le canevas.
        let head = ui::section(ui, &next(), "COMBINER");
        let others: Vec<(u32, String)> =
            self.scene.objects.iter().filter(|other| other.id != id).map(|other| (other.id, other.name.clone())).collect();
        let mut operand = self.operand.filter(|&(a, _)| a == id).map(|(_, b)| b);
        ui::kv_edit(ui, "Avec", "", |ui| {
            let name =
                operand.and_then(|b| others.iter().find(|(other, _)| *other == b)).map_or(tr("Maj + clic sur un objet"), |(_, name)| name);
            let shown = egui::RichText::new(name).font(mono(11.5)).color(if operand.is_some() { t::TEXT_HI } else { t::DIM });
            egui::ComboBox::from_id_salt("operand").width(184.0).selected_text(shown).show_ui(ui, |ui| {
                for (other, name) in &others {
                    ui.selectable_value(&mut operand, Some(*other), name.as_str());
                }
            });
        });
        self.operand = operand.map(|b| (id, b));
        // Un objet verrouillé ne se combine pas : l'opération le remplacerait ou le supprimerait.
        let operand_locked = operand.and_then(|b| self.scene.get(b)).is_some_and(|other| other.locked);
        let ready = operand.is_some() && !locked && !operand_locked;
        let clicked = ui::button_row(ui, "boolean", &[("Union", ready), ("Intersection", ready), ("Différence", ready)]);
        if let (Some(k), Some(b)) = (clicked, operand) {
            match self.scene.boolean(id, b, [BoolOp::Union, BoolOp::Intersection, BoolOp::Difference][k]) {
                Some(_) => self.operand = None,
                None => self.message = tr("Opération sans résultat : il ne resterait aucune matière.").into(),
            }
        }
        if std::mem::take(&mut self.reveal_boolean) {
            ui.scroll_to_rect(head.with_max_y(head.bottom() + 66.0), Some(Align::BOTTOM));
        }
        if import_bh {
            self.import_bh(&mat.name);
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
        let info = format!("{} {} mm · {} {}", tr("longueur"), fr((b - a).length() * 1e3, 1), tr("max"), units::b(max));
        p.text(pos2(head.left() + 130.0, head.center().y), Align2::LEFT_CENTER, info, mono(10.5), t::DIM);
        let controls = self.dock_controls(ui, head, Panel::Graph);
        p.vline(controls - 4.0, head.y_range(), Stroke::new(1.0, t::LINE));
        let mut action = None;
        for (i, label) in ["Effacer", "Exporter CSV"].into_iter().enumerate() {
            let c = Rect::from_min_size(pos2(controls - 4.0 - 110.0 * (i + 1) as f32, head.top()), vec2(110.0, head.height() - 1.0));
            p.vline(c.left(), head.y_range(), Stroke::new(1.0, t::LINE));
            if ui::cell(ui, c, ("cut", i), tr(label), sans(12.0), t::TEXT_MID).clicked() {
                action = Some(i);
            }
        }
        if self.prefs.graph.open {
            let plot = Rect::from_min_max(pos2(r.left() + PAD, head.bottom() + 12.0), pos2(r.right() - PAD, r.bottom() - 12.0));
            p.hline(plot.x_range(), plot.bottom(), Stroke::new(1.0, t::LINE_CTRL));
            let points = samples
                .iter()
                .enumerate()
                .map(|(k, &v)| pos2(plot.left() + plot.width() * k as f32 / 240.0, plot.bottom() - plot.height() * (v / max) as f32));
            p.add(egui::Shape::line(points.collect(), Stroke::new(1.5, t::ACCENT)));
            p.text(plot.left_top(), Align2::LEFT_TOP, units::b(max), mono(10.0), t::DIM);
        }
        // Ancré en haut, le graphe est séparé du canevas par son bord inférieur.
        if self.prefs.graph.side == Side::Top {
            p.hline(r.x_range(), r.bottom() - 0.5, Stroke::new(1.0, t::LINE));
        }
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
        let state = tr(if self.pending { "CALCUL…" } else { "CONVERGÉ" });
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
        segment(format!("{} {}", s.iterations, tr("it.")), t::LABEL);
        segment(format!("{} {:.1e}", tr("résidu"), s.residual), t::LABEL);
        if let Some(newton) = self.newton.as_ref().filter(|n| n.raster.nonlinear.is_some()) {
            segment(format!("Newton {}", newton.iterations), t::LABEL);
        }
        segment(format!("{:.1} ms", s.elapsed.as_secs_f64() * 1e3), t::LABEL);
        if let Some(c) = self.cursor {
            segment(format!("x {} · y {} mm", fr(c.x * 1e3, 1), fr(c.y * 1e3, 1)), t::LABEL);
            if let Some(f) = self.solver.field().sample(c.extend(0.0)) {
                segment(format!("|B| {}", units::b(f.b.length())), t::TEXT_HI);
                segment(format!("Bx {} · By {}", units::b(f.b.x), units::b(f.b.y)), t::LABEL);
            }
        }
        if self.sim.playing {
            segment(format!("{} ×{}", tr("LECTURE"), fr(self.sim.rate, 2)), t::ACCENT_TEXT);
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
        segment(&[(format!("{fps:.0} {}", tr("IMG/S")), t::FORCE)]);
        let depth = format!(" · {} {} MM", tr("PROFONDEUR"), fr(self.scene.depth * 1e3, 0));
        segment(&[(tr("2D PLAN").into(), t::TEXT), (depth, t::LABEL)]);
    }
}
