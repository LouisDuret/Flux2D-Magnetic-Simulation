//! Aimantation d'un objet : motifs (section 2.4) et grilles de valeurs attachées à l'objet
//! (directions peintes au pinceau, désaimantation irréversible).

use crate::scene::Object;
use crate::shape::Shape;
use glam::DVec2;
use serde::{Deserialize, Serialize};
use std::f64::consts::{PI, TAU};

/// Cellules d'une grille locale le long du plus grand côté de l'objet.
const RESOLUTION: f64 = 24.0;

/// Motif d'aimantation. L'angle `mag_angle` de l'objet l'oriente : direction du motif
/// uniforme, écart au rayon du motif radial, déphasage des autres.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MagPattern {
    /// Direction unique (aimantation diamétrale d'un disque ou d'un anneau).
    #[default]
    Uniform,
    /// Le long du rayon issu du centre de l'objet.
    Radial,
    /// Pôles alternés : secteurs radiaux d'une forme ronde, bandes le long des autres formes.
    Multipole { pairs: u32 },
    /// Réseau de Halbach : la direction tourne régulièrement, ce qui concentre le champ d'un
    /// seul côté (à l'intérieur d'une forme ronde, au-dessus des autres ; l'inverse si `flip`).
    Halbach { pairs: u32, flip: bool },
    /// Directions peintes au pinceau, rangées dans `Object::paint`.
    Painted,
}

/// Grille de valeurs attachée à un objet, dans son repère local.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Lattice {
    pub nx: usize,
    pub ny: usize,
    /// Coin inférieur gauche (m) et côté d'une cellule (m).
    pub origin: DVec2,
    pub cell: f64,
    pub values: Vec<f32>,
}

impl Lattice {
    /// Grille couvrant la forme, remplie de `value`.
    pub fn covering(shape: &Shape, value: f32) -> Lattice {
        let (lo, hi) = shape.bounds();
        let size = (hi - lo).max(DVec2::splat(1e-6));
        let cell = size.max_element() / RESOLUTION;
        let (nx, ny) = ((size.x / cell).ceil().max(1.0) as usize, (size.y / cell).ceil().max(1.0) as usize);
        Lattice { nx, ny, origin: lo, cell, values: vec![value; nx * ny] }
    }

    /// Indice de la cellule contenant le point local (la plus proche s'il est hors de la grille).
    pub fn index(&self, local: DVec2) -> usize {
        let g = (local - self.origin) / self.cell;
        let (i, j) = ((g.x.max(0.0) as usize).min(self.nx - 1), (g.y.max(0.0) as usize).min(self.ny - 1));
        j * self.nx + i
    }

    pub fn get(&self, local: DVec2) -> f32 {
        self.values[self.index(local)]
    }

    /// Centre de la cellule `k`, en repère local.
    pub fn center(&self, k: usize) -> DVec2 {
        self.origin + DVec2::new((k % self.nx) as f64 + 0.5, (k / self.nx) as f64 + 0.5) * self.cell
    }
}

/// Direction d'aimantation en chaque point d'un objet.
pub struct Magnetization<'a> {
    obj: &'a Object,
    /// Forme ronde : les motifs suivent l'angle polaire, sinon l'abscisse locale.
    round: bool,
    left: f64,
    width: f64,
}

