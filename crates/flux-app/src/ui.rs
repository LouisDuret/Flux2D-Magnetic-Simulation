//! Boîte à outils de widgets au style « instrument » : lignes de hauteur fixe séparées
//! par des filets, contrôles à angles droits, dégradé ambre pour l'état actif.

use crate::expr::{self, Quantity};
use crate::lang::tr;
use crate::theme::{self as t, mono, mono_bold, sans, sans_bold};
use eframe::egui::{
    self, Align, Align2, Color32, FontId, Layout, Painter, Pos2, Rect, Response, Sense, Stroke, Ui, UiBuilder, epaint, pos2, vec2,
};
use std::hash::Hash;
use std::ops::RangeInclusive;

/// Marge horizontale des panneaux.
pub const PAD: f32 = 14.0;

/// Nombre décimal dans la langue de l'interface (virgule en français, point en anglais).
pub fn fr(v: f64, decimals: usize) -> String {
    crate::lang::decimal(v, decimals)
}

// ───────────────────────────── Peinture ─────────────────────────────

/// Dégradé linéaire à plusieurs arrêts (position dans [0, 1], couleur).
pub fn gradient(p: &Painter, r: Rect, horizontal: bool, stops: &[(f32, Color32)]) {
    let mut mesh = egui::Mesh::default();
    for (k, &(at, color)) in stops.iter().enumerate() {
        let (a, b) = if horizontal {
            let x = egui::lerp(r.x_range(), at);
            (pos2(x, r.top()), pos2(x, r.bottom()))
        } else {
            let y = egui::lerp(r.y_range(), at);
            (pos2(r.left(), y), pos2(r.right(), y))
        };
        mesh.colored_vertex(a, color);
        mesh.colored_vertex(b, color);
        if k > 0 {
            let i = 2 * k as u32;
            mesh.add_triangle(i - 2, i - 1, i);
            mesh.add_triangle(i - 1, i, i + 1);
        }
    }
    p.add(egui::Shape::mesh(mesh));
}

/// Dégradé d'accent horizontal (orange → jaune).
pub fn grad_h(p: &Painter, r: Rect) {
    gradient(p, r, true, &[(0.0, t::GRAD[0]), (0.55, t::GRAD[1]), (1.0, t::GRAD[2])]);
}

/// Dégradé d'accent vertical (jaune → orange).
pub fn grad_v(p: &Painter, r: Rect) {
    gradient(p, r, false, &[(0.0, t::GRAD[2]), (0.5, t::GRAD[1]), (1.0, t::GRAD[0])]);
}

/// Fondu vertical de transparent vers `color`.
pub fn fade_down(p: &Painter, r: Rect, color: Color32) {
    gradient(p, r, false, &[(0.0, Color32::TRANSPARENT), (0.6, color.gamma_multiply(0.7)), (1.0, color)]);
}

/// Texte en capitales espacées (titres de section, en-têtes de colonne).
pub fn tracked(p: &Painter, pos: Pos2, anchor: Align2, text: &str, font: FontId, color: Color32) -> Rect {
    let mut job = egui::text::LayoutJob::default();
    let format = egui::TextFormat { extra_letter_spacing: font.size * 0.12, font_id: font, color, ..Default::default() };
    job.append(text, 0.0, format);
    let galley = p.layout_job(job);
    let rect = anchor.anchor_size(pos, galley.size());
    p.galley(rect.min, galley, color);
    rect
}

pub fn text_width(p: &Painter, text: &str, font: FontId) -> f32 {
    p.layout_no_wrap(text.to_owned(), font, Color32::WHITE).size().x
}

/// Flèche pleine : trait puis pointe triangulaire.
pub fn arrow(p: &Painter, from: Pos2, to: Pos2, width: f32, head: f32, color: Color32) {
    let d = (to - from).normalized();
    let base = to - d * head;
    let side = d.rot90() * head * 0.5;
    p.line_segment([from, base], Stroke::new(width, color));
    p.add(egui::Shape::convex_polygon(vec![to, base + side, base - side], color, Stroke::NONE));
}

