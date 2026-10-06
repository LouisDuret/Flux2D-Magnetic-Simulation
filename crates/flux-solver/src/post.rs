//! Post-traitement : échantillonnage du champ et forces par objet.

use flux_core::MU0;
use flux_core::material::MagClass;
use flux_core::scene::Scene;
use flux_core::shape::Sdf;
use glam::{DVec2, DVec3};

/// Potentiel vecteur Az (T·m) aux nœuds d'une grille de `n`×`n` cellules.
#[derive(Clone, Default)]
pub struct Field {
    pub n: usize,
    pub size: f64,
    pub a: Vec<f32>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct FieldSample {
    /// Potentiel vecteur Az (T·m).
    pub a: f64,
    /// Induction (T), z = 0 en 2D plan.
    pub b: DVec3,
}

/// Force et couple magnétiques sur un objet, par mètre de profondeur.
#[derive(Clone, Copy, Debug)]
pub struct Wrench {
    pub id: u32,
    /// N/m.
    pub force: DVec3,
    /// Couple autour de z, au centre de l'objet (N·m/m).
    pub torque: f64,
    /// Un entrefer plus fin que la grille rend la valeur dépendante de la résolution.
    pub resolution_limited: bool,
}

/// Épaisseur de la coquille d'intégration, en cellules.
const SHELL: f64 = 6.0;

impl Field {
    pub fn new(n: usize, size: f64) -> Field {
        Field { n, size, a: vec![0.0; (n + 1) * (n + 1)] }
    }

    pub fn h(&self) -> f64 {
        self.size / self.n as f64
    }

    fn node(&self, i: usize, j: usize) -> f64 {
        self.a[j * (self.n + 1) + i] as f64
    }

    /// B constant dans la cellule : Bx = ∂A/∂y, By = −∂A/∂x.
    pub fn b_cell(&self, ci: usize, cj: usize) -> DVec2 {
        let (a00, a10) = (self.node(ci, cj), self.node(ci + 1, cj));
        let (a01, a11) = (self.node(ci, cj + 1), self.node(ci + 1, cj + 1));
        DVec2::new((a01 + a11) - (a00 + a10), (a00 + a01) - (a10 + a11)) / (2.0 * self.h())
    }

    /// Champ en un point ; `None` hors du domaine.
    pub fn sample(&self, p: DVec3) -> Option<FieldSample> {
        if self.n == 0 {
            return None;
        }
        let h = self.h();
        let g = (p.truncate() + DVec2::splat(self.size / 2.0)) / h;
        if g.min_element() < 0.0 || g.max_element() > self.n as f64 {
            return None;
        }
        let ci = (g.x as usize).min(self.n - 1);
        let cj = (g.y as usize).min(self.n - 1);
        let (fx, fy) = (g.x - ci as f64, g.y - cj as f64);
        let (a00, a10) = (self.node(ci, cj), self.node(ci + 1, cj));
        let (a01, a11) = (self.node(ci, cj + 1), self.node(ci + 1, cj + 1));
        let a = (1.0 - fy) * ((1.0 - fx) * a00 + fx * a10) + fy * ((1.0 - fx) * a01 + fx * a11);
        // B est constant par cellule : on l'interpole entre centres de cellules (ordre 2).
        let c = (g - 0.5).clamp(DVec2::ZERO, DVec2::splat((self.n - 1) as f64));
        let (bi, bj) = ((c.x as usize).min(self.n - 2), (c.y as usize).min(self.n - 2));
        let (tx, ty) = (c.x - bi as f64, c.y - bj as f64);
        let b = (1.0 - ty) * ((1.0 - tx) * self.b_cell(bi, bj) + tx * self.b_cell(bi + 1, bj))
            + ty * ((1.0 - tx) * self.b_cell(bi, bj + 1) + tx * self.b_cell(bi + 1, bj + 1));
        Some(FieldSample { a, b: b.extend(0.0) })
    }

    /// Plus grande induction du domaine (T).
    pub fn b_max(&self) -> f64 {
        let n = self.n;
        (0..n * n).map(|c| self.b_cell(c % n, c / n).length_squared()).fold(0.0, f64::max).sqrt()
    }

    /// Étendue (min, max) du potentiel.
    pub fn a_range(&self) -> (f64, f64) {
        self.a.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), &v| (lo.min(v as f64), hi.max(v as f64)))
    }
}