impl<'a> Magnetization<'a> {
    pub fn of(obj: &'a Object) -> Magnetization<'a> {
        let round = matches!(obj.shape, Shape::Circle { .. } | Shape::Ring { .. } | Shape::Ellipse { .. });
        let (lo, hi) = obj.shape.bounds();
        Magnetization { obj, round, left: lo.x, width: (hi.x - lo.x).max(1e-9) }
    }

    /// Le motif suit l'angle polaire (forme ronde) plutôt que la longueur de l'objet.
    pub fn is_round(&self) -> bool {
        self.round
    }

    /// Angle de l'aimantation au point `local`, dans le repère de l'objet (rad).
    pub fn local_angle(&self, local: DVec2) -> f64 {
        let base = self.obj.mag_angle;
        let theta = local.to_angle();
        // Phase le long de l'objet : un tour par paire de pôles.
        let along = TAU * (local.x - self.left) / self.width;
        match self.obj.pattern {
            MagPattern::Uniform => base,
            MagPattern::Radial => theta + base,
            MagPattern::Multipole { pairs } => {
                let p = pairs.max(1) as f64;
                let south = if self.round { (p * theta).cos() < 0.0 } else { (p * along).sin() < 0.0 };
                base + if self.round { theta } else { 0.0 } + if south { PI } else { 0.0 }
            }
            MagPattern::Halbach { pairs, flip } => {
                let p = if flip { -1.0 } else { 1.0 } * pairs.max(1) as f64;
                if self.round { (1.0 + p) * theta + base } else { base + p * along }
            }
            MagPattern::Painted => self.obj.paint.as_ref().map_or(base, |lattice| lattice.get(local) as f64),
        }
    }

    /// Direction de l'aimantation au point `local`, en repère monde.
    pub fn dir(&self, local: DVec2) -> DVec2 {
        DVec2::from_angle(self.obj.angle + self.local_angle(local))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::Scene;
    use std::f64::consts::FRAC_PI_2;

    fn magnet(shape: Shape, pattern: MagPattern) -> Object {
        let mut s = Scene::default();
        let id = s.add("aimant", shape, DVec2::ZERO, "NdFeB N42");
        let o = s.get_mut(id).unwrap();
        o.pattern = pattern;
        o.clone()
    }

    #[test]
    fn patterns_on_a_ring() {
        let ring = Shape::Ring { r_in: 0.01, r_out: 0.02 };
        let at = |a: f64| DVec2::from_angle(a) * 0.015;
        let radial = magnet(ring.clone(), MagPattern::Radial);
        let m = Magnetization::of(&radial);
        for a in [0.3, 2.0, -1.2] {
            assert!((m.dir(at(a)) - DVec2::from_angle(a)).length() < 1e-12);
        }
        // Quatre pôles : nord vers l'extérieur en 0° et 180°, vers l'intérieur en ±90°.
        let quad = magnet(ring.clone(), MagPattern::Multipole { pairs: 2 });
        let m = Magnetization::of(&quad);
        assert!((m.dir(at(0.1)) - DVec2::from_angle(0.1)).length() < 1e-12);
        assert!((m.dir(at(FRAC_PI_2)) + DVec2::Y).length() < 1e-12);
        assert!((m.dir(at(PI - 0.1)) - DVec2::from_angle(PI - 0.1)).length() < 1e-12);
        // Cylindre de Halbach dipolaire : l'aimantation tourne de 2θ.
        let halbach = magnet(ring, MagPattern::Halbach { pairs: 1, flip: false });
        let m = Magnetization::of(&halbach);
        assert!((m.dir(at(FRAC_PI_2)) + DVec2::X).length() < 1e-12);
        assert!((m.dir(at(PI)) - DVec2::X).length() < 1e-12);
    }

    #[test]
    fn patterns_along_a_bar() {
        let bar = Shape::Rect { w: 0.08, h: 0.01 };
        let mut stripes = magnet(bar.clone(), MagPattern::Multipole { pairs: 2 });
        stripes.mag_angle = FRAC_PI_2;
        let m = Magnetization::of(&stripes);
        // Quatre bandes de 20 mm, aimantées vers le haut puis vers le bas.
        for (x, up) in [(-0.03, 1.0), (-0.01, -1.0), (0.01, 1.0), (0.03, -1.0)] {
            assert!((m.dir(DVec2::new(x, 0.0)) - DVec2::Y * up).length() < 1e-12, "x = {x}");
        }
        let halbach = magnet(bar, MagPattern::Halbach { pairs: 1, flip: false });
        let m = Magnetization::of(&halbach);
        // Un tour complet sur la longueur, dans le sens trigonométrique.
        assert!((m.dir(DVec2::new(-0.04, 0.0)) - DVec2::X).length() < 1e-12);
        assert!((m.dir(DVec2::new(-0.02, 0.0)) - DVec2::Y).length() < 1e-12);
        assert!((m.dir(DVec2::new(0.0, 0.0)) + DVec2::X).length() < 1e-12);
    }

    #[test]
    fn lattice_covers_the_shape() {
        let lattice = Lattice::covering(&Shape::Rect { w: 0.048, h: 0.012 }, 1.0);
        assert_eq!((lattice.nx, lattice.ny), (24, 6));
        assert!((lattice.cell - 0.002).abs() < 1e-12);
        let k = lattice.index(DVec2::new(0.0231, -0.0059));
        assert_eq!(k, 23);
        assert!((lattice.center(k) - DVec2::new(0.023, -0.005)).length() < 1e-12);
        // Hors de la grille, la cellule la plus proche répond.
        assert_eq!(lattice.index(DVec2::new(1.0, 1.0)), 24 * 6 - 1);
    }
}
