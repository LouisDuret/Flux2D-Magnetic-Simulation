//! Formes décrites par une fonction distance signée (SDF), définie en 3D.

use glam::{DVec2, DVec3};
use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::single::SingleFloatOverlay;
use serde::{Deserialize, Serialize};

/// Distance signée (négative à l'intérieur). Les formes 2D ignorent z.
pub trait Sdf {
    fn distance(&self, p: DVec3) -> f64;
    /// Rayon d'une sphère englobante centrée sur l'origine locale.
    fn bounding_radius(&self) -> f64;
}

/// Contour fermé : suite de sommets, le dernier relié au premier.
pub type Contour = Vec<DVec2>;

/// Formes en repère local (centrées sur l'origine), dimensions en mètres.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum Shape {
    Rect {
        w: f64,
        h: f64,
    },
    Circle {
        r: f64,
    },
    Ellipse {
        rx: f64,
        ry: f64,
    },
    Ring {
        r_in: f64,
        r_out: f64,
    },
    Polygon {
        pts: Contour,
    },
    /// Région à trous (résultat d'une opération booléenne) : le premier contour est
    /// l'extérieur, les suivants sont des trous.
    Region {
        contours: Vec<Contour>,
    },
}

/// Nombre de segments d'un cercle ou d'une ellipse convertis en polygone.
const ARC_SEGMENTS: usize = 96;

/// Aire signée d'un contour, positive dans le sens trigonométrique.
fn signed_area(c: &[DVec2]) -> f64 {
    let n = c.len();
    (0..n).map(|i| c[i].perp_dot(c[(i + 1) % n])).sum::<f64>() / 2.0
}

impl Shape {
    /// Aire exacte de la section (m²).
    pub fn area(&self) -> f64 {
        use std::f64::consts::PI;
        match self {
            Shape::Rect { w, h } => w * h,
            Shape::Circle { r } => PI * r * r,
            Shape::Ellipse { rx, ry } => PI * rx * ry,
            Shape::Ring { r_in, r_out } => PI * (r_out * r_out - r_in * r_in),
            Shape::Polygon { pts } => signed_area(pts).abs(),
            Shape::Region { contours } => {
                contours.iter().enumerate().map(|(i, c)| if i == 0 { signed_area(c).abs() } else { -signed_area(c).abs() }).sum()
            }
        }
    }

    /// Rectangle englobant en repère local : coins inférieur gauche et supérieur droit.
    pub fn bounds(&self) -> (DVec2, DVec2) {
        match self {
            Shape::Rect { w, h } => (DVec2::new(-w / 2.0, -h / 2.0), DVec2::new(w / 2.0, h / 2.0)),
            Shape::Circle { r } | Shape::Ring { r_out: r, .. } => (DVec2::splat(-r), DVec2::splat(*r)),
            Shape::Ellipse { rx, ry } => (DVec2::new(-rx, -ry), DVec2::new(*rx, *ry)),
            _ => self
                .contours()
                .iter()
                .flatten()
                .fold((DVec2::splat(f64::INFINITY), DVec2::splat(f64::NEG_INFINITY)), |(lo, hi), p| (lo.min(*p), hi.max(*p))),
        }
    }

    /// Longueur totale des contours, trous compris (m).
    pub fn perimeter(&self) -> f64 {
        self.contours().iter().map(|c| (0..c.len()).map(|i| c[i].distance(c[(i + 1) % c.len()])).sum::<f64>()).sum()
    }

    /// Contours de la forme en repère local (le premier extérieur, les suivants des trous).
    /// Les arcs sont convertis en polygones.
    pub fn contours(&self) -> Vec<Contour> {
        let ellipse = |rx: f64, ry: f64| -> Contour {
            (0..ARC_SEGMENTS)
                .map(|k| {
                    let a = k as f64 / ARC_SEGMENTS as f64 * std::f64::consts::TAU;
                    DVec2::new(rx * a.cos(), ry * a.sin())
                })
                .collect()
        };
        match self {
            Shape::Rect { w, h } => {
                vec![[(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)].map(|(x, y)| DVec2::new(x * w / 2.0, y * h / 2.0)).to_vec()]
            }
            Shape::Circle { r } => vec![ellipse(*r, *r)],
            Shape::Ellipse { rx, ry } => vec![ellipse(*rx, *ry)],
            Shape::Ring { r_in, r_out } => vec![ellipse(*r_out, *r_out), ellipse(*r_in, *r_in)],
            Shape::Polygon { pts } => vec![pts.clone()],
            Shape::Region { contours } => contours.clone(),
        }
    }

