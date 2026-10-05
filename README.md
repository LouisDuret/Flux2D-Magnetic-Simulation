# Flux2D

Simulateur interactif de champs magnétiques en 2D plan, écrit en Rust, calculé sur GPU (wgpu).
Spécification : `Flux2D_preparation_developpement.docx`.

```bash
cargo run --release -p flux-app
```

```bash
cargo test --release --workspace
```

## Organisation

| Crate | Rôle |
|---|---|
| `flux-core` | Matériaux, formes (SDF), scène RON (`.flux`), rastérisation. Sans fenêtre ni GPU. |
| `flux-solver` | Trait `FieldSolver`, MG-PCG `Planar2DGpu` (compute wgpu, f32) et `Cpu64Reference` (f64), forces par tenseur de Maxwell pondéré. |
| `flux-app` | Application `flux2d` : eframe (winit + egui + wgpu), thème sombre, canevas. |

## Raccourcis

`V` sélection · `R` rectangle · `E` disque · `M` aimant · `C` bobine (clic : fil) · `H` sonde · `L` ligne de coupe
`G` graine d'une ligne de champ · `S` saupoudrer de la limaille
`1` lignes · `2` carte · `3` vecteurs · `4` LIC · `5` limaille · `6` boussoles · `7` particules · `F` cadrer · `Tab` masquer l'interface · `Suppr` supprimer
`Ctrl Z` / `Ctrl Maj Z` annuler / rétablir · `Ctrl D` ou `Alt`+glisser dupliquer · `Ctrl S` enregistrer
Molette : zoom · clic milieu ou glisser dans le vide : déplacer la vue

## État par rapport à la feuille de route

- **Phase 0** — fait : MG-PCG GPU, référence CPU f64, isolignes, cas analytiques (`crates/flux-solver/tests/validation.rs`).
- **Phase 1** — en grande partie : formes, aimants, fils et bobines, matériaux linéaires, lignes/carte/vecteurs, sonde, ligne de coupe, sauvegarde, annuler/rétablir, thème sombre. Manquent : polygone et Bézier à la souris, poignée de rotation, i18n, polices embarquées, ancrage des panneaux.
- **Phase 2** — amorcé : forces et couples (Maxwell pondéré), Br(T). Restent : B(H) non linéaire, Rapier, frottement, lois de température complètes.
- **Phase 4** — amorcé : supraconducteurs (effet Meissner sous Tc). Le reste des phases 3 à 5 n'est pas commencé.

Para- et diamagnétiques ne dévient pas visiblement les lignes de champ (effet de l'ordre de χ/2, soit 0,01 % pour le bismuth) : le solveur les traite comme le vide et calcule leur force par la densité de Kelvin. Seul un supraconducteur refroidi sous Tc (χ = −1) expulse le champ.

Une scène peut être passée en argument : `cargo run --release -p flux-app -- examples/supraconducteur.flux`. L'option `--modes=1245` choisit les modes de visualisation actifs au démarrage.

La comparaison avant/après (vue scindée ou carte de différence) se règle dans le panneau de droite, sans objet sélectionné : « Figer l'état actuel comme référence ».
