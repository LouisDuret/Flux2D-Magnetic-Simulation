//! Lois de température (section 2.5) dans le calcul du champ et des forces : Kuz'min,
//! Curie–Weiss, et la force de Kelvin d'un diamagnétique anisotrope.

use flux_core::material::NuTable;
use flux_core::raster::rasterize;
use flux_core::scene::Scene;
use flux_core::shape::Shape;
use flux_core::{DVec2, MU0};
use flux_solver::{Cpu64Reference, Field, FieldSolver, Newton, forces};
use std::f64::consts::{FRAC_PI_4, PI};

/// Induction B telle que H(B) = `h`, par dichotomie sur la table.
fn b_of_h(table: &NuTable, h: f64) -> f64 {
    let (mut lo, mut hi) = (0.0, 10.0);
    for _ in 0..80 {
        let mid = 0.5 * (lo + hi);
        if table.h(mid) < h { lo = mid } else { hi = mid }
    }
    lo
}

/// Anneau d'acier autour d'un fil, où H = I/(2πr) est imposé : l'induction suit la courbe
/// B(H) de la température de l'anneau, dont la saturation baisse selon Kuz'min. Au-dessus de
/// Tc, l'acier n'est plus que paramagnétique (Curie–Weiss).
#[test]
fn hot_iron_saturates_lower_then_turns_paramagnetic() {
    let (name, amps, r) = ("Acier doux (S235)", 2000.0, 0.016);
    let h = amps / (2.0 * PI * r);
    let mut found = Vec::new();
    for t in [20.0, 600.0, 760.0, 800.0] {
        let mut scene = Scene::default();
        scene.size = 0.16;
        let wire = scene.add("fil", Shape::Circle { r: 0.003 }, DVec2::ZERO, "Cuivre (bobinage)");
        let o = scene.get_mut(wire).unwrap();
        (o.turns, o.current) = (1.0, amps);
        let ring = scene.add("anneau", Shape::Ring { r_in: 0.010, r_out: 0.022 }, DVec2::ZERO, name);
        scene.get_mut(ring).unwrap().temperature = t;
        let mat = scene.material(name).unwrap().clone();
        let mut cpu = Cpu64Reference::default();
        let (newton, _) = Newton::solve(&mut cpu, rasterize(&scene, 512), 1e-7);
        let mean =
            (0..32).map(|k| cpu.field().sample((DVec2::from_angle(k as f64 * PI / 16.0) * r).extend(0.0)).unwrap().b.length()).sum::<f64>()
                / 32.0;
        let want = match mat.curve_at(t) {
            Some(curve) => b_of_h(&curve.table(), h),
            // χ = C/(T − Tc) = 2,1/30.
            None => MU0 * (1.0 + mat.chi_at(t)) * h,
        };
        println!("{t} °C : B = {mean:.4} T (attendu {want:.4} T), {} itérations de Newton", newton.iterations);
        assert!((mean / want - 1.0).abs() < 0.01, "{t} °C : {mean} T au lieu de {want} T");
        found.push(mean);
    }
    // 2 T à froid, 1,5 T à 600 °C, quelques dixièmes de tesla à 10 °C de Tc, puis presque le vide.
    assert!(found[0] > 1.9 && found[1] < 0.8 * found[0] && found[2] < 0.5 * found[0]);
    assert!((found[3] / (MU0 * h) - 1.07).abs() < 0.01);
}

/// Force sur une bille de nickel près d'un aimant : forte sous Tc, elle suit χ = C/(T − Tc)
/// au-dessus. À 10 K de Tc le nickel entre encore dans le calcul du champ (tenseur de
/// Maxwell) ; à 100 K, il n'est plus traité qu'en perturbation (force de Kelvin). Les deux
/// méthodes doivent se raccorder.
#[test]
fn nickel_lets_go_above_curie_temperature() {
    let mut scene = Scene::default();
    scene.size = 0.2;
    scene.add("aimant", Shape::Rect { w: 0.02, h: 0.03 }, DVec2::new(-0.02, 0.0), "NdFeB N42");
    let ball = scene.add("bille", Shape::Circle { r: 0.004 }, DVec2::new(0.006, 0.0), "Nickel");
    let mut pull = |t: f64| {
        scene.get_mut(ball).unwrap().temperature = t;
        let mut cpu = Cpu64Reference::default();
        Newton::solve(&mut cpu, rasterize(&scene, 512), 1e-9);
        let f = forces(cpu.field(), &scene).iter().find(|w| w.id == ball).unwrap().force;
        assert!(f.x < 0.0 && f.y.abs() < 2e-3 * f.x.abs(), "{t} °C : {f:?}");
        -f.x
    };
    let (cold, warm, near, far) = (pull(20.0), pull(340.0), pull(364.0), pull(454.0));
    println!("force sur la bille : {cold:.2} N/m à 20 °C, {warm:.2} à 340 °C, {near:.4} à 364 °C, {far:.5} à 454 °C");
    assert!(warm < cold && near < 0.05 * cold);
    // Un cylindre de susceptibilité χ s'aimante comme χ/(1 + χ/2) : rapport attendu entre
    // χ = 0,061 (Maxwell) et χ = 0,0061 (Kelvin, premier ordre).
    let want = (0.061 / (1.0 + 0.061 / 2.0)) / 0.0061;
    assert!((near / far / want - 1.0).abs() < 0.05, "rapport {} au lieu de {want}", near / far);
}

