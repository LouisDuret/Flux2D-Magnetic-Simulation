//! Langue de l'interface. Le texte français du code sert de clé ; l'anglais est une table
//! de correspondance. Une clé absente de la table s'affiche en français.

use serde::{Deserialize, Serialize};
use std::cell::Cell;
use std::collections::HashMap;
use std::sync::OnceLock;

#[derive(Clone, Copy, PartialEq, Debug, Default, Serialize, Deserialize)]
pub enum Lang {
    #[default]
    Fr,
    En,
}

thread_local! {
    static LANG: Cell<Lang> = const { Cell::new(Lang::Fr) };
    /// Textes demandés à `tr` sans traduction anglaise, relevés pendant les tests.
    #[cfg(test)]
    static MISSING: std::cell::RefCell<std::collections::BTreeSet<String>> = const { std::cell::RefCell::new(std::collections::BTreeSet::new()) };
}

pub fn set(lang: Lang) {
    LANG.with(|l| l.set(lang));
}

pub fn get() -> Lang {
    LANG.with(Cell::get)
}

/// Traduit un texte de l'interface dans la langue courante.
pub fn tr(text: &str) -> &str {
    static TABLE: OnceLock<HashMap<&str, &str>> = OnceLock::new();
    match TABLE.get_or_init(|| ENGLISH.iter().copied().collect()).get(text) {
        Some(english) if get() == Lang::En => english,
        Some(_) => text,
        None => {
            #[cfg(test)]
            MISSING.with(|m| m.borrow_mut().insert(text.to_owned()));
            text
        }
    }
}

/// Nom d'objet par défaut dans la langue courante : « Aimant 3 » devient « Magnet 3 ».
pub fn tr_name(name: &str) -> String {
    match name.rsplit_once(' ') {
        Some((stem, number)) if number.parse::<u32>().is_ok() => format!("{} {number}", tr(stem)),
        _ => tr(name).to_owned(),
    }
}

/// Nombre décimal dans la langue courante : virgule en français, point en anglais.
pub fn decimal(v: f64, decimals: usize) -> String {
    let text = format!("{v:.decimals$}");
    if get() == Lang::Fr { text.replace('.', ",") } else { text }
}

#[cfg(test)]
pub fn take_missing() -> Vec<String> {
    MISSING.with(|m| std::mem::take(&mut *m.borrow_mut()).into_iter().collect())
}

