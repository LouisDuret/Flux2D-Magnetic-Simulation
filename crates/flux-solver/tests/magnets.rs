//! Aimants réels : désaimantation irréversible (coercivité, température) et motifs d'aimantation.

use flux_core::magnet::MagPattern;
use flux_core::material::{KNEE, MagClass, Material};
use flux_core::raster::rasterize;
use flux_core::scene::Scene;
use flux_core::shape::Shape;
use flux_core::{DVec2, DVec3, MU0};
use flux_solver::{Cpu64Reference, FieldSolver, Newton, demagnetize, solve_with_demagnetization};
use std::f64::consts::{FRAC_PI_2, PI};

fn b_at(cpu: &Cpu64Reference, x: f64, y: f64) -> DVec2 {
    cpu.field().sample(DVec3::new(x, y, 0.0)).unwrap().b.truncate()
}

/// Aimant idéal : μrec = 1, rémanence de 1 T, indésaimantable.
fn ideal(scene: &mut Scene) -> &'static str {
    scene.materials.push(Material { br: 1.0, density: 7500.0, ..Material::new("idéal", MagClass::Magnet) });
    "idéal"
}

/// Un cylindre d'AlNiCo aimanté en travers ne tient pas son propre champ démagnétisant. Son
/// état final se calcule à la main : le champ y est uniforme (facteur démagnétisant 1/2), et
/// le point de fonctionnement est l'intersection de la droite de charge avec la courbe de
/// désaimantation.
#[test]
fn alnico_cylinder_demagnetizes_itself() {
    let mut scene = Scene::default();
    scene.size = 0.2;
    let id = scene.add("alnico", Shape::Circle { r: 0.02 }, DVec2::ZERO, "AlNiCo 5");
    let mat = scene.material("AlNiCo 5").unwrap().clone();
    let mut cpu = Cpu64Reference::default();
    let rounds = solve_with_demagnetization(&mut cpu, &mut scene, 256, 1e-7);

    let (br, hc, mu) = (mat.br, mat.hcj, mat.mu_r);
    let slope = (br - MU0 * (mu - 1.0) * hc) / (hc * (1.0 - KNEE));
    // Induction dans le cylindre pour une rémanence x : B = c·x.
    let c = 0.5 / (1.0 + 0.5 * (mu - 1.0));
    let want = (br + KNEE * hc * slope) / (1.0 + slope / (MU0 * mu) * (1.0 - c));
    let left = scene.get(id).unwrap().mean_remanence_left() * br;
    let b = b_at(&cpu, 0.0, 0.0).x;
    println!("{rounds} résolutions : rémanence {left:.4} T (attendu {want:.4} T), B = {b:.4} T (attendu {:.4} T)", c * want);
    assert!(rounds > 2 && rounds < 30);
    assert!((left / want - 1.0).abs() < 0.01, "rémanence restante {left} T au lieu de {want} T");
    // La désaimantation est uniforme, comme le champ.
    let o = scene.get(id).unwrap();
    let spread = [-0.012, -0.004, 0.006, 0.013].map(|x| o.remanence_left(DVec2::new(x, 0.3 * x)));
    assert!(spread.iter().all(|f| (f * br / want - 1.0).abs() < 0.03), "{spread:?}");
    // Le champ interne est revenu entre le coude et HcJ.
    let h = (b - left) / (MU0 * mu);
    assert!(h < -KNEE * hc * 0.98 && h > -hc, "H = {h} A/m");

    // L'état est stable : une résolution de plus ne retire rien, et le fichier le conserve.
    assert!(demagnetize(cpu.field(), &mut scene) < 2e-3);
    assert_eq!(Scene::from_ron(&scene.to_ron().unwrap()).unwrap(), scene);
    // Désaimantation ignorée : l'aimant retrouve son induction d'aimant idéal, B = c·Br (à
    // l'erreur de la grille près : 25 cellules dans le rayon, μrec = 3,5). Entre les deux
    // états, l'induction a baissé comme la rémanence.
    scene.demagnetization = false;
    Newton::solve(&mut cpu, rasterize(&scene, 256), 1e-7);
    let ideal = b_at(&cpu, 0.0, 0.0).x;
    println!("aimant idéal : B = {ideal:.4} T (attendu {:.4} T)", c * br);
    assert!((ideal / (c * br) - 1.0).abs() < 0.05);
    assert!((b / ideal / (left / br) - 1.0).abs() < 0.03);
}