/// Intégrale sur un rectangle (centre `c`, demi-côtés `half`, tourné de `angle`) par la
/// méthode du point milieu.
fn integrate(c: DVec2, half: DVec2, angle: f64, f: impl Fn(DVec2) -> f64) -> f64 {
    let (nx, ny) = (160, 60);
    let mut sum = 0.0;
    for j in 0..ny {
        for i in 0..nx {
            let local =
                DVec2::new(((i as f64 + 0.5) / nx as f64 * 2.0 - 1.0) * half.x, ((j as f64 + 0.5) / ny as f64 * 2.0 - 1.0) * half.y);
            sum += f(c + DVec2::from_angle(angle).rotate(local));
        }
    }
    sum * 4.0 * half.x * half.y / (nx * ny) as f64
}

fn b(field: &Field, p: DVec2) -> DVec2 {
    field.sample(p.extend(0.0)).unwrap().b.truncate()
}

/// Lévitation du graphite pyrolytique (section 3.9) : la plaque flotte quand B·∂B/∂z atteint
/// μ0·ρ·g/|χ⊥| ≈ 60 T²/m. La force calculée sur la plaque est comparée à l'intégrale de la
/// densité de force de Kelvin, évaluée à part sur le champ.
#[test]
fn graphite_levitation_threshold() {
    let mut scene = Scene::default();
    scene.size = 0.1;
    // Damier : deux aimants côte à côte, aimantés vers le haut et vers le bas. Le champ varie
    // vite au-dessus de leur jonction, bien plus qu'au-dessus d'un aimant seul.
    for (x, angle) in [(-0.004, PI / 2.0), (0.004, -PI / 2.0)] {
        let magnet = scene.add("aimant", Shape::Rect { w: 0.008, h: 0.008 }, DVec2::new(x, -0.004), "NdFeB N52");
        scene.get_mut(magnet).unwrap().mag_angle = angle;
    }
    let (center, half) = (DVec2::new(0.0, 0.0010), DVec2::new(0.003, 0.0004));
    let plate = scene.add("graphite", Shape::Rect { w: 2.0 * half.x, h: 2.0 * half.y }, center, "Graphite pyrolytique");
    let mat = scene.material("Graphite pyrolytique").unwrap().clone();
    let [perp, par] = mat.weak_chi(20.0).unwrap();
    let mut cpu = Cpu64Reference::default();
    Newton::solve(&mut cpu, rasterize(&scene, 512), 1e-9);
    let field = cpu.field();
    let lift = forces(field, &scene).iter().find(|w| w.id == plate).unwrap().force.y;

    // Densité d'énergie u = (χ⊥·By² + χ∥·Bx²)/(2μ0), les feuillets étant horizontaux.
    let e = field.h();
    let u = |p: DVec2| (perp * b(field, p).y.powi(2) + par * b(field, p).x.powi(2)) / (2.0 * MU0);
    let want = integrate(center, half, 0.0, |p| (u(p + DVec2::Y * e) - u(p - DVec2::Y * e)) / (2.0 * e));
    println!("portance du graphite : {lift:.4} N/m (intégrale de Kelvin : {want:.4} N/m)");
    assert!(lift > 0.0 && (lift / want - 1.0).abs() < 0.05);

    // Seuil : la portance égale le poids quand la moyenne de B·∂B/∂z vaut μ0·ρ·g/|χ⊥|.
    let weight = mat.density * 9.81 * 4.0 * half.x * half.y;
    let product = integrate(center, half, 0.0, |p| {
        let (up, down) = (b(field, p + DVec2::Y * e), b(field, p - DVec2::Y * e));
        // La part de Bx, de susceptibilité χ∥, est ramenée à l'échelle de χ⊥.
        (up.y.powi(2) - down.y.powi(2) + par / perp * (up.x.powi(2) - down.x.powi(2))) / (4.0 * e)
    }) / (4.0 * half.x * half.y);
    let threshold = product.abs() * weight / lift;
    let exact = MU0 * mat.density * 9.81 / perp.abs();
    println!(
        "B·dB/dz moyen : {:.1} T²/m ; portance / poids = {:.2} ; seuil {threshold:.1} T²/m (exact {exact:.1})",
        product.abs(),
        lift / weight
    );
    assert!((exact - 60.3).abs() < 0.2 && (threshold / exact - 1.0).abs() < 0.05);
    assert!(lift > weight, "à 0,6 mm du damier, la plaque de graphite est soulevée");
    // Deux millimètres plus haut, le champ ne la porte plus : elle flotte entre les deux.
    scene.get_mut(plate).unwrap().pos.y += 0.002;
    let high = forces(field, &scene).iter().find(|w| w.id == plate).unwrap().force.y;
    assert!(high > 0.0 && high < weight);
    // Le bismuth, moins diamagnétique et plus dense, ne flotte pas au même endroit.
    scene.get_mut(plate).unwrap().material = "Bismuth".into();
    let lift = forces(field, &scene).iter().find(|w| w.id == plate).unwrap().force.y;
    assert!(lift > 0.0 && lift < 0.2 * scene.material("Bismuth").unwrap().density * 9.81 * 4.0 * half.x * half.y);
}

