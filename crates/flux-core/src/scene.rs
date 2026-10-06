//! Modèle de scène, sérialisé en RON versionné (extension `.flux`).

use crate::ABSOLUTE_ZERO_C;
use crate::magnet::{Lattice, MagPattern, Magnetization};
use crate::material::{Material, library};
use crate::shape::{BoolOp, Contour, Sdf, Shape, boolean};
use crate::thermal::Thermal;
use glam::{DVec2, DVec3};
use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 3;

/// Vue mécanique de la scène (section 2.7).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MechView {
    /// Vue de dessus : les objets reposent sur une table, la gravité est perpendiculaire au plan.
    #[default]
    Top,
    /// Vue de côté : la gravité est dans le plan, vers le bas.
    Side,
}

/// Réglages mécaniques de la scène.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(default)]
pub struct Mechanics {
    pub view: MechView,
    /// Accélération de la pesanteur (m/s²).
    pub gravity: f64,
}

impl Default for Mechanics {
    fn default() -> Self {
        Mechanics { view: MechView::Top, gravity: 9.81 }
    }
}

/// Liaison d'un objet mobile avec le support.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Default)]
pub enum Link {
    #[default]
    Free,
    /// Pivot : le point `anchor` de l'objet (repère local) est épinglé au support.
    Pivot { anchor: DVec2 },
    /// Glissière : translation le long de l'axe d'angle `angle` (rad, repère monde), sans rotation.
    Slider { angle: f64 },
    /// Ressort et amortisseur entre le centre de l'objet et le point fixe `anchor` (repère
    /// monde) : raideur en N/m, amortissement en N·s/m, longueur au repos en m.
    Spring { anchor: DVec2, stiffness: f64, damping: f64, length: f64 },
    /// Fil inextensible de longueur `length` (m) entre le centre de l'objet et le point fixe
    /// `anchor` (repère monde) : il retient l'objet sans le pousser (pendule).
    Rope { anchor: DVec2, length: f64 },
}

/// Couples de matériaux à sec (section 6.5) : nom, μs, μk.
pub const SURFACES: [(&str, f64, f64); 10] = [
    ("Acier / acier", 0.74, 0.57),
    ("Aluminium / acier", 0.61, 0.47),
    ("Cuivre / acier", 0.53, 0.36),
    ("Verre / verre", 0.94, 0.40),
    ("Bois / bois", 0.40, 0.20),
    ("PTFE / acier", 0.04, 0.04),
    ("Caoutchouc / béton", 1.0, 0.8),
    ("Glace / glace", 0.10, 0.03),
    ("Coussin d'air", 0.0, 0.001),
    // Résistance au roulement d'une bille ou d'un cylindre.
    ("Roulement acier / acier", 0.001, 0.001),
];

/// Comportement mécanique d'un objet.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(default)]
pub struct Body {
    /// Un objet fixe sert d'obstacle ; un objet mobile obéit aux forces.
    pub mobile: bool,
    /// Coefficient de frottement statique.
    pub mu_s: f64,
    /// Coefficient de frottement dynamique.
    pub mu_k: f64,
    pub link: Link,
}

