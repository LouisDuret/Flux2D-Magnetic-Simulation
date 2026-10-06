//! Désaimantation irréversible des aimants (section 2.4).
//!
//! Chaque aimant porte une grille locale de la part de rémanence qui lui reste. Une fois le
//! champ convergé — jamais pendant les itérations —, chaque cellule est confrontée au champ
//! qu'elle subit le long de son axe : au-delà du coude de la courbe de désaimantation, sa
//! rémanence baisse pour de bon. Le champ est alors recalculé, et ainsi de suite jusqu'à
//! l'accord. La correction est évaluée à induction constante : elle approche l'état final
//! par valeurs supérieures, sans jamais désaimanter trop.

use crate::FieldSolver;
use crate::nonlinear::Newton;
use crate::post::Field;
use flux_core::DVec2;
use flux_core::magnet::Lattice;
use flux_core::material::MagClass;
use flux_core::raster::rasterize;
use flux_core::scene::Scene;
use flux_core::shape::Sdf;

/// Baisse de la part de rémanence au-delà de laquelle le champ doit être recalculé.
pub const DEMAG_TOLERANCE: f64 = 2e-3;
/// Baisse en dessous de laquelle une cellule n'est pas modifiée (bruit du solveur).
const NOISE: f64 = 1e-4;

/// Met à jour la désaimantation des aimants d'après le champ `field`, calculé pour cette
/// scène. Renvoie la plus grande baisse de la part de rémanence d'une cellule.
pub fn demagnetize(field: &Field, scene: &mut Scene) -> f64 {
    if !scene.demagnetization || field.n == 0 {
        return 0.0;
    }
    let depth = 0.75 * field.h();
    let mut worst: f64 = 0.0;
    for k in 0..scene.objects.len() {
        let o = &scene.objects[k];
        let Some(mat) = scene.material(&o.material).filter(|m| o.visible && m.class == MagClass::Magnet && m.hcj > 0.0) else {
            continue;
        };
        let br = mat.br_at(o.temperature);
        let mut lattice = o.demag.clone().unwrap_or_else(|| Lattice::covering(&o.shape, 1.0));
        let mut drop: f64 = 0.0;
        if br <= 0.0 {
            // Au-delà de la température de Curie, l'aimantation est perdue pour de bon.
            drop = lattice.values.iter().fold(0.0, |m, v| m.max(v.abs() as f64));
            lattice.values.fill(0.0);
        } else {
            let magnetization = o.magnetization();
            let distance = |p: DVec2| o.shape.distance(p.extend(0.0));
            for c in 0..lattice.values.len() {
                let local = lattice.center(c);
                let d = distance(local);
                if d > 0.75 * lattice.cell {
                    continue;
                }
                // Près du bord, B est lu un peu plus profond, hors des cellules de la grille
                // de calcul que l'aimant ne remplit pas.
                let mut at = local;
                if d > -depth {
                    let e = 0.25 * depth;
                    let slope = DVec2::new(
                        distance(local + DVec2::X * e) - distance(local - DVec2::X * e),
                        distance(local + DVec2::Y * e) - distance(local - DVec2::Y * e),
                    );
                    at -= slope.normalize_or_zero() * (d + depth);
                }
                let Some(sample) = field.sample(o.to_world(at).extend(0.0)) else { continue };
                let left = lattice.values[c] as f64;
                let new = mat.demagnetized(o.temperature, sample.b.truncate().dot(magnetization.dir(local)), left * br) / br;
                if new < left - NOISE {
                    lattice.values[c] = new as f32;
                    drop = drop.max(left - new);
                }
            }
        }
        if drop > 0.0 {
            scene.objects[k].demag = Some(lattice);
            worst = worst.max(drop);
        }
    }
    worst
}

/// Résout la scène sur une grille de `n`² cellules jusqu'à ce que le champ et la
/// désaimantation des aimants s'accordent. Renvoie le nombre de résolutions du champ.
pub fn solve_with_demagnetization(solver: &mut dyn FieldSolver, scene: &mut Scene, n: usize, tol: f64) -> u32 {
    const MAX_ROUNDS: u32 = 80;
    for round in 1..=MAX_ROUNDS {
        Newton::solve(solver, rasterize(scene, n), tol);
        if demagnetize(solver.field(), scene) < DEMAG_TOLERANCE {
            return round;
        }
    }
    MAX_ROUNDS
}
