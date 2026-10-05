//! Solveur MG-PCG en compute shaders wgpu (simple précision).
//!
//! Une itération de gradient conjugué est une liste figée de dispatchs,
//! construite une fois par taille de grille puis rejouée. Les produits scalaires
//! et les coefficients alpha/beta restent sur le GPU ; seuls le résidu et, en fin
//! de calcul, le potentiel remontent vers le CPU.

use crate::{COARSE_SWEEPS, Field, FieldSolver, MAX_ITERATIONS, OMEGA, SMOOTH_SWEEPS, SolveStatus, nu_hierarchy};
use flux_core::raster::RasterizedScene;
use std::time::{Duration, Instant};
use wgpu::util::DeviceExt;

// La liste de dispatchs alterne deux tampons : elle suppose ces parités.
const _: () = assert!(SMOOTH_SWEEPS == 2 && COARSE_SWEEPS.is_multiple_of(2));

const K_STENCIL: usize = 0;
const K_RESTRICT: usize = 1;
const K_PROLONG: usize = 2;
const K_AXPY: usize = 3;
const K_DOT_ROWS: usize = 4;
const K_DOT_FINAL: usize = 5;
const ENTRY_POINTS: [&str; 6] = ["k_stencil", "k_restrict", "k_prolong", "k_axpy", "k_dot_rows", "k_dot_final"];

/// Itérations de gradient conjugué par soumission.
const BATCH: u32 = 2;
/// Itérations sans progrès du résidu avant de constater la stagnation en f32.
const STALL_ITERATIONS: u32 = 16;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    n: u32,
    mode: u32,
    omega: f32,
    pad: f32,
}

struct Step {
    kernel: usize,
    bind: wgpu::BindGroup,
    groups: [u32; 2],
}

struct Level {
    n: usize,
    nu: wgpu::Buffer,
    sol: wgpu::Buffer,
    rhs: wgpu::Buffer,
    res: wgpu::Buffer,
    tmp: wgpu::Buffer,
}

struct Grid {
    n: usize,
    /// Au niveau 0, `sol` joue le rôle de z et `rhs` celui du résidu r.
    levels: Vec<Level>,
    x: wgpu::Buffer,
    b: wgpu::Buffer,
    scalars: wgpu::Buffer,
    read_x: wgpu::Buffer,
    read_scalars: wgpu::Buffer,
    init: Vec<Step>,
    iter: Vec<Step>,
}

pub struct Planar2DGpu {
    /// Résidu relatif cible (10⁻⁶ est la limite pratique en f32).
    pub tol: f64,
    device: wgpu::Device,
    queue: wgpu::Queue,
    layout: wgpu::BindGroupLayout,
    pipelines: Vec<wgpu::ComputePipeline>,
    dummies: [wgpu::Buffer; 2],
    grid: Option<Grid>,
    field: Field,
    b_norm: f64,
    residual: f64,
    best: f64,
    since_best: u32,
    iterations: u32,
    fresh: bool,
}

struct Builder<'a> {
    gpu: &'a Planar2DGpu,
    scalars: &'a wgpu::Buffer,
    steps: Vec<Step>,
}