    /// Forme délimitée par des contours en repère monde (le premier extérieur, les suivants
    /// des trous), recentrée sur son centre de gravité. Renvoie la forme et ce centre, ou
    /// `None` si l'aire est nulle.
    pub fn from_contours(mut contours: Vec<Contour>) -> Option<(Shape, DVec2)> {
        contours.retain(|c| c.len() >= 3);
        let (mut area, mut moment) = (0.0, DVec2::ZERO);
        for (i, c) in contours.iter().enumerate() {
            let sign = if i == 0 { 1.0 } else { -1.0 };
            let a = signed_area(c);
            // Centre de gravité du contour multiplié par son aire signée.
            let m = (0..c.len()).map(|k| (c[k] + c[(k + 1) % c.len()]) * c[k].perp_dot(c[(k + 1) % c.len()])).sum::<DVec2>() / 6.0;
            area += sign * a.abs();
            moment += m * (sign * a.signum());
        }
        // En dessous de 10⁻⁴ mm², la forme est dégénérée.
        if area < 1e-10 {
            return None;
        }
        let center = moment / area;
        contours.iter_mut().flatten().for_each(|p| *p -= center);
        let shape = if contours.len() == 1 { Shape::Polygon { pts: contours.remove(0) } } else { Shape::Region { contours } };
        Some((shape, center))
    }
}

