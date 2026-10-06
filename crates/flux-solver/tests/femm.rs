//! Comparaison à FEMM (section 3.9) : inductions à 2 % près, forces à 3 % près.
//!
//! `scripts_are_written` dépose dans `validation/femm/` un script Lua par scène de référence.
//! Lancé dans FEMM 4.2 (`femm.exe -lua-script=<scène>.lua`), chaque script écrit à côté de lui
//! un fichier `<scène>.txt` : forces, couples et inductions calculés par FEMM.
//! `flux2d_matches_femm` les compare à ceux de Flux2D ; une scène dont le fichier manque est
//! signalée, sans faire échouer le test.

use flux_core::femm::{lua_script, parse_results};
use flux_core::raster::rasterize;
use flux_core::scene::Scene;
use flux_core::shape::Shape;
use flux_core::{DVec2, DVec3};
use flux_solver::{Cpu64Reference, FieldSolver, Newton, forces};
use std::path::PathBuf;

fn directory() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../validation/femm")
}

/// Scènes de référence, avec trois sondes dans l'air.
fn scenes() -> Vec<(&'static str, Scene)> {
    let mut plate = Scene::demo();
    plate.probes = vec![DVec3::new(0.0, 0.0, 0.0), DVec3::new(0.0, 0.03, 0.0), DVec3::new(0.04, 0.02, 0.0)];

    let mut magnets = Scene::default();
    magnets.name = "Deux aimants".into();
    magnets.add("Gauche", Shape::Rect { w: 0.02, h: 0.02 }, DVec2::new(-0.022, 0.0), "NdFeB N42");
    let right = magnets.add("Droite", Shape::Rect { w: 0.02, h: 0.02 }, DVec2::new(0.022, 0.004), "NdFeB N42");
    magnets.get_mut(right).unwrap().mag_angle = 0.5;
    magnets.probes = vec![DVec3::new(0.0, 0.0, 0.0), DVec3::new(0.0, 0.02, 0.0), DVec3::new(0.05, -0.01, 0.0)];

    // Électroaimant : noyau d'acier entre les deux sections d'une bobine, et une armature.
    let mut lifter = Scene::default();
    lifter.name = "Électroaimant".into();
    lifter.add("Noyau", Shape::Rect { w: 0.02, h: 0.04 }, DVec2::ZERO, "Acier doux (S235)");
    for (x, amps) in [(-0.015, 4.0), (0.015, -4.0)] {
        let id = lifter.add("Bobine", Shape::Rect { w: 0.008, h: 0.036 }, DVec2::new(x, 0.0), "Cuivre (bobinage)");
        let o = lifter.get_mut(id).unwrap();
        (o.turns, o.current, o.fill) = (300.0, amps, 0.6);
    }
    lifter.add("Armature", Shape::Rect { w: 0.05, h: 0.01 }, DVec2::new(0.0, 0.028), "Acier doux (S235)");
    lifter.probes = vec![DVec3::new(0.0, 0.0215, 0.0), DVec3::new(0.03, 0.0, 0.0), DVec3::new(0.0, -0.03, 0.0)];

    let mut sheet = Scene::saturation_demo();
    sheet.probes = vec![DVec3::new(0.0, 0.006, 0.0), DVec3::new(0.03, -0.01, 0.0), DVec3::new(0.0, -0.035, 0.0)];
    vec![("aimant_plaque", plate), ("deux_aimants", magnets), ("electroaimant", lifter), ("tole_saturee", sheet)]
}

/// Écrit les scripts : `cargo test --release -p flux-solver --test femm`.
#[test]
fn scripts_are_written() {
    let dir = directory();
    std::fs::create_dir_all(&dir).unwrap();
    for (name, scene) in scenes() {
        let results = dir.join(format!("{name}.txt"));
        // Chemin absolu sans préfixe « \\?\ », que le Lua de FEMM ne comprend pas.
        let results = std::path::absolute(&results).unwrap();
        let lua = lua_script(&scene, &results.display().to_string(), true);
        assert!(!lua.contains("ATTENTION"), "{name} : {lua}");
        std::fs::write(dir.join(format!("{name}.lua")), lua).unwrap();
    }
}

#[test]
fn flux2d_matches_femm() {
    let mut compared = 0;
    for (name, scene) in scenes() {
        let Ok(text) = std::fs::read_to_string(directory().join(format!("{name}.txt"))) else {
            println!("{name} : pas de résultats FEMM (lancer validation/femm/{name}.lua dans FEMM 4.2)");
            continue;
        };
        let reference = parse_results(&text);
        assert_eq!(reference.wrenches.len(), scene.objects.len(), "{name} : résultats FEMM incomplets");
        let mut cpu = Cpu64Reference::default();
        Newton::solve(&mut cpu, rasterize(&scene, 2048), 1e-7);
        let found = forces(cpu.field(), &scene);
        // Les forces sont rapportées à la plus grande de la scène : une composante presque
        // nulle n'a pas d'écart relatif qui ait un sens.
        let scale = reference.wrenches.iter().map(|w| w.force.length()).fold(0.0, f64::max);
        for w in &reference.wrenches {
            let ours = found.iter().find(|f| f.id == w.id).unwrap();
            let gap = (ours.force.truncate() - w.force).length() / scale;
            println!("{name}, objet {} : F = {:?} N/m, FEMM {:?} N/m, écart {:.2} %", w.id, ours.force.truncate(), w.force, gap * 100.0);
            assert!(gap < 0.03, "{name}, objet {} : force à {:.2} % de FEMM", w.id, gap * 100.0);
        }
        for (p, b) in &reference.probes {
            let ours = cpu.field().sample(p.extend(0.0)).unwrap().b.truncate();
            let gap = (ours - *b).length() / b.length();
            println!("{name}, sonde {p:?} : B = {ours:?} T, FEMM {b:?} T, écart {:.2} %", gap * 100.0);
            assert!(gap < 0.02, "{name}, sonde {p:?} : induction à {:.2} % de FEMM", gap * 100.0);
        }
        compared += 1;
    }
    println!("{compared} scène(s) comparée(s) à FEMM");
}
