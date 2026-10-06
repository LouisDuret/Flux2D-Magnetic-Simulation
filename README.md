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
| `flux-core` | Matériaux et lois de température, formes (SDF, booléens via `i_overlay`), motifs d'aimantation, scène RON (`.flux`), rastérisation, bilan thermique, export vers FEMM. Sans fenêtre ni GPU. |
| `flux-solver` | Trait `FieldSolver`, MG-PCG `Planar2DGpu` (compute wgpu, f32) et `Cpu64Reference` (f64), itération de Newton pour les matériaux saturables (`nonlinear.rs`), désaimantation irréversible des aimants (`demag.rs`), forces par tenseur de Maxwell pondéré et densité de Kelvin. |
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
- **Liaisons** : libre, pivot (point de l'objet épinglé), glissière (un axe, sans rotation), ressort avec amortisseur vers un point fixe, fil inextensible (pendule).
- **Couplage avec le champ** : la mécanique avance par pas de 1/240 s. Le champ et les forces sont recalculés dès qu'un objet s'est déplacé d'un quart de cellule ; entre deux calculs, la force suit la droite qui joint les deux derniers relevés. Si le calcul ne suit pas, la simulation ralentit (le rapport obtenu s'affiche dans la barre d'état) plutôt que d'avancer avec des forces périmées. Quand seuls des para- ou diamagnétiques bougent, le champ ne change pas : seules leurs forces sont recalculées.
- **Conservation de l'énergie** : les efforts sont donnés comme une impulsion au début de chaque pas (schéma d'Euler symplectique). Un objet qui oscille dans un puits magnétique, comme une plaque en lévitation, ne gagne ni ne perd d'énergie ; rien ne l'amortit non plus, faute de courants de Foucault (phase 4).

La vue se change dans l'inspecteur de la scène ou d'un clic sur l'étiquette en haut à droite du canevas.

## Matériaux saturables

Les ferromagnétiques doux ont une courbe B(H) : une table de points (H, B), interpolée par une spline cubique monotone (Fritsch–Carlson) et prolongée au-delà du dernier point par B = Bn + μ0·(H − Hn). Le solveur résout le problème non linéaire par une itération de Newton amortie par une recherche linéaire sur l'énergie, sur GPU comme sur CPU ; le nombre d'itérations s'affiche dans la barre d'état.

L'inspecteur d'un objet ferromagnétique donne sa perméabilité initiale, sa polarisation à saturation Js, l'induction moyenne dans l'objet, sa perméabilité effective, une jauge B / Js et la courbe B(H) avec le point de fonctionnement.

