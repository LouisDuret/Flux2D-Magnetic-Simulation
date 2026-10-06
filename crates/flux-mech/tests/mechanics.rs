//! Validation de la mécanique : seuil de glissement, frottement dynamique, chute, rampe,
//! pendule, ressort et collisions.

use flux_core::DVec2;
use flux_core::scene::{Body, Link, MechView, Scene};
use flux_core::shape::Shape;
use flux_mech::{DT, Load, World};
use std::f64::consts::PI;

const G: f64 = 9.81;

fn block(scene: &mut Scene, pos: DVec2, mu_s: f64, mu_k: f64) -> u32 {
    let id = scene.add("bloc", Shape::Rect { w: 0.02, h: 0.01 }, pos, "Fer pur (Armco)");
    scene.get_mut(id).unwrap().body = Body { mobile: true, mu_s, mu_k, link: Link::Free };
    id
}

/// Avance de `seconds` sous une force constante appliquée à l'objet `id`.
fn run(world: &mut World, scene: &mut Scene, id: u32, force: DVec2, torque: f64, seconds: f64) {
    for _ in 0..(seconds / DT).round() as usize {
        world.step(scene, &[Load { id, force, torque }]);
    }
    world.write(scene);
}

fn assert_close(got: f64, want: f64, tol: f64, what: &str) {
    let err = (got - want).abs() / want.abs();
    println!("{what} : {got:.6e} (attendu {want:.6e}, écart {:.2} %)", err * 100.0);
    assert!(err < tol, "{what} : écart {:.2} % > {} %", err * 100.0, tol * 100.0);
}

/// Vue de dessus : l'objet reste immobile tant que |F| ≤ μs·m·g, puis glisse avec μk.
#[test]
fn static_friction_threshold() {
    let (mu_s, mu_k) = (0.4, 0.2);
    for (ratio, moves) in [(0.5, false), (0.97, false), (1.03, true), (2.0, true)] {
        let mut scene = Scene::default();
        let id = block(&mut scene, DVec2::ZERO, mu_s, mu_k);
        let mass = scene.mass(scene.get(id).unwrap());
        let force = ratio * mu_s * mass * G;
        let mut world = World::new(&scene);
        run(&mut world, &mut scene, id, DVec2::new(force, 0.0), 0.0, 0.25);
        let x = scene.get(id).unwrap().pos.x;
        if moves {
            // Une fois décollé, a = F/m − μk·g.
            assert_close(x, 0.5 * (force / mass - mu_k * G) * 0.25f64.powi(2), 0.03, &format!("F = {ratio}·μs·m·g : déplacement"));
            assert!(world.motion(id).unwrap().sliding);
        } else {
            assert!(x.abs() < 1e-9, "F = {ratio}·μs·m·g : l'objet a bougé de {x} m");
            assert!(!world.motion(id).unwrap().sliding);
        }
    }
}

/// Lancé puis lâché, l'objet freine avec μk·g et s'arrête pour de bon.
#[test]
fn kinetic_friction_stops_the_object() {
    let (mu_s, mu_k) = (0.4, 0.2);
    let mut scene = Scene::default();
    let id = block(&mut scene, DVec2::new(-0.1, 0.0), mu_s, mu_k);
    let mass = scene.mass(scene.get(id).unwrap());
    let mut world = World::new(&scene);
    let (push, t1) = (3.0 * mu_s * mass * G, 0.1);
    run(&mut world, &mut scene, id, DVec2::new(push, 0.0), 0.0, t1);
    let a1 = push / mass - mu_k * G;
    let v1 = a1 * t1;
    assert_close(world.motion(id).unwrap().velocity.x, v1, 0.02, "vitesse en fin de poussée");
    run(&mut world, &mut scene, id, DVec2::ZERO, 0.0, 1.0);
    let travelled = scene.get(id).unwrap().pos.x + 0.1;
    assert_close(travelled, 0.5 * a1 * t1 * t1 + v1 * v1 / (2.0 * mu_k * G), 0.03, "distance d'arrêt");
    assert_eq!(world.motion(id).unwrap().velocity, DVec2::ZERO);
    assert!(!world.moving());
}

