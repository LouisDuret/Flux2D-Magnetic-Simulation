//! Export vers FEMM (femm.info), le logiciel de référence de la validation (section 3.9).
//!
//! `lua_script` écrit un script Lua pour FEMM 4.2 : il reconstruit la géométrie et les
//! matériaux de la scène, lance le calcul, puis range dans un fichier texte la force et le
//! couple subis par chaque objet (tenseur de Maxwell pondéré) et l'induction aux sondes.
//! `parse_results` relit ce fichier.

use crate::MU0;
use crate::magnet::MagPattern;
use crate::material::MagClass;
use crate::scene::{Object, Scene};
use crate::shape::{Sdf, Shape};
use glam::DVec2;
use std::fmt::Write;

/// Force et couple calculés par FEMM sur un objet, par mètre de profondeur.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FemmWrench {
    pub id: u32,
    /// N/m.
    pub force: DVec2,
    /// Couple autour du centre de l'objet (N·m/m).
    pub torque: f64,
}

/// Contenu du fichier de résultats écrit par le script.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FemmResults {
    pub wrenches: Vec<FemmWrench>,
    /// Position (m) et induction (T) de chaque sonde.
    pub probes: Vec<(DVec2, DVec2)>,
}

/// Point de l'objet le plus éloigné de son contour, en repère monde.
fn interior_point(o: &Object) -> Option<DVec2> {
    let (lo, hi) = o.shape.bounds();
    let mut best: Option<(f64, DVec2)> = None;
    for j in 0..=40 {
        for i in 0..=40 {
            let local = lo + (hi - lo) * DVec2::new(i as f64 / 40.0, j as f64 / 40.0);
            let d = o.shape.distance(local.extend(0.0));
            if d < 0.0 && best.is_none_or(|(deepest, _)| d < deepest) {
                best = Some((d, local));
            }
        }
    }
    best.map(|(_, local)| o.to_world(local))
}

/// Point d'air le plus dégagé parmi `candidates` : hors de tout objet.
fn air_point(scene: &Scene, candidates: impl Iterator<Item = DVec2>) -> Option<DVec2> {
    let clearance = |p: DVec2| scene.objects.iter().filter(|o| o.visible).map(|o| o.distance(p.extend(0.0))).fold(f64::INFINITY, f64::min);
    candidates.map(|p| (clearance(p), p)).filter(|(d, _)| *d > 0.0).max_by(|a, b| a.0.total_cmp(&b.0)).map(|(_, p)| p)
}

/// Le point est-il à l'intérieur du contour (règle pair-impair) ?
fn inside(contour: &[DVec2], p: DVec2) -> bool {
    let n = contour.len();
    (0..n)
        .filter(|&i| {
            let (a, b) = (contour[i], contour[(i + 1) % n]);
            (a.y > p.y) != (b.y > p.y) && p.x < a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x)
        })
        .count()
        % 2
        == 1
}

/// Direction d'aimantation pour FEMM : un angle en degrés, ou une formule en x et y (les
/// fonctions trigonométriques du Lua de FEMM travaillent en degrés).
fn magnet_direction(o: &Object) -> String {
    let (c, base) = (o.pos.truncate(), (o.angle + o.mag_angle).to_degrees());
    let theta = format!("atan2(y - ({:.9}), x - ({:.9}))", c.y, c.x);
    let round = o.magnetization().is_round();
    // Abscisse le long de l'objet, de 0 à 1.
    let (lo, hi) = o.shape.bounds();
    let (cos, sin) = (o.angle.cos(), o.angle.sin());
    let along = format!("(((x - ({:.9})) * ({cos:.9}) + (y - ({:.9})) * ({sin:.9}) - ({:.9})) / {:.9})", c.x, c.y, lo.x, hi.x - lo.x);
    match o.pattern {
        MagPattern::Uniform | MagPattern::Painted => format!("{base:.6}"),
        MagPattern::Radial => format!("\"{theta} + ({base:.6})\""),
        MagPattern::Multipole { pairs } if round => {
            format!("\"{theta} + ({base:.6}) + 90 - 90 * cos({pairs} * {theta}) / abs(cos({pairs} * {theta}))\"")
        }
        MagPattern::Multipole { pairs } => {
            format!("\"({base:.6}) + 90 - 90 * sin({pairs} * 360 * {along}) / abs(sin({pairs} * 360 * {along}))\"")
        }
        MagPattern::Halbach { pairs, flip } => {
            let p = if flip { -(pairs as f64) } else { pairs as f64 };
            if round { format!("\"({}) * {theta} + ({base:.6})\"", 1.0 + p) } else { format!("\"({base:.6}) + ({}) * 360 * {along}\"", p) }
        }
    }
}

