//! Rastérisation de la scène sur la grille cartésienne (section 3.3).
//!
//! Propriétés au centre des cellules, mélangées par fraction volumique.
//! Les grandeurs conservées (ampères-tours, moment magnétique) sont
//! renormalisées pour rester exactes quelle que soit la position sur la grille.

use crate::MU0;
use crate::magnet::MagPattern;
use crate::material::{MagClass, NuTable};
use crate::scene::Scene;
use crate::shape::Sdf;
use glam::DVec3;
use std::collections::BTreeMap;
use std::sync::Arc;

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
    /// Matériaux saturables (section 3.6) ; `None` si la scène est linéaire.
    pub nonlinear: Option<Nonlinear>,
    /// Cellules où un courant ou une aimantation peut être non nul.
    sources: Vec<u32>,
}

/// Cellule contenant un matériau saturable.
#[derive(Clone, Copy, Debug)]
pub struct NlCell {
    pub cell: u32,
    /// Indice de sa courbe dans `Nonlinear::curves`.
    pub curve: u16,
    /// Fraction de la cellule occupée par ce matériau.
    pub weight: f32,
    /// Réluctivité relative apportée par le reste de la cellule.
    pub base: f32,
}

/// Données de l'itération de Newton : `nu_r` est la réluctivité sécante ν(B) au potentiel de
/// linéarisation, et la jacobienne lui ajoute par cellule le terme κ·ĝ·ĝᵀ, où g = K⁰·a.
pub struct Nonlinear {
    pub curves: Vec<Arc<NuTable>>,
    pub cells: Vec<NlCell>,
    /// Coefficient κ du terme tangent, par cellule (nul hors des cellules saturables).
    pub kappa: Vec<f32>,
    /// Potentiel de linéarisation aux nœuds.
    pub lin_a: Vec<f32>,
}

/// Valeurs nodales d'une cellule (nœuds (0,0), (1,0), (0,1), (1,1)) et g = K⁰·a, où K⁰ est la
/// matrice de raideur Q1 d'une cellule carrée. Alors a·g = h²·⟨B²⟩ sur la cellule.
#[inline]
pub fn element(a: &[f32], n: usize, cell: usize) -> ([f64; 4], [f64; 4]) {
    let k = cell / n * (n + 1) + cell % n;
    let v = [a[k] as f64, a[k + 1] as f64, a[k + n + 1] as f64, a[k + n + 2] as f64];
    (v, stiffness(&v))
}

/// K⁰·v : matrice de raideur Q1 d'une cellule carrée appliquée à ses valeurs nodales.
#[inline]
pub fn stiffness(v: &[f64; 4]) -> [f64; 4] {
    [
        (4.0 * v[0] - v[1] - v[2] - 2.0 * v[3]) / 6.0,
        (4.0 * v[1] - v[0] - v[3] - 2.0 * v[2]) / 6.0,
        (4.0 * v[2] - v[0] - v[3] - 2.0 * v[1]) / 6.0,
        (4.0 * v[3] - v[1] - v[2] - 2.0 * v[0]) / 6.0,
    ]
}

/// Indices des quatre nœuds d'une cellule, dans l'ordre de `element`.
#[inline]
pub fn element_nodes(n: usize, cell: usize) -> [usize; 4] {
    let k = cell / n * (n + 1) + cell % n;
    [k, k + 1, k + n + 1, k + n + 2]
}

impl RasterizedScene {
    pub fn cell_center(&self, ci: usize, cj: usize) -> DVec3 {
        let o = -self.size / 2.0;
        DVec3::new(o + (ci as f64 + 0.5) * self.h, o + (cj as f64 + 0.5) * self.h, 0.0)
    }

    /// Second membre du système linéarisé : sources, plus le terme tangent de Newton appliqué
    /// au potentiel de linéarisation.
    pub fn rhs(&self) -> Vec<f64> {
        let mut b = self.sources();
        if let Some(nl) = &self.nonlinear {
            for c in &nl.cells {
                let kappa = nl.kappa[c.cell as usize] as f64;
                let (v, g) = element(&nl.lin_a, self.n, c.cell as usize);
                let gg: f64 = g.iter().map(|x| x * x).sum();
                if kappa == 0.0 || gg == 0.0 {
                    continue;
                }
                let scale = kappa * (0..4).map(|i| g[i] * v[i]).sum::<f64>() / gg;
                for (node, gi) in element_nodes(self.n, c.cell as usize).into_iter().zip(g) {
                    b[node] += scale * gi;
                }
            }
        }
        b
    }