impl Default for Body {
    fn default() -> Self {
        Body { mobile: false, mu_s: 0.40, mu_k: 0.20, link: Link::Free }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Object {
    pub id: u32,
    pub name: String,
    pub shape: Shape,
    /// Position du centre (z = 0 en 2D).
    pub pos: DVec3,
    /// Rotation autour de z (rad).
    pub angle: f64,
    pub material: String,
    /// Direction d'aimantation dans le repère de l'objet (rad).
    #[serde(default)]
    pub mag_angle: f64,
    /// Motif d'aimantation, orienté par `mag_angle`.
    #[serde(default)]
    pub pattern: MagPattern,
    /// Directions du motif peint (rad, repère de l'objet).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paint: Option<Lattice>,
    /// Part de la rémanence qui reste, cellule par cellule, après une désaimantation
    /// irréversible (1 : intacte, négative : aimantation retournée). `None` : aimant intact.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub demag: Option<Lattice>,
    /// Nombre de spires traversant la section.
    #[serde(default)]
    pub turns: f64,
    /// Courant par spire (A), positif = sortant (⊙).
    #[serde(default)]
    pub current: f64,
    /// Part de la section d'un bobinage occupée par le cuivre (1 : conducteur massif).
    #[serde(default = "one")]
    pub fill: f64,
    /// Température de l'objet (°C).
    pub temperature: f64,
    /// Température imposée (bain, thermostat) : le bilan thermique ne la modifie pas.
    #[serde(default)]
    pub thermostat: bool,
    /// Un objet masqué n'est ni affiché ni pris en compte dans le calcul.
    #[serde(default = "yes")]
    pub visible: bool,
    /// Un objet verrouillé ne se déplace, ne se tourne et ne se supprime pas sur le canevas.
    #[serde(default)]
    pub locked: bool,
    #[serde(default)]
    pub body: Body,
}

fn yes() -> bool {
    true
}

fn one() -> f64 {
    1.0
}

impl Object {
    pub fn to_local(&self, p: DVec3) -> DVec3 {
        let d = (p - self.pos).truncate();
        DVec2::from_angle(-self.angle).rotate(d).extend(0.0)
    }

    pub fn to_world(&self, local: DVec2) -> DVec2 {
        self.pos.truncate() + DVec2::from_angle(self.angle).rotate(local)
    }

    /// Contours de l'objet en repère monde (le premier extérieur, les suivants des trous).
    pub fn world_contours(&self) -> Vec<Contour> {
        let mut contours = self.shape.contours();
        contours.iter_mut().flatten().for_each(|p| *p = self.to_world(*p));
        contours
    }

    /// Distance signée en repère monde.
    pub fn distance(&self, p: DVec3) -> f64 {
        self.shape.distance(self.to_local(p))
    }

    pub fn amp_turns(&self) -> f64 {
        self.turns * self.current
    }

    /// Direction de référence de l'aimantation en repère monde (celle du motif uniforme).
    pub fn mag_dir(&self) -> DVec2 {
        DVec2::from_angle(self.angle + self.mag_angle)
    }

    /// Direction de l'aimantation en chaque point, selon le motif.
    pub fn magnetization(&self) -> Magnetization<'_> {
        Magnetization::of(self)
    }

    /// Part de la rémanence qui reste au point `local` après désaimantation irréversible.
    pub fn remanence_left(&self, local: DVec2) -> f64 {
        self.demag.as_ref().map_or(1.0, |lattice| lattice.get(local) as f64)
    }

    /// Moyenne sur l'objet de la part de rémanence restante (1 : aimant intact).
    pub fn mean_remanence_left(&self) -> f64 {
        let Some(lattice) = &self.demag else { return 1.0 };
        let inside = (0..lattice.values.len()).filter(|&k| self.shape.distance(lattice.center(k).extend(0.0)) <= 0.0);
        let (sum, count) = inside.fold((0.0, 0), |(sum, count), k| (sum + lattice.values[k] as f64, count + 1));
        if count > 0 { sum / count as f64 } else { 1.0 }
    }

    /// Passe au motif peint en gardant les directions du motif actuel, et renvoie la grille
    /// des directions (rad, repère de l'objet).
    pub fn painted(&mut self) -> &mut Lattice {
        if self.pattern != MagPattern::Painted || self.paint.is_none() {
            let mut lattice = Lattice::covering(&self.shape, 0.0);
            let magnetization = self.magnetization();
            let angles: Vec<f32> = (0..lattice.values.len()).map(|k| magnetization.local_angle(lattice.center(k)) as f32).collect();
            lattice.values = angles;
            (self.pattern, self.paint) = (MagPattern::Painted, Some(lattice));
        }
        self.paint.as_mut().unwrap()
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Scene {
    pub schema_version: u32,
    pub name: String,
    /// Profondeur d'extrusion (m) : convertit les N/m en N.
    pub depth: f64,
    /// Côté du domaine de calcul carré, centré sur l'origine (m).
    pub size: f64,
    pub ambient: f64,
    pub objects: Vec<Object>,
    #[serde(default = "library")]
    pub materials: Vec<Material>,
    #[serde(default)]
    pub probes: Vec<DVec3>,
    /// Graines des lignes de champ tracées par intégration.
    #[serde(default)]
    pub seeds: Vec<DVec3>,
    #[serde(default)]
    pub cut_line: Option<[DVec3; 2]>,
    #[serde(default)]
    pub mechanics: Mechanics,
    #[serde(default)]
    pub thermal: Thermal,
    /// Désaimantation irréversible des aimants prise en compte (coercivité HcJ).
    #[serde(default = "yes")]
    pub demagnetization: bool,
    next_id: u32,
}

impl Default for Scene {
    fn default() -> Self {
        Scene {
            schema_version: SCHEMA_VERSION,
            name: "Scène sans titre".into(),
            depth: 0.010,
            size: 0.4,
            ambient: 20.0,
            objects: Vec::new(),
            materials: library(),
            probes: Vec::new(),
            seeds: Vec::new(),
            cut_line: None,
            mechanics: Mechanics::default(),
            thermal: Thermal::default(),
            demagnetization: true,
            next_id: 1,
        }
    }
}

impl Scene {
    pub fn material(&self, name: &str) -> Option<&Material> {
        self.materials.iter().find(|m| m.name == name)
    }

    /// Masse d'un objet (kg) : ρ·S·profondeur.
    pub fn mass(&self, obj: &Object) -> f64 {
        self.material(&obj.material).map_or(0.0, |m| m.density) * obj.shape.area() * self.depth
    }

    /// Ajoute un objet et renvoie son identifiant.
    pub fn add(&mut self, name: &str, shape: Shape, pos: DVec2, material: &str) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        self.objects.push(Object {
            id,
            name: format!("{name} {id}"),
            shape,
            pos: pos.extend(0.0),
            angle: 0.0,
            material: material.into(),
            mag_angle: 0.0,
            pattern: MagPattern::Uniform,
            paint: None,
            demag: None,
            turns: 0.0,
            current: 0.0,
            fill: 1.0,
            temperature: self.ambient,
            thermostat: false,
            visible: true,
            locked: false,
            body: Body::default(),
        });
        id
    }

    /// Ajoute un objet délimité par des contours en repère monde ; `None` si l'aire est nulle.
    pub fn add_contours(&mut self, name: &str, contours: Vec<Contour>, material: &str) -> Option<u32> {
        let (shape, center) = Shape::from_contours(contours)?;
        Some(self.add(name, shape, center, material))
    }

    /// Remplace l'objet `a` par le résultat de `a op b` et supprime `b`. Un résultat en
    /// plusieurs morceaux donne plusieurs objets. Renvoie l'identifiant du premier, ou `None`
    /// (scène inchangée) si le résultat est vide.
    pub fn boolean(&mut self, a: u32, b: u32, op: BoolOp) -> Option<u32> {
        let (first, second) = (self.get(a)?, self.get(b).filter(|_| a != b)?);
        let regions = boolean(&first.world_contours(), &second.world_contours(), op);
        let mut template = first.clone();
        // Le résultat n'est plus tourné : l'aimantation garde sa direction en repère monde.
        (template.mag_angle, template.angle) = (template.mag_angle + template.angle, 0.0);
        let mut pieces = Vec::new();
        for (shape, center) in regions.into_iter().filter_map(Shape::from_contours) {
            let mut piece = template.clone();
            (piece.shape, piece.pos) = (shape, center.extend(0.0));
            if !pieces.is_empty() {
                piece.id = self.next_id;
                self.next_id += 1;
            }
            pieces.push(piece);
        }
        if pieces.is_empty() {
            return None;
        }
        self.objects.retain(|o| o.id != b);
        let at = self.objects.iter().position(|o| o.id == a)?;
        self.objects.splice(at..=at, pieces);
        Some(a)
    }

    pub fn get(&self, id: u32) -> Option<&Object> {
        self.objects.iter().find(|o| o.id == id)
    }

    pub fn get_mut(&mut self, id: u32) -> Option<&mut Object> {
        self.objects.iter_mut().find(|o| o.id == id)
    }

    /// Duplique un objet avec un décalage ; renvoie le nouvel identifiant.
    pub fn duplicate(&mut self, id: u32, offset: DVec2) -> Option<u32> {
        let mut o = self.get(id)?.clone();
        o.id = self.next_id;
        self.next_id += 1;
        o.pos += offset.extend(0.0);
        self.objects.push(o);
        Some(self.next_id - 1)
    }

    /// Objet visible le plus haut dans la pile contenant le point.
    pub fn pick(&self, p: DVec3) -> Option<u32> {
        self.objects.iter().rev().find(|o| o.visible && o.distance(p) <= 0.0).map(|o| o.id)
    }

    /// Ramène toute température sous le zéro absolu à −273,15 °C.
    pub fn clamp_temperatures(&mut self) {
        self.ambient = self.ambient.max(ABSOLUTE_ZERO_C);
        for o in &mut self.objects {
            o.temperature = o.temperature.max(ABSOLUTE_ZERO_C);
        }
    }

    pub fn to_ron(&self) -> Result<String, ron::Error> {
        ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())
    }

    pub fn from_ron(s: &str) -> Result<Scene, String> {
        let mut scene: Scene = ron::from_str(s).map_err(|e| e.to_string())?;
        scene.clamp_temperatures();
        if scene.schema_version > SCHEMA_VERSION {
            return Err(format!("version de schéma {} non prise en charge", scene.schema_version));
        }
        if scene.schema_version < 2 {
            // Avant la version 2, les ferromagnétiques n'avaient pas de courbe B(H) : ceux de la
            // bibliothèque reçoivent la leur.
            let lib = library();
            for m in scene.materials.iter_mut().filter(|m| m.bh.is_none()) {
                m.bh = lib.iter().find(|l| l.name == m.name && l.class == m.class).and_then(|l| l.bh.clone());
            }
        }
        if scene.schema_version < 3 {
            // Avant la version 3 : ni coercivité, ni lois de température, ni données thermiques.
            // Les matériaux de la bibliothèque les reçoivent, et ceux qui manquaient sont ajoutés.
            for l in library() {
                match scene.materials.iter_mut().find(|m| m.name == l.name && m.class == l.class) {
                    Some(m) => {
                        // Une courbe donnée à une autre température que 20 °C remplace l'ancienne.
                        let bh = if l.t_ref == 20.0 { m.bh.take() } else { l.bh.clone() };
                        *m = Material { mu_r: m.mu_r, chi: m.chi, br: m.br, alpha_br: m.alpha_br, density: m.density, bh, ..l };
                    }
                    None => scene.materials.push(l),
                }
            }
            // Les températures réglées à la main étaient tenues : elles le restent.
            let ambient = scene.ambient;
            scene.objects.iter_mut().for_each(|o| o.thermostat = o.temperature != ambient);
        }
        scene.schema_version = SCHEMA_VERSION;
        Ok(scene)
    }

    /// Rend leur aimantation d'origine à un aimant, ou à tous (`None`).
    pub fn remagnetize(&mut self, id: Option<u32>) {
        self.objects.iter_mut().filter(|o| id.is_none_or(|id| o.id == id)).for_each(|o| o.demag = None);
    }

    /// Effet Meissner : un supraconducteur refroidi à l'azote liquide et une plaque de
    /// graphite au-dessus d'un aimant.
    pub fn meissner_demo() -> Scene {
        let mut s = Scene { name: "Supraconducteur et diamagnétique".into(), ..Scene::default() };
        let magnet = s.add("Aimant", Shape::Rect { w: 0.050, h: 0.012 }, DVec2::new(0.0, -0.012), "NdFeB N42");
        s.get_mut(magnet).unwrap().mag_angle = std::f64::consts::FRAC_PI_2;
        let supra = s.add("Supra", Shape::Circle { r: 0.008 }, DVec2::new(-0.010, 0.012), "YBCO");
        // Le supraconducteur trempe dans l'azote liquide : sa température est tenue.
        let supra = s.get_mut(supra).unwrap();
        (supra.temperature, supra.thermostat) = (-196.0, true);
        s.add("Graphite", Shape::Rect { w: 0.016, h: 0.003 }, DVec2::new(0.022, 0.004), "Graphite pyrolytique");
        s
    }

    /// Réseau de Halbach : l'aimantation tourne le long du barreau et concentre le champ
    /// au-dessus de lui ; la tôle posée de ce côté est bien plus attirée que celle du dessous.
    pub fn halbach_demo() -> Scene {
        let mut s = Scene { name: "Réseau de Halbach".into(), ..Scene::default() };
        let bar = s.add("Halbach", Shape::Rect { w: 0.080, h: 0.010 }, DVec2::ZERO, "NdFeB N42");
        s.get_mut(bar).unwrap().pattern = MagPattern::Halbach { pairs: 2, flip: false };
        s.add("Tôle dessus", Shape::Rect { w: 0.080, h: 0.003 }, DVec2::new(0.0, 0.0125), "Acier doux (S235)");
        s.add("Tôle dessous", Shape::Rect { w: 0.080, h: 0.003 }, DVec2::new(0.0, -0.0125), "Acier doux (S235)");
        s
    }

    /// Lévitation diamagnétique (vue de côté) : une plaque de graphite pyrolytique flotte à
    /// un millimètre d'un damier d'aimants, là où B·∂B/∂z atteint μ0·ρ·g/|χ|.
    pub fn levitation_demo() -> Scene {
        let mut s = Scene { name: "Lévitation du graphite".into(), size: 0.1, ..Scene::default() };
        s.mechanics.view = MechView::Side;
        for (k, x) in [-0.012, -0.004, 0.004, 0.012].into_iter().enumerate() {
            let magnet = s.add("Aimant", Shape::Rect { w: 0.008, h: 0.008 }, DVec2::new(x, -0.004), "NdFeB N52");
            s.get_mut(magnet).unwrap().mag_angle = if k % 2 == 0 { 1.0 } else { -1.0 } * std::f64::consts::FRAC_PI_2;
        }
        let plate = s.add("Graphite", Shape::Rect { w: 0.020, h: 0.0008 }, DVec2::new(0.0, 0.0015), "Graphite pyrolytique");
        s.get_mut(plate).unwrap().body = Body { mobile: true, ..Body::default() };
        s
    }

    /// Aimant surchauffé : à 120 °C, la coercivité d'un NdFeB N42 ne suffit plus à tenir son
    /// propre champ démagnétisant. La perte est irréversible : elle reste après refroidissement.
    pub fn overheated_demo() -> Scene {
        let mut s = Scene { name: "Aimant surchauffé".into(), ..Scene::default() };
        let hot = s.add("Aimant chaud", Shape::Rect { w: 0.030, h: 0.010 }, DVec2::new(-0.030, 0.0), "NdFeB N42");
        let hot = s.get_mut(hot).unwrap();
        (hot.mag_angle, hot.temperature, hot.thermostat) = (std::f64::consts::FRAC_PI_2, 120.0, true);
        let cold = s.add("Aimant témoin", Shape::Rect { w: 0.030, h: 0.010 }, DVec2::new(0.030, 0.0), "NdFeB N42");
        s.get_mut(cold).unwrap().mag_angle = std::f64::consts::FRAC_PI_2;
        s
    }

    /// Exemple chiffré de la section 2.7 : la plaque de fer, posée sur une table, se précipite
    /// vers l'aimant si le frottement est faible.
    pub fn friction_demo() -> Scene {
        let mut s = Scene { name: "Plaque attirée sur une table".into(), ..Scene::demo() };
        let plate = &mut s.objects[1];
        plate.pos.x = 0.030;
        // Bois sur bois : la force vaut ici près de trois fois le seuil μs·m·g.
        plate.body = Body { mobile: true, ..Body::default() };
        s
    }

    /// Une tôle mince devant un aimant puissant : le fer sature et laisse fuir le champ.
    pub fn saturation_demo() -> Scene {
        let mut s = Scene { name: "Tôle saturée".into(), ..Scene::default() };
        let magnet = s.add("Aimant", Shape::Rect { w: 0.030, h: 0.020 }, DVec2::new(0.0, -0.013), "NdFeB N52");
        s.get_mut(magnet).unwrap().mag_angle = std::f64::consts::FRAC_PI_2;
        s.add("Tôle", Shape::Rect { w: 0.080, h: 0.002 }, DVec2::new(0.0, 0.0), "Acier doux (S235)");
        s
    }

    /// Scène de référence du document : aimant NdFeB + plaque de fer.
    pub fn demo() -> Scene {
        let mut s = Scene { name: "Aimant + plaque de fer".into(), ..Scene::default() };
        s.add("Aimant", Shape::Rect { w: 0.020, h: 0.040 }, DVec2::new(-0.025, 0.0), "NdFeB N42");
        s.add("Plaque", Shape::Rect { w: 0.016, h: 0.060 }, DVec2::new(0.020, 0.0), "Fer pur (Armco)");
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ron_roundtrip() {
        let s = Scene::demo();
        assert_eq!(Scene::from_ron(&s.to_ron().unwrap()).unwrap(), s);
    }

    #[test]
    fn no_temperature_below_absolute_zero() {
        let mut s = Scene::demo();
        s.ambient = -500.0;
        s.objects[0].temperature = -300.0;
        let loaded = Scene::from_ron(&s.to_ron().unwrap()).unwrap();
        assert_eq!((loaded.ambient, loaded.objects[0].temperature), (ABSOLUTE_ZERO_C, ABSOLUTE_ZERO_C));
        assert_eq!(loaded.objects[1].temperature, 20.0);
    }

    #[test]
    fn boolean_replaces_both_objects() {
        let mut s = Scene::default();
        let magnet = s.add("a", Shape::Rect { w: 0.04, h: 0.02 }, DVec2::ZERO, "NdFeB N42");
        s.get_mut(magnet).unwrap().angle = std::f64::consts::FRAC_PI_2;
        let hole = s.add("b", Shape::Circle { r: 0.005 }, DVec2::ZERO, "Fer pur (Armco)");
        assert_eq!(s.boolean(magnet, hole, BoolOp::Difference), Some(magnet));
        assert_eq!(s.objects.len(), 1);
        let o = &s.objects[0];
        assert!(matches!(o.shape, Shape::Region { .. }));
        // L'aimant tourné de 90° reste aimanté vers le haut, et le trou est bien vide.
        assert!((o.mag_dir() - DVec2::Y).length() < 1e-12);
        assert_eq!(s.pick(DVec3::ZERO), None);
        assert_eq!(s.pick(DVec3::new(0.0, 0.015, 0.0)), Some(magnet));
        assert_eq!(s.pick(DVec3::new(0.015, 0.0, 0.0)), None);

        // Un résultat vide laisse la scène intacte.
        let far = s.add("c", Shape::Circle { r: 0.005 }, DVec2::new(0.1, 0.1), "Fer pur (Armco)");
        assert_eq!(s.boolean(magnet, far, BoolOp::Intersection), None);
        assert_eq!(s.objects.len(), 2);
        assert_eq!(Scene::from_ron(&s.to_ron().unwrap()).unwrap(), s);
    }

    /// Un fichier de la version 1 (sans mécanique ni courbe B(H)) reste lisible et gagne les
    /// courbes de la bibliothèque.
    #[test]
    fn version_1_files_are_upgraded() {
        let mut s = Scene::demo();
        s.schema_version = 1;
        s.materials.iter_mut().for_each(|m| m.bh = None);
        // Les blocs « body: ( … ), » et « mechanics: ( … ), » sont retirés ligne à ligne.
        let mut skipping = false;
        let mut old = String::new();
        for line in s.to_ron().unwrap().lines() {
            let word = line.trim();
            skipping |= word.starts_with("body: (") || word.starts_with("mechanics: (") || word.starts_with("thermal: (");
            if !skipping {
                old += line;
                old.push('\n');
            }
            skipping &= word != "),";
        }
        assert!(!old.contains("body:") && !old.contains("mechanics:") && old.contains("next_id"));
        let loaded = Scene::from_ron(&old).unwrap();
        assert_eq!(loaded.schema_version, SCHEMA_VERSION);
        assert_eq!(loaded.objects[1].body, Body::default());
        assert_eq!(loaded.mechanics, Mechanics::default());
        assert!(loaded.material("Fer pur (Armco)").unwrap().bh.is_some());
        assert!(loaded.material("NdFeB N42").unwrap().bh.is_none());
    }

    /// Un fichier de la version 2 gagne la coercivité, les lois de température, les données
    /// thermiques et les matériaux ajoutés depuis ; les températures réglées à la main sont tenues.
    #[test]
    fn version_2_files_are_upgraded() {
        let mut s = Scene::meissner_demo();
        s.schema_version = 2;
        s.materials.retain(|m| !["Magnétite (Fe3O4)", "Inox austénitique 304", "Oxygène liquide"].contains(&m.name.as_str()));
        for m in &mut s.materials {
            let (mu_r, chi, br, alpha_br, t_curie, t_critical, density) =
                (m.mu_r, m.chi, m.br, m.alpha_br, m.t_curie, m.t_critical, m.density);
            let bh = m.bh.take();
            *m = Material { mu_r, chi, br, alpha_br, t_curie, t_critical, density, bh, ..Material::new(&m.name, m.class) };
        }
        s.objects.iter_mut().for_each(|o| o.thermostat = false);
        assert_eq!(s.material("NdFeB N42").unwrap().hcj, 0.0);
        let loaded = Scene::from_ron(&s.to_ron().unwrap()).unwrap();
        assert_eq!(loaded.schema_version, SCHEMA_VERSION);
        assert_eq!(loaded.materials.len(), library().len());
        for l in library() {
            assert_eq!(loaded.material(&l.name), Some(&l), "{}", l.name);
        }
        // Le supraconducteur à −196 °C garde sa température, les objets à l'ambiante restent libres.
        let held: Vec<bool> = loaded.objects.iter().map(|o| o.thermostat).collect();
        assert_eq!(held, [false, true, false]);
        assert!(loaded.thermal.enabled && loaded.demagnetization);
    }

    /// Les grilles d'un aimant peint ou désaimanté survivent à l'enregistrement, et le motif
    /// peint part des directions du motif qu'il remplace.
    #[test]
    fn painted_and_demagnetized_magnets_roundtrip() {
        let mut s = Scene::halbach_demo();
        let o = &mut s.objects[0];
        let before: Vec<DVec2> = [-0.03, -0.01, 0.02].iter().map(|x| o.magnetization().dir(DVec2::new(*x, 0.001))).collect();
        let lattice = o.painted();
        assert_eq!((lattice.nx, lattice.ny), (24, 3));
        assert_eq!(o.pattern, MagPattern::Painted);
        for (x, dir) in [-0.03, -0.01, 0.02].iter().zip(before) {
            // À la résolution de la grille près : une cellule fait 3,3 mm, soit 30° de rotation.
            assert!(o.magnetization().dir(DVec2::new(*x, 0.001)).dot(dir) > 0.8);
        }
        let mut demag = Lattice::covering(&o.shape, 1.0);
        demag.values[..36].fill(0.5);
        o.demag = Some(demag);
        assert!((o.mean_remanence_left() - 0.75).abs() < 1e-12);
        assert_eq!(o.remanence_left(DVec2::new(-0.039, -0.004)), 0.5);
        let loaded = Scene::from_ron(&s.to_ron().unwrap()).unwrap();
        assert_eq!(loaded, s);
        s.remagnetize(None);
        assert_eq!(s.objects[0].mean_remanence_left(), 1.0);
        // Un fichier sans ces grilles ne les écrit pas.
        assert!(!Scene::demo().to_ron().unwrap().contains("demag:"));
    }

    /// Un objet masqué est ignoré par la sélection, et un fichier sans ces champs reste lisible.
    #[test]
    fn hidden_objects_and_old_files() {
        let mut s = Scene::demo();
        s.objects[1].visible = false;
        assert_eq!(s.pick(DVec3::new(0.02, 0.0, 0.0)), None);
        let old = s.to_ron().unwrap().replace("visible: false,", "").replace("visible: true,", "").replace("locked: false,", "");
        let loaded = Scene::from_ron(&old).unwrap();
        assert!(loaded.objects.iter().all(|o| o.visible && !o.locked));
    }

    #[test]
    fn pick_respects_rotation() {
        let mut s = Scene::default();
        let id = s.add("r", Shape::Rect { w: 0.1, h: 0.01 }, DVec2::ZERO, "Fer pur (Armco)");
        assert_eq!(s.pick(DVec3::new(0.04, 0.0, 0.0)), Some(id));
        s.get_mut(id).unwrap().angle = std::f64::consts::FRAC_PI_2;
        assert_eq!(s.pick(DVec3::new(0.04, 0.0, 0.0)), None);
        assert_eq!(s.pick(DVec3::new(0.0, 0.04, 0.0)), Some(id));
    }
}