/// Étiquette posée sur le canevas : fond sombre, barre de couleur à gauche.
/// `pos` est le coin inférieur gauche.
pub fn tag(p: &Painter, pos: Pos2, text: &str, bar: Color32, color: Color32) -> Rect {
    let galley = p.layout_no_wrap(text.to_owned(), mono(10.5), color);
    let rect = Rect::from_min_size(pos - vec2(0.0, 18.0), vec2(galley.size().x + 14.0, 18.0));
    p.rect_filled(rect, 0.0, t::overlay());
    p.rect_filled(Rect::from_min_size(rect.min, vec2(2.0, 18.0)), 0.0, bar);
    p.galley(pos2(rect.left() + 8.0, rect.center().y - galley.size().y / 2.0), galley, color);
    rect
}

// ───────────────────────────── Icônes ─────────────────────────────

/// Icône au trait, décrite par un chemin SVG (sous-ensemble : M L H V C S A Z).
pub struct Icon {
    view: f32,
    paths: Vec<(Vec<Pos2>, bool)>,
}

impl Icon {
    pub fn parse(view: f32, d: &str) -> Icon {
        // Découpage : une lettre est une commande ; un signe ou un second point commence un nombre.
        let mut tokens: Vec<(char, f32)> = Vec::new();
        let mut num = String::new();
        let flush = |num: &mut String, tokens: &mut Vec<(char, f32)>| {
            if !num.is_empty() {
                tokens.push(('#', num.parse().unwrap_or(0.0)));
                num.clear();
            }
        };
        for c in d.chars() {
            match c {
                '0'..='9' => num.push(c),
                '.' | '-' => {
                    if c == '-' || num.contains('.') {
                        flush(&mut num, &mut tokens);
                    }
                    num.push(c);
                }
                c if c.is_ascii_alphabetic() => {
                    flush(&mut num, &mut tokens);
                    tokens.push((c, 0.0));
                }
                _ => flush(&mut num, &mut tokens),
            }
        }
        flush(&mut num, &mut tokens);

        let mut paths: Vec<(Vec<Pos2>, bool)> = Vec::new();
        let (mut cur, mut start, mut ctrl) = (Pos2::ZERO, Pos2::ZERO, Pos2::ZERO);
        let mut cmd = 'M';
        let mut i = 0;
        while i < tokens.len() {
            if tokens[i].0 != '#' {
                cmd = tokens[i].0;
                i += 1;
                if cmd.eq_ignore_ascii_case(&'z') {
                    if let Some(path) = paths.last_mut() {
                        path.1 = true;
                    }
                    cur = start;
                }
                continue;
            }
            let mut next = || {
                let v = tokens.get(i).map_or(0.0, |t| t.1);
                i += 1;
                v
            };
            let rel = cmd.is_ascii_lowercase();
            let origin = if rel { cur.to_vec2() } else { egui::Vec2::ZERO };
            let point = |next: &mut dyn FnMut() -> f32| {
                let x = next();
                pos2(x, next()) + origin
            };
            let from = cur;
            match cmd.to_ascii_uppercase() {
                'M' => {
                    cur = point(&mut next);
                    start = cur;
                    paths.push((vec![cur], false));
                    // Les paires suivantes d'un « M » sont des segments.
                    cmd = if rel { 'l' } else { 'L' };
                    continue;
                }
                'L' => cur = point(&mut next),
                'H' => cur.x = next() + origin.x,
                'V' => cur.y = next() + origin.y,
                'C' | 'S' => {
                    let c1 = if cmd.eq_ignore_ascii_case(&'c') { point(&mut next) } else { from + (from - ctrl) };
                    let c2 = point(&mut next);
                    cur = point(&mut next);
                    ctrl = c2;
                    let bezier =
                        epaint::CubicBezierShape::from_points_stroke([from, c1, c2, cur], false, Color32::TRANSPARENT, Stroke::NONE);
                    if let Some(path) = paths.last_mut() {
                        path.0.extend((1..=12).map(|k| bezier.sample(k as f32 / 12.0)));
                    }
                    continue;
                }
                'A' => {
                    let (rx, ry, _rotation) = (next(), next(), next());
                    let (large, sweep) = (next() != 0.0, next() != 0.0);
                    cur = point(&mut next);
                    if let Some(path) = paths.last_mut() {
                        arc(from, rx, ry, large, sweep, cur, &mut path.0);
                    }
                    ctrl = cur;
                    continue;
                }
                _ => {
                    i += 1;
                    continue;
                }
            }
            ctrl = cur;
            if let Some(path) = paths.last_mut() {
                path.0.push(cur);
            }
        }
        Icon { view, paths }
    }