/// Script Lua pour FEMM 4.2. Les résultats sont écrits dans le fichier `results` (chemin
/// absolu de préférence) ; le problème est enregistré à côté, avec l'extension `.fem`.
/// Avec `quit`, FEMM se ferme une fois le calcul terminé.
///
/// Le domaine ouvert est rendu par la condition aux limites absorbante de FEMM, sur un cercle
/// de rayon égal à la moitié du domaine de la scène. Les objets ne doivent pas se chevaucher.
pub fn lua_script(scene: &Scene, results: &str, quit: bool) -> String {
    let results = results.replace('\\', "/");
    let problem = format!("{}.fem", results.rsplit_once('.').map_or(results.as_str(), |(stem, _)| stem));
    let mut lua = String::new();
    let mut warnings = Vec::new();
    let objects: Vec<&Object> = scene.objects.iter().filter(|o| o.visible).collect();
    // `writeln!` vers une chaîne n'échoue jamais.
    macro_rules! line {
        ($($arg:tt)*) => { let _ = writeln!(lua, $($arg)*); };
    }
    line!("-- Flux2D -> FEMM 4.2 : {}", scene.name.replace('\n', " "));
    line!("newdocument(0)");
    line!("mi_probdef(0, \"meters\", \"planar\", 1e-8, {:.9}, 30)", scene.depth);
    line!("mi_addmaterial(\"Air\", 1, 1, 0, 0)");

    for (k, o) in objects.iter().enumerate() {
        let Some(mat) = scene.material(&o.material) else { continue };
        let (name, group, c) = (format!("m{}", o.id), k + 1, o.pos.truncate());
        line!("-- {} ({})", o.name, mat.name);
        // Géométrie : arcs exacts pour les cercles, segments pour le reste.
        match o.shape {
            Shape::Circle { r } | Shape::Ring { r_out: r, .. } => {
                let radii = if let Shape::Ring { r_in, .. } = o.shape { vec![r, r_in] } else { vec![r] };
                for r in radii {
                    let (a, b) = (c - DVec2::X * r, c + DVec2::X * r);
                    line!("mi_addnode({:.9}, {:.9})", a.x, a.y);
                    line!("mi_addnode({:.9}, {:.9})", b.x, b.y);
                    line!("mi_addarc({:.9}, {:.9}, {:.9}, {:.9}, 180, 2)", a.x, a.y, b.x, b.y);
                    line!("mi_addarc({:.9}, {:.9}, {:.9}, {:.9}, 180, 2)", b.x, b.y, a.x, a.y);
                }
            }
            _ => {
                for contour in o.world_contours() {
                    for p in &contour {
                        line!("mi_addnode({:.9}, {:.9})", p.x, p.y);
                    }
                    for i in 0..contour.len() {
                        let (a, b) = (contour[i], contour[(i + 1) % contour.len()]);
                        line!("mi_addsegment({:.9}, {:.9}, {:.9}, {:.9})", a.x, a.y, b.x, b.y);
                    }
                }
            }
        }
        // Matériau propre à l'objet : il dépend de sa température, de son courant, de son état.
        let mut direction = "0".to_owned();
        match mat.class {
            MagClass::Magnet => {
                let left = o.mean_remanence_left();
                if o.demag.is_some() {
                    warnings.push(format!("{} : désaimantation rendue par sa moyenne ({left:.3})", o.name));
                }
                if o.pattern == MagPattern::Painted {
                    warnings.push(format!("{} : motif peint rendu par une aimantation uniforme", o.name));
                }
                let hc = mat.br_at(o.temperature) * left / (MU0 * mat.mu_r);
                line!("mi_addmaterial(\"{name}\", {0:.9}, {0:.9}, {hc:.6}, 0)", mat.mu_r);
                direction = magnet_direction(o);
            }
            _ => {
                let mu = match mat.weak_chi(o.temperature) {
                    Some([chi, _]) => 1.0 + chi,
                    None => mat.mu_r_solver(o.temperature),
                };
                let current = o.amp_turns() / o.shape.area() * 1e-6;
                match mat.curve_at(o.temperature) {
                    Some(curve) => {
                        let mu = curve.mu_r_initial();
                        line!("mi_addmaterial(\"{name}\", {mu:.6}, {mu:.6}, 0, {current:.9})");
                        line!("mi_addbhpoint(\"{name}\", 0, 0)");
                        for p in &curve.points {
                            line!("mi_addbhpoint(\"{name}\", {:.9}, {:.6})", p[1], p[0]);
                        }
                        // Au-delà du dernier point, la pente est celle du vide.
                        let last = curve.points.last().copied().unwrap_or([1.0, MU0]);
                        for extra in [1e6, 1e7] {
                            line!("mi_addbhpoint(\"{name}\", {:.9}, {:.6})", last[1] + MU0 * extra, last[0] + extra);
                        }
                    }
                    None => {
                        line!("mi_addmaterial(\"{name}\", {mu:.9}, {mu:.9}, 0, {current:.9})");
                    }
                }
            }
        }
        match interior_point(o) {
            Some(p) => {
                let mesh = o.shape.bounding_radius() / 20.0;
                line!("mi_addblocklabel({:.9}, {:.9})", p.x, p.y);
                line!("mi_selectlabel({:.9}, {:.9})", p.x, p.y);
                line!("mi_setblockprop(\"{name}\", 0, {mesh:.9}, \"<None>\", {direction}, {group}, 0)");
                line!("mi_clearselected()");
            }
            None => warnings.push(format!("{} : aucun point intérieur trouvé", o.name)),
        }
    }

    // Air : autour des objets, et dans chacun de leurs trous.
    let radius = scene.size / 2.0;
    let around = (0..360).map(|k| DVec2::from_angle((k as f64).to_radians()) * (0.92 * radius));
    let mut air: Vec<DVec2> = air_point(scene, around).into_iter().collect();
    for o in &objects {
        for hole in o.world_contours().iter().skip(1) {
            let (lo, hi) =
                hole.iter().fold((DVec2::splat(f64::INFINITY), DVec2::splat(f64::NEG_INFINITY)), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
            let grid = (0..=30 * 31 + 30).map(|k| lo + (hi - lo) * DVec2::new((k % 31) as f64 / 30.0, (k / 31) as f64 / 30.0));
            air.extend(air_point(scene, grid.filter(|p| inside(hole, *p))));
        }
    }
    for p in &air {
        line!("mi_addblocklabel({:.9}, {:.9})", p.x, p.y);
        line!("mi_selectlabel({:.9}, {:.9})", p.x, p.y);
        line!("mi_setblockprop(\"Air\", 1, 0, \"<None>\", 0, 0, 0)");
        line!("mi_clearselected()");
    }
    line!("mi_makeABC(7, {radius:.9}, 0, 0, 0)");
    line!("mi_zoomnatural()");
    line!("mi_saveas(\"{problem}\")");
    line!("mi_analyze(1)");
    line!("mi_loadsolution()");
    line!("handle = openfile(\"{results}\", \"w\")");
    for (k, o) in objects.iter().enumerate() {
        let c = o.pos.truncate();
        line!("mo_groupselectblock({})", k + 1);
        line!("fx = mo_blockintegral(18) / {:.9}", scene.depth);
        line!("fy = mo_blockintegral(19) / {:.9}", scene.depth);
        // FEMM donne le couple autour de l'origine : on le ramène au centre de l'objet.
        line!("tz = mo_blockintegral(22) / {:.9} - (({:.9}) * fy - ({:.9}) * fx)", scene.depth, c.x, c.y);
        line!("mo_clearblock()");
        line!("write(handle, format(\"F {} %.10g %.10g %.10g\\n\", fx, fy, tz))", o.id);
    }
    for p in &scene.probes {
        line!("bx, by = mo_getb({:.9}, {:.9})", p.x, p.y);
        line!("write(handle, format(\"B {:.9} {:.9} %.10g %.10g\\n\", bx, by))", p.x, p.y);
    }
    line!("closefile(handle)");
    if quit {
        line!("quit()");
    }
    let mut head = String::new();
    for w in warnings {
        let _ = writeln!(head, "-- ATTENTION : {w}");
    }
    head + &lua
}

/// Relit le fichier de résultats écrit par le script : lignes « F id fx fy couple » et
/// « B x y bx by ». Les lignes illisibles sont ignorées.
pub fn parse_results(text: &str) -> FemmResults {
    let mut out = FemmResults::default();
    for line in text.lines() {
        let mut words = line.split_whitespace();
        let kind = words.next();
        let numbers: Vec<f64> = words.filter_map(|w| w.parse().ok()).collect();
        match (kind, numbers.as_slice()) {
            (Some("F"), &[id, fx, fy, torque]) => out.wrenches.push(FemmWrench { id: id as u32, force: DVec2::new(fx, fy), torque }),
            (Some("B"), &[x, y, bx, by]) => out.probes.push((DVec2::new(x, y), DVec2::new(bx, by))),
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec3;

    #[test]
    fn script_describes_every_object() {
        let mut scene = Scene::demo();
        let ring = scene.add("Anneau", Shape::Ring { r_in: 0.006, r_out: 0.01 }, DVec2::new(0.0, 0.05), "NdFeB N42");
        scene.get_mut(ring).unwrap().pattern = MagPattern::Radial;
        let wire = scene.add("Fil", Shape::Circle { r: 0.002 }, DVec2::new(0.0, -0.05), "Cuivre (bobinage)");
        let o = scene.get_mut(wire).unwrap();
        (o.turns, o.current) = (10.0, 2.0);
        scene.probes.push(DVec3::new(0.0, 0.0, 0.0));
        let lua = lua_script(&scene, "C:\\tmp\\demo.txt", true);
        // Aimant N42 : Hc = Br/(μ0·μrec) ; plaque de fer : sa courbe B(H) ; fil : 20 A sur 12,57 mm².
        assert!(lua.contains(&format!("mi_addmaterial(\"m1\", 1.050000000, 1.050000000, {:.6}, 0)", 1.3 / (MU0 * 1.05))));
        assert_eq!(lua.matches("mi_addbhpoint(\"m2\"").count(), 26 + 3);
        assert!(lua.contains(&format!(", 0, {:.9})", 20.0 / (std::f64::consts::PI * 4e-6) * 1e-6)));
        assert!(lua.contains("\"atan2(y - (0.050000000), x - (0.000000000)) + (0.000000)\""));
        // Un bloc par objet, l'air autour et celui du trou de l'anneau.
        assert_eq!(lua.matches("mi_addblocklabel").count(), 4 + 2);
        assert_eq!(lua.matches("mi_addarc").count(), 4 + 2);
        assert!(lua.contains("mi_saveas(\"C:/tmp/demo.fem\")") && lua.contains("openfile(\"C:/tmp/demo.txt\", \"w\")"));
        assert!(lua.contains("mo_getb(0.000000000, 0.000000000)") && lua.trim_end().ends_with("quit()"));
        assert!(!lua.contains("ATTENTION"));
    }

    #[test]
    fn results_are_parsed() {
        let text = "F 1 -26.1 0.02 1e-3\nF 2 26.0 -0.01 -2e-3\nligne étrangère\nB 0.01 0 0.25 -0.5\n";
        let results = parse_results(text);
        assert_eq!(results.wrenches.len(), 2);
        assert_eq!(results.wrenches[1], FemmWrench { id: 2, force: DVec2::new(26.0, -0.01), torque: -2e-3 });
        assert_eq!(results.probes, vec![(DVec2::new(0.01, 0.0), DVec2::new(0.25, -0.5))]);
    }
}