/// Découpe la région délimitée par des contours (règle pair-impair) en trapèzes à bases
/// horizontales : de quoi remplir ou faire entrer en collision n'importe quelle forme,
/// concave ou trouée.
pub fn trapezoids(contours: &[Contour]) -> Vec<[DVec2; 4]> {
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

/// Aire, centre de gravité et moment quadratique polaire ∫r² dS autour de ce centre, pour une
/// région délimitée par des contours (le premier extérieur, les suivants des trous).
pub fn moments(contours: &[Contour]) -> (f64, DVec2, f64) {
    let (mut area, mut first, mut polar) = (0.0, DVec2::ZERO, 0.0);
    for (i, c) in contours.iter().enumerate() {
        let (mut a, mut m, mut j) = (0.0, DVec2::ZERO, 0.0);
        for k in 0..c.len() {
            let (p, q) = (c[k], c[(k + 1) % c.len()]);
            let cross = p.perp_dot(q);
            a += cross / 2.0;
            m += (p + q) * cross / 6.0;
            j += cross * (p.length_squared() + p.dot(q) + q.length_squared()) / 12.0;
        }
        // Un trou se retranche, quel que soit le sens de parcours de son contour.
        let sign = if i == 0 { a.signum() } else { -a.signum() };
        area += sign * a;
        first += m * sign;
        polar += sign * j;
    }
    if area <= 0.0 {
        return (0.0, DVec2::ZERO, 0.0);
    }
    let center = first / area;
    (area, center, polar - area * center.length_squared())
}

/// Retire d'un contour fermé les sommets qui s'écartent de moins de `tol` de la ligne de leurs
/// voisins conservés (Douglas–Peucker).
pub fn simplify(contour: &[DVec2], tol: f64) -> Contour {
    fn reduce(pts: &[DVec2], tol: f64, out: &mut Contour) {
        let (a, b) = (pts[0], pts[pts.len() - 1]);
        let e = b - a;
        let far = (1..pts.len() - 1)
            .map(|k| {
                let w = pts[k] - a;
                let t = (w.dot(e) / e.length_squared().max(1e-300)).clamp(0.0, 1.0);
                (k, (w - e * t).length())
            })
            .max_by(|p, q| p.1.total_cmp(&q.1));
        match far {
            Some((k, d)) if d > tol => {
                reduce(&pts[..=k], tol, out);
                reduce(&pts[k..], tol, out);
            }
            _ => out.push(a),
        }
    }
    let n = contour.len();
    if n < 4 {
        return contour.to_vec();
    }
    // Le contour est coupé en deux chaînes entre son premier sommet et le plus éloigné de lui.
    let far =
        (1..n).max_by(|&i, &j| (contour[i] - contour[0]).length_squared().total_cmp(&(contour[j] - contour[0]).length_squared())).unwrap();
    let mut out = Vec::new();
    reduce(&contour[..=far], tol, &mut out);
    let mut back: Vec<DVec2> = contour[far..].to_vec();
    back.push(contour[0]);
    reduce(&back, tol, &mut out);
    if out.len() < 3 { contour.to_vec() } else { out }
}

/// Distance signée à une ellipse de demi-axes `a` et `b` : recherche itérative du point
/// le plus proche dans le premier quadrant.
fn ellipse_distance(p: DVec2, a: f64, b: f64) -> f64 {
    let q = p.abs();
    let (mut tx, mut ty) = (std::f64::consts::FRAC_1_SQRT_2, std::f64::consts::FRAC_1_SQRT_2);
    for _ in 0..5 {
        // Centre de courbure au point courant, puis projection de q sur le cercle osculateur.
        let e = DVec2::new((a * a - b * b) * tx.powi(3) / a, (b * b - a * a) * ty.powi(3) / b);
        let r = DVec2::new(a * tx, b * ty) - e;
        let w = q - e;
        let scale = r.length() / w.length().max(1e-300);
        tx = ((w.x * scale + e.x) / a).clamp(0.0, 1.0);
        ty = ((w.y * scale + e.y) / b).clamp(0.0, 1.0);
        let norm = tx.hypot(ty).max(1e-300);
        (tx, ty) = (tx / norm, ty / norm);
    }
    let d = (q - DVec2::new(a * tx, b * ty)).length();
    if (q.x / a).powi(2) + (q.y / b).powi(2) < 1.0 { -d } else { d }
}

/// Distance signée à une région délimitée par des contours (règle pair-impair).
fn contours_distance<'a>(contours: impl IntoIterator<Item = &'a Contour>, p: DVec2) -> f64 {
    let mut d2 = f64::INFINITY;
    let mut inside = false;
    for pts in contours {
        let n = pts.len();
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
    }
    if inside { -d2.sqrt() } else { d2.sqrt() }
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
            Shape::Ellipse { rx, ry } => ellipse_distance(p, *rx, *ry),
            Shape::Ring { r_in, r_out } => {
                let mid = (r_in + r_out) / 2.0;
                (p.length() - mid).abs() - (r_out - r_in) / 2.0
            }
            Shape::Polygon { pts } => contours_distance(std::iter::once(pts), p),
            Shape::Region { contours } => contours_distance(contours, p),
        }
    }

    fn bounding_radius(&self) -> f64 {
        match self {
            Shape::Rect { w, h } => 0.5 * w.hypot(*h),
            Shape::Circle { r } => *r,
            Shape::Ellipse { rx, ry } => rx.max(*ry),
            Shape::Ring { r_out, .. } => *r_out,
            Shape::Polygon { pts } => pts.iter().map(|p| p.length()).fold(0.0, f64::max),
            Shape::Region { contours } => contours.iter().flatten().map(|p| p.length()).fold(0.0, f64::max),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BoolOp {
    Union,
    Intersection,
    Difference,
}

/// Opération booléenne entre deux ensembles de contours (règle pair-impair). Renvoie des
/// régions disjointes, chacune formée d'un contour extérieur suivi de ses trous.
pub fn boolean(a: &[Contour], b: &[Contour], op: BoolOp) -> Vec<Vec<Contour>> {
    let arrays = |cs: &[Contour]| -> Vec<Vec<[f64; 2]>> { cs.iter().map(|c| c.iter().map(|p| p.to_array()).collect()).collect() };
    let rule = match op {
        BoolOp::Union => OverlayRule::Union,
        BoolOp::Intersection => OverlayRule::Intersect,
        BoolOp::Difference => OverlayRule::Difference,
    };
    let regions = arrays(a).overlay(&arrays(b), rule, FillRule::EvenOdd);
    regions.into_iter().map(|region| region.into_iter().map(|c| c.into_iter().map(DVec2::from_array).collect()).collect()).collect()
}

/// Nœud d'un chemin de Bézier : point d'ancrage et poignée de sortie (la poignée d'entrée
/// lui est symétrique).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PathNode {
    pub p: DVec2,
    pub handle: DVec2,
}

/// Aplatit un chemin de courbes de Bézier cubiques en polygone. Un segment dont les deux
/// poignées sont nulles reste une droite.
pub fn flatten_path(nodes: &[PathNode], closed: bool) -> Contour {
    const STEPS: usize = 16;
    let n = nodes.len();
    let mut out = Vec::new();
    for i in 0..if closed { n } else { n.saturating_sub(1) } {
        let (a, b) = (nodes[i], nodes[(i + 1) % n]);
        out.push(a.p);
        if a.handle != DVec2::ZERO || b.handle != DVec2::ZERO {
            let (c1, c2) = (a.p + a.handle, b.p - b.handle);
            out.extend((1..STEPS).map(|k| {
                let t = k as f64 / STEPS as f64;
                let u = 1.0 - t;
                a.p * (u * u * u) + c1 * (3.0 * u * u * t) + c2 * (3.0 * u * t * t) + b.p * (t * t * t)
            }));
        }
    }
    if !closed {
        out.extend(nodes.last().map(|node| node.p));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    fn rect(x: f64, y: f64, w: f64, h: f64) -> Vec<Contour> {
        vec![vec![DVec2::new(x, y), DVec2::new(x + w, y), DVec2::new(x + w, y + h), DVec2::new(x, y + h)]]
    }

    fn total_area(regions: Vec<Vec<Contour>>) -> f64 {
        regions.into_iter().filter_map(Shape::from_contours).map(|(shape, _)| shape.area()).sum()
    }

    /// La distance à l'ellipse coïncide avec une recherche exhaustive sur son contour.
    #[test]
    fn ellipse_distance_matches_brute_force() {
        let (a, b) = (0.03, 0.008);
        let shape = Shape::Ellipse { rx: a, ry: b };
        for (x, y) in [(0.05, 0.0), (0.0, 0.02), (0.02, 0.01), (-0.031, -0.004), (0.01, 0.002), (0.0, 0.0), (0.029, 0.0005)] {
            let p = DVec2::new(x, y);
            let brute = (0..200_000)
                .map(|k| (k as f64 / 200_000.0 * 2.0 * PI).sin_cos())
                .map(|(s, c)| (p - DVec2::new(a * c, b * s)).length())
                .fold(f64::INFINITY, f64::min);
            let got = shape.distance(p.extend(0.0));
            assert!((got.abs() - brute).abs() < 2e-6, "{p:?} : {got} au lieu de {brute}");
            assert_eq!(got < 0.0, (x / a).powi(2) + (y / b).powi(2) < 1.0);
        }
        let circle = Shape::Ellipse { rx: 0.01, ry: 0.01 }.distance(DVec3::new(0.03, 0.04, 0.0));
        assert!((circle - 0.04).abs() < 1e-12);
    }

    #[test]
    fn boolean_areas() {
        let (a, b) = (rect(0.0, 0.0, 2.0, 1.0), rect(1.0, 0.0, 2.0, 1.0));
        assert!((total_area(boolean(&a, &b, BoolOp::Union)) - 3.0).abs() < 1e-9);
        assert!((total_area(boolean(&a, &b, BoolOp::Intersection)) - 1.0).abs() < 1e-9);
        assert!((total_area(boolean(&a, &b, BoolOp::Difference)) - 1.0).abs() < 1e-9);
        assert!(boolean(&a, &rect(5.0, 5.0, 1.0, 1.0), BoolOp::Intersection).is_empty());
        // Une bande qui traverse le rectangle le coupe en deux morceaux.
        assert_eq!(boolean(&a, &rect(0.9, -1.0, 0.2, 3.0), BoolOp::Difference).len(), 2);
    }

    /// Un disque percé d'un disque plus petit donne une région à trou, équivalente à un anneau.
    #[test]
    fn difference_makes_a_hole() {
        let (outer, inner) = (Shape::Circle { r: 0.02 }, Shape::Circle { r: 0.01 });
        let mut regions = boolean(&outer.contours(), &inner.contours(), BoolOp::Difference);
        assert_eq!(regions.len(), 1);
        let (shape, center) = Shape::from_contours(regions.remove(0)).unwrap();
        assert!(matches!(&shape, Shape::Region { contours } if contours.len() == 2));
        assert!(center.length() < 1e-9);
        let ring = Shape::Ring { r_in: 0.01, r_out: 0.02 };
        assert!((shape.area() / ring.area() - 1.0).abs() < 2e-3);
        for x in [0.0, 0.005, 0.012, 0.015, 0.019, 0.025] {
            let p = DVec3::new(x, 0.001, 0.0);
            assert!((shape.distance(p) - ring.distance(p)).abs() < 2e-5, "x = {x}");
        }
    }

    #[test]
    fn from_contours_recenters() {
        let (shape, center) = Shape::from_contours(rect(1.0, 2.0, 4.0, 2.0)).unwrap();
        assert!((center - DVec2::new(3.0, 3.0)).length() < 1e-12);
        assert!((shape.area() - 8.0).abs() < 1e-12);
        assert!((shape.distance(DVec3::ZERO) + 1.0).abs() < 1e-12);
        assert!(Shape::from_contours(vec![vec![DVec2::ZERO, DVec2::X, DVec2::X * 2.0]]).is_none());
    }

    /// Quatre nœuds à poignées tangentes (κ = 0,5523) approchent un cercle.
    #[test]
    fn bezier_circle() {
        let k = 0.552_284_75;
        let node = |a: f64| PathNode { p: DVec2::from_angle(a), handle: DVec2::from_angle(a + PI / 2.0) * k };
        let pts = flatten_path(&[node(0.0), node(PI / 2.0), node(PI), node(1.5 * PI)], true);
        assert_eq!(pts.len(), 64);
        assert!(pts.iter().all(|p| (p.length() - 1.0).abs() < 3e-4));
        let corner = |x: f64, y: f64| PathNode { p: DVec2::new(x, y), handle: DVec2::ZERO };
        let square = [corner(0.0, 0.0), corner(1.0, 0.0), corner(1.0, 1.0), corner(0.0, 1.0)];
        assert_eq!(flatten_path(&square, true).len(), 4);
        assert_eq!(flatten_path(&square, false).len(), 4);
    }

    /// Les trapèzes pavent exactement la région ; les moments suivent les formules connues.
    #[test]
    fn trapezoids_and_moments() {
        let ring = Shape::Ring { r_in: 0.01, r_out: 0.02 };
        let contours = ring.contours();
        let tiled: f64 = trapezoids(&contours).iter().map(|t| signed_area(t).abs()).sum();
        let (area, center, polar) = moments(&contours);
        assert!((tiled / area - 1.0).abs() < 1e-9 && (area / ring.area() - 1.0).abs() < 2e-3);
        assert!(center.length() < 1e-12);
        // Anneau : J = S·(r1² + r2²)/2.
        assert!((polar / (area * (0.01f64.powi(2) + 0.02f64.powi(2)) / 2.0) - 1.0).abs() < 2e-3);

        // Rectangle décentré : le centre est retrouvé, J = S·(w² + h²)/12.
        let rect: Contour = [(0.01, 0.02), (0.05, 0.02), (0.05, 0.04), (0.01, 0.04)].map(|(x, y)| DVec2::new(x, y)).to_vec();
        let (area, center, polar) = moments(&[rect]);
        assert!((area - 8e-4).abs() < 1e-15 && (center - DVec2::new(0.03, 0.03)).length() < 1e-12);
        assert!((polar / (8e-4 * (0.04f64.powi(2) + 0.02f64.powi(2)) / 12.0) - 1.0).abs() < 1e-9);
    }

    /// La simplification garde la forme à la tolérance près et retire les sommets superflus.
    #[test]
    fn simplify_keeps_the_outline() {
        let circle = Shape::Circle { r: 0.02 }.contours().remove(0);
        let coarse = simplify(&circle, 2e-4);
        assert!(coarse.len() < 40 && coarse.len() >= 12, "{}", coarse.len());
        assert!((signed_area(&coarse).abs() / signed_area(&circle).abs() - 1.0).abs() < 0.02);
        let square = Shape::Rect { w: 0.02, h: 0.01 }.contours().remove(0);
        assert_eq!(simplify(&square, 1e-5), square);
    }
}