    /// Dessine l'icône dans `rect` ; l'épaisseur du trait est donnée en unités du dessin.
    pub fn paint(&self, p: &Painter, rect: Rect, width: f32, color: Color32) {
        let scale = rect.width() / self.view;
        let stroke = Stroke::new(width * scale, color);
        for (points, closed) in &self.paths {
            let pts: Vec<Pos2> = points.iter().map(|q| rect.min + q.to_vec2() * scale).collect();
            p.add(if *closed { egui::Shape::closed_line(pts, stroke) } else { egui::Shape::line(pts, stroke) });
        }
    }
}

/// Arc elliptique SVG (sans rotation d'axe), converti en segments.
fn arc(from: Pos2, rx: f32, ry: f32, large: bool, sweep: bool, to: Pos2, out: &mut Vec<Pos2>) {
    let d = (from - to) / 2.0;
    let (mut rx, mut ry) = (rx.abs().max(1e-6), ry.abs().max(1e-6));
    let lambda = (d.x / rx).powi(2) + (d.y / ry).powi(2);
    if lambda > 1.0 {
        rx *= lambda.sqrt();
        ry *= lambda.sqrt();
    }
    let num = (rx * rx * ry * ry - rx * rx * d.y * d.y - ry * ry * d.x * d.x).max(0.0);
    let den = (rx * rx * d.y * d.y + ry * ry * d.x * d.x).max(1e-12);
    let k = if large != sweep { 1.0 } else { -1.0 } * (num / den).sqrt();
    let c = vec2(k * rx * d.y / ry, -k * ry * d.x / rx);
    let center = pos2((from.x + to.x) / 2.0, (from.y + to.y) / 2.0) + c;
    let a0 = ((d.y - c.y) / ry).atan2((d.x - c.x) / rx);
    let a1 = ((-d.y - c.y) / ry).atan2((-d.x - c.x) / rx);
    let mut da = a1 - a0;
    if sweep && da < 0.0 {
        da += std::f32::consts::TAU;
    } else if !sweep && da > 0.0 {
        da -= std::f32::consts::TAU;
    }
    out.extend((1..=16).map(|i| {
        let a = a0 + da * i as f32 / 16.0;
        center + vec2(rx * a.cos(), ry * a.sin())
    }));
}

// ───────────────────────────── Lignes et contrôles ─────────────────────────────

/// Alloue une ligne de pleine largeur et trace son filet inférieur.
pub fn row(ui: &mut Ui, height: f32, line: Color32) -> Rect {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    ui.painter().hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, line));
    rect
}

/// Sous-interface placée dans `rect`, sans avancer le curseur du parent.
pub fn place(ui: &mut Ui, rect: Rect, layout: Layout) -> Ui {
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect).layout(layout));
    child.spacing_mut().item_spacing = vec2(4.0, 0.0);
    child
}

/// Titre de section numéroté : « 01  SCÈNE ».
pub fn section(ui: &mut Ui, number: &str, title: &str) -> Rect {
    let r = row(ui, 32.0, t::LINE);
    let p = ui.painter();
    p.hline(r.x_range(), r.top() - 0.5, Stroke::new(1.0, t::LINE));
    let mut x = r.left() + PAD;
    if !number.is_empty() {
        x = tracked(p, pos2(x, r.center().y), Align2::LEFT_CENTER, number, mono_bold(10.5), t::ACCENT).right() + 10.0;
    }
    tracked(p, pos2(x, r.center().y), Align2::LEFT_CENTER, tr(title), mono_bold(10.5), t::TEXT_HI);
    r
}

