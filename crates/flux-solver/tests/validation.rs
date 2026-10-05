//! Validation physique (section 3.9 du document) et comparaison GPU/CPU.

use flux_core::material::{MagClass, Material};
use flux_core::raster::rasterize;
use flux_core::scene::Scene;
use flux_core::shape::Shape;
use flux_core::{DVec2, DVec3, MU0};
use flux_solver::{Cpu64Reference, FieldSolver, Planar2DGpu, SolveStatus, forces};
use std::f64::consts::PI;
use std::time::{Duration, Instant};

fn solve(solver: &mut dyn FieldSolver, scene: &Scene, n: usize) -> SolveStatus {
    solver.upload(&rasterize(scene, n));
    let start = Instant::now();
    loop {
        let status = solver.solve(Duration::from_millis(200));
        if status.converged {
            println!("{} {n}² : {} itérations, résidu {:.1e}, {:?}", solver.name(), status.iterations, status.residual, start.elapsed());
            return status;
        }
    }
}

fn wire(scene: &mut Scene, x: f64, amps: f64) -> u32 {
    let id = scene.add("fil", Shape::Circle { r: 0.003 }, DVec2::new(x, 0.0), "Cuivre (bobinage)");
    let o = scene.get_mut(id).unwrap();
    (o.turns, o.current) = (1.0, amps);
    id
}

fn assert_close(got: f64, want: f64, tol: f64, what: &str) {
    let err = (got - want).abs() / want.abs();
    println!("{what} : {got:.6e} (attendu {want:.6e}, écart {:.3} %)", err * 100.0);
    assert!(err < tol, "{what} : écart {:.3} % > {} %", err * 100.0, tol * 100.0);
}

/// Fil infini : B = μ0·I / (2πr).
#[test]
fn infinite_wire() {
    let mut scene = Scene::default();
    scene.size = 0.2;
    wire(&mut scene, 0.0, 10.0);
    let mut cpu = Cpu64Reference::default();
    assert!(solve(&mut cpu, &scene, 256).residual <= 1e-8);
    for r in [0.008, 0.015, 0.025] {
        for dir in [DVec2::X, DVec2::Y, DVec2::ONE.normalize()] {
            let b = cpu.field().sample((dir * r).extend(0.0)).unwrap().b;
            assert_close(b.length(), MU0 * 10.0 / (2.0 * PI * r), 0.005, &format!("|B| à r = {r}"));
            // Courant sortant : B tourne dans le sens trigonométrique.
            assert!(dir.perp_dot(b.truncate()) > 0.0);
        }
    }
}

/// Cylindre aimanté transversalement (μrec = 1) : B intérieur uniforme = Br/2.
#[test]
fn magnetized_cylinder() {
    let mut scene = Scene::default();
    scene.materials.push(Material {
        name: "idéal".into(),
        class: MagClass::Magnet,
        mu_r: 1.0,
        chi: 0.0,
        br: 1.0,
        alpha_br: 0.0,
        t_curie: 0.0,
        t_critical: 0.0,
        density: 7500.0,
    });
    scene.add("cyl", Shape::Circle { r: 0.02 }, DVec2::ZERO, "idéal");
    let mut cpu = Cpu64Reference::default();
    solve(&mut cpu, &scene, 256);
    for p in [DVec3::ZERO, DVec3::new(0.006, 0.005, 0.0), DVec3::new(-0.004, 0.009, 0.0)] {
        let b = cpu.field().sample(p).unwrap().b;
        assert_close(b.x, 0.5, 0.01, "Bx intérieur");
        assert!(b.y.abs() < 0.005, "By = {}", b.y);
    }
}

/// Deux fils parallèles : F/ℓ = μ0·I1·I2 / (2πd), attractive pour des courants de même sens.
#[test]
fn parallel_wires_force() {
    let mut scene = Scene::default();
    let left = wire(&mut scene, -0.02, 100.0);
    let right = wire(&mut scene, 0.02, 100.0);
    let mut cpu = Cpu64Reference::default();
    solve(&mut cpu, &scene, 256);
    let w = forces(cpu.field(), &scene);
    let want = MU0 * 1e4 / (2.0 * PI * 0.04);
    let (fl, fr) = (w.iter().find(|w| w.id == left).unwrap(), w.iter().find(|w| w.id == right).unwrap());
    assert_close(fl.force.x, want, 0.01, "Fx fil gauche");
    assert_close(fr.force.x, -want, 0.01, "Fx fil droit");
    assert!(fl.force.y.abs() < 1e-3 * want && !fl.resolution_limited);
}

/// Aimant + plaque de fer : attraction, et action = réaction.
#[test]
fn magnet_attracts_iron() {
    let scene = Scene::demo();
    let mut cpu = Cpu64Reference::default();
    solve(&mut cpu, &scene, 512);
    let w = forces(cpu.field(), &scene);
    println!("aimant {:?}\nplaque {:?}", w[0], w[1]);
    assert!(w[1].force.x < -1.0, "la plaque doit être attirée vers l'aimant");
    assert_close(w[0].force.x, -w[1].force.x, 0.03, "action = réaction");
}

