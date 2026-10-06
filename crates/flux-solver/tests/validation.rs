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
    scene.materials.push(Material { br: 1.0, density: 7500.0, ..Material::new("idéal", MagClass::Magnet) });
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

/// Une ligne de champ tracée autour d'un fil est un cercle qui se referme.
#[test]
fn traced_line_around_wire_is_a_circle() {
    let mut scene = Scene::default();
    scene.size = 0.2;
    wire(&mut scene, 0.0, 10.0);
    let mut cpu = Cpu64Reference::default();
    solve(&mut cpu, &scene, 256);
    let seed = DVec3::new(0.02, 0.0, 0.0);
    let line = cpu.field().trace(seed);
    assert_eq!(line.last(), Some(&seed), "la ligne doit se refermer");
    let length: f64 = line.windows(2).map(|w| w[0].distance(w[1])).sum();
    assert_close(length, 2.0 * PI * 0.02, 0.005, "longueur de la ligne");
    for p in &line {
        assert!((p.length() - 0.02).abs() < 0.005 * 0.02, "rayon {}", p.length());
    }
}

/// Matériau linéaire de perméabilité `mu_r`, sans courbe B(H).
fn linear(scene: &mut Scene, mu_r: f64) -> &'static str {
    scene.materials.retain(|m| m.name != "linéaire");
    scene.materials.push(Material { mu_r, density: 7800.0, ..Material::new("linéaire", MagClass::Ferro) });
    "linéaire"
}

/// Aimant idéal : μrec = 1, rémanence de 1 T.
fn ideal(scene: &mut Scene) -> &'static str {
    scene.materials.push(Material { br: 1.0, density: 7500.0, ..Material::new("idéal", MagClass::Magnet) });
    "idéal"
}

/// Champ appliqué au centre d'une scène de 200 mm : deux nappes de courant opposées.
fn applied_field() -> Scene {
    let mut scene = Scene::default();
    scene.size = 0.2;
    for (y, amps) in [(0.04, 1500.0), (-0.04, -1500.0)] {
        let id = scene.add("nappe", Shape::Rect { w: 0.15, h: 0.002 }, DVec2::new(0.0, y), "Cuivre (bobinage)");
        let o = scene.get_mut(id).unwrap();
        (o.turns, o.current) = (1.0, amps);
    }
    scene
}

/// Cylindre de perméabilité μr dans un champ appliqué B0 : B intérieur = 2·μr·B0 / (μr + 1).
/// Pour un cylindre, la relation vaut en chaque point intérieur, que B0 y soit uniforme ou non.
#[test]
fn permeable_cylinder_in_applied_field() {
    let mut scene = applied_field();
    let mut cpu = Cpu64Reference::default();
    solve(&mut cpu, &scene, 1024);
    let points = [DVec3::ZERO, DVec3::new(0.008, 0.006, 0.0), DVec3::new(-0.005, -0.011, 0.0)];
    let applied = points.map(|p| cpu.field().sample(p).unwrap().b);
    // Entre deux nappes de largeur w écartées de 2L : B0 = μ0·K·(2/π)·atan(w/2L), ici 8,6 mT.
    assert_close(applied[0].x, MU0 * 1e4 * 2.0 / PI * (0.075f64 / 0.04).atan(), 0.02, "champ appliqué au centre");
    let id = scene.add("cylindre", Shape::Circle { r: 0.02 }, DVec2::ZERO, "Cuivre (bobinage)");
    for mu_r in [2.0, 10.0, 1000.0] {
        scene.get_mut(id).unwrap().material = linear(&mut scene, mu_r).into();
        solve(&mut cpu, &scene, 1024);
        for (p, b0) in points.iter().zip(applied) {
            let b = cpu.field().sample(*p).unwrap().b;
            assert_close(b.x, 2.0 * mu_r / (mu_r + 1.0) * b0.x, 0.01, &format!("B intérieur, μr = {mu_r}"));
            assert!((b.y - 2.0 * mu_r / (mu_r + 1.0) * b0.y).abs() < 0.01 * b.x.abs());
        }
    }
}