/// À 20 °C, un NdFeB tient son champ démagnétisant ; à 120 °C, sa coercivité n'y suffit plus,
/// et la perte reste après refroidissement.
#[test]
fn overheated_neodymium_loses_flux_for_good() {
    let mut scene = Scene::overheated_demo();
    let (hot, cold) = (scene.objects[0].id, scene.objects[1].id);
    let above = |x: f64| DVec3::new(x, 0.012, 0.0);
    let mut cpu = Cpu64Reference::default();
    let rounds = solve_with_demagnetization(&mut cpu, &mut scene, 512, 1e-7);
    let left = scene.get(hot).unwrap().mean_remanence_left();
    println!("{rounds} résolutions, rémanence restante de l'aimant chaud : {:.1} %", left * 100.0);
    assert!(left > 0.3 && left < 0.8, "{left}");
    assert!(scene.get(cold).unwrap().demag.is_none(), "l'aimant à 20 °C doit rester intact");

    // Refroidi, l'aimant ne retrouve que la part réversible : Br(T), pas la rémanence perdue.
    scene.get_mut(hot).unwrap().temperature = 20.0;
    solve_with_demagnetization(&mut cpu, &mut scene, 512, 1e-7);
    let after = scene.get(hot).unwrap().mean_remanence_left();
    assert!((after - left).abs() < 0.01, "{after} au lieu de {left}");
    let (b_hot, b_cold) = (cpu.field().sample(above(-0.03)).unwrap().b.y, cpu.field().sample(above(0.03)).unwrap().b.y);
    println!("B au-dessus des aimants, de retour à 20 °C : {b_hot:.4} T contre {b_cold:.4} T");
    // Le milieu des faces, où le champ démagnétisant est le plus fort, a perdu plus que la moyenne.
    assert!(b_hot / b_cold > 0.2 && b_hot / b_cold < left);
    // Seule la part réversible est revenue : Br(20 °C)/Br(120 °C) = 1/0,88. L'aimant témoin
    // est masqué pour ne mesurer que le champ de l'aimant chauffé.
    let mut alone = scene.clone();
    alone.get_mut(cold).unwrap().visible = false;
    let mut flux = |t: f64| {
        alone.get_mut(hot).unwrap().temperature = t;
        Newton::solve(&mut cpu, rasterize(&alone, 512), 1e-7);
        cpu.field().sample(above(-0.03)).unwrap().b.y
    };
    let (at_120, at_20) = (flux(120.0), flux(20.0));
    assert!((at_20 / at_120 * 0.88 - 1.0).abs() < 0.01, "{at_20} T à 20 °C contre {at_120} T à 120 °C");

    // Ré-aimanté, il retrouve tout son flux.
    scene.remagnetize(Some(hot));
    solve_with_demagnetization(&mut cpu, &mut scene, 512, 1e-7);
    let (b_hot, b_cold) = (cpu.field().sample(above(-0.03)).unwrap().b.y, cpu.field().sample(above(0.03)).unwrap().b.y);
    assert!((b_hot / b_cold - 1.0).abs() < 0.02 && scene.get(hot).unwrap().demag.is_none());
}

/// Une ferrite poussée pôle contre pôle sur un NdFeB : le champ opposé dépasse sa coercivité
/// près de la face en regard, qui se désaimante ; le NdFeB, lui, ne perd rien.
#[test]
fn opposing_field_demagnetizes_ferrite() {
    let mut scene = Scene::default();
    scene.size = 0.2;
    let strong = scene.add("NdFeB", Shape::Rect { w: 0.02, h: 0.03 }, DVec2::new(-0.0105, 0.0), "NdFeB N42SH");
    let weak = scene.add("ferrite", Shape::Rect { w: 0.008, h: 0.03 }, DVec2::new(0.0045, 0.0), "Ferrite Sr (Y30)");
    scene.get_mut(weak).unwrap().mag_angle = PI;
    let mut cpu = Cpu64Reference::default();
    solve_with_demagnetization(&mut cpu, &mut scene, 512, 1e-7);
    let o = scene.get(weak).unwrap();
    let (near, far) = (o.remanence_left(DVec2::new(-0.0035, 0.0)), o.remanence_left(DVec2::new(0.0035, 0.0)));
    println!("ferrite : rémanence restante {near:.2} côté NdFeB, {far:.2} côté libre, {:.2} en moyenne", o.mean_remanence_left());
    assert!(near < 0.5 && near < far - 0.1, "la face en regard doit être la plus touchée : {near} contre {far}");
    assert!(scene.get(strong).unwrap().demag.is_none());
}