impl Builder<'_> {
    #[allow(clippy::too_many_arguments)]
    fn push(&mut self, kernel: usize, n: usize, mode: u32, nu: &wgpu::Buffer, a: &wgpu::Buffer, b: &wgpu::Buffer, c: &wgpu::Buffer) {
        let device = &self.gpu.device;
        let params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::bytes_of(&Params { n: n as u32, mode, omega: OMEGA as f32, pad: 0.0 }),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let entries: Vec<_> = [&params, nu, a, b, c, self.scalars]
            .iter()
            .enumerate()
            .map(|(i, buf)| wgpu::BindGroupEntry { binding: i as u32, resource: buf.as_entire_binding() })
            .collect();
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &self.gpu.layout, entries: &entries });
        let m = n as u32 + 1;
        let groups = match kernel {
            K_DOT_ROWS => [m.div_ceil(64), 1],
            K_DOT_FINAL => [1, 1],
            _ => [m.div_ceil(16); 2],
        };
        self.steps.push(Step { kernel, bind, groups });
    }

    /// Produit scalaire a·b (ou a·a si `b` est absent), puis mise à jour `mode` des scalaires.
    fn dot(&mut self, lv: &Level, a: &wgpu::Buffer, b: Option<&wgpu::Buffer>, mode: u32) {
        let [d0, d1] = &self.gpu.dummies;
        // `tmp` du niveau 0 est libre hors du V-cycle : il reçoit les sommes par ligne.
        self.push(K_DOT_ROWS, lv.n, b.is_none() as u32, &lv.nu, a, b.unwrap_or(d0), &lv.tmp);
        self.push(K_DOT_FINAL, lv.n, mode, &lv.nu, &lv.tmp, d0, d1);
    }

    /// V-cycle au niveau `l` : sol ≈ A⁻¹·rhs.
    fn vcycle(&mut self, levels: &[Level], l: usize) {
        let lv = &levels[l];
        let d0 = &self.gpu.dummies[0];
        // Premier balayage depuis zéro (mode 3 : `a` n'est pas lu, `res` sert de bouche-trou).
        self.push(K_STENCIL, lv.n, 3, &lv.nu, &lv.res, &lv.rhs, &lv.tmp);
        if l + 1 == levels.len() {
            for k in 1..COARSE_SWEEPS {
                let (from, to) = if k % 2 == 1 { (&lv.tmp, &lv.sol) } else { (&lv.sol, &lv.tmp) };
                self.push(K_STENCIL, lv.n, 2, &lv.nu, from, &lv.rhs, to);
            }
            return;
        }
        let coarse = &levels[l + 1];
        self.push(K_STENCIL, lv.n, 2, &lv.nu, &lv.tmp, &lv.rhs, &lv.sol);
        self.push(K_STENCIL, lv.n, 0, &lv.nu, &lv.sol, &lv.rhs, &lv.res);
        self.push(K_RESTRICT, coarse.n, 0, &coarse.nu, &lv.res, d0, &coarse.rhs);
        self.vcycle(levels, l + 1);
        self.push(K_PROLONG, lv.n, 0, &lv.nu, &coarse.sol, d0, &lv.sol);
        self.push(K_STENCIL, lv.n, 2, &lv.nu, &lv.sol, &lv.rhs, &lv.tmp);
        self.push(K_STENCIL, lv.n, 2, &lv.nu, &lv.tmp, &lv.rhs, &lv.sol);
    }
}

impl Planar2DGpu {
    pub fn new(device: wgpu::Device, queue: wgpu::Queue) -> Planar2DGpu {
        let entry = |binding, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer { ty, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        };
        let storage = |read_only| wgpu::BufferBindingType::Storage { read_only };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("flux mg-pcg"),
            entries: &[
                entry(0, wgpu::BufferBindingType::Uniform),
                entry(1, storage(true)),
                entry(2, storage(false)),
                entry(3, storage(false)),
                entry(4, storage(false)),
                entry(5, storage(false)),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let module = device.create_shader_module(wgpu::include_wgsl!("kernels.wgsl"));
        let pipelines = ENTRY_POINTS
            .iter()
            .map(|&entry_point| {
                device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(entry_point),
                    layout: Some(&pipeline_layout),
                    module: &module,
                    entry_point: Some(entry_point),
                    compilation_options: Default::default(),
                    cache: None,
                })
            })
            .collect();
        let dummies = [storage_buffer(&device, 4), storage_buffer(&device, 4)];
        Planar2DGpu {
            tol: 1e-5,
            device,
            queue,
            layout,
            pipelines,
            dummies,
            grid: None,
            field: Field::default(),
            b_norm: 0.0,
            residual: 0.0,
            best: f64::INFINITY,
            since_best: 0,
            iterations: 0,
            fresh: false,
        }
    }

    /// Crée un solveur sur son propre périphérique (tests, outils en ligne de commande).
    pub fn headless() -> Option<Planar2DGpu> {
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .ok()?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()?;
        Some(Planar2DGpu::new(device, queue))
    }

    fn build(&self, n: usize) -> Grid {
        let nodes = (n + 1) * (n + 1);
        let levels: Vec<Level> = nu_hierarchy(n, &vec![1.0; n * n])
            .iter()
            .map(|&(n, _)| {
                let buf = || storage_buffer(&self.device, (n + 1) * (n + 1));
                Level { n, nu: storage_buffer(&self.device, n * n), sol: buf(), rhs: buf(), res: buf(), tmp: buf() }
            })
            .collect();
        let buf = || storage_buffer(&self.device, nodes);
        let (x, b, p, ap) = (buf(), buf(), buf(), buf());
        let scalars = storage_buffer(&self.device, 8);
        let staging = |len: usize| {
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: 4 * len as u64,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let l0 = &levels[0];
        let (z, r, d0) = (&l0.sol, &l0.rhs, &self.dummies[0]);

        let mut init = Builder { gpu: self, scalars: &scalars, steps: Vec::new() };
        init.push(K_STENCIL, n, 0, &l0.nu, &x, &b, r);
        init.dot(l0, r, None, 2);
        let init = init.steps;

        let mut it = Builder { gpu: self, scalars: &scalars, steps: Vec::new() };
        it.vcycle(&levels, 0);
        it.dot(l0, r, Some(z), 0);
        it.push(K_AXPY, n, 2, &l0.nu, z, d0, &p);
        it.push(K_STENCIL, n, 1, &l0.nu, &p, d0, &ap);
        it.dot(l0, &p, Some(&ap), 1);
        it.push(K_AXPY, n, 0, &l0.nu, &p, d0, &x);
        it.push(K_AXPY, n, 1, &l0.nu, &ap, d0, r);
        it.dot(l0, r, None, 2);
        let iter = it.steps;

        Grid { n, read_x: staging(nodes), read_scalars: staging(8), levels, x, b, scalars, init, iter }
    }

    /// Exécute `steps` `repeat` fois et renvoie r·r.
    fn run(&self, g: &Grid, steps: &[Step], repeat: u32) -> f64 {
        let mut enc = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_compute_pass(&Default::default());
            for _ in 0..repeat {
                for s in steps {
                    pass.set_pipeline(&self.pipelines[s.kernel]);
                    pass.set_bind_group(0, &s.bind, &[]);
                    pass.dispatch_workgroups(s.groups[0], s.groups[1], 1);
                }
            }
        }
        enc.copy_buffer_to_buffer(&g.scalars, 0, &g.read_scalars, 0, None);
        self.queue.submit([enc.finish()]);
        self.read(&g.read_scalars)[3] as f64
    }

    fn read(&self, staging: &wgpu::Buffer) -> Vec<f32> {
        let slice = staging.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        self.device.poll(wgpu::PollType::wait_indefinitely()).expect("périphérique GPU perdu");
        let data = bytemuck::pod_collect_to_vec(&slice.get_mapped_range().expect("tampon non mappé"));
        staging.unmap();
        data
    }
}

