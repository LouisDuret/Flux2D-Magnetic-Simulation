//! Validation des matériaux saturables : courbe B(H), itération de Newton, accord GPU/CPU.

use flux_core::material::NuTable;
use flux_core::raster::rasterize;
use flux_core::scene::Scene;
use flux_core::shape::Shape;
use flux_core::{DVec2, DVec3, MU0};
use flux_solver::{Cpu64Reference, FieldSolver, Newton, Planar2DGpu};
use std::f64::consts::PI;

/// Induction B telle que H(B) = `h`, par dichotomie sur la table.
fn b_of_h(table: &NuTable, h: f64) -> f64 {
    let (mut lo, mut hi) = (0.0, 10.0);
    for _ in 0..80 {
        let mid = 0.5 * (lo + hi);
        if table.h(mid) < h { lo = mid } else { hi = mid }
    }
    lo
}

/// Anneau de fer autour d'un fil : par symétrie H = I/(2πr), donc B suit exactement la courbe
/// B(H) du matériau, du régime linéaire à la saturation profonde. C'est le cas le plus dur pour
/// une itération de point fixe (H imposé) ; Newton y converge en quelques pas.
#[test]
fn iron_ring_follows_bh_curve() {
    let name = "Acier doux (S235)";
    let table = Scene::default().material(name).unwrap().bh.as_ref().unwrap().table();
    for amps in [20.0, 200.0, 2000.0, 20000.0] {
        let mut scene = Scene::default();
        scene.size = 0.16;
        let wire = scene.add("fil", Shape::Circle { r: 0.003 }, DVec2::ZERO, "Cuivre (bobinage)");
        let o = scene.get_mut(wire).unwrap();
        (o.turns, o.current) = (1.0, amps);
        scene.add("anneau", Shape::Ring { r_in: 0.010, r_out: 0.022 }, DVec2::ZERO, name);
        let mut cpu = Cpu64Reference::default();
        let (newton, status) = Newton::solve(&mut cpu, rasterize(&scene, 512), 1e-7);
        println!(
            "I = {amps} A : {} itérations de Newton, variation {:.1e}, résidu {:.1e}",
            newton.iterations, newton.change, status.residual
        );
        assert!(newton.iterations < 25, "Newton n'a pas convergé : {} itérations", newton.iterations);
        for r in [0.013, 0.016, 0.019] {
            let want = b_of_h(&table, amps / (2.0 * PI * r));
            // La grille module légèrement B autour de l'anneau (bords en escalier) : la moyenne
            // sur un tour, fixée par le théorème d'Ampère, est comparée plus strictement.
            let around: Vec<f64> = (0..32)
                .map(|k| cpu.field().sample((DVec2::from_angle(k as f64 * PI / 16.0) * r).extend(0.0)).unwrap().b.length())
                .collect();
            let mean = around.iter().sum::<f64>() / 32.0;
            let worst = around.iter().map(|b| (b / want - 1.0).abs()).fold(0.0, f64::max);
            println!(
                "  r = {} mm : B moyen {mean:.4} T (attendu {want:.4} T, écart {:.2} %, pire point {:.2} %)",
                r * 1e3,
                (mean / want - 1.0).abs() * 100.0,
                worst * 100.0
            );
            assert!((mean / want - 1.0).abs() < 0.01 && worst < 0.03, "I = {amps} A, r = {r} : {mean} T au lieu de {want} T");
        }
        // Hors du fer, le champ reste celui du fil seul.
        let b = cpu.field().sample(DVec3::new(0.03, 0.03, 0.0)).unwrap().b.length();
        let want = MU0 * amps / (2.0 * PI * 0.03 * 2f64.sqrt());
        println!("  dans l'air : {b:.5} T (attendu {want:.5} T)");
        assert!((b / want - 1.0).abs() < 0.02);
    }
}

