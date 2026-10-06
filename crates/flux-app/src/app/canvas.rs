//! Canevas : interactions, rendu du champ et des objets, habillage (règles, modes
//! d'affichage, légende, échelle, zoom).

use super::{App, Compare, Drag, Hot, Tool, View, nice_step};
use crate::lang::tr;
use crate::render::{
    FLAG_ANIMATE, FLAG_DIFF, FLAG_FILINGS, FLAG_LIC, FLAG_LINES, FLAG_MAP, FLAG_SPLIT, FLAG_SRGB, FieldCallback, Uniforms,
};
use crate::theme::{self as t, mono, mono_bold, sans};
use crate::ui::{self, fr};
use crate::units;
use eframe::egui::{self, Align2, Color32, CursorIcon, Painter, PointerButton, Pos2, Rect, Sense, Stroke, Ui, Vec2, epaint, pos2, vec2};
use eframe::egui_wgpu;
use flux_core::DVec2;
use flux_core::material::MagClass;
use flux_core::scene::Object;
use flux_core::shape::{Contour, PathNode, Sdf, Shape, flatten_path};
use std::f64::consts::{FRAC_PI_2, PI, TAU};

/// Épaisseur des règles graduées.
const RULER: f32 = 20.0;
/// Rayon de saisie des poignées et des sommets (points d'écran).
const GRAB: f32 = 9.0;
/// Pas de rotation avec Maj enfoncée.
const ANGLE_STEP: f64 = PI / 12.0;

/// Poignées de l'objet sélectionné, en coordonnées écran.
#[derive(Clone, Copy)]
pub(super) struct Handles {
    /// Point du contour d'où part la tige de la poignée de rotation.
    stem: Pos2,
    rotate: Pos2,
    /// Poignée d'aimantation, au bout de la flèche d'un aimant.
    magnet: Option<Pos2>,
}

/// Ramène un angle dans [−π, π[.
fn wrap(a: f64) -> f64 {
    (a + PI).rem_euclid(TAU) - PI
}

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

/// Découpe la région délimitée par des contours (règle pair-impair) en trapèzes à bases
/// horizontales : de quoi remplir n'importe quelle forme, concave ou trouée.
fn trapezoids(contours: &[Contour]) -> Vec<[DVec2; 4]> {
    let mut ys: Vec<f64> = contours.iter().flatten().map(|p| p.y).collect();
    ys.sort_by(f64::total_cmp);
    ys.dedup();
    let edges: Vec<(DVec2, DVec2)> =
        contours.iter().flat_map(|c| (0..c.len()).map(move |i| (c[i], c[(i + 1) % c.len()]))).filter(|(a, b)| a.y != b.y).collect();
    let mut out = Vec::new();
    let mut xs: Vec<(f64, f64)> = Vec::new();
    for band in ys.windows(2) {
        let (y0, y1) = (band[0], band[1]);
        let mid = (y0 + y1) / 2.0;
        // Abscisses, en bas et en haut de la bande, des arêtes qui la traversent.
        xs.clear();
        for &(a, b) in &edges {
            if (a.y > mid) != (b.y > mid) {
                let x = |y: f64| a.x + (y - a.y) / (b.y - a.y) * (b.x - a.x);
                xs.push((x(y0), x(y1)));
            }
        }
        xs.sort_by(|p, q| (p.0 + p.1).total_cmp(&(q.0 + q.1)));
        for pair in xs.as_chunks::<2>().0 {
            out.push([DVec2::new(pair[0].0, y0), DVec2::new(pair[1].0, y0), DVec2::new(pair[1].1, y1), DVec2::new(pair[0].1, y1)]);
        }
    }
    out
}

