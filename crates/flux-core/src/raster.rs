//! Rastérisation de la scène sur la grille cartésienne (section 3.3).
//!
//! Propriétés au centre des cellules, mélangées par fraction volumique.
//! Les grandeurs conservées (ampères-tours, moment magnétique) sont
//! renormalisées pour rester exactes quelle que soit la position sur la grille.

use crate::MU0;
use crate::material::MagClass;
use crate::scene::Scene;
use crate::shape::Sdf;
use glam::DVec3;

pub struct RasterizedScene {
    /// Cellules par côté (puissance de deux).
    pub n: usize,
    /// Côté du domaine (m) et pas de grille (m).
    pub size: f64,
    pub h: f64,
    /// Réluctivité relative 1/μr par cellule.
    pub nu_r: Vec<f32>,
    /// Densité de courant Jz (A/m²).
    pub jz: Vec<f32>,
    /// ν_r·Br (T), composantes x et y.
    pub mx: Vec<f32>,
    pub my: Vec<f32>,
}

impl RasterizedScene {
    pub fn cell_center(&self, ci: usize, cj: usize) -> DVec3 {
        let o = -self.size / 2.0;
        DVec3::new(o + (ci as f64 + 0.5) * self.h, o + (cj as f64 + 0.5) * self.h, 0.0)
    }

    /// Second membre aux nœuds du système normalisé −∇·(ν_r ∇A) = μ0 J + rot(ν_r Br).
    pub fn rhs(&self) -> Vec<f64> {
        let (n, h) = (self.n, self.h);
        let m = n + 1;
        let mut b = vec![0.0; m * m];
        for cj in 0..n {
            for ci in 0..n {
                let c = cj * n + ci;
                let (j, mx, my) = (self.jz[c] as f64, self.mx[c] as f64, self.my[c] as f64);
                if j == 0.0 && mx == 0.0 && my == 0.0 {
                    continue;
                }
                let src = MU0 * j * h * h / 4.0;
                for (a, bb) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    // ∫∂v/∂x = ±h/2 selon que le nœud est à droite ou à gauche de la cellule.
                    let sx = if a == 1 { 1.0 } else { -1.0 };
                    let sy = if bb == 1 { 1.0 } else { -1.0 };
                    b[(cj + bb) * m + ci + a] += src + h / 2.0 * (sy * mx - sx * my);
                }
            }
        }
        b
    }
}

/// Fraction de la cellule centrée en `c` couverte par la forme (sur-échantillonnage 4×4 au bord).
fn coverage(sdf: impl Fn(DVec3) -> f64, c: DVec3, h: f64) -> f64 {
    let d = sdf(c);
    if d > 0.75 * h {
        return 0.0;
    }
    if d < -0.75 * h {
        return 1.0;
    }
    let mut hit = 0;
    for sj in 0..4 {
        for si in 0..4 {
            let off = DVec3::new((si as f64 + 0.5) / 4.0 - 0.5, (sj as f64 + 0.5) / 4.0 - 0.5, 0.0);
            hit += (sdf(c + off * h) <= 0.0) as u32;
        }
    }
    hit as f64 / 16.0
}

