//! Solveur de référence CPU en double précision.

use crate::{COARSE_SWEEPS, Field, FieldSolver, MAX_ITERATIONS, OMEGA, SMOOTH_SWEEPS, SolveStatus, nu_hierarchy};
use flux_core::raster::RasterizedScene;
use rayon::prelude::*;
use std::time::{Duration, Instant};

/// Terme de bord de la condition de Robin asymptotique ∂A/∂n + A·(r̂·n)/r = 0,
/// concentré aux nœuds. Indépendant du pas : (n/2) / r² en unités de cellules.
fn robin(n: usize, i: usize, j: usize) -> f64 {
    if i == 0 || j == 0 || i == n || j == n {
        let c = n as f64 / 2.0;
        let (dx, dy) = (i as f64 - c, j as f64 - c);
        c / (dx * dx + dy * dy)
    } else {
        0.0
    }
}

/// Stencil Q1 à 9 points calculé à la volée : renvoie ((A·v)(i,j), diagonale).
#[inline]
fn stencil(n: usize, nu: &[f64], v: &[f64], i: usize, j: usize) -> (f64, f64) {
    let m = n + 1;
    let vc = v[j * m + i];
    let (mut av, mut diag) = (0.0, 0.0);
    for (cj, jj) in [(j.wrapping_sub(1), j.wrapping_sub(1)), (j, j + 1)] {
        if cj >= n {
            continue;
        }
        for (ci, ii) in [(i.wrapping_sub(1), i.wrapping_sub(1)), (i, i + 1)] {
            if ci >= n {
                continue;
            }
            let k = nu[cj * n + ci] / 6.0;
            av += k * (4.0 * vc - v[j * m + ii] - v[jj * m + i] - 2.0 * v[jj * m + ii]);
            diag += 4.0 * k;
        }
    }
    let rb = robin(n, i, j);
    (av + rb * vc, diag + rb)
}

/// out(i,j) = f((A·v)(i,j), diagonale, indice), en parallèle par lignes.
fn map_stencil(n: usize, nu: &[f64], v: &[f64], out: &mut [f64], f: impl Fn(f64, f64, usize) -> f64 + Sync) {
    let m = n + 1;
    out.par_chunks_mut(m).enumerate().for_each(|(j, row)| {
        for (i, o) in row.iter_mut().enumerate() {
            let (av, diag) = stencil(n, nu, v, i, j);
            *o = f(av, diag, j * m + i);
        }
    });
}

fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.par_iter().zip(b).map(|(x, y)| x * y).sum()
}

struct Level {
    n: usize,
    nu: Vec<f64>,
    x: Vec<f64>,
    b: Vec<f64>,
    r: Vec<f64>,
    t: Vec<f64>,
}

impl Level {
    /// Un balayage de Jacobi pondéré.
    fn smooth(&mut self) {
        let (x, b) = (&self.x, &self.b);
        map_stencil(self.n, &self.nu, x, &mut self.t, |av, diag, k| x[k] + OMEGA * (b[k] - av) / diag);
        std::mem::swap(&mut self.x, &mut self.t);
    }
}

/// Restriction Pᵀ (transposée de l'interpolation bilinéaire) vers `nc` cellules.
fn restrict(nc: usize, fine: &[f64], coarse: &mut [f64]) {
    let (mc, mf) = (nc + 1, 2 * nc + 1);
    for cj in 0..mc {
        for ci in 0..mc {
            let mut sum = 0.0;
            for dj in -1i64..=1 {
                for di in -1i64..=1 {
                    let (x, y) = (2 * ci as i64 + di, 2 * cj as i64 + dj);
                    if x >= 0 && y >= 0 && x < mf as i64 && y < mf as i64 {
                        let w = (1.0 - 0.5 * di.abs() as f64) * (1.0 - 0.5 * dj.abs() as f64);
                        sum += w * fine[y as usize * mf + x as usize];
                    }
                }
            }
            coarse[cj * mc + ci] = sum;
        }
    }
}

/// fine += P·coarse (interpolation bilinéaire depuis `nc` cellules).
fn prolong_add(nc: usize, coarse: &[f64], fine: &mut [f64]) {
    let (mc, mf) = (nc + 1, 2 * nc + 1);
    for j in 0..mf {
        for i in 0..mf {
            let (i0, i1, j0, j1) = (i / 2, i.div_ceil(2), j / 2, j.div_ceil(2));
            fine[j * mf + i] += 0.25 * (coarse[j0 * mc + i0] + coarse[j0 * mc + i1] + coarse[j1 * mc + i0] + coarse[j1 * mc + i1]);
        }
    }
}