fn storage_buffer(device: &wgpu::Device, len: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 4 * len as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

impl FieldSolver for Planar2DGpu {
    fn upload(&mut self, scene: &RasterizedScene) {
        if self.grid.as_ref().is_none_or(|g| g.n != scene.n) {
            self.grid = Some(self.build(scene.n));
            self.field = Field::new(scene.n, scene.size);
        }
        self.field.size = scene.size;
        let g = self.grid.as_ref().unwrap();
        for (lv, (_, nu)) in g.levels.iter().zip(nu_hierarchy(scene.n, &scene.nu_r)) {
            let nu: Vec<f32> = nu.iter().map(|&v| v as f32).collect();
            self.queue.write_buffer(&lv.nu, 0, bytemuck::cast_slice(&nu));
        }
        let rhs = scene.rhs();
        self.b_norm = rhs.iter().map(|v| v * v).sum::<f64>().sqrt();
        let rhs: Vec<f32> = rhs.iter().map(|&v| v as f32).collect();
        self.queue.write_buffer(&g.b, 0, bytemuck::cast_slice(&rhs));
        (self.iterations, self.since_best, self.best) = (0, 0, f64::INFINITY);
        self.fresh = true;
    }

    fn solve(&mut self, budget: Duration) -> SolveStatus {
        let start = Instant::now();
        let Some(g) = self.grid.take() else {
            return SolveStatus { converged: true, ..Default::default() };
        };
        let was_fresh = self.fresh;
        if self.b_norm == 0.0 {
            self.queue.write_buffer(&g.x, 0, bytemuck::cast_slice(&vec![0f32; self.field.a.len()]));
            self.residual = 0.0;
        } else if self.fresh {
            self.queue.write_buffer(&g.scalars, 0, bytemuck::cast_slice(&[0f32, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0]));
            self.residual = self.run(&g, &g.init, 1).sqrt() / self.b_norm;
        }
        self.fresh = false;
        let mut ran = false;
        let done = |s: &Self| s.residual <= s.tol || s.iterations >= MAX_ITERATIONS || s.since_best >= STALL_ITERATIONS;
        while !done(self) && (!ran || start.elapsed() < budget) {
            self.residual = self.run(&g, &g.iter, BATCH).sqrt() / self.b_norm;
            self.iterations += BATCH;
            if self.residual < 0.9 * self.best {
                (self.best, self.since_best) = (self.residual, 0);
            } else {
                self.since_best += BATCH;
            }
            ran = true;
        }
        if ran || was_fresh {
            let mut enc = self.device.create_command_encoder(&Default::default());
            enc.copy_buffer_to_buffer(&g.x, 0, &g.read_x, 0, None);
            self.queue.submit([enc.finish()]);
            self.field.a = self.read(&g.read_x);
        }
        let converged = done(self);
        self.grid = Some(g);
        SolveStatus { converged, iterations: self.iterations, residual: self.residual, elapsed: start.elapsed() }
    }

    fn field(&self) -> &Field {
        &self.field
    }

    fn name(&self) -> &'static str {
        "GPU f32"
    }
}
