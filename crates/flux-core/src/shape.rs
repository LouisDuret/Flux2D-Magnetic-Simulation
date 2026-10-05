//! Formes décrites par une fonction distance signée (SDF), définie en 3D.

use glam::{DVec2, DVec3};
use serde::{Deserialize, Serialize};

/// Distance signée (négative à l'intérieur). Les formes 2D ignorent z.
pub trait Sdf {
    fn distance(&self, p: DVec3) -> f64;
    /// Rayon d'une sphère englobante centrée sur l'origine locale.
    fn bounding_radius(&self) -> f64;
}

/// Formes en repère local (centrées sur l'origine), dimensions en mètres.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum Shape {
    Rect { w: f64, h: f64 },
    Circle { r: f64 },
    Ring { r_in: f64, r_out: f64 },
    Polygon { pts: Vec<DVec2> },
}

impl Shape {
    /// Aire exacte de la section (m²).
    pub fn area(&self) -> f64 {
        use std::f64::consts::PI;
        match self {
            Shape::Rect { w, h } => w * h,
            Shape::Circle { r } => PI * r * r,
            Shape::Ring { r_in, r_out } => PI * (r_out * r_out - r_in * r_in),
            Shape::Polygon { pts } => {
                let n = pts.len();
                (0..n).map(|i| pts[i].perp_dot(pts[(i + 1) % n])).sum::<f64>().abs() / 2.0
            }
        }
    }
}

impl Sdf for Shape {
    fn distance(&self, p: DVec3) -> f64 {
        let p = p.truncate();
        match self {
            Shape::Rect { w, h } => {
                let d = p.abs() - DVec2::new(w / 2.0, h / 2.0);
                d.max(DVec2::ZERO).length() + d.x.max(d.y).min(0.0)
            }
            Shape::Circle { r } => p.length() - r,
            Shape::Ring { r_in, r_out } => {
                let mid = (r_in + r_out) / 2.0;
                (p.length() - mid).abs() - (r_out - r_in) / 2.0
            }
            Shape::Polygon { pts } => {
                let n = pts.len();
                let mut d2 = f64::INFINITY;
                let mut inside = false;
                for i in 0..n {
                    let (a, b) = (pts[i], pts[(i + 1) % n]);
                    let e = b - a;
                    let w = p - a;
                    let t = (w.dot(e) / e.length_squared().max(1e-300)).clamp(0.0, 1.0);
                    d2 = d2.min((w - e * t).length_squared());
                    if (a.y > p.y) != (b.y > p.y) && p.x < a.x + (p.y - a.y) / (b.y - a.y) * e.x {
                        inside = !inside;
                    }
                }
                if inside { -d2.sqrt() } else { d2.sqrt() }
            }
        }
    }

    fn bounding_radius(&self) -> f64 {
        match self {
            Shape::Rect { w, h } => 0.5 * w.hypot(*h),
            Shape::Circle { r } => *r,
            Shape::Ring { r_out, .. } => *r_out,
            Shape::Polygon { pts } => pts.iter().map(|p| p.length()).fold(0.0, f64::max),
        }
    }
}
