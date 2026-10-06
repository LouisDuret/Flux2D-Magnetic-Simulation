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
| `flux-solver` | Trait `FieldSolver`, MG-PCG `Planar2DGpu` (compute wgpu, f32) et `Cpu64Reference` (f64), itération de Newton pour les matériaux saturables (`nonlinear.rs`), forces par tenseur de Maxwell pondéré. |
| `flux-mech` | Mécanique des objets mobiles : Rapier 2D (collisions, contacts, liaisons), table de la vue de dessus, frottement statique et dynamique. |
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

## Mécanique

Chaque objet est **fixe** (obstacle) ou **mobile** (section « Mécanique » de l'inspecteur). Sa masse vaut ρ·S·profondeur. La simulation se lance avec `Espace` ou le bouton de lecture de la barre supérieure ; `.` avance d'un pas, et le bouton de retour replace les objets où ils étaient au lancement.

- **Vue de dessus** (par défaut) : les objets reposent sur une table, la pesanteur est perpendiculaire au plan. Un objet ne démarre que si la force dépasse μs·m·g, puis glisse avec μk. L'inspecteur affiche la jauge de glissement F / (μs·m·g).
- **Vue de côté** : la pesanteur est dans le plan. Les objets fixes et le bord du domaine servent d'appuis ; un contact passe de μs à μk dès que l'objet glisse.
- **Frottement** : μs et μk par objet, avec les couples de matériaux du document en préréglages (acier/acier, bois/bois, PTFE/acier, coussin d'air, roulement…).
- **Liaisons** : libre, pivot (point de l'objet épinglé), glissière (un axe, sans rotation), ressort avec amortisseur vers un point fixe.
- **Couplage avec le champ** : la mécanique avance par pas de 1/240 s. Le champ et les forces sont recalculés dès qu'un objet s'est déplacé d'un quart de cellule ; si le calcul ne suit pas, la simulation ralentit (le rapport obtenu s'affiche dans la barre d'état) plutôt que d'avancer avec des forces périmées.

La vue se change dans l'inspecteur de la scène ou d'un clic sur l'étiquette en haut à droite du canevas.

## Matériaux saturables

Les ferromagnétiques doux ont une courbe B(H) : une table de points (H, B), interpolée par une spline cubique monotone (Fritsch–Carlson) et prolongée au-delà du dernier point par B = Bn + μ0·(H − Hn). Le solveur résout le problème non linéaire par une itération de Newton amortie par une recherche linéaire sur l'énergie, sur GPU comme sur CPU ; le nombre d'itérations s'affiche dans la barre d'état.

L'inspecteur d'un objet ferromagnétique donne sa perméabilité initiale, sa polarisation à saturation Js, l'induction moyenne dans l'objet, sa perméabilité effective, une jauge B / Js et la courbe B(H) avec le point de fonctionnement.

Les courbes de la bibliothèque sont des **courbes modèles**, calées sur la perméabilité initiale et sur Js de chaque matériau (J = Js·x/√(1+x²)) : elles ne viennent pas de fiches techniques mesurées. Les points sont enregistrés dans le fichier `.flux` (champ `bh` du matériau), où des valeurs mesurées peuvent les remplacer. Au-dessus de sa température de Curie, un matériau redevient linéaire (μr = 1).

## Raccourcis

`V` sélection · `R` rectangle · `E` disque · `O` ellipse · `A` anneau · `M` aimant · `C` bobine (clic : fil) · `H` sonde · `L` ligne de coupe
`P` polygone · `B` courbe de Bézier — clic : sommet, glisser : tangente (courbe), double-clic, `Entrée` ou clic sur le premier sommet : fermer, `Retour arrière` : retirer un sommet, `Échap` : abandonner
`G` graine d'une ligne de champ · `S` saupoudrer de la limaille
`1` lignes · `2` carte · `3` vecteurs · `4` LIC · `5` limaille · `6` boussoles · `7` particules · `F` cadrer · `Tab` masquer l'interface · `Suppr` supprimer
`Espace` lecture / pause de la simulation · `.` un pas
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
- **Phase 2** — en cours. Fait : forces et couples (Maxwell pondéré), Br(T), mécanique (Rapier, pesanteur, collisions, frottement statique et dynamique, vues de dessus et de côté, pivots, glissières, ressorts, lecture/pause/pas à pas, jauge de glissement), courbes B(H), saturation et solveur non linéaire. Restent : coercivité et désaimantation des aimants, motifs d'aimantation, lois de température complètes (Kuz'min, Curie–Weiss, bilan thermique), comparaison à FEMM. Écarts par rapport au document : les courbes B(H) de la bibliothèque sont des modèles et non des fiches techniques ; la première itération non linéaire est un pas de Picard, les suivantes des pas de Newton, sans la mise à jour entrelacée avec le gradient conjugué ; les liaisons se limitent au pivot, à la glissière et au ressort ; le frottement est réglé par objet, sans matrice de couples de matériaux.
- **Phase 4** — amorcé : supraconducteurs (effet Meissner sous Tc). Le reste des phases 3 à 5 n'est pas commencé.

Para- et diamagnétiques ne dévient pas visiblement les lignes de champ (effet de l'ordre de χ/2, soit 0,01 % pour le bismuth) : le solveur les traite comme le vide et calcule leur force par la densité de Kelvin. Seul un supraconducteur refroidi sous Tc (χ = −1) expulse le champ.

Validation : `crates/flux-mech/tests/mechanics.rs` (seuil de glissement, distance d'arrêt, chute, plan incliné, pendule, ressort, collisions) et `crates/flux-solver/tests/saturation.rs` (anneau de fer autour d'un fil, dont B suit exactement la courbe B(H) ; tôle saturée ; accord GPU/CPU).

Une scène peut être passée en argument : `cargo run --release -p flux-app -- examples/supraconducteur.flux`. L'option `--modes=1245` choisit les modes de visualisation actifs au démarrage.

La comparaison avant/après (vue scindée ou carte de différence) se règle dans le panneau de droite, sans objet sélectionné : « Figer l'état actuel comme référence ».
