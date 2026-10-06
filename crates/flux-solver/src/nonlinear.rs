//! Matériaux saturables : itération de Newton amortie autour d'un solveur linéaire (section 3.6).
//!
//! L'énergie discrète Σ h²·w(⟨B²⟩) est convexe quand B(H) est monotone. Chaque itération
//! résout le système linéarisé autour du potentiel courant (réluctivité sécante ν plus un
//! terme tangent de rang un par cellule), puis une recherche linéaire sur la dérivée de
//! l'énergie le long du pas garantit la descente, même en partant loin de la solution.

use crate::cpu::stencil;
use crate::{FieldSolver, SolveStatus};
use flux_core::raster::{NlCell, Nonlinear, RasterizedScene, element, stiffness};
use rayon::prelude::*;
use std::time::{Duration, Instant};

/// Itérations de Newton au-delà desquelles on renonce à converger.
const MAX_NEWTON: u32 = 40;

/// Réluctivité sécante d'une cellule saturable et coefficient κ de son terme tangent, pour
/// les valeurs nodales `v` et g = K⁰·v.
fn cell_state(nl: &Nonlinear, c: &NlCell, h: f64, v: &[f64; 4], g: &[f64; 4]) -> (f64, f64) {
    let ga: f64 = (0..4).map(|i| g[i] * v[i]).sum::<f64>().max(0.0);
    let gg: f64 = g.iter().map(|x| x * x).sum();
    let (nu, nu_d) = nl.curves[c.curve as usize].eval(ga.sqrt() / h);
    let w = c.weight as f64;
    (c.base as f64 + w * nu, if ga > 0.0 { w * (nu_d - nu) * gg / ga } else { 0.0 })
}

/// Linéarise le système autour du potentiel `a` : réluctivités sécantes et termes tangents.
pub fn linearize(raster: &mut RasterizedScene, a: &[f32]) {
    let (n, h) = (raster.n, raster.h);
    let Some(nl) = &mut raster.nonlinear else { return };
    nl.lin_a.copy_from_slice(a);
    for k in 0..nl.cells.len() {
        let c = nl.cells[k];
        let (v, g) = element(&nl.lin_a, n, c.cell as usize);
        let (nu, kappa) = cell_state(nl, &c, h, &v, &g);
        raster.nu_r[c.cell as usize] = nu as f32;
        nl.kappa[c.cell as usize] = kappa as f32;
    }
}

/// Pas de Newton accepté.
#[derive(Clone, Copy, Debug)]
pub struct NewtonStep {
    /// Fraction du pas conservée par la recherche linéaire (1 = pas complet).
    pub damping: f64,
    /// Variation relative du potentiel ‖Δa‖ / ‖a‖.
    pub change: f64,
}