/// Vue de dessus, pivot au centre : un couple constant fait tourner l'objet, θ = ½·(τ/I)·t².
/// Avec frottement, il ne démarre qu'au-delà de μs·m·g·r, r étant le rayon moyen d'appui.
#[test]
fn pivot_and_torque() {
    let mut scene = Scene::default();
    let id = scene.add("aiguille", Shape::Rect { w: 0.04, h: 0.004 }, DVec2::new(0.02, 0.03), "NdFeB N42");
    scene.get_mut(id).unwrap().body = Body { mobile: true, mu_s: 0.0, mu_k: 0.0, link: Link::Pivot { anchor: DVec2::ZERO } };
    let mass = scene.mass(scene.get(id).unwrap());
    let inertia = mass * (0.04f64.powi(2) + 0.004f64.powi(2)) / 12.0;
    let torque = 1e-5;
    let mut world = World::new(&scene);
    run(&mut world, &mut scene, id, DVec2::new(0.3, -0.2), torque, 0.2);
    let o = scene.get(id).unwrap();
    assert_close(o.angle, 0.5 * torque / inertia * 0.04, 0.02, "angle sous couple constant");
    // Le pivot retient le centre malgré la force.
    assert!((o.pos.truncate() - DVec2::new(0.02, 0.03)).length() < 2e-5, "{:?}", o.pos);

    // Sur une table rugueuse, un couple trop faible ne fait rien.
    let mut scene = Scene::default();
    let id = scene.add("aiguille", Shape::Rect { w: 0.04, h: 0.004 }, DVec2::ZERO, "NdFeB N42");
    scene.get_mut(id).unwrap().body = Body { mobile: true, mu_s: 0.5, mu_k: 0.3, link: Link::Pivot { anchor: DVec2::ZERO } };
    // Barre mince : rayon moyen d'appui ≈ L/4.
    let limit = 0.5 * mass * G * 0.01;
    let mut world = World::new(&scene);
    run(&mut world, &mut scene, id, DVec2::ZERO, 0.8 * limit, 0.2);
    assert_eq!(scene.get(id).unwrap().angle, 0.0);
    run(&mut world, &mut scene, id, DVec2::ZERO, 1.3 * limit, 0.05);
    let angle = scene.get(id).unwrap().angle;
    assert!(angle > 0.1 && angle < 1.0, "{angle}");
}

/// Vue de dessus, glissière : seule la composante de la force le long de l'axe compte.
#[test]
fn slider_keeps_the_axis() {
    let mut scene = Scene::default();
    let id = block(&mut scene, DVec2::ZERO, 0.4, 0.2);
    scene.get_mut(id).unwrap().body.link = Link::Slider { angle: PI / 6.0 };
    let mass = scene.mass(scene.get(id).unwrap());
    let axis = DVec2::from_angle(PI / 6.0);
    let mut world = World::new(&scene);
    // Force forte, mais presque perpendiculaire à l'axe : sa composante utile reste sous le seuil.
    let weak = axis.perp() * (5.0 * mass * G) + axis * (0.9 * 0.4 * mass * G);
    run(&mut world, &mut scene, id, weak, 0.0, 0.2);
    assert!(scene.get(id).unwrap().pos.length() < 1e-5, "{:?}", scene.get(id).unwrap().pos);
    let strong = axis * (2.0 * 0.4 * mass * G);
    run(&mut world, &mut scene, id, strong, 0.0, 0.2);
    let pos = scene.get(id).unwrap().pos.truncate();
    assert_close(pos.dot(axis), 0.5 * (0.8 - 0.2) * G * 0.04, 0.03, "course le long de la glissière");
    assert!(pos.perp_dot(axis).abs() < 2e-5 && scene.get(id).unwrap().angle.abs() < 1e-3);
}

/// Masse au bout d'un ressort, sans frottement : période 2π·√(m/k).
#[test]
fn spring_period() {
    let mut scene = Scene::default();
    let id = block(&mut scene, DVec2::new(0.012, 0.0), 0.0, 0.0);
    let mass = scene.mass(scene.get(id).unwrap());
    let stiffness = 20.0;
    scene.get_mut(id).unwrap().body.link = Link::Spring { anchor: DVec2::new(-0.03, 0.0), stiffness, damping: 0.0, length: 0.04 };
    let mut world = World::new(&scene);
    // Écarté de 2 mm de sa position de repos (x = 10 mm), il oscille autour d'elle.
    let (mut crossings, mut last) = (Vec::new(), 0.002);
    for k in 0..2400 {
        world.step(&scene, &[]);
        world.write(&mut scene);
        let x = scene.get(id).unwrap().pos.x - 0.010;
        if last > 0.0 && x <= 0.0 {
            crossings.push((k as f64 + last / (last - x)) * DT);
        }
        last = x;
    }
    assert!(crossings.len() >= 3);
    assert_close(crossings[2] - crossings[1], 2.0 * PI * (mass / stiffness).sqrt(), 0.01, "période du ressort");
}

