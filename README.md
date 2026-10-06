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
| `flux-core` | Matériaux, formes (SDF, booléens via `i_overlay`), scène RON (`.flux`), rastérisation. Sans fenêtre ni GPU. |
| `flux-solver` | Trait `FieldSolver`, MG-PCG `Planar2DGpu` (compute wgpu, f32) et `Cpu64Reference` (f64), forces par tenseur de Maxwell pondéré. |
| `flux-app` | Application `flux2d` : eframe (winit + egui + wgpu), thème « instrument », canevas. |

## Interface

L'interface suit la maquette « style instrument » (maquette HTML de référence, non versionnée) : thème dans `crates/flux-app/src/theme.rs`, widgets dans `ui.rs`, panneaux et canevas dans `app/`.

Polices : IBM Plex Sans et IBM Plex Mono sont embarquées dans l'exécutable (`crates/flux-app/assets/fonts/`, licence SIL OFL 1.1, voir `OFL.txt`).

- **Palette de commandes** : `Ctrl K` ou le bouton « Commandes » ouvre la liste de toutes les actions ; taper pour filtrer (sans se soucier des accents), flèches et `Entrée` pour lancer, `Échap` pour fermer.
- **Langue et unités** : les sélecteurs FR / EN et SI / CGS sont à droite de la barre supérieure. En CGS, l'induction s'affiche en gauss, les forces en dynes et χ est divisé par 4π ; le calcul reste en SI. En anglais, les nombres prennent un point décimal.
- **Saisie d'expressions** : tout champ numérique accepte une expression avec unités, par exemple `12 mm + 3 mm`, `2 * (1 cm + 0,5)`, `0,25 tr`, `300 K` ou `500 mA`. Sans unité, le nombre est dans l'unité du champ.
- **Panneaux** : la bibliothèque, l'inspecteur et le graphe de coupe se replient (chevron de leur en-tête) et s'ancrent sur le bord opposé (bouton ⇄, ou en glissant l'en-tête).
- **Arbre de scène** : chaque objet porte un bouton de visibilité (un objet masqué est retiré du calcul) et un bouton de verrouillage (ni déplacement, ni rotation, ni suppression sur le canevas).
- **Préférences** : langue, unités et disposition des panneaux sont conservées d'un lancement à l'autre.

## Raccourcis

`V` sélection · `R` rectangle · `E` disque · `O` ellipse · `A` anneau · `M` aimant · `C` bobine (clic : fil) · `H` sonde · `L` ligne de coupe
`P` polygone · `B` courbe de Bézier — clic : sommet, glisser : tangente (courbe), double-clic, `Entrée` ou clic sur le premier sommet : fermer, `Retour arrière` : retirer un sommet, `Échap` : abandonner
`G` graine d'une ligne de champ · `S` saupoudrer de la limaille
`1` lignes · `2` carte · `3` vecteurs · `4` LIC · `5` limaille · `6` boussoles · `7` particules · `F` cadrer · `Tab` masquer l'interface · `Suppr` supprimer
`Ctrl Z` / `Ctrl Maj Z` annuler / rétablir · `Ctrl D` ou `Alt`+glisser dupliquer · `Ctrl S` enregistrer · `Ctrl K` palette de commandes
Molette : zoom · clic milieu ou glisser dans le vide : déplacer la vue

## Gestes sur le canevas

- **Tourner** : poignée ronde au-dessus de l'objet sélectionné ; `Maj` : pas de 15°.
- **Orienter l'aimantation** : poignée au bout de la flèche blanche de l'aimant sélectionné (`Maj` : pas de 15°) ; double-clic sur l'aimant pour saisir l'angle.
- **Inverser un courant** : clic sur le symbole ⊙ / ⊗ du conducteur.
- **Changer de matériau** : glisser une ligne de la bibliothèque sur un objet, qui en montre l'aperçu avant le relâcher.
- **Opérations booléennes** : sélectionner un objet, `Maj` + clic sur un second, puis Union, Intersection ou Différence dans la section « Combiner » de l'inspecteur. Le résultat garde le matériau du premier objet.

## État par rapport à la feuille de route

- **Phase 0** — fait : MG-PCG GPU, référence CPU f64, isolignes, cas analytiques (`crates/flux-solver/tests/validation.rs`).
- **Phase 1** — fait : formes (dont polygone, courbe de Bézier et opérations booléennes), aimants, fils et bobines, matériaux linéaires, lignes/carte/vecteurs, sonde, ligne de coupe, sauvegarde, annuler/rétablir, gestes directs, palette de commandes, unités SI/CGS, saisie d'expressions, panneaux repliables et ancrables, visibilité et verrouillage, français et anglais, polices embarquées. Écarts par rapport au document : la traduction utilise une table interne (`lang.rs`) plutôt que `fluent`, et l'ancrage des panneaux est fait maison plutôt qu'avec `egui_tiles`, pour conserver le style de la maquette.
- **Phase 2** — amorcé : forces et couples (Maxwell pondéré), Br(T). Restent : B(H) non linéaire, Rapier, frottement, lois de température complètes.
- **Phase 4** — amorcé : supraconducteurs (effet Meissner sous Tc). Le reste des phases 3 à 5 n'est pas commencé.

Para- et diamagnétiques ne dévient pas visiblement les lignes de champ (effet de l'ordre de χ/2, soit 0,01 % pour le bismuth) : le solveur les traite comme le vide et calcule leur force par la densité de Kelvin. Seul un supraconducteur refroidi sous Tc (χ = −1) expulse le champ.

Une scène peut être passée en argument : `cargo run --release -p flux-app -- examples/supraconducteur.flux`. L'option `--modes=1245` choisit les modes de visualisation actifs au démarrage.

La comparaison avant/après (vue scindée ou carte de différence) se règle dans le panneau de droite, sans objet sélectionné : « Figer l'état actuel comme référence ».