Les courbes de la bibliothèque sont des **courbes modèles**, calées sur la perméabilité initiale et sur Js de chaque matériau (J = Js·x/√(1+x²)) : elles ne viennent pas de fiches techniques mesurées. Le bouton « Importer une courbe B(H)… » de l'inspecteur remplace celle d'un matériau par une table mesurée : un fichier texte ou CSV, une ligne par point, H en A/m et B en teslas (dans un ordre ou dans l'autre, virgule décimale acceptée). Les points sont enregistrés dans le fichier `.flux` (champ `bh` du matériau).

## Aimants

**Motifs d'aimantation** (inspecteur, « Motif ») : uniforme (diamétrale sur un disque ou un anneau), radiale, multipolaire, Halbach, peinte. Sur une forme ronde, les motifs suivent l'angle polaire (secteurs alternés, cylindre de Halbach à champ intérieur ou extérieur) ; sur les autres formes, ils suivent la longueur de l'objet (bandes alternées, réseau de Halbach à champ concentré dessus ou dessous). L'angle d'aimantation oriente ou déphase le motif. Le **pinceau** (`N`) peint l'aimantation d'un aimant : elle suit la direction du geste, sous l'empreinte du pinceau. Le canevas montre la direction locale par des flèches.

**Désaimantation irréversible** : chaque aimant a une coercivité intrinsèque HcJ(T) = HcJ(20 °C)·[1 + β·(T − 20)]. Tant que le champ inverse le long de son axe reste en deçà du coude de sa courbe (0,9·HcJ), il suit sa droite de recul B = μ0·μrec·H + Br. Au-delà, sa rémanence baisse pour de bon, cellule par cellule ; elle s'annule en −HcJ et s'inverse ensuite. L'état est enregistré avec la scène et ne revient qu'avec le bouton « Ré-aimanter ». L'inspecteur donne HcJ à la température de l'objet, la température maximale d'emploi, une jauge « champ inverse / coude » et la rémanence restante ; le canevas assombrit les zones désaimantées. Conséquences visibles : un AlNiCo trapu se désaimante seul, une ferrite poussée contre un NdFeB perd sa face en regard, un NdFeB chauffé au-delà de sa température d'emploi perd du flux qui ne revient pas en refroidissant. Au-dessus de Tc, toute l'aimantation est perdue. Le réglage « Désaimantation des aimants » de la scène rend les aimants idéaux.

La désaimantation est évaluée une fois le champ convergé, puis le champ est recalculé, jusqu'à l'accord. Le coude à 0,9·HcJ et la chute en ligne droite jusqu'à −HcJ sont un modèle : les courbes de désaimantation mesurées des fabricants ne sont pas dans la bibliothèque.

## Température et thermique

| Famille | Loi | Où |
|---|---|---|
| Paramagnétiques | Curie : χ(T) = χ(T0)·T0/T | force de Kelvin |
| Ferromagnétiques sous Tc | Kuz'min : Js(T)/Js(0) = [1 − s·τ^1,5 − (1 − s)·τ^2,5]^(1/3), τ = T/Tc. La courbe B(H) est mise à l'échelle à perméabilité initiale inchangée | calcul du champ |
| Ferromagnétiques au-dessus de Tc | Curie–Weiss : χ = C/(T − Tc), bornée à 1 K de Tc. Au-dessus de χ = 0,01 le matériau entre dans le calcul du champ ; en dessous, il ne subit que la force de Kelvin | champ ou force de Kelvin |
| Aimants | Br(T) = Br(20)·[1 + α·(T − 20)], HcJ(T) avec β | champ, désaimantation |
| Diamagnétiques | indépendants de T ; le graphite pyrolytique est anisotrope (χ = −4,5·10⁻⁴ en travers des feuillets, −8,5·10⁻⁵ le long), ses feuillets suivant la largeur de l'objet | force et couple de Kelvin |
| Bobinages | ρ(T) = ρ20·[1 + 0,00393·(T − 20)] ; résistance d'une section R = ρ·N²·profondeur / (remplissage·S) | inspecteur, effet Joule |

Les paramètres s de Kuz'min et les constantes de Curie du fer, du nickel, du cobalt et du gadolinium viennent de la littérature ; ceux des alliages sont des estimations. La loi de Kuz'min est appliquée avec l'exposant 5/2 du document pour tous les matériaux.

**Bilan thermique** (inspecteur de la scène, « Thermique ») : pendant la lecture, chaque objet est un bloc à température uniforme, m·c·dT/dt = P_Joule − h·S·(T − T_amb) − ε·σ·S·(T⁴ − T_amb⁴), mis à jour à 10 Hz. Le coefficient de convection h est réglable, et le temps thermique peut être accéléré (×10, ×100) : un bloc de fer met une dizaine de minutes à refroidir. Un objet à « température imposée » (bain d'azote, thermostat) garde la sienne. Les pertes par hystérésis et la conduction entre objets ne sont pas modélisées.

**Pistolet chauffant** (`T`) et **bombe de froid** (`Y`) : tenus sur un objet, ils l'amènent vers 500 °C ou −50 °C, d'autant plus vite qu'il est petit. Un objet plus chaud ou plus froid que l'air affiche sa température sur le canevas.

## Raccourcis

`V` sélection · `R` rectangle · `E` disque · `O` ellipse · `A` anneau · `M` aimant · `C` bobine (clic : fil) · `H` sonde · `L` ligne de coupe
`P` polygone · `B` courbe de Bézier — clic : sommet, glisser : tangente (courbe), double-clic, `Entrée` ou clic sur le premier sommet : fermer, `Retour arrière` : retirer un sommet, `Échap` : abandonner
`G` graine d'une ligne de champ · `S` saupoudrer de la limaille · `N` pinceau d'aimantation · `T` pistolet chauffant · `Y` bombe de froid
`1` lignes · `2` carte · `3` vecteurs · `4` LIC · `5` limaille · `6` boussoles · `7` particules · `F` cadrer · `Tab` masquer l'interface · `Suppr` supprimer
`Espace` lecture / pause de la simulation · `.` un pas
`Ctrl Z` / `Ctrl Maj Z` annuler / rétablir · `Ctrl D` ou `Alt`+glisser dupliquer · `Ctrl S` enregistrer · `Ctrl K` palette de commandes
Molette : zoom · clic milieu ou glisser dans le vide : déplacer la vue

## Gestes sur le canevas

- **Tourner** : poignée ronde au-dessus de l'objet sélectionné ; `Maj` : pas de 15°.
- **Orienter l'aimantation** : poignée au bout de la flèche blanche de l'aimant sélectionné (`Maj` : pas de 15°) ; double-clic sur l'aimant pour saisir l'angle.
- **Inverser un courant** : clic sur le symbole ⊙ / ⊗ du conducteur.
- **Bobine** : glisser avec l'outil `C` crée un seul objet, vu en coupe : deux sections parcourues en sens opposés (⊙ à droite, ⊗ à gauche pour un courant positif), entre lesquelles un noyau peut prendre place. Elle se déplace, se tourne et s'inverse d'un bloc ; l'inspecteur règle sa largeur, sa hauteur et l'épaisseur du bobinage. Chaque section porte tous les ampères-tours, la résistance compte l'aller et le retour, et la force est celle de la bobine entière.
- **Changer de matériau** : glisser une ligne de la bibliothèque sur un objet, qui en montre l'aperçu avant le relâcher.
- **Opérations booléennes** : sélectionner un objet, `Maj` + clic sur un second, puis Union, Intersection ou Différence dans la section « Combiner » de l'inspecteur. Le résultat garde le matériau du premier objet.

## État par rapport à la feuille de route

- **Phase 0** — fait : MG-PCG GPU, référence CPU f64, isolignes, cas analytiques (`crates/flux-solver/tests/validation.rs`).
- **Phase 1** — fait : formes (dont polygone, courbe de Bézier et opérations booléennes), aimants, fils et bobines, matériaux linéaires, lignes/carte/vecteurs, sonde, ligne de coupe, sauvegarde, annuler/rétablir, gestes directs, palette de commandes, unités SI/CGS, saisie d'expressions, panneaux repliables et ancrables, visibilité et verrouillage, français et anglais, polices embarquées. Écarts par rapport au document : la traduction utilise une table interne (`lang.rs`) plutôt que `fluent`, et l'ancrage des panneaux est fait maison plutôt qu'avec `egui_tiles`, pour conserver le style de la maquette.
- **Phase 2** — fait, à une réserve près (la comparaison à FEMM, ci-dessous) : courbes B(H), saturation et solveur non linéaire ; bibliothèque de matériaux (coercivité, lois de température, données thermiques) ; forces et couples (Maxwell pondéré, densité de Kelvin anisotrope) ; aimants à motifs et désaimantation irréversible ; mécanique (Rapier, pesanteur, collisions, frottement statique et dynamique, vues de dessus et de côté, pivots, glissières, ressorts, fils, lecture/pause/pas à pas, jauge de glissement) ; lois de température de la section 2.5, bilan thermique, pistolet chauffant et bombe de froid. Écarts par rapport au document : les courbes B(H) et de désaimantation de la bibliothèque sont des modèles et non des fiches techniques (une table mesurée s'importe) ; la première itération non linéaire est un pas de Picard, les suivantes des pas de Newton, sans la mise à jour entrelacée avec le gradient conjugué ; le frottement est réglé par objet, sans matrice de couples de matériaux ; pas d'épaisseur minimale de contact ; le mélange des matériaux dans les cellules de bord reste scalaire (moyenne de ν), d'où une erreur d'ordre 1 aux interfaces : une pièce de fer parcourue en long par le flux perd l'équivalent d'une cellule d'épaisseur de chaque côté.
- **Phase 4** — amorcé : supraconducteurs (effet Meissner sous Tc). Le reste des phases 3 à 5 n'est pas commencé.

Para- et diamagnétiques ne dévient pas visiblement les lignes de champ (effet de l'ordre de χ/2, soit 0,01 % pour le bismuth) : le solveur les traite comme le vide et calcule leur force par la densité de Kelvin. Seul un supraconducteur refroidi sous Tc (χ = −1) expulse le champ.

## Validation

`cargo test --release --workspace` rejoue les cas de la section 3.9 du document. Écarts mesurés avec le solveur de référence CPU f64 :

| Cas | Référence | Écart | Grille |
|---|---|---|---|
| Fil infini | B = μ0·I/(2πr) | < 0,5 % | 256² |
| Deux fils parallèles | F = μ0·I1·I2/(2πd) | < 1 % | 256² |
| Cylindre aimanté en travers | B = Br/2 | < 1 % | 256² |
| Cylindre perméable dans un champ appliqué | B = 2·μr·B0/(μr + 1) | 0,1 % (μr = 2) à 0,6 % (μr = 1000) | 1024² |
| Blindage par un tube | 4·μr·b²/[(μr+1)²·b² − (μr−1)²·a²] | 0,8 % (μr = 10), 1,7 % (μr = 100) | 2048² |
| Force entre deux aimants | modèle exact des charges magnétiques | 0,1 % | 1024² |
| Fil près d'un cylindre perméable | méthode des images | < 0,7 % | 1024² |
| Cylindre de Halbach | B = Br·ln(r_ext/r_int), nul dehors | 0,02 % | 512² |
| Anneau de fer autour d'un fil | courbe B(H), de 20 °C à Tc | < 1 % | 512² |
| Lévitation du graphite pyrolytique | B·∂B/∂z = μ0·ρ·g/\|χ\| = 60,3 T²/m | 0,1 % | 512² |
| Cylindre d'AlNiCo désaimanté par son propre champ | intersection droite de charge / courbe | 0,4 % | 256² |
| Convergence en maillage | ordre 2 dans l'air, 1 aux interfaces | 1,9 et 0,9 à 1,8 mesurés | 64² à 512² |
| Seuil de démarrage d'une bille | F = μs·m·g | démarre à 0,95·μs, pas à 1,05·μs | 256² |

Fichiers : `crates/flux-solver/tests/` (`validation.rs`, `saturation.rs`, `magnets.rs`, `temperature.rs`), `crates/flux-mech/tests/mechanics.rs` (seuil de glissement, distance d'arrêt, chute, plan incliné, pendules, ressort, collisions, conservation de l'énergie), `crates/flux-app/src/app/tests.rs` (gestes, simulation, outils).

### Comparaison à FEMM

Le critère de sortie de la phase 2 demande des forces à moins de 3 % de FEMM. **Cette comparaison n'a pas été faite : FEMM n'est pas installé sur la machine de développement.** Tout est prêt pour la faire :

1. `cargo test --release -p flux-solver --test femm` écrit dans `validation/femm/` un script Lua par scène de référence (aimant et plaque de fer, deux aimants, électroaimant, tôle saturée).
2. Lancer chaque script dans FEMM 4.2 : `femm.exe -lua-script=validation/femm/aimant_plaque.lua`. Il reconstruit la scène, calcule, puis écrit `aimant_plaque.txt` à côté (force et couple par objet, induction aux sondes).
3. Relancer le test : il compare Flux2D à ces fichiers et échoue si une force s'écarte de plus de 3 % ou une induction de plus de 2 %.

Une fois FEMM installé, `validation/femm/comparer.ps1` enchaîne les trois étapes.

La commande « Exporter la scène pour FEMM (.lua)… » de la palette écrit le même script pour la scène ouverte. Les objets ne doivent pas se chevaucher ; un motif peint ou une désaimantation partielle sont rendus par une aimantation uniforme, ce que le script signale en tête. Les scripts n'ont pas pu être essayés dans FEMM.

Sept scènes d'exemple sont livrées dans `examples/` et dans le menu « Exemples » : aimant et plaque de fer, supraconducteur et diamagnétique, plaque attirée sur une table, tôle saturée, réseau de Halbach, aimant surchauffé, lévitation du graphite. Une scène peut être passée en argument : `cargo run --release -p flux-app -- examples/supraconducteur.flux`. L'option `--modes=1245` choisit les modes de visualisation actifs au démarrage.

La comparaison avant/après (vue scindée ou carte de différence) se règle dans le panneau de droite, sans objet sélectionné : « Figer l'état actuel comme référence ».