/// Distance du centre au contour le long de `dir` (intersection la plus lointaine du rayon).
fn ray_reach(contour: &[DVec2], center: DVec2, dir: DVec2) -> f64 {
    let mut reach = 0.0;
    for (i, &a) in contour.iter().enumerate() {
        let (e, w) = (contour[(i + 1) % contour.len()] - a, a - center);
        let den = dir.perp_dot(e);
        if den.abs() > 1e-300 && (0.0..=1.0).contains(&(w.perp_dot(dir) / den)) {
            reach = f64::max(reach, w.perp_dot(e) / den);
        }
    }
    reach
}

/// Flèche d'aimantation : direction à l'écran, demi-longueur, et distance du centre au
/// bord vers le pôle nord puis vers le pôle sud (points d'écran).
fn magnet_arrow(o: &Object, outer: &[DVec2], view: View) -> (Vec2, f32, [f32; 2]) {
    let d = o.mag_dir();
    let reach = |dir: DVec2| {
        let r = ray_reach(outer, o.pos.truncate(), dir);
        (if r > 0.0 { r } else { o.shape.bounding_radius() } / view.scale) as f32
    };
    let poles = [reach(d), reach(-d)];
    (vec2(d.x as f32, -d.y as f32), (poles[0].min(poles[1]) * 0.42).clamp(6.0, 46.0), poles)
}