/// Titre vertical d'un panneau replié, lu de bas en haut à partir de `bottom`.
pub fn vertical_title(p: &Painter, x: f32, bottom: f32, text: &str, color: Color32) {
    let mut job = egui::text::LayoutJob::default();
    let format = egui::TextFormat { extra_letter_spacing: 1.26, font_id: mono_bold(10.5), color, ..Default::default() };
    job.append(text, 0.0, format);
    let galley = p.layout_job(job);
    let at = pos2(x - galley.size().y / 2.0, bottom);
    p.add(epaint::TextShape::new(at, galley, color).with_angle(-std::f32::consts::FRAC_PI_2));
}

/// Bouton carré portant une icône au trait.
pub fn icon_button(ui: &Ui, rect: Rect, id: impl Hash + std::fmt::Debug, icon: &Icon, color: Color32, tip: &str) -> Response {
    let resp = ui.interact(rect, ui.id().with(id), Sense::click()).on_hover_text(tr(tip));
    if resp.hovered() {
        ui.painter().rect_filled(rect, 0.0, t::tint());
    }
    icon.paint(ui.painter(), Rect::from_center_size(rect.center(), vec2(14.0, 14.0)), 1.3, if resp.hovered() { t::TEXT_HI } else { color });
    resp
}

/// Ligne « libellé … valeur unité », en lecture seule.
pub fn kv(ui: &mut Ui, label: &str, value: &str, unit: &str) {
    kv_colored(ui, label, value, unit, t::TEXT_HI);
}

pub fn kv_colored(ui: &mut Ui, label: &str, value: &str, unit: &str, color: Color32) {
    let r = row(ui, 32.0, t::LINE_SOFT);
    let p = ui.painter();
    p.text(pos2(r.left() + PAD, r.center().y), Align2::LEFT_CENTER, tr(label), sans(12.0), t::LABEL);
    let mut right = r.right() - PAD;
    if !unit.is_empty() {
        right = p.text(pos2(right, r.center().y), Align2::RIGHT_CENTER, unit, mono(12.0), t::DIM).left() - 5.0;
    }
    p.text(pos2(right, r.center().y), Align2::RIGHT_CENTER, value, mono(12.0), color);
}

/// Ligne « libellé … [contrôle] unité » ; le contrôle est aligné à droite.
pub fn kv_edit<R>(ui: &mut Ui, label: &str, unit: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    let r = row(ui, 32.0, t::LINE_SOFT);
    ui.painter().text(pos2(r.left() + PAD, r.center().y), Align2::LEFT_CENTER, tr(label), sans(12.0), t::LABEL);
    let inner = Rect::from_min_max(pos2(r.left() + 96.0, r.top() + 1.0), pos2(r.right() - PAD, r.bottom() - 1.0));
    let mut child = place(ui, inner, Layout::right_to_left(Align::Center));
    if !unit.is_empty() {
        child.label(egui::RichText::new(unit).font(mono(12.0)).color(t::DIM));
    }
    add(&mut child)
}

/// Champ numérique à glissement, affiché dans l'unité de la grandeur. La saisie accepte
/// une expression avec unités : « 12 mm + 3 mm », « 2 * 1,5 cm ».
pub fn number(ui: &mut Ui, v: &mut f64, quantity: Quantity, speed: f64, decimals: usize) -> Response {
    let factor = quantity.factor();
    let mut shown = *v * factor;
    let widget = egui::DragValue::new(&mut shown)
        .speed(speed)
        .custom_formatter(move |x, _| fr(x, decimals))
        .custom_parser(move |s| expr::eval(s, quantity));
    let r = ui.add(widget);
    if r.changed() {
        *v = shown / factor;
    }
    r
}

