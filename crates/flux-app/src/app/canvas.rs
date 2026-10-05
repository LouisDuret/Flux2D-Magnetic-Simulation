//! Canevas : interactions, rendu du champ et des objets, habillage (règles, modes
//! d'affichage, légende, échelle, zoom).

use super::{App, Compare, Drag, Tool, View, fmt_b, fmt_force, nice_step};
use crate::render::{
    FLAG_ANIMATE, FLAG_DIFF, FLAG_FILINGS, FLAG_LIC, FLAG_LINES, FLAG_MAP, FLAG_SPLIT, FLAG_SRGB, FieldCallback, Uniforms,
};
use crate::theme::{self as t, mono, mono_bold, sans};
use crate::ui::{self, fr};
use eframe::egui::{self, Align2, Color32, Painter, PointerButton, Pos2, Rect, Sense, Stroke, Ui, Vec2, epaint, pos2, vec2};
use eframe::egui_wgpu;
use flux_core::DVec2;
use flux_core::material::MagClass;
use flux_core::scene::Object;
use flux_core::shape::{Sdf, Shape};

/// Épaisseur des règles graduées.
const RULER: f32 = 20.0;

/// Partie d'un polygone convexe située du côté de `normal` par rapport à `origin`.
fn clip_half(poly: &[DVec2], origin: DVec2, normal: DVec2) -> Vec<DVec2> {
    let mut out = Vec::with_capacity(poly.len() + 2);
    for (i, &a) in poly.iter().enumerate() {
        let b = poly[(i + 1) % poly.len()];
        let (da, db) = ((a - origin).dot(normal), (b - origin).dot(normal));
        if da >= 0.0 {
            out.push(a);
        }
        if (da >= 0.0) != (db >= 0.0) {
            out.push(a + (b - a) * (da / (da - db)));
        }
    }
    out
}

/// Contour extérieur de l'objet, en repère monde.
fn outline(o: &Object) -> Vec<DVec2> {
    let circle = |r: f64| (0..48).map(|k| DVec2::from_angle(k as f64 / 48.0 * std::f64::consts::TAU) * r).collect();
    let local: Vec<DVec2> = match &o.shape {
        Shape::Rect { w, h } => {
            [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)].map(|(x, y)| DVec2::new(x * w / 2.0, y * h / 2.0)).to_vec()
        }
        Shape::Circle { r } => circle(*r),
        Shape::Ring { r_out, .. } => circle(*r_out),
        Shape::Polygon { pts } => pts.clone(),
    };
    local.into_iter().map(|l| o.pos.truncate() + DVec2::from_angle(o.angle).rotate(l)).collect()
}

/// Induction sur deux chiffres significatifs, pour la légende.
fn fmt_b_short(tesla: f64) -> String {
    let (v, unit) = if tesla >= 1.0 {
        (tesla, "T")
    } else if tesla >= 1e-3 {
        (tesla * 1e3, "mT")
    } else {
        (tesla * 1e6, "µT")
    };
    format!("{} {unit}", fr(v, if v >= 10.0 { 0 } else { 1 }))
}