    /// Second membre aux nœuds du système normalisé −∇·(ν_r ∇A) = μ0 J + rot(ν_r Br).
    pub fn sources(&self) -> Vec<f64> {
        let (n, h) = (self.n, self.h);
        let m = n + 1;
        let mut b = vec![0.0; m * m];
        for &c in &self.sources {
            let (ci, cj, c) = (c as usize % n, c as usize / n, c as usize);
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
    let mut r = RasterizedScene {
        n,
        size,
        h,
        nu_r: vec![1.0; n * n],
        jz: vec![0.0; n * n],
        mx: vec![0.0; n * n],
        my: vec![0.0; n * n],
        nonlinear: None,
        sources: Vec::new(),
    };
    let mut cover: Vec<(usize, f64)> = Vec::new();
    // Matériaux saturables : tables ν(B) et, par cellule, (courbe, fraction, reste). Deux objets
    // du même matériau à des températures différentes n'ont pas la même courbe.
    let mut curves: Vec<Arc<NuTable>> = Vec::new();
    let mut saturable: BTreeMap<usize, (u16, f32, f32)> = BTreeMap::new();
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
        let curve = mat.curve_at(obj.temperature).map(|bh| {
            let table = bh.table();
            let k = curves.iter().position(|known| Arc::ptr_eq(known, &table)).unwrap_or_else(|| {
                curves.push(table);
                curves.len() - 1
            });
            k as u16
        });
        // Un matériau saturable part de sa réluctivité à champ nul.
        let nu = curve.map_or(1.0 / mat.mu_r_solver(obj.temperature), |k| curves[k as usize].eval(0.0).0);
        let jz = obj.amp_turns() / area;
        // ν_r·Br d'un aimant, renormalisé pour que son moment magnétique soit exact.
        let strength = if mat.class == MagClass::Magnet { nu * mat.br_at(obj.temperature) * obj.shape.area() / area } else { 0.0 };
        let demag = obj.demag.as_ref().filter(|_| scene.demagnetization);
        // Un motif ou une désaimantation partielle se lisent cellule par cellule.
        let varying = (strength != 0.0 && (obj.pattern != MagPattern::Uniform || demag.is_some())).then(|| obj.magnetization());
        let uniform = obj.mag_dir() * strength;
        if jz != 0.0 || strength != 0.0 {
            r.sources.extend(cover.iter().map(|&(c, _)| c as u32));
        }
        for &(c, f) in &cover {
            let m = match &varying {
                Some(field) => {
                    let local = obj.to_local(r.cell_center(c % n, c / n)).truncate();
                    field.dir(local) * (strength * demag.map_or(1.0, |lattice| lattice.get(local) as f64))
                }
                None => uniform,
            };
            let keep = 1.0 - f;
            // Le flux traverse la surface du fer (réluctances en série : moyenne de ν) mais
            // longe celle d'un supraconducteur (en parallèle : moyenne de μ).
            let old = r.nu_r[c] as f64;
            r.nu_r[c] = (if nu > 1.0 { 1.0 / (keep / old + f / nu) } else { old * keep + nu * f }) as f32;
            if let Some(k) = curve {
                saturable.insert(c, (k, f as f32, (old * keep) as f32));
            } else if nu > 1.0 {
                saturable.remove(&c);
            } else if let Some(entry) = saturable.get_mut(&c) {
                // Un objet linéaire recouvre en partie un matériau saturable déjà posé.
                *entry = (entry.0, entry.1 * keep as f32, (entry.2 as f64 * keep + nu * f) as f32);
            }
            r.jz[c] = (r.jz[c] as f64 * keep + jz * f) as f32;
            r.mx[c] = (r.mx[c] as f64 * keep + m.x * f) as f32;
            r.my[c] = (r.my[c] as f64 * keep + m.y * f) as f32;
        }
    }
    let cells: Vec<NlCell> =
        saturable.iter().filter(|(_, e)| e.1 > 0.0).map(|(c, e)| NlCell { cell: *c as u32, curve: e.0, weight: e.1, base: e.2 }).collect();
    r.sources.sort_unstable();
    r.sources.dedup();
    if !cells.is_empty() {
        r.nonlinear = Some(Nonlinear { curves, cells, kappa: vec![0.0; n * n], lin_a: vec![0.0; (n + 1) * (n + 1)] });
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