/// Bouton plat occupant `rect`.
pub fn cell(ui: &Ui, rect: Rect, id: impl Hash + std::fmt::Debug, text: &str, font: FontId, color: Color32) -> Response {
    let resp = ui.interact(rect, ui.id().with(id), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 0.0, t::tint());
    }
    let color = if resp.hovered() { t::TEXT_HI } else { color };
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, text, font, color);
    resp
}

/// Rangée de boutons plats de largeur égale. Renvoie l'indice du bouton cliqué.
pub fn button_row(ui: &mut Ui, id: &str, labels: &[(&str, bool)]) -> Option<usize> {
    let r = row(ui, 34.0, t::LINE_SOFT);
    let w = r.width() / labels.len() as f32;
    let mut clicked = None;
    for (i, &(label, enabled)) in labels.iter().enumerate() {
        let c = Rect::from_min_size(pos2(r.left() + w * i as f32, r.top()), vec2(w, r.height() - 1.0));
        if i > 0 {
            ui.painter().vline(c.left(), r.y_range(), Stroke::new(1.0, t::LINE));
        }
        if !enabled {
            ui.painter().text(c.center(), Align2::CENTER_CENTER, tr(label), sans(12.0), t::DISABLED);
        } else if cell(ui, c, (id, i), tr(label), sans(12.0), t::TEXT_MID).clicked() {
            clicked = Some(i);
        }
    }
    clicked
}

/// Sélecteur segmenté aligné à droite : le segment actif est rempli du dégradé d'accent.
pub fn segmented<T: Copy + PartialEq>(ui: &mut Ui, label: &str, id: &str, value: &mut T, options: &[(T, &str)]) -> bool {
    let r = row(ui, 36.0, t::LINE_SOFT);
    ui.painter().text(pos2(r.left() + PAD, r.center().y), Align2::LEFT_CENTER, tr(label), sans(12.0), t::LABEL);
    segmented_box(ui, r.right() - PAD, r.center().y, id, value, options).0
}

/// Boîte segmentée dont le bord droit est en `right`. Renvoie si la valeur a changé, et le bord gauche.
pub fn segmented_box<T: Copy + PartialEq>(
    ui: &Ui,
    right: f32,
    center_y: f32,
    id: &str,
    value: &mut T,
    options: &[(T, &str)],
) -> (bool, f32) {
    let p = ui.painter().clone();
    let pad = if options.len() > 2 { 7.0 } else { 9.0 };
    let widths: Vec<f32> = options.iter().map(|o| text_width(&p, o.1, mono_bold(11.0)) + 2.0 * pad).collect();
    let total: f32 = widths.iter().sum();
    let frame = Rect::from_min_size(pos2(right - total - 2.0, center_y - 12.0), vec2(total + 2.0, 24.0));
    p.rect_stroke(frame, 0.0, Stroke::new(1.0, t::LINE_CTRL), egui::StrokeKind::Inside);
    let mut x = frame.left() + 1.0;
    let mut changed = false;
    for (&(option, name), w) in options.iter().zip(widths) {
        let c = Rect::from_min_size(pos2(x, frame.top() + 1.0), vec2(w, 22.0));
        x += w;
        let resp = ui.interact(c, ui.id().with((id, name)), Sense::click());
        if *value == option {
            grad_h(&p, c);
            p.text(c.center(), Align2::CENTER_CENTER, name, mono_bold(11.0), t::ON_ACCENT);
        } else {
            p.text(c.center(), Align2::CENTER_CENTER, name, mono(11.0), if resp.hovered() { t::TEXT_HI } else { t::DIM });
        }
        if resp.clicked() && *value != option {
            *value = option;
            changed = true;
        }
    }
    (changed, frame.left())
}