/// Un anneau aimanté radialement ne crée aucun champ en 2D : le flux, de symétrie de
/// révolution, n'a nulle part où aller.
#[test]
fn radial_ring_has_no_field() {
    let mut scene = Scene::default();
    scene.size = 0.2;
    let name = ideal(&mut scene);
    let id = scene.add("anneau", Shape::Ring { r_in: 0.012, r_out: 0.024 }, DVec2::ZERO, name);
    scene.get_mut(id).unwrap().pattern = MagPattern::Radial;
    let mut cpu = Cpu64Reference::default();
    Newton::solve(&mut cpu, rasterize(&scene, 512), 1e-8);
    for r in [0.0, 0.006, 0.018, 0.04, 0.07] {
        let b = (0..8)
            .map(|k| b_at(&cpu, r * (k as f64 * PI / 4.0 + 0.2).cos(), r * (k as f64 * PI / 4.0 + 0.2).sin()).length())
            .fold(0.0, f64::max);
        println!("r = {} mm : |B| ≤ {b:.2e} T", r * 1e3);
        assert!(b < 0.02, "r = {r} : {b} T pour une rémanence de 1 T");
    }
}

/// Cylindre de Halbach dipolaire (μrec = 1) : champ uniforme Br·ln(r_ext/r_int) dans l'alésage,
/// nul à l'extérieur. Retourné, il n'est plus qu'un anneau aimanté en travers.
#[test]
fn halbach_cylinder_field() {
    let mut scene = Scene::default();
    scene.size = 0.2;
    let name = ideal(&mut scene);
    let id = scene.add("halbach", Shape::Ring { r_in: 0.010, r_out: 0.025 }, DVec2::ZERO, name);
    scene.get_mut(id).unwrap().pattern = MagPattern::Halbach { pairs: 1, flip: false };
    let mut cpu = Cpu64Reference::default();
    Newton::solve(&mut cpu, rasterize(&scene, 512), 1e-8);
    let want = (0.025f64 / 0.010).ln();
    for (x, y) in [(0.0, 0.0), (0.004, 0.003), (-0.005, 0.002), (0.0, -0.006)] {
        let b = b_at(&cpu, x, y);
        println!("alésage ({x}, {y}) : B = ({:.4}, {:.4}) T, attendu ({want:.4}, 0)", b.x, b.y);
        assert!((b.x / want - 1.0).abs() < 0.02 && b.y.abs() < 0.02 * want);
    }
    let outside =
        (0..8).map(|k| b_at(&cpu, 0.04 * (k as f64 * PI / 4.0).cos(), 0.04 * (k as f64 * PI / 4.0).sin()).length()).fold(0.0, f64::max);
    println!("à l'extérieur : |B| ≤ {outside:.2e} T");
    assert!(outside < 0.02 * want);

    scene.get_mut(id).unwrap().pattern = MagPattern::Halbach { pairs: 1, flip: true };
    Newton::solve(&mut cpu, rasterize(&scene, 512), 1e-8);
    assert!(b_at(&cpu, 0.04, 0.0).length() > 0.05, "l'anneau aimanté en travers rayonne à l'extérieur");
}

