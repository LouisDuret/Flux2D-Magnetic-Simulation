//! Modes de visualisation calculés sur le CPU : particules de flux, boussoles,
//! limaille saupoudrée et lignes de champ tracées depuis des graines.

use crate::app::View;
use crate::theme;
use eframe::egui::{self, Color32, Painter, Stroke, vec2};
use flux_core::{DVec2, DVec3};
use flux_solver::Field;

const PARTICLES: usize = 1500;
const TRAIL: usize = 10;
const MAX_FILINGS: usize = 8000;
const IRON: Color32 = Color32::from_rgb(0xCC, 0xD1, 0xDB);

struct Particle {
    /// Positions récentes (m), la plus récente en tête.
    trail: [DVec2; TRAIL],
    /// Durée de vie restante (s).
    life: f32,
}

pub struct Visuals {
    rng: u64,
    particles: Vec<Particle>,
    /// Angle (rad) et vitesse angulaire (rad/s) de chaque aiguille de la grille.
    needles: Vec<[f32; 2]>,
    needle_dims: (usize, usize),
    /// Grains de limaille saupoudrés à la main (m).
    pub filings: Vec<DVec2>,
    seed_lines: Vec<Vec<DVec3>>,
    /// (version du champ, nombre de graines) pour laquelle `seed_lines` est valide.
    seed_key: (u64, usize),
}

impl Default for Visuals {
    fn default() -> Self {
        Visuals {
            rng: 0x9E37_79B9_7F4A_7C15,
            particles: Vec::new(),
            needles: Vec::new(),
            needle_dims: (0, 0),
            filings: Vec::new(),
            seed_lines: Vec::new(),
            seed_key: (u64::MAX, 0),
        }
    }
}

/// Intensité ramenée à [0, 1] en échelle logarithmique, sur trois décades sous `b_max`.
fn level(b: f64, b_max: f64) -> f32 {
    (1.0 + (b / b_max).max(1e-12).log10() / 3.0).clamp(0.0, 1.0) as f32
}

impl Visuals {
    /// Nombre pseudo-aléatoire uniforme dans [0, 1) (xorshift).
    fn rand(&mut self) -> f64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Particules advectées le long de B, d'autant plus vite que le champ est fort, avec traînées.
    pub fn particles(&mut self, painter: &Painter, view: View, field: &Field, b_max: f64, dt: f32) {
        let rect = view.rect;
        let mut particles = std::mem::take(&mut self.particles);
        particles.resize_with(PARTICLES, || Particle { trail: [DVec2::NAN; TRAIL], life: 0.0 });
        for p in &mut particles {
            let head = p.trail[0];
            let alive = p.life > 0.0 && rect.contains(view.to_screen(head));
            match field.sample(head.extend(0.0)).filter(|s| alive && s.b.length_squared() > 0.0) {
                Some(s) => {
                    let speed = (30.0 + 150.0 * level(s.b.length(), b_max)) as f64 * view.scale;
                    p.trail.copy_within(0..TRAIL - 1, 1);
                    p.trail[0] = head + s.b.truncate().normalize() * speed * dt as f64;
                    p.life -= dt;
                }
                None => {
                    let at = rect.min + vec2(self.rand() as f32 * rect.width(), self.rand() as f32 * rect.height());
                    p.trail = [view.to_world(at); TRAIL];
                    p.life = 1.0 + 3.0 * self.rand() as f32;
                }
            }
            let color = Color32::from_rgb(0xDC, 0xEB, 0xFF).gamma_multiply(0.85 * p.life.clamp(0.0, 1.0));
            painter.add(egui::Shape::line(p.trail.iter().map(|w| view.to_screen(*w)).collect(), Stroke::new(1.2, color)));
        }
        self.particles = particles;
    }