/// Onglets de largeur égale, soulignés du dégradé d'accent.
pub fn tabs<T: Copy + PartialEq>(ui: &mut Ui, id: &str, value: &mut T, options: &[(T, &str)], enabled: bool) {
    let r = row(ui, 34.0, t::LINE_SOFT);
    let p = ui.painter().clone();
    let w = r.width() / options.len() as f32;
    for (i, &(option, name)) in options.iter().enumerate() {
        let c = Rect::from_min_size(pos2(r.left() + w * i as f32, r.top()), vec2(w, r.height() - 1.0));
        if i > 0 {
            p.vline(c.left(), r.y_range(), Stroke::new(1.0, t::LINE_SOFT));
        }
        let on = *value == option;
        let resp = ui.interact(c, ui.id().with((id, i)), if enabled { Sense::click() } else { Sense::hover() });
        let color = match (enabled, on || resp.hovered()) {
            (false, _) => t::DISABLED,
            (true, true) => t::TEXT_HI,
            (true, false) => t::DIM,
        };
        p.text(c.center(), Align2::CENTER_CENTER, name, if on { sans_bold(11.5) } else { sans(11.5) }, color);
        if on && enabled {
            grad_h(&p, Rect::from_min_max(pos2(c.left() + 12.0, c.bottom() - 2.0), pos2(c.right() - 12.0, c.bottom())));
        }
        if resp.clicked() {
            *value = option;
        }
    }
}

/// Curseur : piste d'un pixel, partie active en dégradé, repère rectangulaire.
pub fn slider(ui: &mut Ui, label: &str, label_width: f32, v: &mut f64, range: RangeInclusive<f64>, shown: Option<String>, enabled: bool) {
    let r = row(ui, 36.0, t::LINE_SOFT);
    let p = ui.painter().clone();
    let dim = |c: Color32| if enabled { c } else { c.gamma_multiply(0.4) };
    p.text(pos2(r.left() + PAD, r.center().y), Align2::LEFT_CENTER, tr(label), sans(12.0), dim(t::LABEL));
    let mut right = r.right() - PAD;
    if let Some(text) = shown {
        p.text(pos2(right, r.center().y), Align2::RIGHT_CENTER, text, mono(12.0), dim(t::TEXT_HI));
        right -= 36.0;
    }
    let track = Rect::from_min_max(pos2(r.left() + PAD + label_width + 14.0, r.center().y - 1.0), pos2(right, r.center().y + 1.0));
    let resp =
        ui.interact(track.expand2(vec2(4.0, 12.0)), ui.id().with(label), if enabled { Sense::click_and_drag() } else { Sense::hover() });
    let (lo, hi) = (*range.start(), *range.end());
    if let Some(pos) = resp.interact_pointer_pos().filter(|_| enabled) {
        *v = lo + (hi - lo) * ((pos.x - track.left()) / track.width()).clamp(0.0, 1.0) as f64;
    }
    let x = track.left() + track.width() * ((*v - lo) / (hi - lo)).clamp(0.0, 1.0) as f32;
    p.rect_filled(track, 0.0, dim(t::LINE_CTRL));
    if enabled {
        grad_h(&p, Rect::from_min_max(track.min, pos2(x, track.bottom())));
    }
    let knob = Rect::from_center_size(pos2(x, r.center().y), vec2(4.0, 12.0));
    p.rect_filled(knob, 0.0, if enabled { t::TEXT_HI } else { dim(t::LABEL) });
}

/// Interrupteur rectangulaire.
pub fn switch(ui: &mut Ui, label: &str, on: &mut bool) {
    let r = row(ui, 36.0, t::LINE_SOFT);
    let p = ui.painter().clone();
    p.text(pos2(r.left() + PAD, r.center().y), Align2::LEFT_CENTER, tr(label), sans(12.0), t::TEXT_MID);
    let outer = Rect::from_min_size(pos2(r.right() - PAD - 38.0, r.center().y - 11.0), vec2(38.0, 22.0));
    if ui.interact(outer, ui.id().with(label), Sense::click()).clicked() {
        *on = !*on;
    }
    let inner = outer.shrink(3.0);
    if *on {
        grad_h(&p, outer.shrink(1.0));
        p.rect_filled(Rect::from_min_size(pos2(inner.right() - 12.0, inner.top() + 2.0), vec2(12.0, 12.0)), 0.0, t::ON_ACCENT);
    } else {
        p.rect_stroke(outer, 0.0, Stroke::new(1.0, Color32::from_rgb(0x3A, 0x3D, 0x48)), egui::StrokeKind::Inside);
        p.rect_filled(Rect::from_min_size(pos2(inner.left(), inner.top() + 2.0), vec2(12.0, 12.0)), 0.0, t::FAINT);
    }
}