/// Accepte tout ou partie du pas vers `trial`, solution du système linéarisé, puis relinéarise
/// autour du potentiel retenu.
pub fn newton_step(raster: &mut RasterizedScene, trial: &[f32]) -> NewtonStep {
    let (n, h) = (raster.n, raster.h);
    let Some(nl) = &raster.nonlinear else { return NewtonStep { damping: 1.0, change: 0.0 } };
    // Dérivée de l'énergie le long du pas δ = trial − a : φ'(t) = −(P + Q) + t·P + N(t), où
    // P = δᵀ·K·δ, Q vient des termes tangents et N des cellules saturables. En t = 1, P disparaît.
    let cells: Vec<([f64; 4], [f64; 4], f64)> = nl
        .cells
        .iter()
        .map(|c| {
            let (v, _) = element(&nl.lin_a, n, c.cell as usize);
            let (w, _) = element(trial, n, c.cell as usize);
            (v, [w[0] - v[0], w[1] - v[1], w[2] - v[2], w[3] - v[3]], raster.nu_r[c.cell as usize] as f64)
        })
        .collect();
    let dot = |a: &[f64; 4], b: &[f64; 4]| (0..4).map(|i| a[i] * b[i]).sum::<f64>();
    let q: f64 = nl
        .cells
        .iter()
        .zip(&cells)
        .map(|(c, (v, d, _))| {
            let g = stiffness(v);
            let gg = dot(&g, &g);
            if gg > 0.0 { nl.kappa[c.cell as usize] as f64 * dot(&g, d).powi(2) / gg } else { 0.0 }
        })
        .sum();
    let excess = |t: f64| -> f64 {
        nl.cells
            .iter()
            .zip(&cells)
            .map(|(c, (v, d, nu_k))| {
                let at = [v[0] + t * d[0], v[1] + t * d[1], v[2] + t * d[2], v[3] + t * d[3]];
                let g = stiffness(&at);
                (cell_state(nl, c, h, &at, &g).0 - nu_k) * dot(&g, d)
            })
            .sum()
    };
    let mut damping = 1.0;
    let full = excess(1.0);
    // Près de la solution, φ'(1) n'est plus que du bruit d'arrondi : le pas complet est gardé.
    if full - q > 0.02 * q.max(full.abs()) {
        // Le pas complet dépasse le minimum de l'énergie : on cherche le zéro de φ' sur ]0, 1[.
        let m = n + 1;
        let nu: Vec<f64> = raster.nu_r.iter().map(|&v| v as f64).collect();
        let delta: Vec<f64> = trial.iter().zip(&nl.lin_a).map(|(t, a)| (*t - *a) as f64).collect();
        let p: f64 = (0..m).into_par_iter().map(|j| (0..m).map(|i| stencil(n, &nu, &delta, i, j).0 * delta[j * m + i]).sum::<f64>()).sum();
        let slope = |t: f64| -(p + q) + t * p + excess(t);
        let (mut lo, mut f_lo, mut hi, mut f_hi) = (0.0, -(p + q), 1.0, full - q);
        for _ in 0..8 {
            if f_lo >= 0.0 {
                break;
            }
            // Fausse position, tenue à l'écart des bornes pour ne pas stagner.
            let t = (lo - f_lo * (hi - lo) / (f_hi - f_lo)).clamp(lo + 0.1 * (hi - lo), hi - 0.1 * (hi - lo));
            let f = slope(t);
            damping = t;
            if f.abs() < 0.02 * (p + q) {
                break;
            }
            if f < 0.0 { (lo, f_lo) = (t, f) } else { (hi, f_hi) = (t, f) }
        }
    }
    let nl = raster.nonlinear.as_ref().unwrap();
    let accepted: Vec<f32> =
        if damping == 1.0 { trial.to_vec() } else { trial.iter().zip(&nl.lin_a).map(|(t, a)| a + damping as f32 * (t - a)).collect() };
    let (mut moved, mut norm) = (0.0, 0.0);
    for (new, old) in accepted.iter().zip(&nl.lin_a) {
        moved += ((new - old) as f64).powi(2);
        norm += (*new as f64).powi(2);
    }
    linearize(raster, &accepted);
    NewtonStep { damping, change: if norm > 0.0 { (moved / norm).sqrt() } else { 0.0 } }
}

/// Résolution d'une scène, saturable ou non : chaque itération de Newton est une résolution
/// linéaire complète, reprise depuis la précédente.
pub struct Newton {
    pub raster: RasterizedScene,
    /// Variation relative du potentiel en dessous de laquelle l'itération s'arrête.
    pub tol: f64,
    /// Itérations de Newton effectuées (0 pour une scène linéaire).
    pub iterations: u32,
    /// Variation relative du potentiel au dernier pas.
    pub change: f64,
    done: bool,
}

impl Newton {
    /// Envoie la scène au solveur. Le champ déjà calculé sert de point de linéarisation.
    pub fn start(solver: &mut dyn FieldSolver, mut raster: RasterizedScene) -> Newton {
        let field = solver.field();
        if raster.nonlinear.is_some() && field.a.len() == (raster.n + 1) * (raster.n + 1) {
            let a = field.a.clone();
            linearize(&mut raster, &a);
        }
        solver.upload(&raster);
        Newton { done: raster.nonlinear.is_none(), raster, tol: 1e-4, iterations: 0, change: 0.0 }
    }

    /// Itère dans le budget. `converged` n'est vrai qu'une fois Newton lui-même convergé.
    pub fn advance(&mut self, solver: &mut dyn FieldSolver, budget: Duration) -> SolveStatus {
        let start = Instant::now();
        loop {
            let mut status = solver.solve(budget.saturating_sub(start.elapsed()));
            if !status.converged || self.done {
                return status;
            }
            let step = newton_step(&mut self.raster, &solver.field().a);
            self.iterations += 1;
            self.change = step.change;
            if step.change <= self.tol || self.iterations >= MAX_NEWTON {
                self.done = true;
                return status;
            }
            solver.upload(&self.raster);
            if start.elapsed() >= budget {
                status.converged = false;
                return status;
            }
        }
    }

    /// Résout jusqu'à convergence complète.
    pub fn solve(solver: &mut dyn FieldSolver, raster: RasterizedScene, tol: f64) -> (Newton, SolveStatus) {
        let mut newton = Newton::start(solver, raster);
        newton.tol = tol;
        loop {
            let status = newton.advance(solver, Duration::from_secs(3600));
            if status.converged {
                return (newton, status);
            }
        }
    }
}