impl App {
    pub(super) fn canvas(&mut self, ui: &mut Ui) {
        ui.spacing_mut().item_spacing = Vec2::ZERO;
        let (resp, full_painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        let margin = if self.show_ui { RULER } else { 0.0 };
        let plot = Rect::from_min_max(resp.rect.min + Vec2::splat(margin), resp.rect.max);
        self.plot = plot;
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
        let shift = ui.input(|i| i.modifiers.shift);

        // Poignées et symboles réactifs sous le curseur.
        let handles = self.handles(view);
        self.hot = match self.drag {
            Some(Drag::Rotate { .. }) => Some(Hot::Rotate),
            Some(Drag::Magnetize { .. }) => Some(Hot::Magnetize),
            Some(_) => None,
            None => hover.and_then(|p| self.hot_at(view, handles, p)),
        };

        if resp.drag_started_by(PointerButton::Primary) {
            let press = ui.input(|i| i.pointer.press_origin());
            let start = press.map(|p| view.to_world(p)).or(cur).unwrap_or_default();
            let grabbed = press.and_then(|p| self.hot_at(view, handles, p));
            self.drag = Some(match (self.tool, grabbed, self.selected) {
                (Tool::Select, Some(Hot::Rotate), Some(id)) => Drag::Rotate { id },
                (Tool::Select, Some(Hot::Magnetize), Some(id)) => Drag::Magnetize { id },
                (Tool::Select, ..) => match self.scene.pick(start.extend(0.0)) {
                    // Un objet verrouillé se sélectionne mais ne se déplace pas : le geste déplace la vue.
                    Some(id) if self.scene.get(id).is_some_and(|o| o.locked) => {
                        self.selected = Some(id);
                        Drag::Pan
                    }
                    Some(mut id) => {
                        if ui.input(|i| i.modifiers.alt) {
                            id = self.scene.duplicate(id, DVec2::ZERO).unwrap_or(id);
                        }
                        self.selected = Some(id);
                        Drag::Move { id, offset: self.scene.get(id).unwrap().pos.truncate() - start }
                    }
                    None => Drag::Pan,
                },
                (Tool::Probe | Tool::Seed, ..) => Drag::Pan,
                (Tool::Sprinkle, ..) => Drag::Sprinkle,
                (Tool::Cut, ..) => Drag::Cut(start),
                (Tool::Polygon | Tool::Bezier, ..) => Drag::Draft,
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
                Some(Drag::Rotate { id } | Drag::Magnetize { id }) => {
                    let rotate = matches!(self.drag, Some(Drag::Rotate { .. }));
                    if let Some(o) = self.scene.get_mut(id)
                        && (cur - o.pos.truncate()).length() > 2.0 * view.scale
                    {
                        // La poignée de rotation pointe vers le « haut » de l'objet.
                        let mut a = (cur - o.pos.truncate()).to_angle() - if rotate { FRAC_PI_2 } else { 0.0 };
                        if shift {
                            a = (a / ANGLE_STEP).round() * ANGLE_STEP;
                        }
                        if rotate {
                            o.angle = wrap(a);
                        } else {
                            o.mag_angle = wrap(a - o.angle);
                        }
                    }
                }
                Some(Drag::Cut(start)) => self.scene.cut_line = Some([start.extend(0.0), cur.extend(0.0)]),
                Some(Drag::Sprinkle) => self.visuals.sprinkle(cur, brush, self.solver.field(), self.b_max),
                _ => {}
            }
        }

        // Tracé d'un polygone ou d'une courbe : chaque appui pose un sommet, glisser tire sa tangente.
        let drafting = matches!(self.tool, Tool::Polygon | Tool::Bezier);
        if !drafting {
            self.draft.clear();
        }
        let closing = self.draft.len() >= 3 && hover.is_some_and(|p| p.distance(view.to_screen(self.draft[0].p)) <= GRAB);
        if drafting && let Some(cur) = cur {
            if hover.is_some() && resp.contains_pointer() && ui.input(|i| i.pointer.primary_pressed()) {
                if !closing {
                    self.draft.push(PathNode { p: cur, handle: DVec2::ZERO });
                }
            } else if self.tool == Tool::Bezier
                && resp.dragged_by(PointerButton::Primary)
                && let Some(node) = self.draft.last_mut()
            {
                node.handle = cur - node.p;
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
                Tool::Select => {
                    let hit = self.scene.pick(cur.extend(0.0));
                    match (self.selected, hit, self.hot) {
                        // Un clic sur une poignée ne change pas la sélection.
                        (_, _, Some(Hot::Rotate | Hot::Magnetize)) => {}
                        // Maj + clic : désigne le second objet d'une opération booléenne.
                        (Some(a), Some(b), _) if shift && a != b => (self.operand, self.reveal_boolean) = (Some((a, b)), true),
                        _ => {
                            if let Some(Hot::Current(id)) = self.hot
                                && let Some(o) = self.scene.get_mut(id)
                            {
                                o.current = -o.current;
                            }
                            self.selected = hit;
                            // Double-clic sur un aimant : saisie de l'angle d'aimantation.
                            let magnet = hit.and_then(|id| self.scene.get(id)).and_then(|o| self.scene.material(&o.material));
                            self.focus_angle = resp.double_clicked() && magnet.is_some_and(|m| m.class == MagClass::Magnet);
                        }
                    }
                }
                Tool::Probe => self.scene.probes.push(cur.extend(0.0)),
                Tool::Seed => self.scene.seeds.push(cur.extend(0.0)),
                Tool::Sprinkle => self.visuals.sprinkle(cur, brush, self.solver.field(), self.b_max),
                Tool::Cut => {}
                Tool::Polygon | Tool::Bezier => {
                    if closing || (resp.double_clicked() && self.draft.len() >= 3) {
                        self.finish_draft();
                    }
                }
                _ => self.create(cur, cur),
            }
        }
        (self.view_center, self.view_scale) = (view.center, view.scale);

        // Matériau glissé depuis la bibliothèque : l'objet survolé en montre l'aperçu, le relâcher l'applique.
        let pointer = ui.ctx().pointer_latest_pos().filter(|p| plot.contains(*p));
        let unlocked = |id: &u32| self.scene.get(*id).is_some_and(|o| !o.locked);
        self.drop_target =
            self.drag_material.as_ref().and(pointer).and_then(|p| self.scene.pick(view.to_world(p).extend(0.0))).filter(unlocked);
        if ui.input(|i| i.pointer.any_released())
            && let (Some(name), Some(id)) = (self.drag_material.take(), self.drop_target.take())
            && let Some(o) = self.scene.get_mut(id)
        {
            o.material = name;
            self.selected = Some(id);
        }

        let icon = match (self.hot, &self.drag) {
            (Some(Hot::Rotate | Hot::Magnetize), Some(_)) => Some(CursorIcon::Grabbing),
            (Some(Hot::Rotate | Hot::Magnetize), None) => Some(CursorIcon::Grab),
            (Some(Hot::Current(_)), _) => Some(CursorIcon::PointingHand),
            _ if drafting && hover.is_some() => Some(CursorIcon::Crosshair),
            _ => None,
        };
        if let Some(icon) = icon {
            ui.ctx().set_cursor_icon(icon);
        }
        if matches!(self.hot, Some(Hot::Current(_))) {
            resp.clone().on_hover_text_at_pointer(tr("Clic : inverser le sens du courant"));
        }

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
                for o in reference.scene.objects.iter().filter(|o| o.visible) {
                    self.draw_object(&painter.with_clip_rect(left), view, o);
                }
                for o in self.scene.objects.iter().filter(|o| o.visible) {
                    self.draw_object(&painter.with_clip_rect(right), view, o);
                }
                painter.vline(x, plot.y_range(), Stroke::new(1.0, t::ACCENT));
                let y = plot.top() + 62.0;
                let before = tr("AVANT");
                let width = ui::text_width(&painter, before, mono(10.5)) + 14.0;
                ui::tag(&painter, pos2(x - 8.0 - width, y), before, t::ACCENT, t::TEXT_HI);
                ui::tag(&painter, pos2(x + 8.0, y), tr("APRÈS"), t::ACCENT, t::TEXT_HI);
            }
            _ => {
                for o in self.scene.objects.iter().filter(|o| o.visible) {
                    self.draw_object(&painter, view, o);
                }
            }
        }

        let f_max = self.wrenches.iter().map(|w| w.force.length()).fold(1e-12, f64::max);
        for w in &self.wrenches {
            let Some(o) = self.scene.get(w.id).filter(|o| o.visible) else { continue };
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
                let reach = o.world_contours().iter().flatten().map(|q| (*q - center).dot(f / f.length())).fold(0.0, f64::max);
                origin += dir * ((reach / view.scale) as f32 + 12.0);
            }
            ui::arrow(&painter, origin, origin + dir * len, 2.2, 13.0, t::FORCE);
            if selected && self.show_ui {
                let (value, unit) = units::force_per_length(f.length());
                let text = format!("F {value} {unit}");
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
                ui::tag(&painter, s + vec2(9.0, 9.0), &units::b(f.b.length()), t::ACCENT, t::TEXT_HI);
            }
        }
        if let (Some(Drag::Create(start)), Some(cur)) = (&self.drag, cur) {
            let stroke = Stroke::new(1.0, t::ACCENT);
            let (from, radius) = (view.to_screen(*start), ((cur - *start).length() / view.scale) as f32);
            let r = Rect::from_two_pos(from, view.to_screen(cur));
            match self.tool {
                Tool::Circle => {
                    painter.circle_stroke(from, radius, stroke);
                }
                Tool::Ring => {
                    painter.circle_stroke(from, radius, stroke);
                    painter.circle_stroke(from, 0.6 * radius, stroke);
                }
                Tool::Ellipse => {
                    painter.add(egui::Shape::ellipse_stroke(r.center(), r.size() / 2.0, stroke));
                }
                _ => {
                    painter.rect_stroke(r, 0.0, stroke, egui::StrokeKind::Middle);
                }
            }
        }
        self.draw_draft(&painter, view, closing, ui.input(|i| i.pointer.primary_down()));
        if let Some(handles) = self.handles(view) {
            self.draw_handles(&painter, handles);
        }
        if self.scene.objects.is_empty() && self.draft.is_empty() {
            let hint = tr(
                "Canevas vide — choisissez un outil : M aimant · R bloc · E disque · P polygone · C bobine\nMolette : zoom · clic milieu : déplacer la vue · F : cadrer",
            );
            painter.text(plot.center(), Align2::CENTER_CENTER, hint, sans(14.0), t::DIM);
        }