/// Bouton d'action principal, bordé de l'accent, avec une flèche à droite.
pub fn accent_button(ui: &mut Ui, label: &str, arrow_icon: &Icon) -> bool {
    let r = row(ui, 52.0, t::LINE_SOFT);
    let b = Rect::from_min_max(pos2(r.left() + PAD, r.top() + 10.0), pos2(r.right() - PAD, r.bottom() - 10.0));
    let resp = ui.interact(b, ui.id().with(label), Sense::click());
    let p = ui.painter();
    p.rect_filled(b, 0.0, t::ACCENT.gamma_multiply(if resp.hovered() { 0.14 } else { 0.06 }));
    p.rect_stroke(b, 0.0, Stroke::new(1.0, t::ACCENT), egui::StrokeKind::Inside);
    p.text(pos2(b.left() + 12.0, b.center().y), Align2::LEFT_CENTER, tr(label), sans(12.0), t::ACCENT_TEXT);
    arrow_icon.paint(p, Rect::from_center_size(pos2(b.right() - 18.0, b.center().y), vec2(12.0, 12.0)), 1.4, t::ACCENT_TEXT);
    resp.clicked()
}

/// Paragraphe d'aide, précédé d'une barre verticale.
pub fn note(ui: &mut Ui, text: &str, bar: Color32) {
    let width = ui.available_width() - 2.0 * PAD - 12.0;
    let galley = ui.painter().layout(text.to_owned(), sans(11.5), t::DIM, width);
    let r = row(ui, galley.size().y + 24.0, t::LINE_SOFT);
    let p = ui.painter();
    p.rect_filled(Rect::from_min_max(pos2(r.left() + PAD, r.top() + 12.0), pos2(r.left() + PAD + 2.0, r.bottom() - 12.0)), 0.0, bar);
    p.galley(pos2(r.left() + PAD + 12.0, r.top() + 12.0), galley, t::DIM);
}

/// Ligne de liste : pastille carrée, nom, valeur à droite ; barre d'accent si sélectionnée.
/// La ligne se clique et se glisse.
pub fn list_row(
    ui: &mut Ui,
    height: f32,
    id: impl Hash + std::fmt::Debug,
    selected: bool,
    dot: Color32,
    name: &str,
    value: &str,
) -> Response {
    let r = row(ui, height, t::LINE_SOFT);
    let resp = ui.interact(r, ui.id().with(id), Sense::click_and_drag());
    let p = ui.painter();
    if selected || resp.hovered() {
        p.rect_filled(r.shrink2(vec2(0.0, 0.5)), 0.0, if selected { t::tint() } else { t::tint().gamma_multiply(0.5) });
    }
    if selected {
        grad_v(p, Rect::from_min_size(r.min, vec2(2.0, r.height())));
    }
    let y = r.center().y;
    p.rect_filled(Rect::from_center_size(pos2(r.left() + PAD + 3.0, y), vec2(6.0, 6.0)), 0.0, dot);
    let value_left =
        p.text(pos2(r.right() - PAD, y), Align2::RIGHT_CENTER, value, mono(11.0), if selected { t::ACCENT_TEXT } else { t::DIM }).left();
    let clip = Rect::from_min_max(pos2(r.left() + PAD + 16.0, r.top()), pos2(value_left - 8.0, r.bottom()));
    p.with_clip_rect(clip).text(clip.left_center(), Align2::LEFT_CENTER, name, sans(12.0), if selected { t::TEXT_HI } else { t::TEXT_MID });
    resp
}