/// Blindage par un tube de rayons a < b : B intérieur / B0 = 4·μr·b² / [(μr+1)²·b² − (μr−1)²·a²],
/// lu au centre, où seule compte la part uniforme du champ appliqué.
#[test]
fn shielding_tube() {
    let mut scene = applied_field();
    let mut cpu = Cpu64Reference::default();
    solve(&mut cpu, &scene, 2048);
    let b0 = cpu.field().sample(DVec3::ZERO).unwrap().b.x;
    let (a, b) = (0.012, 0.020);
    let id = scene.add("tube", Shape::Ring { r_in: a, r_out: b }, DVec2::ZERO, "Cuivre (bobinage)");
    for mu_r in [10.0, 100.0] {
        scene.get_mut(id).unwrap().material = linear(&mut scene, mu_r).into();
        solve(&mut cpu, &scene, 2048);
        let want = 4.0 * mu_r * b * b / ((mu_r + 1.0).powi(2) * b * b - (mu_r - 1.0).powi(2) * a * a);
        let inside = cpu.field().sample(DVec3::ZERO).unwrap().b.x;
        assert_close(inside / b0, want, 0.02, &format!("facteur de blindage, μr = {mu_r}"));
    }
}

/// Convergence en maillage (section 3.9) : en divisant le pas par deux, l'écart entre deux
/// grilles successives est divisé par quatre loin des interfaces (ordre 2) et par deux là où
/// une interface en escalier domine (ordre 1). Comparer les grilles entre elles écarte l'erreur
/// de la condition au bord, qui ne dépend pas du pas.
#[test]
fn mesh_convergence_orders() {
    let grids = [64, 128, 256, 512];
    // Écart moyen de B entre grilles successives, aux points donnés.
    let steps = |scene: &Scene, points: &[DVec3]| -> Vec<f64> {
        let fields: Vec<Vec<DVec3>> = grids
            .iter()
            .map(|&n| {
                let mut cpu = Cpu64Reference::default();
                solve(&mut cpu, scene, n);
                points.iter().map(|p| cpu.field().sample(*p).unwrap().b).collect()
            })
            .collect();
        fields.windows(2).map(|f| f[0].iter().zip(&f[1]).map(|(a, b)| (*a - *b).length()).sum::<f64>() / points.len() as f64).collect()
    };
    let orders = |d: &[f64]| -> Vec<f64> { d.windows(2).map(|d| (d[0] / d[1]).log2()).collect() };
    let sci = |d: &[f64]| d.iter().map(|e| format!("{e:.2e}")).collect::<Vec<_>>().join(", ");

    // Deux fils opposés, champ lu dans l'air à distance des conducteurs.
    let mut wires = Scene::default();
    wires.size = 0.2;
    wire(&mut wires, -0.02, 100.0);
    wire(&mut wires, 0.02, -100.0);
    let air: Vec<DVec3> = (0..24).map(|k| (DVec2::from_angle(k as f64 * 0.7) * (0.03 + 0.002 * k as f64)).extend(0.0)).collect();
    let smooth = steps(&wires, &air);
    println!("deux fils : écarts {} T, ordres {:.2?}", sci(&smooth), orders(&smooth));

    // Cylindre de fer dans le champ d'un fil : champ lu près de sa surface.
    let mut iron = wires.clone();
    let name = linear(&mut iron, 100.0);
    iron.add("cylindre", Shape::Circle { r: 0.0113 }, DVec2::new(0.0003, 0.0352), name);
    let skin: Vec<DVec3> = (0..24)
        .map(|k| (DVec2::new(0.0003, 0.0352) + DVec2::from_angle(k as f64 * 0.7) * if k % 2 == 0 { 0.0095 } else { 0.0131 }).extend(0.0))
        .collect();
    let interface = steps(&iron, &skin);
    println!("près du fer : écarts {} T, ordres {:.2?}", sci(&interface), orders(&interface));

    let rate = |d: &[f64]| (d[0] / d[d.len() - 1]).log2() / (d.len() - 1) as f64;
    assert!(rate(&smooth) > 1.6 && rate(&smooth) < 2.5, "ordre loin des interfaces : {}", rate(&smooth));
    assert!(rate(&interface) > 0.6 && rate(&interface) < 1.6, "ordre aux interfaces : {}", rate(&interface));
}