        if self.show_ui {
            self.rulers(&full_painter, resp.rect, view, hover);
            self.overlays(ui, &painter, view);
        }
    }

    /// Poignées de l'objet sélectionné : rotation au-dessus de lui, aimantation au bout de sa flèche.
    fn handles(&self, view: View) -> Option<Handles> {
        if self.tool != Tool::Select || !self.show_ui {
            return None;
        }
        let o = self.scene.get(self.selected?).filter(|o| o.visible && !o.locked)?;
        let center = o.pos.truncate();
        let outer = o.world_contours().into_iter().next()?;
        let up = DVec2::from_angle(o.angle + FRAC_PI_2);
        let reach = outer.iter().map(|q| (*q - center).dot(up)).fold(0.0, f64::max);
        let c = view.to_screen(center);
        let up_px = vec2(up.x as f32, -up.y as f32);
        let stem = c + up_px * (reach / view.scale) as f32;
        let magnet = self.scene.material(&o.material).filter(|m| m.class == MagClass::Magnet).map(|_| {
            let (dir, len, _) = magnet_arrow(o, &outer, view);
            c + dir * (len + 7.0)
        });
        Some(Handles { stem, rotate: stem + up_px * 30.0, magnet })
    }

    /// Poignée ou symbole de courant situé sous le point `p` de l'écran.
    fn hot_at(&self, view: View, handles: Option<Handles>, p: Pos2) -> Option<Hot> {
        if self.tool != Tool::Select {
            return None;
        }
        if let Some(h) = handles {
            if p.distance(h.rotate) <= GRAB {
                return Some(Hot::Rotate);
            }
            if h.magnet.is_some_and(|m| p.distance(m) <= GRAB) {
                return Some(Hot::Magnetize);
            }
        }
        let o = self.scene.get(self.scene.pick(view.to_world(p).extend(0.0))?)?;
        let magnet = self.scene.material(&o.material).is_some_and(|m| m.class == MagClass::Magnet);
        let live = !magnet && !o.locked && o.amp_turns() != 0.0;
        (live && p.distance(view.to_screen(o.pos.truncate())) <= GRAB).then_some(Hot::Current(o.id))
    }

    fn draw_handles(&self, p: &Painter, h: Handles) {
        let Some(o) = self.selected.and_then(|id| self.scene.get(id)) else { return };
        p.line_segment([h.stem, h.rotate], Stroke::new(1.0, t::ACCENT));
        let knob = |at: Pos2, hot: bool| {
            p.circle_filled(at, 5.0, if hot { t::ACCENT } else { t::BG });
            p.circle_stroke(at, 5.0, Stroke::new(1.5, t::ACCENT));
        };
        knob(h.rotate, self.hot == Some(Hot::Rotate));
        if let Some(m) = h.magnet {
            knob(m, self.hot == Some(Hot::Magnetize));
        }
        // Lecture de l'angle pendant le geste.
        let degrees = |a: f64| format!("{} °", fr(wrap(a).to_degrees().round() + 0.0, 0));
        match (&self.drag, h.magnet) {
            (Some(Drag::Rotate { .. }), _) => {
                ui::tag(p, h.rotate + vec2(10.0, 9.0), &degrees(o.angle), t::ACCENT, t::TEXT_HI);
            }
            (Some(Drag::Magnetize { .. }), Some(m)) => {
                ui::tag(p, m + vec2(10.0, 9.0), &degrees(o.angle + o.mag_angle), t::ACCENT, t::TEXT_HI);
            }
            _ => {}
        }
    }

    /// Chemin en cours de tracé : segments posés, élastique jusqu'au curseur, sommets et tangentes.
    fn draw_draft(&self, p: &Painter, view: View, closing: bool, pressed: bool) {
        if self.draft.is_empty() {
            return;
        }
        let mut nodes = self.draft.clone();
        if let Some(cur) = self.cursor.filter(|_| !pressed) {
            // Près du premier sommet, l'élastique s'y accroche pour annoncer la fermeture.
            nodes.push(PathNode { p: if closing { nodes[0].p } else { cur }, handle: DVec2::ZERO });
        }
        let line: Vec<Pos2> = flatten_path(&nodes, false).into_iter().map(|w| view.to_screen(w)).collect();
        p.add(egui::Shape::line(line, Stroke::new(1.2, t::ACCENT)));
        for (i, node) in self.draft.iter().enumerate() {
            let at = view.to_screen(node.p);
            if node.handle != DVec2::ZERO {
                let ends = [view.to_screen(node.p - node.handle), view.to_screen(node.p + node.handle)];
                p.line_segment(ends, Stroke::new(1.0, t::ACCENT.gamma_multiply(0.6)));
                for end in ends {
                    p.circle_filled(end, 2.5, t::ACCENT);
                }
            }
            let square = Rect::from_center_size(at, Vec2::splat(7.0));
            p.rect_filled(square, 0.0, if i == 0 && closing { t::ACCENT } else { t::BG });
            p.rect_stroke(square, 0.0, Stroke::new(1.5, t::ACCENT), egui::StrokeKind::Middle);
        }
    }

    fn draw_object(&self, p: &Painter, view: View, o: &Object) {
        // Un matériau glissé au-dessus de l'objet s'y affiche en aperçu.
        let dropped = self.drag_material.as_deref().filter(|_| self.drop_target == Some(o.id));
        let Some(mat) = self.scene.material(dropped.unwrap_or(&o.material)) else { return };
        let selected = self.selected == Some(o.id);
        let operand = self.operand.is_some_and(|(_, b)| b == o.id);
        let color = t::class_color(mat.class);
        let center = o.pos.truncate();
        let contours = o.world_contours();
        let Some(outer) = contours.first() else { return };
        let screen = |pts: &[DVec2]| -> Vec<Pos2> { pts.iter().map(|w| view.to_screen(*w)).collect() };
        let bbox = Rect::from_points(&screen(outer));
        let magnet = mat.class == MagClass::Magnet;

        // Remplissage : la section est découpée en trapèzes, ce qui couvre les formes concaves ou trouées.
        let quads = trapezoids(&contours);
        let mut mesh = egui::Mesh::default();
        let mut fill = |poly: &[DVec2], color: Color32| {
            let base = mesh.vertices.len() as u32;
            poly.iter().for_each(|q| mesh.colored_vertex(view.to_screen(*q), color));
            (2..poly.len() as u32).for_each(|k| mesh.add_triangle(base, base + k - 1, base + k));
        };
        if magnet {
            // Moitié nord en rouge, moitié sud en bleu, de part et d'autre de l'axe d'aimantation.
            let d = o.mag_dir();
            for quad in &quads {
                fill(&clip_half(quad, center, d), t::NORTH.gamma_multiply(0.34));
                fill(&clip_half(quad, center, -d), t::SOUTH.gamma_multiply(0.32));
            }
            quads.iter().for_each(|quad| fill(quad, Color32::from_rgba_unmultiplied(0x1B, 0x1C, 0x22, 64)));
        } else {
            let tint = if mat.class == MagClass::Ferro {
                Color32::from_rgb(0xD8, 0xDC, 0xE6).gamma_multiply(0.14)
            } else {
                color.gamma_multiply(0.18)
            };
            quads.iter().for_each(|quad| fill(quad, tint));
        }
        p.add(egui::Shape::mesh(mesh));
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
        let stroke = match (selected || dropped.is_some(), magnet) {
            (true, _) => Stroke::new(1.8, t::ACCENT),
            (false, true) => Stroke::new(1.2, t::TEXT_HI.gamma_multiply(0.9)),
            (false, false) => {
                Stroke::new(1.2, if mat.class == MagClass::Ferro { t::TEXT.gamma_multiply(0.75) } else { color.gamma_multiply(0.85) })
            }
        };
        for contour in &contours {
            p.add(egui::Shape::closed_line(screen(contour), stroke));
            if operand {
                // Second objet d'une opération booléenne : contour en tirets.
                let mut pts = screen(contour);
                pts.push(pts[0]);
                p.extend(egui::Shape::dashed_line(&pts, Stroke::new(1.8, t::ACCENT), 6.0, 5.0));
            }
        }
        let c = view.to_screen(center);

        if magnet {
            let (dir, len, poles) = magnet_arrow(o, outer, view);
            ui::arrow(p, c - dir * len, c + dir * len, 2.0, len.min(12.0), Color32::WHITE);
            if poles[0].min(poles[1]) > 14.0 {
                // Les pôles sont toujours désignés par leur lettre, pas seulement par la couleur.
                for (reach, letter, fill) in [(poles[0], "N", t::NORTH), (-poles[1], "S", t::SOUTH)] {
                    let badge = Rect::from_center_size(c + dir * reach, Vec2::splat(18.0));
                    p.rect_filled(badge, 0.0, fill);
                    p.text(badge.center(), Align2::CENTER_CENTER, letter, mono_bold(10.5), Color32::from_rgb(0x0B, 0x0C, 0x0F));
                }
            }
        } else if o.amp_turns() != 0.0 {
            // ⊙ courant sortant, ⊗ courant entrant ; un clic sur le symbole inverse le sens.
            let s = Stroke::new(1.5, if self.hot == Some(Hot::Current(o.id)) { t::ACCENT } else { Color32::WHITE });
            p.circle_stroke(c, 6.0, s);
            if o.amp_turns() > 0.0 {
                p.circle_filled(c, 1.8, s.color);
            } else {
                p.line_segment([c + vec2(-4.0, -4.0), c + vec2(4.0, 4.0)], s);
                p.line_segment([c + vec2(-4.0, 4.0), c + vec2(4.0, -4.0)], s);
            }
        }

        if self.show_ui && (selected || operand || bbox.width() >= 36.0) {
            let name = o.name.to_uppercase();
            let text = if operand { format!("{name} · {}", tr("OPÉRANDE")) } else { name };
            let mut at = pos2(bbox.left(), bbox.top() - 4.0);
            // L'étiquette de l'objet sélectionné s'écarte de sa poignée de rotation.
            if let Some(h) = self.handles(view).filter(|_| selected) {
                let width = ui::text_width(p, &text, mono(10.5)) + 14.0;
                let tag = Rect::from_min_max(at - vec2(0.0, 18.0), at + vec2(width, 0.0)).expand(8.0);
                if (0..=4).any(|k| tag.contains(h.stem.lerp(h.rotate, k as f32 / 4.0))) {
                    at.x = h.stem.x.min(h.rotate.x) - 12.0 - width;
                }
            }
            ui::tag(p, at, &text, if operand { t::ACCENT } else { color }, t::TEXT_HI);
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
        let widths = names.map(|(name, _)| 22.0 + 6.0 + ui::text_width(p, "0", mono(10.5)) + ui::text_width(p, tr(name), sans(12.0)));
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
            p.text(pos2(key.right() + 6.0, c.center().y), Align2::LEFT_CENTER, tr(name), sans(12.0), color);
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
            let (strong, weak) = (units::b_short(self.b_max), units::b_short(self.b_max / 1000.0));
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
            let resp = ui.interact(c, ui.id().with(("zoom", i)), Sense::click()).on_hover_text(tr(tip));
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