impl App {
    pub(super) fn canvas(&mut self, ui: &mut Ui) {
        ui.spacing_mut().item_spacing = Vec2::ZERO;
        let (resp, full_painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        let margin = if self.show_ui { RULER } else { 0.0 };
        let plot = Rect::from_min_max(resp.rect.min + Vec2::splat(margin), resp.rect.max);
        self.plot_size = plot.size();
        let painter = full_painter.with_clip_rect(plot);
        let mut view = View { center: self.view_center, scale: self.view_scale, rect: plot };
        let hover = resp.hover_pos().filter(|p| plot.contains(*p));

        // Zoom centré sur le curseur.
        if let Some(p) = hover {
            let zoom = ui.input(|i| i.zoom_delta() as f64 * (i.smooth_scroll_delta.y as f64 * 0.004).exp());
            if zoom != 1.0 {
                let before = view.to_world(p);
                view.scale = (view.scale / zoom).clamp(1e-6, 5e-3);
                view.center += before - view.to_world(p);
            }
        }
        let cur = resp.interact_pointer_pos().or(hover).map(|p| view.to_world(p));
        self.cursor = hover.map(|p| view.to_world(p));

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
                Tool::Probe | Tool::Seed => Drag::Pan,
                Tool::Sprinkle => Drag::Sprinkle,
                Tool::Cut => Drag::Cut(start),
                _ => Drag::Create(start),
            });
        }
        if resp.dragged_by(PointerButton::Middle) || (resp.dragged_by(PointerButton::Primary) && matches!(self.drag, Some(Drag::Pan))) {
            let d = resp.drag_delta();
            view.center -= DVec2::new(d.x as f64, -d.y as f64) * view.scale;
        }
        let brush = 18.0 * view.scale;
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
                Some(Drag::Sprinkle) => self.visuals.sprinkle(cur, brush, self.solver.field(), self.b_max),
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
                Tool::Seed => self.scene.seeds.push(cur.extend(0.0)),
                Tool::Sprinkle => self.visuals.sprinkle(cur, brush, self.solver.field(), self.b_max),
                Tool::Cut => {}
                _ => self.create(cur, cur),
            }
        }
        (self.view_center, self.view_scale) = (view.center, view.scale);

        // Champ : rendu GPU derrière les objets.
        let field = self.solver.field();
        let lines = self.show_lines && self.a_span > 0.0;
        // La comparaison n'a de sens que sur la même grille que la référence.
        let comparable = self.reference.as_ref().is_some_and(|r| r.n == field.n && r.size == field.size);
        let compare = if comparable { self.compare } else { Compare::Off };
        let split_x = (compare == Compare::Split).then(|| plot.left() + self.split as f32 * plot.width());
        if let Some(rs) = &self.render_state {
            let half = plot.size() * 0.5 * view.scale as f32;
            let px = view.scale as f32 / ui.ctx().pixels_per_point();
            let uniforms = Uniforms {
                center: view.center.as_vec2().to_array(),
                half: [half.x, half.y],
                size: field.size as f32,
                n: field.n as u32,
                delta_a: (self.a_span / self.n_lines) as f32,
                b_max: self.b_max as f32,
                flags: (lines as u32 * FLAG_LINES)
                    | (self.show_map as u32 * FLAG_MAP)
                    | (rs.target_format.is_srgb() as u32 * FLAG_SRGB)
                    | (self.show_lic as u32 * FLAG_LIC)
                    | (self.lic_animate as u32 * FLAG_ANIMATE)
                    | (self.show_filings as u32 * FLAG_FILINGS)
                    | (split_x.is_some() as u32 * FLAG_SPLIT)
                    | ((compare == Compare::Diff) as u32 * FLAG_DIFF),
                line_px: 0.9 * ui.ctx().pixels_per_point(),
                px,
                time: (ui.input(|i| i.time) * 0.7).fract() as f32,
                // Maille de la limaille : environ 7 pixels, arrondie à une puissance de deux
                // pour que les grains restent en place quand la vue se déplace.
                grain: (7.0 * px).log2().round().exp2(),
                split: split_x.map_or(0.0, |x| view.to_world(pos2(x, 0.0)).x as f32),
                pad: [0.0; 2],
            };
            painter.add(egui_wgpu::Callback::new_paint_callback(plot, FieldCallback(uniforms)));
        }
        if self.show_vectors {
            let step = 34.0;
            let (nx, ny) = ((plot.width() / step) as i32, (plot.height() / step) as i32);
            for j in 0..=ny {
                for i in 0..=nx {
                    let p = plot.min + vec2(i as f32 + 0.5, j as f32 + 0.5) * step;
                    let Some(s) = field.sample(view.to_world(p).extend(0.0)) else { continue };
                    // Longueur logarithmique sur trois décades.
                    let len = (1.0 + (s.b.length() / self.b_max).max(1e-12).log10() / 3.0).clamp(0.0, 1.0) as f32;
                    let dir = s.b.truncate().normalize_or_zero();
                    let v = vec2(dir.x as f32, -dir.y as f32) * len * step * 0.8;
                    if len > 0.05 {
                        painter.arrow(p - v * 0.5, v, Stroke::new(1.0, t::TEXT.gamma_multiply(0.75)));
                    }
                }
            }
        }

        let dt = ui.input(|i| i.stable_dt).min(0.05);
        let mut animating = self.show_lic && self.lic_animate && self.render_state.is_some();
        if self.show_particles {
            self.visuals.particles(&painter, view, field, self.b_max, dt);
            animating = true;
        }
        self.visuals.draw_filings(&painter, view, field);
        if self.show_compass {
            animating |= self.visuals.compasses(&painter, view, field, self.b_max, dt);
        }
        self.visuals.seed_lines(&painter, view, field, &self.scene.seeds, self.field_version);
        if animating {
            ui.ctx().request_repaint();
        }

        match (&self.reference, split_x) {
            // Vue scindée : objets de référence à gauche de la séparation, objets courants à droite.
            (Some(reference), Some(x)) => {
                let (left, right) = plot.split_left_right_at_x(x);
                for o in &reference.scene.objects {
                    self.draw_object(&painter.with_clip_rect(left), view, o);
                }
                for o in &self.scene.objects {
                    self.draw_object(&painter.with_clip_rect(right), view, o);
                }
                painter.vline(x, plot.y_range(), Stroke::new(1.0, t::ACCENT));
                let y = plot.top() + 62.0;
                ui::tag(&painter, pos2(x - 58.0, y), "AVANT", t::ACCENT, t::TEXT_HI);
                ui::tag(&painter, pos2(x + 8.0, y), "APRÈS", t::ACCENT, t::TEXT_HI);
            }
            _ => {
                for o in &self.scene.objects {
                    self.draw_object(&painter, view, o);
                }
            }
        }

        let f_max = self.wrenches.iter().map(|w| w.force.length()).fold(1e-12, f64::max);
        for w in &self.wrenches {
            let Some(o) = self.scene.get(w.id) else { continue };
            let f = w.force.truncate();
            let selected = self.selected == Some(w.id);
            if f.length() < 1e-12 || (f.length() < 1e-3 * f_max && !selected) {
                continue;
            }
            let len = 16.0 + 40.0 * (f.length() / f_max).sqrt() as f32;
            let dir = vec2(f.x as f32, -f.y as f32) / f.length() as f32;
            let center = o.pos.truncate();
            let mut origin = view.to_screen(center);
            if self.scene.material(&o.material).is_some_and(|m| m.class == MagClass::Magnet) {
                // La flèche part du bord de l'aimant pour ne pas masquer son aimantation et ses pôles.
                let reach = outline(o).iter().map(|q| (*q - center).dot(f / f.length())).fold(0.0, f64::max);
                origin += dir * ((reach / view.scale) as f32 + 12.0);
            }
            ui::arrow(&painter, origin, origin + dir * len, 2.2, 13.0, t::FORCE);
            if selected && self.show_ui {
                let text = format!("F {} N/m", fmt_force(f.length()));
                ui::tag(&painter, origin + dir * len + vec2(8.0, 24.0), &text, t::FORCE, t::FORCE_TEXT);
            }
        }
        if let Some([a, b]) = self.scene.cut_line {
            let (a, b) = (view.to_screen(a.truncate()), view.to_screen(b.truncate()));
            painter.line_segment([a, b], Stroke::new(1.5, t::ACCENT));
            for end in [a, b] {
                painter.rect_filled(Rect::from_center_size(end, Vec2::splat(6.0)), 0.0, t::ACCENT);
            }
        }
        for probe in &self.scene.probes {
            let s = view.to_screen(probe.truncate());
            painter.rect_filled(Rect::from_center_size(s, Vec2::splat(7.0)), 0.0, t::BG);
            painter.rect_stroke(Rect::from_center_size(s, Vec2::splat(7.0)), 0.0, Stroke::new(1.5, t::ACCENT), egui::StrokeKind::Middle);
            if let Some(f) = field.sample(*probe) {
                ui::tag(&painter, s + vec2(9.0, 9.0), &fmt_b(f.b.length()), t::ACCENT, t::TEXT_HI);
            }
        }
        if let (Some(Drag::Create(start)), Some(cur)) = (&self.drag, cur) {
            let stroke = Stroke::new(1.0, t::ACCENT);
            if self.tool == Tool::Circle {
                painter.circle_stroke(view.to_screen(*start), ((cur - *start).length() / view.scale) as f32, stroke);
            } else {
                let r = Rect::from_two_pos(view.to_screen(*start), view.to_screen(cur));
                painter.rect_stroke(r, 0.0, stroke, egui::StrokeKind::Middle);
            }
        }
        if self.scene.objects.is_empty() {
            let hint = "Canevas vide — choisissez un outil : M aimant · R bloc · E disque · C bobine\nMolette : zoom · clic milieu : déplacer la vue · F : cadrer";
            painter.text(plot.center(), Align2::CENTER_CENTER, hint, sans(14.0), t::DIM);
        }

        if self.show_ui {
            self.rulers(&full_painter, resp.rect, view, hover);
            self.overlays(ui, &painter, view);
        }
    }

    fn draw_object(&self, p: &Painter, view: View, o: &Object) {
        let Some(mat) = self.scene.material(&o.material) else { return };
        let selected = self.selected == Some(o.id);
        let color = t::class_color(mat.class);
        let center = o.pos.truncate();
        let poly = outline(o);
        let screen = |pts: &[DVec2]| -> Vec<Pos2> { pts.iter().map(|w| view.to_screen(*w)).collect() };
        let pts = screen(&poly);
        let bbox = Rect::from_points(&pts);
        let convex = matches!(o.shape, Shape::Rect { .. } | Shape::Circle { .. });
        let magnet = mat.class == MagClass::Magnet;

        if convex && magnet {
            // Moitié nord en rouge, moitié sud en bleu, de part et d'autre de l'axe d'aimantation.
            let d = o.mag_dir();
            p.add(egui::Shape::convex_polygon(screen(&clip_half(&poly, center, d)), t::NORTH.gamma_multiply(0.34), Stroke::NONE));
            p.add(egui::Shape::convex_polygon(screen(&clip_half(&poly, center, -d)), t::SOUTH.gamma_multiply(0.32), Stroke::NONE));
            p.add(egui::Shape::convex_polygon(pts.clone(), Color32::from_rgba_unmultiplied(0x1B, 0x1C, 0x22, 64), Stroke::NONE));
        } else if convex {
            let fill = if mat.class == MagClass::Ferro {
                Color32::from_rgb(0xD8, 0xDC, 0xE6).gamma_multiply(0.14)
            } else {
                color.gamma_multiply(0.18)
            };
            p.add(egui::Shape::convex_polygon(pts.clone(), fill, Stroke::NONE));
        }
        if mat.class == MagClass::Ferro && o.angle == 0.0 && matches!(o.shape, Shape::Rect { .. }) {
            // Hachures discrètes des pièces de fer.
            let hatch = p.with_clip_rect(bbox.intersect(p.clip_rect()));
            let mut x = bbox.left() - bbox.height();
            while x < bbox.right() {
                hatch.line_segment(
                    [pos2(x, bbox.bottom()), pos2(x + bbox.height(), bbox.top())],
                    Stroke::new(1.0, t::TEXT_HI.gamma_multiply(0.06)),
                );
                x += 40.0;
            }
        }
        let stroke = match (selected, magnet) {
            (true, _) => Stroke::new(1.8, t::ACCENT),
            (false, true) => Stroke::new(1.2, t::TEXT_HI.gamma_multiply(0.9)),
            (false, false) => {
                Stroke::new(1.2, if mat.class == MagClass::Ferro { t::TEXT.gamma_multiply(0.75) } else { color.gamma_multiply(0.85) })
            }
        };
        p.add(egui::Shape::closed_line(pts, stroke));
        let c = view.to_screen(center);
        if let Shape::Ring { r_in, .. } = &o.shape {
            p.circle_stroke(c, (r_in / view.scale) as f32, stroke);
        }

        if magnet {
            let d = o.mag_dir();
            let half = match &o.shape {
                Shape::Rect { w, h } => {
                    let l = DVec2::from_angle(o.mag_angle);
                    0.5 / (l.x.abs() / w).max(l.y.abs() / h)
                }
                s => s.bounding_radius(),
            };
            let dir = vec2(d.x as f32, -d.y as f32);
            let half_px = (half / view.scale) as f32;
            let len = (half_px * 0.42).clamp(6.0, 46.0);
            ui::arrow(p, c - dir * len, c + dir * len, 2.0, len.min(12.0), Color32::WHITE);
            if half_px > 14.0 {
                // Les pôles sont toujours désignés par leur lettre, pas seulement par la couleur.
                for (sign, letter, fill) in [(1.0, "N", t::NORTH), (-1.0, "S", t::SOUTH)] {
                    let badge = Rect::from_center_size(c + dir * half_px * sign, Vec2::splat(18.0));
                    p.rect_filled(badge, 0.0, fill);
                    p.text(badge.center(), Align2::CENTER_CENTER, letter, mono_bold(10.5), Color32::from_rgb(0x0B, 0x0C, 0x0F));
                }
            }
        } else if o.amp_turns() != 0.0 {
            // ⊙ courant sortant, ⊗ courant entrant.
            let s = Stroke::new(1.5, Color32::WHITE);
            p.circle_stroke(c, 6.0, s);
            if o.amp_turns() > 0.0 {
                p.circle_filled(c, 1.8, Color32::WHITE);
            } else {
                p.line_segment([c + vec2(-4.0, -4.0), c + vec2(4.0, 4.0)], s);
                p.line_segment([c + vec2(-4.0, 4.0), c + vec2(4.0, -4.0)], s);
            }
        }

        if self.show_ui && (selected || bbox.width() >= 36.0) {
            ui::tag(p, pos2(bbox.left(), bbox.top() - 4.0), &o.name.to_uppercase(), color, t::TEXT_HI);
        }
        if selected && self.show_ui {
            // Cote de l'encombrement horizontal.
            let (y, s) = (bbox.bottom() + 20.0, Stroke::new(1.0, t::TEXT_HI.gamma_multiply(0.55)));
            p.vline(bbox.left(), y..=y + 14.0, s);
            p.vline(bbox.right(), y..=y + 14.0, s);
            p.hline(bbox.x_range(), y + 8.0, s);
            let text = format!("{} mm", fr(bbox.width() as f64 * view.scale * 1e3, 1));
            let galley = p.layout_no_wrap(text, mono(10.5), t::TEXT_HI);
            let label = Rect::from_center_size(pos2(bbox.center().x, y + 25.0), galley.size() + vec2(10.0, 4.0));
            p.rect_filled(label, 0.0, t::overlay());
            p.galley(label.min + vec2(5.0, 2.0), galley, t::TEXT_HI);
        }
    }

    /// Règles graduées en millimètres, en haut et à gauche du canevas.
    fn rulers(&self, p: &Painter, full: Rect, view: View, hover: Option<Pos2>) {
        let plot = view.rect;
        let line = Stroke::new(1.0, t::LINE);
        p.rect_filled(Rect::from_min_max(full.min, pos2(full.right(), plot.top())), 0.0, t::PANEL);
        p.rect_filled(Rect::from_min_max(full.min, pos2(plot.left(), full.bottom())), 0.0, t::PANEL);
        // Graduations principales tous les 90 points environ, cinq subdivisions.
        let major = nice_step(90.0 * view.scale * 1e3);
        let minor = major / 5.0;
        let decimals = if major >= 1.0 { 0 } else { (-major.log10()).ceil() as usize };
        let (lo, hi) = (view.to_world(plot.left_bottom()) * 1e3, view.to_world(plot.right_top()) * 1e3);
        let tick = |k: i64| if k % 5 == 0 { (10.0, t::TICK_MAJOR) } else { (5.0, t::TICK_MINOR) };
        for k in (lo.x / minor).ceil() as i64..=(hi.x / minor).floor() as i64 {
            let x = view.to_screen(DVec2::new(k as f64 * minor * 1e-3, 0.0)).x;
            let (len, color) = tick(k);
            p.vline(x, plot.top() - len..=plot.top(), Stroke::new(1.0, color));
            if k % 5 == 0 {
                p.text(pos2(x + 3.0, full.top() + 1.0), Align2::LEFT_TOP, fr(k as f64 * minor, decimals), mono(9.0), t::DIM);
            }
        }
        for k in (lo.y / minor).ceil() as i64..=(hi.y / minor).floor() as i64 {
            let y = view.to_screen(DVec2::new(0.0, k as f64 * minor * 1e-3)).y;
            let (len, color) = tick(k);
            p.hline(plot.left() - len..=plot.left(), y, Stroke::new(1.0, color));
            if k % 5 == 0 {
                // Texte tourné d'un quart de tour, lu de bas en haut.
                let galley = p.layout_no_wrap(fr(k as f64 * minor, decimals), mono(9.0), t::DIM);
                let at = pos2(full.left() + 2.0, y + galley.size().x / 2.0);
                p.add(epaint::TextShape::new(at, galley, t::DIM).with_angle(-std::f32::consts::FRAC_PI_2));
            }
        }
        if let Some(h) = hover {
            // Repères de la position du curseur.
            p.vline(h.x, full.top()..=plot.top(), Stroke::new(1.0, t::ACCENT));
            p.hline(full.left()..=plot.left(), h.y, Stroke::new(1.0, t::ACCENT));
        }
        p.rect_filled(Rect::from_min_max(full.min, plot.min), 0.0, t::PANEL);
        p.hline(full.x_range(), plot.top() - 0.5, line);
        p.vline(plot.left() - 0.5, full.y_range(), line);
    }

    /// Éléments posés sur le canevas : modes d'affichage, échelle, légende, zoom.
    fn overlays(&mut self, ui: &mut Ui, p: &Painter, view: View) {
        let plot = view.rect;
        let line = Stroke::new(1.0, t::LINE);

        // Représentations du champ, attachées en haut à gauche.
        let names = self.modes().map(|(name, on)| (name, *on));
        let widths = names.map(|(name, _)| 22.0 + 6.0 + ui::text_width(p, "0", mono(10.5)) + ui::text_width(p, name, sans(12.0)));
        let bar = Rect::from_min_size(plot.min, vec2(widths.iter().sum(), 32.0));
        p.rect_filled(bar, 0.0, t::overlay());
        let mut x = bar.left();
        let mut toggled = None;
        for (i, ((name, on), w)) in names.into_iter().zip(widths).enumerate() {
            let c = Rect::from_min_size(pos2(x, bar.top()), vec2(w, 32.0));
            x += w;
            let resp = ui.interact(c, ui.id().with(("mode", i)), Sense::click());
            if on {
                p.rect_filled(c, 0.0, t::tint());
                ui::grad_h(p, Rect::from_min_max(pos2(c.left(), c.bottom() - 2.0), c.max));
            }
            let key = p.text(
                pos2(c.left() + 11.0, c.center().y),
                Align2::LEFT_CENTER,
                (i + 1).to_string(),
                mono(10.5),
                if on { t::ACCENT } else { t::FAINT },
            );
            let color = if on || resp.hovered() { t::TEXT_HI } else { t::LABEL };
            p.text(pos2(key.right() + 6.0, c.center().y), Align2::LEFT_CENTER, name, sans(12.0), color);
            p.vline(c.right() - 0.5, c.y_range(), Stroke::new(1.0, t::LINE_SOFT));
            if resp.clicked() {
                toggled = Some(i);
            }
        }
        p.hline(bar.x_range(), bar.bottom() - 0.5, line);
        p.vline(bar.right() - 0.5, bar.y_range(), line);
        if let Some(i) = toggled {
            let flag = self.modes().into_iter().nth(i).unwrap().1;
            *flag = !*flag;
        }

        // Échelle graphique.
        let length = nice_step(70.0 * view.scale * 1e3);
        let w = (length * 1e-3 / view.scale) as f32;
        let text = format!("{} mm", fr(length, if length >= 1.0 { 0 } else { 2 }));
        let scale = Rect::from_min_size(pos2(plot.left() + 16.0, plot.bottom() - 46.0), vec2(w.max(48.0) + 16.0, 34.0));
        p.rect_filled(scale, 0.0, t::overlay());
        p.text(scale.min + vec2(8.0, 5.0), Align2::LEFT_TOP, text, mono(10.5), t::TEXT_HI);
        let (x0, y) = (scale.left() + 8.0, scale.bottom() - 9.0);
        let s = Stroke::new(1.2, t::TEXT_HI);
        p.hline(x0..=x0 + w, y, s);
        p.vline(x0, y - 4.0..=y + 4.0, s);
        p.vline(x0 + w, y - 4.0..=y + 4.0, s);

        // Légende de la carte d'intensité.
        if self.show_map {
            let (strong, weak) = (fmt_b_short(self.b_max), fmt_b_short(self.b_max / 1000.0));
            let w = ui::text_width(p, &strong, mono(10.0)).max(ui::text_width(p, &weak, mono(10.0))) + 22.0;
            let legend = Rect::from_min_size(pos2(plot.right() - w, plot.top() + 52.0), vec2(w, 236.0));
            p.rect_filled(legend, 0.0, t::overlay());
            p.rect_stroke(legend, 0.0, line, egui::StrokeKind::Inside);
            let cx = legend.center().x;
            let label = |y: f32, text: &str, font, color| p.text(pos2(cx, legend.top() + y), Align2::CENTER_CENTER, text, font, color);
            label(16.0, "|B|", mono_bold(10.0), t::TEXT_HI);
            label(34.0, &strong, mono(10.0), t::DIM);
            let stops = [(0.0, 0xFFE8B0), (0.16, 0xF8A060), (0.32, 0xE86062), (0.5, 0xB03480), (0.75, 0x5C1A78), (1.0, 0x220E3E)]
                .map(|(at, rgb): (f32, u32)| (at, Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)));
            ui::gradient(p, Rect::from_min_size(pos2(cx - 4.0, legend.top() + 46.0), vec2(8.0, 150.0)), false, &stops);
            label(208.0, &weak, mono(10.0), t::DIM);
            label(223.0, "log", mono(10.0), t::FAINT);
        }

        // Zoom et cadrage.
        let zoom = Rect::from_min_max(pos2(plot.right() - 108.0, plot.bottom() - 32.0), plot.max);
        p.rect_filled(zoom, 0.0, t::overlay());
        p.hline(zoom.x_range(), zoom.top() + 0.5, line);
        p.vline(zoom.left() + 0.5, zoom.y_range(), line);
        let tips = ["Dézoomer", "Zoomer", "Cadrer la scène (F)"];
        for (i, tip) in tips.into_iter().enumerate() {
            let c = Rect::from_min_size(pos2(zoom.left() + 36.0 * i as f32, zoom.top()), vec2(36.0, 32.0));
            let resp = ui.interact(c, ui.id().with(("zoom", i)), Sense::click()).on_hover_text(tip);
            if resp.hovered() {
                p.rect_filled(c, 0.0, t::tint());
            }
            if i > 0 {
                p.vline(c.left(), c.y_range(), Stroke::new(1.0, t::LINE_SOFT));
            }
            let icon = [&self.icons.zoom_out, &self.icons.zoom_in, &self.icons.frame][i];
            icon.paint(
                p,
                Rect::from_center_size(c.center(), Vec2::splat(13.0)),
                1.4,
                if resp.hovered() { t::TEXT_HI } else { t::TEXT_MID },
            );
            if resp.clicked() {
                match i {
                    0 => self.view_scale = (self.view_scale * 1.25).min(5e-3),
                    1 => self.view_scale = (self.view_scale / 1.25).max(1e-6),
                    _ => self.frame_all(),
                }
            }
        }
    }
}