/// Une plaque de graphite inclinée dans un champ subit un couple qui couche ses feuillets le
/// long du champ : M × B, nul pour un matériau isotrope.
#[test]
fn anisotropic_graphite_feels_a_torque() {
    let mut scene = Scene::default();
    scene.size = 0.1;
    scene.add("aimant", Shape::Rect { w: 0.02, h: 0.02 }, DVec2::new(-0.018, 0.0), "NdFeB N52");
    let (center, half) = (DVec2::new(0.004, 0.0), DVec2::new(0.003, 0.0006));
    let plate = scene.add("graphite", Shape::Rect { w: 2.0 * half.x, h: 2.0 * half.y }, center, "Graphite pyrolytique");
    scene.get_mut(plate).unwrap().angle = FRAC_PI_4;
    let [perp, par] = scene.material("Graphite pyrolytique").unwrap().weak_chi(20.0).unwrap();
    let mut cpu = Cpu64Reference::default();
    Newton::solve(&mut cpu, rasterize(&scene, 512), 1e-9);
    let field = cpu.field();
    let torque = forces(field, &scene).iter().find(|w| w.id == plate).unwrap().torque;

    // Couple de référence : M × B, plus le moment de la densité de force autour du centre.
    let axis = DVec2::from_angle(FRAC_PI_4 + PI / 2.0);
    let e = field.h();
    let u = |p: DVec2| (perp * b(field, p).dot(axis).powi(2) + par * b(field, p).perp_dot(axis).powi(2)) / (2.0 * MU0);
    let want = integrate(center, half, FRAC_PI_4, |p| {
        let at = b(field, p);
        let grad = DVec2::new(u(p + DVec2::X * e) - u(p - DVec2::X * e), u(p + DVec2::Y * e) - u(p - DVec2::Y * e)) / (2.0 * e);
        (par - perp) * at.perp_dot(axis) * at.dot(axis) / MU0 + (p - center).perp_dot(grad)
    });
    println!("couple sur la plaque inclinée : {torque:.3e} N·m/m (référence {want:.3e})");
    assert!((torque / want - 1.0).abs() < 0.05);
    // Le champ, horizontal, ramène les feuillets (à 45°) vers l'horizontale : couple négatif.
    assert!(torque < 0.0);
    // Isotrope, la même plaque ne subit plus que le faible moment des forces de gradient.
    scene.get_mut(plate).unwrap().material = "Bismuth".into();
    let isotropic = forces(field, &scene).iter().find(|w| w.id == plate).unwrap().torque;
    assert!(isotropic.abs() < 0.2 * torque.abs(), "{isotropic}");
}

/// Un paramagnétique est attiré vers les champs forts, deux fois moins à 313 °C qu'à 20 °C
/// (loi de Curie) ; l'oxygène liquide, à −183 °C, l'est bien plus.
#[test]
fn paramagnetic_force_follows_curie_law() {
    let mut scene = Scene::default();
    scene.size = 0.1;
    scene.add("aimant", Shape::Rect { w: 0.02, h: 0.02 }, DVec2::new(-0.015, 0.0), "NdFeB N42");
    let drop = scene.add("goutte", Shape::Circle { r: 0.002 }, DVec2::new(0.0, 0.0), "Platine");
    let mut cpu = Cpu64Reference::default();
    Newton::solve(&mut cpu, rasterize(&scene, 256), 1e-9);
    let mut pull = |material: &str, t: f64| {
        let o = scene.get_mut(drop).unwrap();
        (o.material, o.temperature) = (material.into(), t);
        -forces(cpu.field(), &scene).iter().find(|w| w.id == drop).unwrap().force.x
    };
    let (cold, hot) = (pull("Platine", 20.0), pull("Platine", 313.15));
    assert!(cold > 0.0 && (cold / hot - 2.0).abs() < 1e-9, "{cold} et {hot}");
    let oxygen = pull("Oxygène liquide", -183.0);
    assert!((oxygen / cold - 3.5e-3 / 2.7e-4).abs() < 1e-6);
}