/// Forces par objet : tenseur de Maxwell pondéré (méthode de la coquille).
///
/// F = −∫ T·∇g dS, où g vaut 1 sur l'objet et 0 sur les autres objets et au loin.
/// Le support de ∇g reste dans l'air, où T = (B⊗B − ½B²·I)/μ0.
pub fn forces(field: &Field, scene: &Scene) -> Vec<Wrench> {
    let (n, h) = (field.n, field.h());
    if n == 0 {
        return Vec::new();
    }
    let half = field.size / 2.0;
    let mut out = Vec::with_capacity(scene.objects.len());
    let mut g = Vec::new();
    for obj in scene.objects.iter().filter(|o| o.visible) {
        let rad = obj.shape.bounding_radius() + (SHELL + 2.0) * h;
        let lo = |v: f64| (((v - rad + half) / h).floor().max(0.0) as usize).min(n);
        let hi = |v: f64| (((v + rad + half) / h).ceil().max(0.0) as usize).min(n);
        let (i0, i1, j0, j1) = (lo(obj.pos.x), hi(obj.pos.x), lo(obj.pos.y), hi(obj.pos.y));
        let w = i1 - i0 + 1;
        // Para/diamagnétiques : invisibles pour le solveur (μr = 1), ils subissent la force
        // de Kelvin F = (χ/2μ0)·∫ ∇(B²) dS, exacte au premier ordre en χ.
        let kelvin =
            scene.material(&obj.material).filter(|m| matches!(m.class, MagClass::Para | MagClass::Dia)).map(|m| m.chi_at(obj.temperature));
        let mut limited = false;
        g.clear();
        for j in j0..=j1 {
            for i in i0..=i1 {
                let p = DVec3::new(i as f64 * h - half, j as f64 * h - half, 0.0);
                let ds = obj.distance(p);
                if kelvin.is_some() {
                    // Indicatrice de l'objet, adoucie sur une cellule.
                    g.push((0.5 - ds / h).clamp(0.0, 1.0));
                    continue;
                }
                let d_other =
                    scene.objects.iter().filter(|o| o.visible && o.id != obj.id).map(|o| o.distance(p)).fold(f64::INFINITY, f64::min);
                // La coquille démarre à une cellule de chaque surface, hors des cellules mixtes.
                let s = (ds - h).max(0.0);
                let o = (d_other - h).max(0.0);
                let near = if s + o > 0.0 {
                    o / (s + o)
                } else {
                    limited = true;
                    (ds <= d_other) as u8 as f64
                };
                g.push(near.min((1.0 - s / (SHELL * h)).clamp(0.0, 1.0)));
            }
        }
        let mut f = DVec2::ZERO;
        let mut torque = 0.0;
        for j in j0..j1 {
            for i in i0..i1 {
                let k = (j - j0) * w + (i - i0);
                let (g00, g10, g01, g11) = (g[k], g[k + 1], g[k + w], g[k + w + 1]);
                let grad = DVec2::new((g10 + g11) - (g00 + g01), (g01 + g11) - (g00 + g10)) / (2.0 * h);
                if grad == DVec2::ZERO {
                    continue;
                }
                let b = field.b_cell(i, j);
                let t_grad = match kelvin {
                    // ∫ g·∇(B²) dS = −∫ B²·∇g dS.
                    Some(chi) => 0.5 * chi * b.length_squared() * grad / MU0,
                    None => (b * b.dot(grad) - 0.5 * b.length_squared() * grad) / MU0,
                };
                let df = -t_grad * h * h;
                let r = DVec2::new((i as f64 + 0.5) * h - half, (j as f64 + 0.5) * h - half) - obj.pos.truncate();
                f += df;
                torque += r.perp_dot(df);
            }
        }
        out.push(Wrench { id: obj.id, force: f.extend(0.0), torque, resolution_limited: limited });
    }
    out
}

impl Field {
    /// Ligne de champ passant par `seed`, intégrée par Runge–Kutta 4 dans les deux sens
    /// jusqu'au bord du domaine, à un point de champ nul ou à la fermeture de la boucle.
    pub fn trace(&self, seed: DVec3) -> Vec<DVec3> {
        let step = 0.5 * self.h();
        let dir = |p: DVec3| self.sample(p).map(|s| s.b.normalize_or_zero()).filter(|d| *d != DVec3::ZERO);
        let mut line = vec![seed];
        for sign in [1.0, -1.0] {
            let s = sign * step;
            let mut p = seed;
            let mut pts = Vec::new();
            for k in 0..8 * self.n {
                let Some(k1) = dir(p) else { break };
                let Some(k2) = dir(p + k1 * (s / 2.0)) else { break };
                let Some(k3) = dir(p + k2 * (s / 2.0)) else { break };
                let Some(k4) = dir(p + k3 * s) else { break };
                p += (k1 + 2.0 * k2 + 2.0 * k3 + k4) * (s / 6.0);
                pts.push(p);
                if k > 8 && p.distance(seed) < step {
                    // Boucle fermée : inutile d'intégrer dans l'autre sens.
                    line.extend(pts);
                    line.push(seed);
                    return line;
                }
            }
            if sign > 0.0 {
                line.extend(pts);
            } else {
                pts.reverse();
                pts.extend(line);
                line = pts;
            }
        }
        line
    }
}
