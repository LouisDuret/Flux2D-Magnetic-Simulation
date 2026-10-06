//! Solveurs de champ magnétostatique 2D plan (potentiel vecteur Az).
//!
//! Discrétisation Q1 sur grille cartésienne, sans matrice stockée, résolue par
//! gradient conjugué préconditionné par un V-cycle multigrille (MG-PCG).
//! `Cpu64Reference` (f64) et `Planar2DGpu` (compute wgpu, f32) appliquent
//! exactement le même algorithme.

pub mod cpu;
pub mod demag;
pub mod gpu;
pub mod nonlinear;
pub mod post;

pub use cpu::Cpu64Reference;
pub use demag::{DEMAG_TOLERANCE, demagnetize, solve_with_demagnetization};
pub use gpu::Planar2DGpu;
pub use nonlinear::Newton;
pub use post::{Field, FieldSample, Wrench, forces, forces_on};

use flux_core::raster::RasterizedScene;
use std::time::Duration;

/// Paramètres du V-cycle, partagés par les deux solveurs.
pub(crate) const OMEGA: f64 = 0.8;
pub(crate) const SMOOTH_SWEEPS: usize = 2;
pub(crate) const COARSE_SWEEPS: usize = 24;
pub(crate) const COARSEST_N: usize = 2;
/// Itérations de gradient conjugué au-delà desquelles on renonce à converger.
pub(crate) const MAX_ITERATIONS: u32 = 300;

#[derive(Clone, Copy, Debug, Default)]
pub struct SolveStatus {
    /// Résidu cible atteint (ou stagnation constatée) : inutile de rappeler `solve`.
    pub converged: bool,
    /// Itérations cumulées depuis le dernier `upload`.
    pub iterations: u32,
    /// Résidu relatif ‖b − A·x‖ / ‖b‖.
    pub residual: f64,
    pub elapsed: Duration,
}

pub trait FieldSolver {
    /// Envoie la scène rastérisée. La solution précédente sert de point de départ.
    fn upload(&mut self, scene: &RasterizedScene);
    /// Itère jusqu'au résidu cible ou jusqu'à épuisement du budget.
    fn solve(&mut self, budget: Duration) -> SolveStatus;
    /// Dernier champ calculé.
    fn field(&self) -> &Field;
    fn name(&self) -> &'static str;
}

/// Hiérarchie de réluctivités : chaque niveau moyenne 2×2 cellules du niveau plus fin.
/// La moyenne arithmétique de ν reproduit l'opérateur de Galerkin pour une
/// interpolation bilinéaire.
pub(crate) fn nu_hierarchy(n: usize, nu: &[f32]) -> Vec<(usize, Vec<f64>)> {
    let mut levels = vec![(n, nu.iter().map(|&v| v as f64).collect::<Vec<f64>>())];
    while levels.last().unwrap().0 > COARSEST_N {
        let (nf, fine) = levels.last().unwrap();
        levels.push((nf / 2, coarsen(*nf, fine)));
    }
    levels
}

/// Niveaux grossiers de la hiérarchie (le niveau fin est `nu` lui-même), en simple précision.
pub(crate) fn coarse_levels(n: usize, nu: &[f32]) -> Vec<Vec<f32>> {
    let mut levels: Vec<Vec<f32>> = Vec::new();
    let mut nf = n;
    while nf > COARSEST_N {
        let coarse = coarsen(nf, levels.last().map_or(nu, Vec::as_slice));
        levels.push(coarse);
        nf /= 2;
    }
    levels
}

/// Moyenne 2×2 des cellules d'un niveau de `nf` cellules de côté.
fn coarsen<T>(nf: usize, fine: &[T]) -> Vec<T>
where
    T: Copy + Send + Sync + std::ops::Add<Output = T> + std::ops::Mul<Output = T> + From<f32>,
{
    use rayon::prelude::*;
    let nc = nf / 2;
    let quarter = T::from(0.25);
    let mut coarse = vec![quarter; nc * nc];
    coarse.par_chunks_mut(nc).enumerate().for_each(|(cj, row)| {
        for (ci, out) in row.iter_mut().enumerate() {
            let k = 2 * cj * nf + 2 * ci;
            *out = quarter * (fine[k] + fine[k + 1] + fine[k + nf] + fine[k + nf + 1]);
        }
    });
    coarse
}