/// Force entre deux aimants idéaux (μrec = 1), comparée à la solution exacte du modèle des
/// charges magnétiques : chaque face polaire porte ±Br/μ0, et la force sur une face est
/// l'intégrale du champ de l'autre aimant.
#[test]
fn force_between_two_magnets_is_exact() {
    // Aimants de 20 × 20 mm aimantés vers +x, centrés en ±c.
    let (half, c) = (0.01, 0.022);
    // Attraction exacte par mètre : Br²/(2π·μ0) · Σ ± ∬ dx/(dx² + (y − y')²) dy dy'.
    let pair = |dx: f64| -> f64 {
        // ∫ dy' dx/(dx² + (y − y')²) = atan((y + half)/dx) − atan((y − half)/dx), puis Simpson en y.
        let f = |y: f64| ((y + half) / dx).atan() - ((y - half) / dx).atan();
        let n = 2000;
        let step = 2.0 * half / n as f64;
        (0..=n)
            .map(|k| {
                f(-half + k as f64 * step)
                    * if k == 0 || k == n {
                        1.0
                    } else if k % 2 == 1 {
                        4.0
                    } else {
                        2.0
                    }
            })
            .sum::<f64>()
            * step
            / 3.0
    };
    // Faces de l'aimant de gauche : −half (charge −) et +half (+) autour de −c ; de droite, autour de +c.
    let mut exact = 0.0;
    for (xa, qa) in [(-c - half, -1.0), (-c + half, 1.0)] {
        for (xb, qb) in [(c - half, -1.0), (c + half, 1.0)] {
            exact += qa * qb * pair(xb - xa);
        }
    }
    let exact = exact / (2.0 * PI * MU0);
    assert!(exact < 0.0, "les aimants alignés s'attirent : la force sur celui de droite est vers −x");

    // Domaine de 400 mm : le bord est assez loin pour ne pas peser sur la force.
    let mut scene = Scene::default();
    let name = ideal(&mut scene);
    let left = scene.add("gauche", Shape::Rect { w: 2.0 * half, h: 2.0 * half }, DVec2::new(-c, 0.0), name);
    let right = scene.add("droite", Shape::Rect { w: 2.0 * half, h: 2.0 * half }, DVec2::new(c, 0.0), name);
    let mut cpu = Cpu64Reference::default();
    solve(&mut cpu, &scene, 1024);
    let w = forces(cpu.field(), &scene);
    let (fl, fr) = (w.iter().find(|w| w.id == left).unwrap(), w.iter().find(|w| w.id == right).unwrap());
    assert_close(fr.force.x, exact, 0.01, "Fx aimant de droite");
    assert_close(fl.force.x, -exact, 0.01, "Fx aimant de gauche");
    assert!(fr.force.y.abs() < 1e-3 * exact.abs() && fr.torque.abs() < 1e-4 * exact.abs() * half);

    // Retourné, l'aimant de droite est repoussé avec la même force.
    scene.get_mut(right).unwrap().mag_angle = PI;
    solve(&mut cpu, &scene, 1024);
    let fr = forces(cpu.field(), &scene).into_iter().find(|w| w.id == right).unwrap();
    assert_close(fr.force.x, -exact, 0.01, "Fx en répulsion");
}

/// Fil parallèle à un cylindre perméable : solution exacte par la méthode des images. Le
/// cylindre (rayon a, μr) répond comme un courant image k·I en a²/d et −k·I au centre, avec
/// k = (μr − 1)/(μr + 1) ; à l'intérieur, le champ est celui d'un fil 2·μr/(μr + 1) fois plus fort.
#[test]
fn wire_attracted_by_a_permeable_cylinder() {
    let (a, d, amps) = (0.015, 0.03, 500.0);
    for mu_r in [5.0, 200.0] {
        let mut scene = Scene::default();
        scene.size = 0.2;
        let w = wire(&mut scene, d, amps);
        let p = DVec2::new(-0.004, 0.006);
        let mut cpu = Cpu64Reference::default();
        solve(&mut cpu, &scene, 1024);
        let alone = cpu.field().sample(p.extend(0.0)).unwrap().b.truncate();
        let name = linear(&mut scene, mu_r);
        let cylinder = scene.add("cylindre", Shape::Circle { r: a }, DVec2::ZERO, name);
        solve(&mut cpu, &scene, 1024);
        let k = (mu_r - 1.0) / (mu_r + 1.0);
        let want = MU0 * amps * amps * k / (2.0 * PI) * (1.0 / (d - a * a / d) - 1.0 / d);
        let found = forces(cpu.field(), &scene);
        let (on_wire, on_cylinder) = (found.iter().find(|f| f.id == w).unwrap(), found.iter().find(|f| f.id == cylinder).unwrap());
        assert_close(on_wire.force.x, -want, 0.02, &format!("Fx sur le fil, μr = {mu_r}"));
        assert_close(on_cylinder.force.x, want, 0.02, &format!("Fx sur le cylindre, μr = {mu_r}"));
        // Dans le cylindre, le champ du fil seul est multiplié par 2·μr/(μr + 1).
        let inside = alone * (2.0 * mu_r / (mu_r + 1.0));
        let b = cpu.field().sample(p.extend(0.0)).unwrap().b.truncate();
        assert!((b - inside).length() < 0.01 * inside.length(), "B dans le cylindre : {b:?} au lieu de {inside:?}");
    }
}