/// Une tôle mince devant un aimant puissant sature : son induction plafonne près de Js et le
/// champ fuit derrière elle, là où un fer linéaire ferait écran.
#[test]
fn thin_sheet_saturates_and_leaks() {
    let scene = Scene::saturation_demo();
    let mut cpu = Cpu64Reference::default();
    let (newton, _) = Newton::solve(&mut cpu, rasterize(&scene, 512), 1e-7);
    println!("{} itérations de Newton", newton.iterations);
    assert!(newton.iterations < 30);
    let saturated = cpu.field().clone();

    // Référence linéaire : la même scène sans courbe B(H).
    let mut linear_scene = scene.clone();
    linear_scene.materials.iter_mut().for_each(|m| m.bh = None);
    let mut linear = Cpu64Reference::default();
    Newton::solve(&mut linear, rasterize(&linear_scene, 512), 1e-7);

    let inside = |f: &flux_solver::Field| {
        (-30..=30).map(|k| f.sample(DVec3::new(k as f64 * 1e-3, 0.0, 0.0)).unwrap().b.length()).fold(0.0, f64::max)
    };
    let (b_sat, b_lin) = (inside(&saturated), inside(linear.field()));
    println!("B max dans la tôle : {b_sat:.2} T saturée, {b_lin:.2} T linéaire");
    assert!(b_lin > 3.0, "le cas linéaire devrait dépasser Js : {b_lin}");
    assert!(b_sat < 2.4 && b_sat > 1.6, "{b_sat}");

    let behind = DVec3::new(0.0, 0.006, 0.0);
    let (leak_sat, leak_lin) = (saturated.sample(behind).unwrap().b.length(), linear.field().sample(behind).unwrap().b.length());
    println!("Champ derrière la tôle : {leak_sat:.4} T saturée, {leak_lin:.4} T linéaire");
    assert!(leak_sat > 3.0 * leak_lin, "la tôle saturée devrait laisser fuir le champ");
}

/// À champ faible, la courbe B(H) redonne le matériau linéaire : mêmes résultats.
#[test]
fn weak_field_matches_linear_material() {
    let scene = Scene::demo();
    let mut cpu = Cpu64Reference::default();
    let (newton, _) = Newton::solve(&mut cpu, rasterize(&scene, 256), 1e-7);
    let mut linear_scene = scene.clone();
    linear_scene.materials.iter_mut().for_each(|m| m.bh = None);
    let mut linear = Cpu64Reference::default();
    Newton::solve(&mut linear, rasterize(&linear_scene, 256), 1e-7);
    let (f, f_lin) = (flux_solver::forces(cpu.field(), &scene)[1].force.x, flux_solver::forces(linear.field(), &linear_scene)[1].force.x);
    println!("{} itérations ; force {f:.3} N/m, linéaire {f_lin:.3} N/m", newton.iterations);
    assert!((f / f_lin - 1.0).abs() < 0.01);
}

/// Le solveur GPU (f32) et la référence CPU (f64) donnent le même champ saturé.
#[test]
fn gpu_matches_cpu_when_saturated() {
    let Some(mut gpu) = Planar2DGpu::headless() else {
        println!("pas de GPU : test ignoré");
        return;
    };
    let scene = Scene::saturation_demo();
    let mut cpu = Cpu64Reference::default();
    Newton::solve(&mut cpu, rasterize(&scene, 512), 1e-6);
    let (newton, status) = Newton::solve(&mut gpu, rasterize(&scene, 512), 1e-4);
    println!("GPU : {} itérations de Newton, variation {:.1e}, résidu {:.1e}", newton.iterations, newton.change, status.residual);
    assert!(newton.iterations < 40);
    let (a, b) = (cpu.field(), gpu.field());
    let scale = a.b_max();
    let n = a.n;
    let worst = (0..n * n).map(|c| (a.b_cell(c % n, c / n) - b.b_cell(c % n, c / n)).length()).fold(0.0, f64::max);
    println!("écart max sur B : {:.2e} T pour un champ de {scale:.2} T", worst);
    assert!(worst < 0.02 * scale, "{worst}");

    // Revenir à une scène linéaire efface les termes tangents du GPU.
    let mut plain = Scene::default();
    plain.add("aimant", Shape::Rect { w: 0.02, h: 0.01 }, DVec2::ZERO, "NdFeB N42");
    let mut reference = Cpu64Reference::default();
    Newton::solve(&mut reference, rasterize(&plain, 512), 1e-6);
    Newton::solve(&mut gpu, rasterize(&plain, 512), 1e-4);
    let worst =
        (0..n * n).map(|c| (reference.field().b_cell(c % n, c / n) - gpu.field().b_cell(c % n, c / n)).length()).fold(0.0, f64::max);
    assert!(worst < 0.01 * reference.field().b_max(), "{worst}");
}