/// V-cycle : résout approximativement A·x = b au niveau le plus fin de `levels`.
fn vcycle(levels: &mut [Level]) {
    let (lv, rest) = levels.split_first_mut().unwrap();
    lv.x.fill(0.0);
    if rest.is_empty() {
        (0..COARSE_SWEEPS).for_each(|_| lv.smooth());
        return;
    }
    (0..SMOOTH_SWEEPS).for_each(|_| lv.smooth());
    let b = &lv.b;
    map_stencil(lv.n, &lv.nu, &lv.x, &mut lv.r, |av, _, k| b[k] - av);
    restrict(rest[0].n, &lv.r, &mut rest[0].b);
    vcycle(rest);
    prolong_add(rest[0].n, &rest[0].x, &mut lv.x);
    (0..SMOOTH_SWEEPS).for_each(|_| lv.smooth());
}

pub struct Cpu64Reference {
    /// Résidu relatif cible.
    pub tol: f64,
    levels: Vec<Level>,
    x: Vec<f64>,
    b: Vec<f64>,
    r: Vec<f64>,
    p: Vec<f64>,
    ap: Vec<f64>,
    b_norm: f64,
    rz: f64,
    residual: f64,
    iterations: u32,
    fresh: bool,
    field: Field,
}

impl Default for Cpu64Reference {
    fn default() -> Self {
        Cpu64Reference {
            tol: 1e-8,
            levels: Vec::new(),
            x: Vec::new(),
            b: Vec::new(),
            r: Vec::new(),
            p: Vec::new(),
            ap: Vec::new(),
            b_norm: 0.0,
            rz: 0.0,
            residual: 0.0,
            iterations: 0,
            fresh: false,
            field: Field::default(),
        }
    }
}

impl Cpu64Reference {
    pub fn with_tolerance(tol: f64) -> Self {
        Cpu64Reference { tol, ..Default::default() }
    }

    /// Potentiel en double précision (pour les tests de validation).
    pub fn potential(&self) -> &[f64] {
        &self.x
    }
}

impl FieldSolver for Cpu64Reference {
    fn upload(&mut self, scene: &RasterizedScene) {
        let nodes = (scene.n + 1) * (scene.n + 1);
        if self.x.len() != nodes {
            self.x = vec![0.0; nodes];
            self.field = Field::new(scene.n, scene.size);
        }
        self.field.size = scene.size;
        (self.r, self.p, self.ap) = (vec![0.0; nodes], vec![0.0; nodes], vec![0.0; nodes]);
        self.levels = nu_hierarchy(scene.n, &scene.nu_r)
            .into_iter()
            .map(|(n, nu)| {
                let z = vec![0.0; (n + 1) * (n + 1)];
                Level { n, nu, x: z.clone(), b: z.clone(), r: z.clone(), t: z }
            })
            .collect();
        self.b = scene.rhs();
        self.b_norm = dot(&self.b, &self.b).sqrt();
        self.iterations = 0;
        self.fresh = true;
    }

    fn solve(&mut self, budget: Duration) -> SolveStatus {
        let start = Instant::now();
        if self.levels.is_empty() {
            return SolveStatus { converged: true, ..Default::default() };
        }
        let n = self.levels[0].n;
        if self.b_norm == 0.0 {
            self.x.fill(0.0);
            self.residual = 0.0;
        } else if self.fresh {
            let b = &self.b;
            map_stencil(n, &self.levels[0].nu, &self.x, &mut self.r, |av, _, k| b[k] - av);
            self.residual = dot(&self.r, &self.r).sqrt() / self.b_norm;
        }
        let mut first = std::mem::take(&mut self.fresh);
        let mut ran = false;
        while self.residual > self.tol && self.iterations < MAX_ITERATIONS && (!ran || start.elapsed() < budget) {
            self.levels[0].b.copy_from_slice(&self.r);
            vcycle(&mut self.levels);
            let z = &self.levels[0].x;
            let rz = dot(&self.r, z);
            let beta = if first { 0.0 } else { rz / self.rz };
            self.rz = rz;
            self.p.par_iter_mut().zip(z).for_each(|(p, z)| *p = z + beta * *p);
            map_stencil(n, &self.levels[0].nu, &self.p, &mut self.ap, |av, _, _| av);
            let alpha = rz / dot(&self.p, &self.ap);
            self.x.par_iter_mut().zip(&self.p).for_each(|(x, p)| *x += alpha * p);
            self.r.par_iter_mut().zip(&self.ap).for_each(|(r, ap)| *r -= alpha * ap);
            self.residual = dot(&self.r, &self.r).sqrt() / self.b_norm;
            self.iterations += 1;
            (first, ran) = (false, true);
        }
        self.field.a.iter_mut().zip(&self.x).for_each(|(a, x)| *a = *x as f32);
        SolveStatus {
            converged: self.residual <= self.tol || self.iterations >= MAX_ITERATIONS,
            iterations: self.iterations,
            residual: self.residual,
            elapsed: start.elapsed(),
        }
    }

    fn field(&self) -> &Field {
        &self.field
    }

    fn name(&self) -> &'static str {
        "CPU f64"
    }
}