pub fn rasterize(scene: &Scene, n: usize) -> RasterizedScene {
    assert!(n.is_power_of_two() && n >= 4);
    let size = scene.size;
    let h = size / n as f64;
    let mut r = RasterizedScene { n, size, h, nu_r: vec![1.0; n * n], jz: vec![0.0; n * n], mx: vec![0.0; n * n], my: vec![0.0; n * n] };
    let mut cover: Vec<(usize, f64)> = Vec::new();
    for obj in scene.objects.iter().filter(|o| o.visible) {
        let Some(mat) = scene.material(&obj.material) else { continue };
        // Boîte englobante en indices de cellules.
        let rad = obj.shape.bounding_radius() + h;
        let lo = |v: f64| (((v - rad + size / 2.0) / h).floor().max(0.0) as usize).min(n);
        let hi = |v: f64| (((v + rad + size / 2.0) / h).ceil().max(0.0) as usize).min(n);
        cover.clear();
        let mut area = 0.0;
        for cj in lo(obj.pos.y)..hi(obj.pos.y) {
            for ci in lo(obj.pos.x)..hi(obj.pos.x) {
                let f = coverage(|p| obj.distance(p), r.cell_center(ci, cj), h);
                if f > 0.0 {
                    cover.push((cj * n + ci, f));
                    area += f * h * h;
                }
            }
        }
        if area == 0.0 {
            continue;
        }
        let nu = 1.0 / mat.mu_r_solver(obj.temperature);
        let jz = obj.amp_turns() / area;
        let m = if mat.class == MagClass::Magnet {
            obj.mag_dir() * (nu * mat.br_at(obj.temperature) * obj.shape.area() / area)
        } else {
            glam::DVec2::ZERO
        };
        for &(c, f) in &cover {
            let keep = 1.0 - f;
            // Le flux traverse la surface du fer (réluctances en série : moyenne de ν) mais
            // longe celle d'un supraconducteur (en parallèle : moyenne de μ).
            let old = r.nu_r[c] as f64;
            r.nu_r[c] = (if nu > 1.0 { 1.0 / (keep / old + f / nu) } else { old * keep + nu * f }) as f32;
            r.jz[c] = (r.jz[c] as f64 * keep + jz * f) as f32;
            r.mx[c] = (r.mx[c] as f64 * keep + m.x * f) as f32;
            r.my[c] = (r.my[c] as f64 * keep + m.y * f) as f32;
        }
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shape::Shape;
    use glam::DVec2;

    /// L'aire couverte sur la grille reproduit l'aire exacte de chaque forme.
    #[test]
    fn coverage_matches_area() {
        let star: Vec<DVec2> =
            (0..10).map(|k| DVec2::from_angle(k as f64 * std::f64::consts::PI / 5.0) * if k % 2 == 0 { 0.03 } else { 0.012 }).collect();
        let (pierced, _) = Shape::from_contours(vec![
            Shape::Rect { w: 0.05, h: 0.03 }.contours().remove(0),
            Shape::Circle { r: 0.008 }.contours().remove(0),
        ])
        .unwrap();
        let shapes =
            [Shape::Ellipse { rx: 0.03, ry: 0.012 }, Shape::Ring { r_in: 0.012, r_out: 0.02 }, Shape::Polygon { pts: star }, pierced];
        for shape in shapes {
            let mut s = Scene::default();
            let id = s.add("fer", shape.clone(), DVec2::new(0.0123, -0.0071), "Fer pur (Armco)");
            s.get_mut(id).unwrap().angle = 0.4;
            let r = rasterize(&s, 512);
            // ν relatif vaut 1 dans l'air et 1/μr dans le fer, mélangés par moyenne arithmétique :
            // on en déduit la fraction couverte de chaque cellule.
            let nu_iron = 1.0 / 5000.0;
            let covered: f64 = r.nu_r.iter().map(|&nu| (1.0 - nu as f64) / (1.0 - nu_iron)).sum::<f64>() * r.h * r.h;
            assert!((covered / shape.area() - 1.0).abs() < 0.01, "{shape:?} : {covered} au lieu de {}", shape.area());
        }
    }

    /// Un objet masqué disparaît du calcul.
    #[test]
    fn hidden_objects_are_not_rasterized() {
        let mut s = Scene::demo();
        s.objects.iter_mut().for_each(|o| o.visible = false);
        let r = rasterize(&s, 64);
        assert!(r.nu_r.iter().all(|&nu| nu == 1.0) && r.mx.iter().all(|&m| m == 0.0));
    }

    /// Les ampères-tours rastérisés sont exacts, où que soit l'objet sur la grille.
    #[test]
    fn current_is_conserved() {
        for dx in [0.0, 0.00013, 0.00041] {
            let mut s = Scene::default();
            let id = s.add("fil", Shape::Circle { r: 0.003 }, DVec2::new(dx, 0.0), "Cuivre (bobinage)");
            let o = s.get_mut(id).unwrap();
            (o.turns, o.current) = (10.0, 2.5);
            let r = rasterize(&s, 256);
            let total: f64 = r.jz.iter().map(|&j| j as f64).sum::<f64>() * r.h * r.h;
            assert!((total - 25.0).abs() < 1e-3, "{total}");
        }
    }
}