const ENGLISH: &[(&str, &str)] = &[
    // Barre supérieure.
    ("Nouveau", "New"),
    ("Exemples", "Examples"),
    ("Ouvrir…", "Open…"),
    ("Enregistrer", "Save"),
    ("Enregistrer sous…", "Save as…"),
    ("Aimant + plaque de fer", "Magnet + iron plate"),
    ("Plaque attirée sur une table", "Plate pulled across a table"),
    ("Tôle saturée", "Saturated sheet"),
    ("Tôle", "Sheet"),
    // Simulation mécanique.
    ("Pause (Espace)", "Pause (Space)"),
    ("Lecture (Espace)", "Play (Space)"),
    ("Avancer d’un pas (.)", "Step once (.)"),
    ("Revenir à l’état initial", "Back to the initial state"),
    ("MÉCANIQUE", "MECHANICS"),
    ("Dessus", "Top"),
    ("Côté", "Side"),
    ("Vue", "View"),
    ("Pesanteur", "Gravity"),
    ("Vitesse", "Speed"),
    ("Pause", "Pause"),
    ("Lecture", "Play"),
    ("Un pas", "One step"),
    ("Revenir", "Rewind"),
    (
        "Vue de dessus : les objets mobiles reposent sur une table et ne démarrent que si la force dépasse μs·m·g.",
        "Top view: movable objects rest on a table and only start when the force exceeds μs·m·g.",
    ),
    (
        "Vue de côté : la pesanteur agit vers le bas ; les objets fixes et le bord du domaine servent d’appuis.",
        "Side view: gravity pulls downward; fixed objects and the domain edge act as supports.",
    ),
    ("Fixe", "Fixed"),
    ("Mobile", "Movable"),
    ("Surface", "Surface"),
    ("Personnalisé", "Custom"),
    ("Acier / acier", "Steel / steel"),
    ("Aluminium / acier", "Aluminium / steel"),
    ("Cuivre / acier", "Copper / steel"),
    ("Verre / verre", "Glass / glass"),
    ("Bois / bois", "Wood / wood"),
    ("PTFE / acier", "PTFE / steel"),
    ("Caoutchouc / béton", "Rubber / concrete"),
    ("Glace / glace", "Ice / ice"),
    ("Coussin d'air", "Air cushion"),
    ("Roulement acier / acier", "Rolling steel / steel"),
    ("μs (statique)", "μs (static)"),
    ("μk (dynamique)", "μk (kinetic)"),
    ("Liaison", "Joint"),
    ("Libre", "Free"),
    ("Pivot", "Pivot"),
    ("Glissière", "Slider"),
    ("Ressort", "Spring"),
    ("Pivot X (local)", "Pivot X (local)"),
    ("Pivot Y (local)", "Pivot Y (local)"),
    ("Axe", "Axis"),
    ("Ancrage X", "Anchor X"),
    ("Ancrage Y", "Anchor Y"),
    ("Raideur", "Stiffness"),
    ("Amortissement", "Damping"),
    ("Longueur au repos", "Rest length"),
    ("Mouvement", "Motion"),
    ("en mouvement", "moving"),
    ("immobile", "at rest"),
    ("Glissement F / (μs·m·g)", "Slip F / (μs·m·g)"),
    (
        "Sans frottement statique : la moindre force met l’objet en mouvement.",
        "No static friction: the slightest force sets the object in motion.",
    ),
    ("LECTURE", "PLAYING"),
    (
        "Aucun objet mobile : rendez un objet « Mobile » dans l’inspecteur.",
        "No movable object: set an object to “Movable” in the inspector.",
    ),
    ("VUE DE DESSUS · TABLE", "TOP VIEW · TABLE"),
    ("VUE DE CÔTÉ", "SIDE VIEW"),
    ("Changer de vue : dessus (table) ou côté (pesanteur dans le plan)", "Switch view: top (table) or side (in-plane gravity)"),
    ("Lecture ou pause de la simulation", "Play or pause the simulation"),
    ("Avancer la simulation d’un pas", "Advance the simulation one step"),
    ("Vue de dessus (table)", "Top view (table)"),
    ("Vue de côté (pesanteur dans le plan)", "Side view (in-plane gravity)"),
    ("Espace", "Space"),
    // Matériaux saturables.
    ("μr initiale", "Initial μr"),
    ("Js (saturation)", "Js (saturation)"),
    ("B dans l’objet", "B inside the object"),
    ("μr effective", "Effective μr"),
    ("Saturation B / Js", "Saturation B / Js"),
    ("Supraconducteur et diamagnétique", "Superconductor and diamagnet"),
    ("Scène sans titre", "Untitled scene"),
    ("Annuler (Ctrl Z)", "Undo (Ctrl Z)"),
    ("Rétablir (Ctrl Maj Z)", "Redo (Ctrl Shift Z)"),
    ("Commandes", "Commands"),
    ("Palette de commandes (Ctrl K)", "Command palette (Ctrl K)"),
    ("Unités d'affichage", "Display units"),
    ("Langue", "Language"),
    // Outils.
    ("Sélection", "Select"),
    ("Rect.", "Rect."),
    ("Disque", "Disc"),
    ("Ellipse", "Ellipse"),
    ("Anneau", "Ring"),
    ("Polygone", "Polygon"),
    ("Courbe", "Curve"),
    ("Aimant", "Magnet"),
    ("Bobine", "Coil"),
    ("Sonde", "Probe"),
    ("Coupe", "Cut"),
    ("Graine", "Seed"),
    ("Limaille", "Filings"),
    ("Sélectionner et déplacer (V)", "Select and move (V)"),
    ("Dessiner un bloc du matériau courant (R)", "Draw a block of the current material (R)"),
    ("Dessiner un disque du matériau courant (E)", "Draw a disc of the current material (E)"),
    ("Dessiner une ellipse du matériau courant (O)", "Draw an ellipse of the current material (O)"),
    ("Dessiner un anneau du matériau courant (A)", "Draw a ring of the current material (A)"),
    (
        "Polygone (P) — clic : sommet · double-clic ou Entrée : fermer · Retour arrière : annuler un sommet",
        "Polygon (P) — click: vertex · double-click or Enter: close · Backspace: remove a vertex",
    ),
    (
        "Courbe de Bézier (B) — clic : sommet · glisser : tangente · double-clic ou Entrée : fermer",
        "Bézier curve (B) — click: vertex · drag: tangent · double-click or Enter: close",
    ),
    ("Dessiner un aimant (M)", "Draw a magnet (M)"),
    ("Glisser : bobine (aller + retour) · clic : fil (C)", "Drag: coil (out + return) · click: wire (C)"),
    ("Épingler une sonde (H)", "Pin a probe (H)"),
    ("Tracer une ligne de coupe (L)", "Draw a cut line (L)"),
    ("Tracer la ligne de champ passant par un point (G)", "Trace the field line through a point (G)"),
    ("Saupoudrer de la limaille de fer (S)", "Sprinkle iron filings (S)"),
    // Bibliothèque et arbre de scène.
    ("BIBLIOTHÈQUE", "LIBRARY"),
    ("INSPECTEUR", "INSPECTOR"),
    ("Rechercher un matériau", "Search a material"),
    ("Tous", "All"),
    ("Aimants", "Magnets"),
    ("Ferro", "Ferro"),
    ("Para", "Para"),
    ("Dia", "Dia"),
    ("Conducteurs", "Conductors"),
    ("Supra", "Super"),
    ("MATÉRIAU", "MATERIAL"),
    ("VALEUR", "VALUE"),
    ("SCÈNE", "SCENE"),
    ("Masquer : l'objet est retiré du calcul", "Hide: the object is removed from the calculation"),
    ("Afficher", "Show"),
    ("Verrouiller : ni déplacement, ni rotation, ni suppression sur le canevas", "Lock: no moving, rotating or deleting on the canvas"),
    ("Déverrouiller", "Unlock"),
    ("Replier le panneau", "Collapse the panel"),
    ("Déplier le panneau", "Expand the panel"),
    ("Ancrer de l'autre côté (ou glisser l'en-tête)", "Dock on the other side (or drag the header)"),
    ("ANCRER ICI", "DOCK HERE"),
    // Matériaux.
    ("Fer pur (Armco)", "Pure iron (Armco)"),
    ("Acier doux (S235)", "Mild steel (S235)"),
    ("Acier électrique Fe-3%Si", "Electrical steel Fe-3%Si"),
    ("Mu-métal", "Mu-metal"),
    ("Permalloy 50% Ni", "Permalloy 50% Ni"),
    ("Fer-cobalt (Permendur)", "Iron-cobalt (Permendur)"),
    ("Ferrite douce MnZn", "Soft ferrite MnZn"),
    ("Nickel", "Nickel"),
    ("Cobalt", "Cobalt"),
    ("Gadolinium", "Gadolinium"),
    ("Aluminium", "Aluminium"),
    ("Platine", "Platinum"),
    ("Titane", "Titanium"),
    ("Tungstène", "Tungsten"),
    ("Graphite pyrolytique", "Pyrolytic graphite"),
    ("Bismuth", "Bismuth"),
    ("Eau", "Water"),
    ("Diamant", "Diamond"),
    ("Argent", "Silver"),
    ("Or", "Gold"),
    ("Cuivre (bobinage)", "Copper (winding)"),
    ("Niobium", "Niobium"),
    ("Plomb", "Lead"),
    ("NdFeB N35", "NdFeB N35"),
    ("NdFeB N42", "NdFeB N42"),
    ("NdFeB N52", "NdFeB N52"),
    ("NdFeB N42SH", "NdFeB N42SH"),
    ("SmCo 2:17", "SmCo 2:17"),
    ("Ferrite Sr (Y30)", "Sr ferrite (Y30)"),
    ("AlNiCo 5", "AlNiCo 5"),
    ("YBCO", "YBCO"),
    ("BSCCO-2223", "BSCCO-2223"),
    ("MgB2", "MgB2"),
    // Inspecteur de scène.
    ("Profondeur", "Depth"),
    ("Domaine", "Domain"),
    ("T ambiante", "Ambient T"),
    ("CALCUL", "SOLVER"),
    ("Moteur", "Engine"),
    ("Grille", "Grid"),
    ("Pas", "Cell size"),
    ("Lignes", "Lines"),
    ("AFFICHAGE", "DISPLAY"),
    ("Animer la LIC", "Animate LIC"),
    ("Effacer les graines", "Clear seeds"),
    ("Balayer la limaille", "Sweep filings"),
    ("COMPARAISON", "COMPARISON"),
    ("Figer l’état actuel comme référence", "Freeze current state as reference"),
    ("Aucune", "None"),
    ("Avant | après", "Before | after"),
    ("Différence", "Difference"),
    ("Séparation", "Split"),
    (
        "Sélectionnez un objet pour l’inspecter, ou choisissez un outil et dessinez sur le canevas.",
        "Select an object to inspect it, or pick a tool and draw on the canvas.",
    ),
    // Inspecteur d'objet.
    ("OBJET", "OBJECT"),
    ("Nom", "Name"),
    ("Matériau", "Material"),
    ("Température", "Temperature"),
    ("GÉOMÉTRIE", "GEOMETRY"),
    ("Position X", "Position X"),
    ("Position Y", "Position Y"),
    ("Rotation", "Rotation"),
    ("Largeur", "Width"),
    ("Hauteur", "Height"),
    ("Rayon", "Radius"),
    ("Rayon intérieur", "Inner radius"),
    ("Rayon extérieur", "Outer radius"),
    ("Demi-axe X", "Semi-axis X"),
    ("Demi-axe Y", "Semi-axis Y"),
    ("Sommets", "Vertices"),
    ("Contours", "Contours"),
    ("Masse", "Mass"),
    ("AIMANTATION", "MAGNETIZATION"),
    ("FERROMAGNÉTIQUE", "FERROMAGNETIC"),
    ("PARAMAGNÉTIQUE", "PARAMAGNETIC"),
    ("DIAMAGNÉTIQUE", "DIAMAGNETIC"),
    ("COURANT", "CURRENT"),
    ("SUPRACONDUCTEUR", "SUPERCONDUCTOR"),
    ("Angle", "Angle"),
    ("Inverser les pôles", "Swap poles"),
    ("Br à 20 °C", "Br at 20 °C"),
    ("Br à la température de l’objet", "Br at object temperature"),
    ("μrec", "μrec"),
    ("Tc", "Tc"),
    ("μr (linéaire)", "μr (linear)"),
    ("Spires", "Turns"),
    ("Courant", "Current"),
    ("Sortant — inverser le sens", "Outgoing — reverse"),
    ("Entrant — inverser le sens", "Incoming — reverse"),
    ("Ampères-tours", "Ampere-turns"),
    ("Densité J", "Density J"),
    ("χ", "χ"),
    ("Modifie le champ d’environ", "Changes the field by about"),
    (
        "(χ/2) : invisible sur les lignes de champ. L’objet subit en revanche une force (densité de Kelvin), donnée ci-dessous.",
        "(χ/2): invisible on the field lines. The object does feel a force (Kelvin density), given below.",
    ),
    ("État", "State"),
    ("Meissner", "Meissner"),
    ("normal", "normal"),
    ("Azote liquide", "Liquid nitrogen"),
    ("Hélium liquide", "Liquid helium"),
    ("Ambiante", "Ambient"),
    ("Le champ est expulsé (χ = −1).", "The field is expelled (χ = −1)."),
    ("Refroidir sous Tc pour expulser le champ.", "Cool below Tc to expel the field."),
    ("FORCE", "FORCE"),
    ("|F|", "|F|"),
    ("Fx", "Fx"),
    ("Fy", "Fy"),
    ("|F| réelle", "Actual |F|"),
    ("Couple", "Torque"),
    ("F / (m·g)", "F / (m·g)"),
    ("Entrefer plus fin que la grille : valeur limitée par la résolution.", "Air gap thinner than the grid: value limited by resolution."),
    ("COMBINER", "COMBINE"),
    ("Avec", "With"),
    ("Maj + clic sur un objet", "Shift + click an object"),
    ("Union", "Union"),
    ("Intersection", "Intersection"),
    ("Opération sans résultat : il ne resterait aucune matière.", "The operation has no result: no material would remain."),
    (
        "Objet verrouillé : il ne peut être ni déplacé, ni tourné, ni supprimé sur le canevas.",
        "Locked object: it cannot be moved, rotated or deleted on the canvas.",
    ),
    ("Objet masqué : il est retiré du calcul.", "Hidden object: it is removed from the calculation."),
    // Graphe de coupe et barre d'état.
    ("COUPE · |B|", "CUT · |B|"),
    ("COUPE", "CUT"),
    ("longueur", "length"),
    ("max", "max"),
    ("Effacer", "Clear"),
    ("Exporter CSV", "Export CSV"),
    ("CALCUL…", "SOLVING…"),
    ("CONVERGÉ", "CONVERGED"),
    ("it.", "it."),
    ("résidu", "residual"),
    ("IMG/S", "FPS"),
    ("2D PLAN", "2D PLANAR"),
    ("PROFONDEUR", "DEPTH"),
    // Canevas.
    ("AVANT", "BEFORE"),
    ("APRÈS", "AFTER"),
    ("OPÉRANDE", "OPERAND"),
    ("Carte", "Map"),
    ("Vecteurs", "Vectors"),
    ("LIC", "LIC"),
    ("Boussoles", "Compasses"),
    ("Particules", "Particles"),
    ("Dézoomer", "Zoom out"),
    ("Zoomer", "Zoom in"),
    ("Cadrer la scène (F)", "Frame the scene (F)"),
    ("Clic : inverser le sens du courant", "Click: reverse the current"),
    (
        "Canevas vide — choisissez un outil : M aimant · R bloc · E disque · P polygone · C bobine\nMolette : zoom · clic milieu : déplacer la vue · F : cadrer",
        "Empty canvas — pick a tool: M magnet · R block · E disc · P polygon · C coil\nWheel: zoom · middle click: pan the view · F: frame",
    ),
    // Noms d'objets par défaut.
    ("Bloc", "Block"),
    ("Forme", "Shape"),
    ("Fil", "Wire"),
    ("Plaque", "Plate"),
    ("Graphite", "Graphite"),
    // Messages.
    ("Scène Flux2D", "Flux2D scene"),
    ("Ouverture impossible :", "Cannot open:"),
    ("Enregistré :", "Saved:"),
    ("Enregistrement impossible :", "Cannot save:"),
    ("Export impossible :", "Cannot export:"),
    ("Tracé trop petit : il faut au moins trois sommets non alignés.", "Path too small: at least three non-aligned vertices are needed."),
    ("Objet verrouillé : suppression ignorée.", "Locked object: deletion ignored."),
    // Palette de commandes.
    ("Rechercher une commande", "Search a command"),
    ("Aucune commande", "No command"),
    ("Outil", "Tool"),
    ("Affichage", "Display"),
    ("Exemple", "Example"),
    ("Nouvelle scène", "New scene"),
    ("Ouvrir une scène…", "Open a scene…"),
    ("Annuler", "Undo"),
    ("Rétablir", "Redo"),
    ("Cadrer la scène", "Frame the scene"),
    ("Masquer ou afficher l'interface", "Hide or show the interface"),
    ("Unités SI (T, N)", "SI units (T, N)"),
    ("Unités CGS (G, dyn)", "CGS units (G, dyn)"),
    ("Langue : français", "Language: French"),
    ("Langue : anglais", "Language: English"),
    ("Replier ou déplier la bibliothèque", "Collapse or expand the library"),
    ("Replier ou déplier l'inspecteur", "Collapse or expand the inspector"),
    ("Replier ou déplier le graphe de coupe", "Collapse or expand the cut graph"),
    ("Ancrer la bibliothèque de l'autre côté", "Dock the library on the other side"),
    ("Ancrer l'inspecteur de l'autre côté", "Dock the inspector on the other side"),
    ("Ancrer le graphe de coupe de l'autre côté", "Dock the cut graph on the other side"),
    ("Dupliquer la sélection", "Duplicate the selection"),
    ("Supprimer la sélection", "Delete the selection"),
    ("Masquer ou afficher la sélection", "Hide or show the selection"),
    ("Verrouiller ou déverrouiller la sélection", "Lock or unlock the selection"),
    ("Effacer la ligne de coupe", "Clear the cut line"),
    ("Ctrl Maj Z", "Ctrl Shift Z"),
    ("Suppr", "Del"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translates_and_falls_back() {
        assert_eq!(tr("Aimant"), "Aimant");
        assert_eq!(decimal(1.5, 2), "1,50");
        set(Lang::En);
        assert_eq!((tr("Aimant"), tr("Texte inconnu")), ("Magnet", "Texte inconnu"));
        assert_eq!(
            (tr_name("Aimant 12"), tr_name("Mon aimant 3"), tr_name("Plaque")),
            ("Magnet 12".into(), "Mon aimant 3".into(), "Plate".into())
        );
        assert_eq!(decimal(1.5, 2), "1.50");
        assert_eq!(take_missing(), ["Mon aimant", "Texte inconnu"]);
    }

    /// Aucune clé en double, aucune traduction vide.
    #[test]
    fn table_is_consistent() {
        let mut keys: Vec<&str> = ENGLISH.iter().map(|(fr, _)| *fr).collect();
        keys.sort_unstable();
        let total = keys.len();
        keys.dedup();
        assert_eq!(keys.len(), total, "clé en double");
        assert!(ENGLISH.iter().all(|(fr, en)| !fr.is_empty() && !en.is_empty()));
    }
}