    /// Grille de boussoles : chaque aiguille est rappelée vers B (couple m × B) avec
    /// inertie et amortissement. Renvoie `true` tant qu'une aiguille bouge encore.
    pub fn compasses(&mut self, painter: &Painter, view: View, field: &Field, b_max: f64, dt: f32) -> bool {
        let step = 44.0;
        let rect = view.rect;
        let dims = ((rect.width() / step) as usize + 1, (rect.height() / step) as usize + 1);
        let fresh = dims != self.needle_dims;
        if fresh {
            (self.needles, self.needle_dims) = (vec![[0.0; 2]; dims.0 * dims.1], dims);
        }
        let dt = dt.min(1.0 / 30.0);
        let mut moving = false;
        for j in 0..dims.1 {
            for i in 0..dims.0 {
                let c = rect.min + vec2(i as f32 + 0.5, j as f32 + 0.5) * step;
                let Some(s) = field.sample(view.to_world(c).extend(0.0)) else { continue };
                let b = s.b.truncate();
                if b.length() <= 1e-9 * b_max {
                    continue;
                }
                let [theta, omega] = &mut self.needles[j * dims.0 + i];
                let target = b.y.atan2(b.x) as f32;
                if fresh {
                    *theta = target;
                }
                let stiffness = 80.0 * (0.15 + level(b.length(), b_max));
                *omega += (-stiffness * (*theta - target).sin() - 7.0 * *omega) * dt;
                *theta += *omega * dt;
                moving |= omega.abs() > 0.02 || (*theta - target).sin().abs() > 0.005;

                let dir = vec2(theta.cos(), -theta.sin());
                let side = vec2(-dir.y, dir.x) * 4.0;
                let north = vec![c + dir * 15.0, c + side, c - side];
                let south = vec![c - dir * 15.0, c - side, c + side];
                painter.add(egui::Shape::convex_polygon(north, theme::NORTH, Stroke::NONE));
                painter.add(egui::Shape::convex_polygon(south, theme::TEXT, Stroke::NONE));
                painter.circle_filled(c, 1.5, theme::BG);
            }
        }
        moving
    }

    /// Saupoudre de la limaille autour d'un point : les grains s'accumulent là où le
    /// champ est fort (densité ∝ |B|^0,55).
    pub fn sprinkle(&mut self, at: DVec2, radius: f64, field: &Field, b_max: f64) {
        for _ in 0..24 {
            let p = at + DVec2::from_angle(std::f64::consts::TAU * self.rand()) * radius * self.rand().sqrt();
            let keep = field.sample(p.extend(0.0)).map_or(0.0, |s| (s.b.length() / b_max).min(1.0).powf(0.55));
            if self.rand() < keep.max(0.04) {
                self.filings.push(p);
            }
        }
        let excess = self.filings.len().saturating_sub(MAX_FILINGS);
        self.filings.drain(..excess);
    }

    /// Grains saupoudrés, orientés selon B.
    pub fn draw_filings(&self, painter: &Painter, view: View, field: &Field) {
        for p in &self.filings {
            let c = view.to_screen(*p);
            if let Some(s) = field.sample(p.extend(0.0)).filter(|_| view.rect.contains(c)) {
                let d = s.b.truncate().normalize_or_zero();
                let v = vec2(d.x as f32, -d.y as f32) * 3.5;
                painter.line_segment([c - v, c + v], Stroke::new(1.3, IRON));
            }
        }
    }

    /// Lignes de champ intégrées (Runge–Kutta 4) depuis les graines posées par l'utilisateur.
    /// `version` change à chaque nouveau champ : les lignes ne sont recalculées qu'alors.
    pub fn seed_lines(&mut self, painter: &Painter, view: View, field: &Field, seeds: &[DVec3], version: u64) {
        if self.seed_key != (version, seeds.len()) {
            self.seed_lines = seeds.iter().map(|s| field.trace(*s)).collect();
            self.seed_key = (version, seeds.len());
        }
        let stroke = Stroke::new(1.6, theme::ACCENT);
        for (seed, line) in seeds.iter().zip(&self.seed_lines) {
            // Un point sur deux suffit à l'écran (pas d'intégration d'une demi-cellule).
            painter.add(egui::Shape::line(line.iter().step_by(2).map(|p| view.to_screen(p.truncate())).collect(), stroke));
            let c = view.to_screen(seed.truncate());
            if let Some(s) = field.sample(*seed) {
                let d = s.b.truncate().normalize_or_zero();
                let v = vec2(d.x as f32, -d.y as f32) * 7.0;
                painter.arrow(c - v, v * 2.0, stroke);
            }
            painter.circle_filled(c, 3.0, theme::ACCENT);
        }
    }
}