/// Le solveur GPU f32 reproduit la référence CPU f64.
#[test]
fn gpu_matches_cpu() {
    let Some(mut gpu) = Planar2DGpu::headless() else {
        println!("aucun adaptateur GPU : test ignoré");
        return;
    };
    let scene = Scene::demo();
    let mut cpu = Cpu64Reference::default();
    solve(&mut cpu, &scene, 256);
    solve(&mut gpu, &scene, 256);
    let scale = cpu.field().a.iter().fold(0f32, |m, v| m.max(v.abs()));
    let diff = cpu.field().a.iter().zip(&gpu.field().a).fold(0f32, |m, (a, b)| m.max((a - b).abs()));
    println!("écart max GPU/CPU : {:.2e} (relatif {:.2e})", diff, diff / scale);
    assert!(diff / scale < 2e-3);
    // Performance avec reprise : la scène bouge d'un quart de millimètre.
    let mut moved = scene.clone();
    solve(&mut gpu, &scene, 1024);
    moved.objects[1].pos.x += 0.00025;
    solve(&mut gpu, &moved, 1024);
}

/// Force de Kelvin sur un diamagnétique près d'un fil : F = (χ/2μ0)·∫ ∇(B²) dS, répulsive.
#[test]
fn kelvin_force_on_bismuth() {
    let mut scene = Scene::default();
    scene.size = 0.2;
    wire(&mut scene, 0.0, 1000.0);
    let (d, a) = (0.03, 0.003);
    let id = scene.add("bi", Shape::Circle { r: a }, DVec2::new(d, 0.0), "Bismuth");
    let mut cpu = Cpu64Reference::default();
    solve(&mut cpu, &scene, 256);
    let w = forces(cpu.field(), &scene);
    let got = w.iter().find(|w| w.id == id).unwrap().force;
    // Référence : quadrature de ∂(B²)/∂x = −2k²x/r⁴ sur le disque, avec B = k/r.
    let k = MU0 * 1000.0 / (2.0 * PI);
    let chi = scene.material("Bismuth").unwrap().chi;
    let mut integral = 0.0;
    for i in 0..400 {
        for j in 0..400 {
            let (rho, phi) = ((i as f64 + 0.5) / 400.0 * a, (j as f64 + 0.5) / 400.0 * 2.0 * PI);
            let (x, y) = (d + rho * phi.cos(), rho * phi.sin());
            integral += -2.0 * k * k * x / (x * x + y * y).powi(2) * rho * (a / 400.0) * (2.0 * PI / 400.0);
        }
    }
    let want = chi / (2.0 * MU0) * integral;
    assert!(want > 0.0, "un diamagnétique est repoussé vers les champs faibles");
    assert_close(got.x, want, 0.02, "Fx Kelvin");
    assert!(got.y.abs() < 1e-3 * want);
}

/// Effet Meissner : sous Tc le supraconducteur expulse le champ et est repoussé par l'aimant ;
/// au-dessus de Tc il est invisible.
#[test]
fn superconductor_expels_field() {
    let mut scene = Scene::default();
    scene.add("aimant", Shape::Rect { w: 0.02, h: 0.04 }, DVec2::new(-0.03, 0.0), "NdFeB N42");
    let center = DVec3::new(0.015, 0.0, 0.0);
    let mut cpu = Cpu64Reference::default();
    solve(&mut cpu, &scene, 256);
    let free = cpu.field().sample(center).unwrap().b.length();

    let id = scene.add("supra", Shape::Circle { r: 0.01 }, center.truncate(), "YBCO");
    solve(&mut cpu, &scene, 256);
    let warm = cpu.field().sample(center).unwrap().b.length();
    assert_close(warm, free, 1e-9, "YBCO à 20 °C");

    scene.get_mut(id).unwrap().temperature = -196.0;
    solve(&mut cpu, &scene, 256);
    let cold = cpu.field().sample(center).unwrap().b.length();
    println!("|B| au centre : {free:.4} T libre, {cold:.2e} T sous Tc");
    assert!(cold < 2e-3 * free);
    let w = forces(cpu.field(), &scene);
    let f = w.iter().find(|w| w.id == id).unwrap().force;
    println!("force sur le supraconducteur : {f:?}");
    assert!(f.x > 1.0, "le supraconducteur doit être repoussé");

    if let Some(mut gpu) = Planar2DGpu::headless() {
        let status = solve(&mut gpu, &scene, 256);
        let scale = cpu.field().a.iter().fold(0f32, |m, v| m.max(v.abs()));
        let diff = cpu.field().a.iter().zip(&gpu.field().a).fold(0f32, |m, (a, b)| m.max((a - b).abs()));
        println!("GPU : résidu {:.1e}, écart relatif {:.2e}", status.residual, diff / scale);
        assert!(diff / scale < 5e-3);
        solve(&mut gpu, &scene, 1024);
    }
}