/// Vue de côté : chute libre y = y0 − ½·g·t², puis arrêt sur le sol (bord du domaine).
#[test]
fn free_fall_and_floor() {
    let mut scene = Scene::default();
    scene.mechanics.view = MechView::Side;
    let id = block(&mut scene, DVec2::new(0.0, 0.1), 0.4, 0.2);
    let mut world = World::new(&scene);
    run(&mut world, &mut scene, id, DVec2::ZERO, 0.0, 0.2);
    assert_close(0.1 - scene.get(id).unwrap().pos.y, 0.5 * G * 0.04, 0.02, "chute libre");
    run(&mut world, &mut scene, id, DVec2::ZERO, 0.0, 1.0);
    // Le sol est à −size/2 ; le bloc de 10 mm de haut repose dessus.
    let y = scene.get(id).unwrap().pos.y;
    assert!((y - (-0.2 + 0.005)).abs() < 3e-4, "le bloc devrait reposer sur le sol : y = {y}");
    assert!(world.motion(id).unwrap().velocity.length() < 2e-3);
}

/// Vue de côté, plan incliné : le bloc tient si tan θ < μs, sinon glisse avec a = g·(sin θ − μk·cos θ).
#[test]
fn inclined_plane() {
    let (mu_s, mu_k) = (0.5, 0.3);
    for (degrees, slides) in [(20.0f64, false), (35.0, true)] {
        let theta = degrees.to_radians();
        let mut scene = Scene::default();
        scene.mechanics.view = MechView::Side;
        let ramp = scene.add("rampe", Shape::Rect { w: 0.3, h: 0.01 }, DVec2::ZERO, "Aluminium");
        scene.get_mut(ramp).unwrap().angle = -theta;
        scene.get_mut(ramp).unwrap().body.mu_s = 1.0;
        // Bloc posé sur la rampe, parallèle à elle, en haut de la pente.
        let (along, normal) = (DVec2::from_angle(-theta), DVec2::from_angle(-theta).perp());
        let start = -along * 0.08 + normal * (0.005 + 0.005 + 2e-5);
        let id = block(&mut scene, start, mu_s, mu_k);
        scene.get_mut(id).unwrap().angle = -theta;
        let mut world = World::new(&scene);
        // Le temps de se poser.
        run(&mut world, &mut scene, id, DVec2::ZERO, 0.0, 0.1);
        let settled = scene.get(id).unwrap().pos.truncate();
        run(&mut world, &mut scene, id, DVec2::ZERO, 0.0, 0.2);
        let moved = (scene.get(id).unwrap().pos.truncate() - settled).dot(along);
        if slides {
            let a = G * (theta.sin() - mu_k * theta.cos());
            let v0 = world.motion(id).unwrap().velocity.dot(along) - a * 0.2;
            assert_close(moved, v0 * 0.2 + 0.5 * a * 0.04, 0.08, &format!("glissement à {degrees}°"));
        } else {
            assert!(moved.abs() < 2e-4, "à {degrees}° le bloc ne devrait pas glisser : {moved} m");
        }
    }
}

/// Vue de côté, pendule pesant : T = 2π·√(I_pivot / (m·g·d)).
#[test]
fn pendulum_period() {
    let mut scene = Scene::default();
    scene.mechanics.view = MechView::Side;
    let (length, width) = (0.08, 0.004);
    // Barre verticale suspendue par son extrémité haute.
    let id = scene.add("pendule", Shape::Rect { w: width, h: length }, DVec2::new(0.0, 0.0), "Aluminium");
    let o = scene.get_mut(id).unwrap();
    o.body = Body { mobile: true, mu_s: 0.0, mu_k: 0.0, link: Link::Pivot { anchor: DVec2::new(0.0, length / 2.0) } };
    // Écartée de 0,1 rad autour du pivot.
    let pivot = DVec2::new(0.0, length / 2.0);
    o.angle = 0.1;
    let offset = DVec2::from_angle(0.1).rotate(-pivot);
    (o.pos.x, o.pos.y) = (pivot.x + offset.x, pivot.y + offset.y);
    let mut world = World::new(&scene);
    let (mut crossings, mut last) = (Vec::new(), 0.1);
    for k in 0..1200 {
        world.step(&scene, &[]);
        world.write(&mut scene);
        let angle = scene.get(id).unwrap().angle;
        if last > 0.0 && angle <= 0.0 {
            crossings.push((k as f64 + last / (last - angle)) * DT);
        }
        last = angle;
    }
    assert!(crossings.len() >= 3, "{crossings:?}");
    let d = length / 2.0;
    let inertia_over_mass = (length * length + width * width) / 12.0 + d * d;
    assert_close(crossings[2] - crossings[1], 2.0 * PI * (inertia_over_mass / (G * d)).sqrt(), 0.02, "période du pendule");
}

