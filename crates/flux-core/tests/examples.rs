//! Les scènes d'exemple livrées dans `examples/` restent lisibles.

use flux_core::scene::Scene;

#[test]
fn example_scenes_load() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples");
    let mut count = 0;
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "flux") {
            let text = std::fs::read_to_string(&path).unwrap();
            Scene::from_ron(&text).unwrap_or_else(|e| panic!("{} : {e}", path.display()));
            count += 1;
        }
    }
    assert!(count > 0);
}

/// Régénère les fichiers d'exemple : `cargo test -p flux-core -- --ignored write_examples`.
#[test]
#[ignore]
fn write_examples() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples");
    std::fs::create_dir_all(dir).unwrap();
    for (file, scene) in [("aimant_plaque.flux", Scene::demo()), ("supraconducteur.flux", Scene::meissner_demo())] {
        std::fs::write(format!("{dir}/{file}"), scene.to_ron().unwrap()).unwrap();
    }
}