/// Réseau de Halbach linéaire : le champ est concentré au-dessus du barreau, et dessous si
/// le sens de rotation est inversé. Un barreau multipolaire, lui, rayonne des deux côtés.
#[test]
fn halbach_bar_is_one_sided() {
    let mut scene = Scene::default();
    scene.size = 0.2;
    let id = scene.add("barreau", Shape::Rect { w: 0.08, h: 0.01 }, DVec2::ZERO, "NdFeB N42");
    let mut cpu = Cpu64Reference::default();
    let mut sides = |scene: &Scene| {
        Newton::solve(&mut cpu, rasterize(scene, 512), 1e-8);
        let mean = |y: f64| (-20..=20).map(|k| b_at(&cpu, k as f64 * 1e-3, y).length()).sum::<f64>() / 41.0;
        (mean(0.009), mean(-0.009))
    };
    scene.get_mut(id).unwrap().pattern = MagPattern::Halbach { pairs: 2, flip: false };
    let (above, below) = sides(&scene);
    println!("Halbach : {above:.4} T au-dessus, {below:.4} T en dessous");
    assert!(above > 5.0 * below && above > 0.2);
    scene.get_mut(id).unwrap().pattern = MagPattern::Halbach { pairs: 2, flip: true };
    let (flipped_above, flipped_below) = sides(&scene);
    assert!((flipped_below / above - 1.0).abs() < 0.02 && (flipped_above / below - 1.0).abs() < 0.1);

    let o = scene.get_mut(id).unwrap();
    (o.pattern, o.mag_angle) = (MagPattern::Multipole { pairs: 2 }, FRAC_PI_2);
    let (above, below) = sides(&scene);
    println!("multipolaire : {above:.4} T au-dessus, {below:.4} T en dessous");
    assert!((above / below - 1.0).abs() < 0.01 && above > 0.1);
    // Au-dessus des bandes, le champ sort d'un pôle nord puis rentre dans un pôle sud.
    Newton::solve(&mut cpu, rasterize(&scene, 512), 1e-8);
    assert!(b_at(&cpu, -0.03, 0.007).y > 0.1 && b_at(&cpu, -0.01, 0.007).y < -0.1);
}

/// Anneau à quatre pôles : le champ extérieur change de signe d'un pôle au suivant.
#[test]
fn multipole_ring_alternates() {
    let mut scene = Scene::default();
    scene.size = 0.2;
    let id = scene.add("anneau", Shape::Ring { r_in: 0.012, r_out: 0.02 }, DVec2::ZERO, "NdFeB N42");
    scene.get_mut(id).unwrap().pattern = MagPattern::Multipole { pairs: 2 };
    let mut cpu = Cpu64Reference::default();
    Newton::solve(&mut cpu, rasterize(&scene, 512), 1e-8);
    let radial = |a: f64| b_at(&cpu, 0.026 * a.cos(), 0.026 * a.sin()).dot(DVec2::from_angle(a));
    let (east, north, west, south) = (radial(0.0), radial(FRAC_PI_2), radial(PI), radial(-FRAC_PI_2));
    println!("Br à 26 mm : est {east:.4}, nord {north:.4}, ouest {west:.4}, sud {south:.4} T");
    assert!(east > 0.05 && (west / east - 1.0).abs() < 0.02);
    assert!((north / east + 1.0).abs() < 0.02 && (south / east + 1.0).abs() < 0.02);
}

/// Le motif peint à partir d'un motif existant donne le même champ, à la résolution de la
/// grille de directions près.
#[test]
fn painted_pattern_reproduces_its_source() {
    let mut scene = Scene::default();
    scene.size = 0.2;
    let id = scene.add("aimant", Shape::Rect { w: 0.03, h: 0.02 }, DVec2::ZERO, "NdFeB N42");
    scene.get_mut(id).unwrap().mag_angle = 0.4;
    let mut cpu = Cpu64Reference::default();
    Newton::solve(&mut cpu, rasterize(&scene, 256), 1e-8);
    let before = b_at(&cpu, 0.02, 0.015);
    scene.get_mut(id).unwrap().painted();
    Newton::solve(&mut cpu, rasterize(&scene, 256), 1e-8);
    assert!((b_at(&cpu, 0.02, 0.015) - before).length() < 1e-6 * before.length());
    // Repeindre la moitié droite vers le haut change le champ de ce côté.
    let o = scene.get_mut(id).unwrap();
    let lattice = o.paint.as_mut().unwrap();
    for k in 0..lattice.values.len() {
        if lattice.center(k).x > 0.0 {
            lattice.values[k] = FRAC_PI_2 as f32;
        }
    }
    Newton::solve(&mut cpu, rasterize(&scene, 256), 1e-8);
    assert!((b_at(&cpu, 0.02, 0.015) - before).length() > 0.05);
}