/// Un objet mobile poussé contre un obstacle fixe s'y arrête, quelle que soit la forme.
#[test]
fn collisions_stop_at_obstacles() {
    let star: Vec<DVec2> = (0..10).map(|k| DVec2::from_angle(k as f64 * PI / 5.0) * if k % 2 == 0 { 0.012 } else { 0.005 }).collect();
    for shape in [Shape::Circle { r: 0.008 }, Shape::Ring { r_in: 0.004, r_out: 0.008 }, Shape::Polygon { pts: star }] {
        let mut scene = Scene::default();
        scene.add("mur", Shape::Rect { w: 0.01, h: 0.1 }, DVec2::new(0.04, 0.0), "Fer pur (Armco)");
        let id = scene.add("mobile", shape.clone(), DVec2::ZERO, "Fer pur (Armco)");
        scene.get_mut(id).unwrap().body = Body { mobile: true, mu_s: 0.1, mu_k: 0.1, link: Link::Free };
        let mass = scene.mass(scene.get(id).unwrap());
        let mut world = World::new(&scene);
        run(&mut world, &mut scene, id, DVec2::new(20.0 * mass, 0.0), 0.0, 1.0);
        let o = scene.get(id).unwrap();
        // Le mur commence à x = 35 mm : l'objet ne le traverse pas et vient s'y appuyer.
        let reach = o.world_contours()[0].iter().map(|p| p.x).fold(f64::MIN, f64::max);
        println!("{shape:?} : bord droit à {:.3} mm", reach * 1e3);
        assert!(reach < 0.035 + 3e-4 && reach > 0.035 - 1.5e-3, "{reach}");
    }
    // Une force de cent fois le poids (aimant collé à du fer) n'enfonce pas l'objet dans l'obstacle.
    let mut scene = Scene::default();
    scene.add("mur", Shape::Rect { w: 0.01, h: 0.1 }, DVec2::new(0.04, 0.0), "Fer pur (Armco)");
    let id = block(&mut scene, DVec2::new(0.02, 0.0), 0.1, 0.1);
    let mass = scene.mass(scene.get(id).unwrap());
    let mut world = World::new(&scene);
    run(&mut world, &mut scene, id, DVec2::new(1000.0 * mass, 0.0), 0.0, 1.0);
    let x = scene.get(id).unwrap().pos.x;
    println!("bloc sous 100 g : x = {:.4} mm (contact à 25 mm)", x * 1e3);
    assert!((x - 0.025).abs() < 1e-4, "{x}");

    // Les murs du domaine retiennent aussi les objets.
    let mut scene = Scene::default();
    let id = block(&mut scene, DVec2::new(0.15, 0.0), 0.0, 0.0);
    let mass = scene.mass(scene.get(id).unwrap());
    let mut world = World::new(&scene);
    run(&mut world, &mut scene, id, DVec2::new(50.0 * mass, 0.0), 0.0, 1.0);
    assert!((scene.get(id).unwrap().pos.x - 0.19).abs() < 5e-4, "{}", scene.get(id).unwrap().pos.x);
}

/// Un objet déplacé à la main repart sans vitesse ; un changement de réglage garde le mouvement.
#[test]
fn sync_follows_the_editor() {
    let mut scene = Scene::default();
    let id = block(&mut scene, DVec2::ZERO, 0.0, 0.0);
    let mass = scene.mass(scene.get(id).unwrap());
    let mut world = World::new(&scene);
    run(&mut world, &mut scene, id, DVec2::new(mass, 0.0), 0.0, 0.1);
    let v = world.motion(id).unwrap().velocity.x;
    assert_close(v, 0.1, 0.01, "vitesse acquise");
    assert!(world.drift() > 4e-3);
    world.mark();
    assert_eq!(world.drift(), 0.0);

    // Changer le frottement reconstruit le corps sans l'arrêter.
    scene.get_mut(id).unwrap().body.mu_k = 0.05;
    world.sync(&scene);
    assert_close(world.motion(id).unwrap().velocity.x, v, 1e-6, "vitesse conservée");
    world.step(&scene, &[]);
    assert!(world.motion(id).unwrap().velocity.x < v);

    // Le déplacer dans l'éditeur l'arrête.
    scene.get_mut(id).unwrap().pos.y = 0.05;
    world.sync(&scene);
    assert_eq!(world.motion(id).unwrap().velocity, DVec2::ZERO);
    world.step(&scene, &[]);
    world.write(&mut scene);
    assert!((scene.get(id).unwrap().pos.y - 0.05).abs() < 1e-7);

    // Un objet masqué quitte le monde.
    scene.get_mut(id).unwrap().visible = false;
    world.sync(&scene);
    assert!(world.motion(id).is_none());
}
