//! Modèle de scène, sérialisé en RON versionné (extension `.flux`).

use crate::ABSOLUTE_ZERO_C;
use crate::material::{Material, library};
use crate::shape::{BoolOp, Contour, Sdf, Shape, boolean};
use glam::{DVec2, DVec3};
use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 1;

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
    /// Nombre de spires traversant la section.
    #[serde(default)]
    pub turns: f64,
    /// Courant par spire (A), positif = sortant (⊙).
    #[serde(default)]
    pub current: f64,
    /// Température de l'objet (°C).
    pub temperature: f64,
    /// Un objet masqué n'est ni affiché ni pris en compte dans le calcul.
    #[serde(default = "yes")]
    pub visible: bool,
    /// Un objet verrouillé ne se déplace, ne se tourne et ne se supprime pas sur le canevas.
    #[serde(default)]
    pub locked: bool,
}

fn yes() -> bool {
    true
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

    /// Direction d'aimantation en repère monde.
    pub fn mag_dir(&self) -> DVec2 {
        DVec2::from_angle(self.angle + self.mag_angle)
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
            next_id: 1,
        }
    }
}

impl Scene {
    pub fn material(&self, name: &str) -> Option<&Material> {
        self.materials.iter().find(|m| m.name == name)
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
            turns: 0.0,
            current: 0.0,
            temperature: self.ambient,
            visible: true,
            locked: false,
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
        Ok(scene)
    }

    /// Effet Meissner : un supraconducteur refroidi à l'azote liquide et une plaque de
    /// graphite au-dessus d'un aimant.
    pub fn meissner_demo() -> Scene {
        let mut s = Scene { name: "Supraconducteur et diamagnétique".into(), ..Scene::default() };
        let magnet = s.add("Aimant", Shape::Rect { w: 0.050, h: 0.012 }, DVec2::new(0.0, -0.012), "NdFeB N42");
        s.get_mut(magnet).unwrap().mag_angle = std::f64::consts::FRAC_PI_2;
        let supra = s.add("Supra", Shape::Circle { r: 0.008 }, DVec2::new(-0.010, 0.012), "YBCO");
        s.get_mut(supra).unwrap().temperature = -196.0;
        s.add("Graphite", Shape::Rect { w: 0.016, h: 0.003 }, DVec2::new(0.022, 0.004), "Graphite pyrolytique");
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
