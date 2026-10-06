//! Mécanique des objets mobiles (section 2.7) : masse, gravité, contacts, liaisons et frottement.
//!
//! Rapier 2D détecte les collisions et résout contacts et liaisons. Ce module y ajoute ce que
//! Rapier ne modélise pas : la table de la vue de dessus (réaction normale m·g) et la
//! transition explicite entre frottement statique et dynamique.

use flux_core::DVec2;
use flux_core::scene::{Body, Link, MechView, Object, Scene};
use flux_core::shape::{Contour, Sdf, Shape, moments, simplify, trapezoids};
use rapier2d::prelude::*;

/// Sous-pas d'intégration (s).
pub const DT: f64 = 1.0 / 240.0;

/// Vitesses en dessous desquelles un objet de la vue de dessus est tenu pour immobile.
const REST_SPEED: f64 = 1e-5;
const REST_SPIN: f64 = 1e-4;
/// Vitesse de glissement (m/s) au-delà de laquelle un contact de la vue de côté passe au
/// frottement dynamique.
const SLIP_SPEED: f64 = 3e-3;
/// Déplacement (m) en dessous duquel la pose d'un objet n'est pas reportée dans la scène.
const STILL: f64 = 2e-7;

/// Effort appliqué à un objet pendant un pas : force (N) et couple (N·m) autour de son centre.
#[derive(Clone, Copy, Debug)]
pub struct Load {
    pub id: u32,
    pub force: DVec2,
    pub torque: f64,
}

/// Mouvement d'un objet mobile.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Motion {
    /// Vitesse du centre de gravité (m/s).
    pub velocity: DVec2,
    /// Vitesse de rotation (rad/s).
    pub spin: f64,
    /// L'objet glisse : le frottement dynamique s'applique.
    pub sliding: bool,
}

/// Ce qui, dans un objet, impose de reconstruire son corps quand il change.
#[derive(Clone, PartialEq)]
struct Key {
    shape: Shape,
    density: f64,
    body: Body,
}

struct Entry {
    id: u32,
    handle: RigidBodyHandle,
    key: Key,
    /// Masse (kg), moment d'inertie au centre de gravité (kg·m²) et centre de gravité local.
    mass: f64,
    inertia: f64,
    com: DVec2,
    /// Rayon moyen d'appui sur la table : le couple de frottement vaut μ·m·g·grip.
    grip: f64,
    radius: f64,
    /// Point du support où le pivot est épinglé (repère monde).
    pivot: Option<DVec2>,
    /// Dernière pose connue de la scène, et pose au dernier calcul du champ.
    pose: (DVec2, f64),
    mark: (DVec2, f64),
    motion: Motion,
}

/// Monde mécanique construit depuis une scène : un corps par objet visible, fixe ou mobile.
pub struct World {
    physics: PhysicsWorld,
    entries: Vec<Entry>,
    view: MechView,
    gravity: f64,
    depth: f64,
    size: f64,
    /// Temps simulé (s).
    pub time: f64,
}

fn vector(v: DVec2) -> Vector {
    Vector::new(v.x as f32, v.y as f32)
}

fn dvec(v: Vector) -> DVec2 {
    DVec2::new(v.x as f64, v.y as f64)
}

/// Ramène un angle dans ]−π, π].
fn wrap(angle: f64) -> f64 {
    let turn = std::f64::consts::TAU;
    angle - turn * (angle / turn).round()
}

fn is_convex(c: &[DVec2]) -> bool {
    let n = c.len();
    let turns = (0..n).map(|i| (c[(i + 1) % n] - c[i]).perp_dot(c[(i + 2) % n] - c[(i + 1) % n]));
    turns.clone().all(|t| t >= 0.0) || turns.clone().all(|t| t <= 0.0)
}

/// Forme de collision d'un objet, en repère local. Les formes concaves ou trouées deviennent
/// un assemblage de pièces convexes.
fn collider(shape: &Shape) -> ColliderBuilder {
    let hull = |pts: &[DVec2]| SharedShape::convex_hull(&pts.iter().map(|p| vector(*p)).collect::<Vec<_>>());
    let compound = |pieces: Vec<Vec<DVec2>>| {
        let parts: Vec<(Pose, SharedShape)> = pieces.iter().filter_map(|p| hull(p)).map(|s| (Pose::identity(), s)).collect();
        if parts.is_empty() { ColliderBuilder::ball(shape.bounding_radius() as f32) } else { ColliderBuilder::compound(parts) }
    };
    // Les arcs sont allégés : la collision n'a pas besoin de 96 segments par cercle.
    let outline = |contours: Vec<Contour>| -> Vec<Contour> {
        let tol = 0.004 * shape.bounding_radius();
        contours.iter().map(|c| simplify(c, tol)).collect()
    };
    match shape {
        Shape::Rect { w, h } => ColliderBuilder::cuboid(*w as f32 / 2.0, *h as f32 / 2.0),
        Shape::Circle { r } => ColliderBuilder::ball(*r as f32),
        Shape::Ring { r_in, r_out } => {
            // Couronne de trapèzes entre les deux cercles.
            let at = |k: usize, r: f64| DVec2::from_angle(k as f64 / 32.0 * std::f64::consts::TAU) * r;
            compound((0..32).map(|k| vec![at(k, *r_in), at(k, *r_out), at(k + 1, *r_out), at(k + 1, *r_in)]).collect())
        }
        _ => {
            let contours = outline(shape.contours());
            match contours.as_slice() {
                [only] if is_convex(only) => match hull(only) {
                    Some(s) => ColliderBuilder::new(s),
                    None => ColliderBuilder::ball(shape.bounding_radius() as f32),
                },
                _ => compound(trapezoids(&contours).iter().map(|t| t.to_vec()).collect()),
            }
        }
    }
}

impl World {
    pub fn new(scene: &Scene) -> World {
        let mut physics = PhysicsWorld::new();
        let side = scene.mechanics.view == MechView::Side;
        physics.gravity = if side { Vector::new(0.0, -scene.mechanics.gravity as f32) } else { Vector::ZERO };
        physics.integration_parameters.dt = DT as f32;
        // Rapier est réglé pour des objets d'un mètre ; les nôtres font un centimètre.
        physics.integration_parameters.length_unit = 0.01;
        // Un aimant collé à du fer appuie avec des dizaines de fois son poids : les contacts
        // souples de Rapier, réglés pour la pesanteur, le laisseraient s'enfoncer.
        let stiff = SpringCoefficients::new(1000.0, 10.0);
        (physics.integration_parameters.contact_softness, physics.integration_parameters.static_contact_softness) = (stiff, stiff);

        // Le bord du domaine de calcul sert de murs, et de sol en vue de côté.
        let ground = physics.bodies.insert(RigidBodyBuilder::fixed());
        let half = scene.size as f32 / 2.0;
        for (x, y) in [(-1.0, 0.0), (1.0, 0.0), (0.0, -1.0), (0.0, 1.0)] {
            let wall = ColliderBuilder::cuboid(if x == 0.0 { 4.0 * half } else { half }, if y == 0.0 { 4.0 * half } else { half })
                .translation(Vector::new(2.0 * half * x, 2.0 * half * y))
                .friction(1.0)
                .friction_combine_rule(CoefficientCombineRule::Min);
            physics.colliders.insert_with_parent(wall, ground, &mut physics.bodies);
        }

        let mut entries = Vec::new();
        for o in scene.objects.iter().filter(|o| o.visible) {
            let density = scene.material(&o.material).map_or(0.0, |m| m.density);
            let (pos, mobile) = (o.pos.truncate(), o.body.mobile);
            let (area, com, polar) = moments(&o.shape.contours());
            let mass = (density * o.shape.area() * scene.depth).max(1e-9);
            let inertia = (mass * polar / area.max(1e-300)).max(1e-15);
            let mut builder = if mobile { RigidBodyBuilder::dynamic() } else { RigidBodyBuilder::fixed() };
            builder = builder.translation(vector(pos)).rotation(o.angle as f32).can_sleep(false);
            if mobile {
                builder =
                    builder.ccd_enabled(true).additional_mass_properties(MassProperties::new(vector(com), mass as f32, inertia as f32));
            }
            let handle = physics.bodies.insert(builder);
            let shape = collider(&o.shape).density(0.0).friction(o.body.mu_s as f32).friction_combine_rule(CoefficientCombineRule::Min);
            physics.colliders.insert_with_parent(shape, handle, &mut physics.bodies);

            let mut pivot = None;
            let mut grip = 2.0 / 3.0 * (2.0 * inertia / mass).sqrt();
            match o.body.link {
                Link::Pivot { anchor } if mobile => {
                    let at = o.to_world(anchor);
                    physics.impulse_joints.insert(
                        ground,
                        handle,
                        RevoluteJointBuilder::new().local_anchor1(vector(at)).local_anchor2(vector(anchor)),
                        true,
                    );
                    grip = grip.hypot((anchor - com).length());
                    pivot = Some(at);
                }
                Link::Slider { angle } if mobile => {
                    let axis = DVec2::from_angle(angle);
                    let joint = PrismaticJointBuilder::new(vector(axis))
                        .local_anchor1(vector(pos))
                        .local_anchor2(Vector::ZERO)
                        .local_axis2(vector(DVec2::from_angle(angle - o.angle)));
                    physics.impulse_joints.insert(ground, handle, joint, true);
                }
                _ => {}
            }
            let key = Key { shape: o.shape.clone(), density, body: o.body };
            let pose = (pos, o.angle);
            let radius = o.shape.bounding_radius();
            entries.push(Entry {
                id: o.id,
                handle,
                key,
                mass,
                inertia,
                com,
                grip,
                radius,
                pivot,
                pose,
                mark: pose,
                motion: Motion::default(),
            });
        }
        World {
            physics,
            entries,
            view: scene.mechanics.view,
            gravity: scene.mechanics.gravity,
            depth: scene.depth,
            size: scene.size,
            time: 0.0,
        }
    }

    /// La scène a-t-elle encore les objets, formes et réglages avec lesquels le monde a été construit ?
    fn matches(&self, scene: &Scene) -> bool {
        let mut visible = scene.objects.iter().filter(|o| o.visible);
        self.view == scene.mechanics.view
            && self.gravity == scene.mechanics.gravity
            && self.depth == scene.depth
            && self.size == scene.size
            && self.entries.iter().all(|e| {
                visible.next().is_some_and(|o| {
                    o.id == e.id
                        && o.body == e.key.body
                        && o.shape == e.key.shape
                        && scene.material(&o.material).map_or(0.0, |m| m.density) == e.key.density
                })
            })
            && visible.next().is_none()
    }

    /// Met le monde en accord avec la scène : reconstruit si elle a changé de structure (les
    /// vitesses sont conservées), et replace sans vitesse les objets déplacés à la main.
    pub fn sync(&mut self, scene: &Scene) {
        if !self.matches(scene) {
            let mut fresh = World::new(scene);
            fresh.time = self.time;
            for e in &mut fresh.entries {
                if let Some(old) = self.entries.iter().find(|old| old.id == e.id && old.pose == e.pose) {
                    let (from, to) = (&self.physics.bodies[old.handle], &mut fresh.physics.bodies[e.handle]);
                    if to.is_dynamic() && from.is_dynamic() {
                        to.set_linvel(from.linvel(), true);
                        to.set_angvel(from.angvel(), true);
                    }
                    (e.mark, e.motion) = (old.mark, old.motion);
                }
            }
            *self = fresh;
            return;
        }
        for (e, o) in self.entries.iter_mut().zip(scene.objects.iter().filter(|o| o.visible)) {
            let pose = (o.pos.truncate(), o.angle);
            if pose != e.pose {
                let body = &mut self.physics.bodies[e.handle];
                body.set_position(Pose::new(vector(pose.0), pose.1 as f32), true);
                if body.is_dynamic() {
                    body.set_linvel(Vector::ZERO, true);
                    body.set_angvel(0.0, true);
                }
                (e.pose, e.motion) = (pose, Motion::default());
            }
        }
    }

    /// Avance d'un sous-pas `DT` sous les efforts `loads` (les objets absents de la liste ne
    /// subissent que la gravité, les contacts et leurs liaisons).
    pub fn step(&mut self, scene: &Scene, loads: &[Load]) {
        let top = self.view == MechView::Top;
        for e in &mut self.entries {
            let body = &mut self.physics.bodies[e.handle];
            if !body.is_dynamic() {
                continue;
            }
            let Some(o) = scene.get(e.id) else { continue };
            let load = loads.iter().find(|l| l.id == e.id);
            let (pos, angle) = (dvec(body.translation()), body.rotation().angle() as f64);
            let center = pos + DVec2::from_angle(angle).rotate(e.com);
            let (v, spin) = (dvec(body.linvel()), body.angvel() as f64);
            let mut force = load.map_or(DVec2::ZERO, |l| l.force);
            // Le couple est donné autour du centre de l'objet ; Rapier l'attend au centre de gravité.
            let mut torque = load.map_or(0.0, |l| l.torque) - (center - pos).perp_dot(force);
            if let Link::Spring { anchor, stiffness, damping, length } = o.body.link {
                let d = anchor - center;
                let dir = d.normalize_or_zero();
                force += dir * (stiffness * (d.length() - length) - damping * v.dot(dir));
            }
            let mut sliding = v.length() > SLIP_SPEED;
            if top {
                // Table : réaction normale m·g, frottement statique tant que l'objet est immobile.
                let (mu_s, mu_k, g) = (o.body.mu_s, o.body.mu_k, self.gravity);
                let brake = |speed: f64, rest: f64| if speed > rest { mu_k } else { mu_s } * g;
                match (o.body.link, e.pivot) {
                    (Link::Pivot { .. }, Some(pivot)) => {
                        // Seule la rotation autour du pivot est libre.
                        let arm = center - pivot;
                        let inertia = e.inertia + e.mass * arm.length_squared();
                        let free = spin + (torque + arm.perp_dot(force)) / inertia * DT;
                        let cap = e.mass * e.grip / inertia;
                        sliding = free.abs() > brake(spin.abs(), REST_SPIN) * cap * DT;
                        torque += if sliding { -mu_k * g * cap * inertia * free.signum() } else { -inertia * free / DT };
                    }
                    (Link::Slider { angle }, _) => {
                        let axis = DVec2::from_angle(angle);
                        let free = v.dot(axis) + force.dot(axis) / e.mass * DT;
                        sliding = free.abs() > brake(v.dot(axis).abs(), REST_SPEED) * DT;
                        force += axis * if sliding { -mu_k * g * e.mass * free.signum() } else { -e.mass * free / DT };
                    }
                    _ => {
                        let free = v + force / e.mass * DT;
                        sliding = free.length() > brake(v.length(), REST_SPEED) * DT;
                        force += if sliding { -free.normalize() * (mu_k * g * e.mass) } else { -free * (e.mass / DT) };
                        let turn = spin + torque / e.inertia * DT;
                        let cap = e.mass * e.grip / e.inertia;
                        let turning = turn.abs() > brake(spin.abs(), REST_SPIN) * cap * DT;
                        torque += if turning { -mu_k * g * e.mass * e.grip * turn.signum() } else { -e.inertia * turn / DT };
                        sliding |= turning;
                    }
                }
            } else {
                // Vue de côté : Rapier n'a qu'un coefficient de Coulomb par contact, basculé ici
                // de μs à μk dès que l'objet glisse.
                let mu = if sliding { o.body.mu_k } else { o.body.mu_s } as f32;
                for &c in body.colliders() {
                    self.physics.colliders[c].set_friction(mu);
                }
            }
            e.motion.sliding = sliding;
            body.reset_forces(true);
            body.reset_torques(true);
            body.add_force(vector(force), true);
            body.add_torque(torque as f32, true);
        }
        self.physics.step();
        self.time += DT;
        for e in &mut self.entries {
            let body = &self.physics.bodies[e.handle];
            if body.is_dynamic() {
                (e.motion.velocity, e.motion.spin) = (dvec(body.linvel()), body.angvel() as f64);
                // Un objet retenu par un obstacle ne glisse pas, quelle que soit la force.
                e.motion.sliding &= e.motion.velocity.length() > REST_SPEED || e.motion.spin.abs() > REST_SPIN;
            }
        }
    }

    /// Reporte dans la scène la pose des objets mobiles.
    pub fn write(&mut self, scene: &mut Scene) {
        for e in &mut self.entries {
            let body = &self.physics.bodies[e.handle];
            if !body.is_dynamic() {
                continue;
            }
            let Some(o) = scene.get_mut(e.id) else { continue };
            // L'angle de la scène reste continu d'un tour à l'autre.
            let angle = o.angle + wrap(body.rotation().angle() as f64 - o.angle);
            let pos = dvec(body.translation());
            // Un objet appuyé sur un autre frémit de quelques nanomètres : la scène l'ignore, ce
            // qui évite de recalculer le champ pour rien.
            if (pos - e.pose.0).length() + e.radius * (angle - e.pose.1).abs() < STILL {
                continue;
            }
            (o.pos.x, o.pos.y, o.angle) = (pos.x, pos.y, angle);
            e.pose = (pos, angle);
        }
    }

    /// Plus grand déplacement d'un objet mobile depuis le dernier `mark` (m), rotation comprise.
    pub fn drift(&self) -> f64 {
        let moved = |e: &Entry| {
            let body = &self.physics.bodies[e.handle];
            (dvec(body.translation()) - e.mark.0).length() + e.radius * wrap(body.rotation().angle() as f64 - e.mark.1).abs()
        };
        self.entries.iter().filter(|e| self.physics.bodies[e.handle].is_dynamic()).map(moved).fold(0.0, f64::max)
    }

    /// Note les poses actuelles : le champ vient d'être recalculé pour elles.
    pub fn mark(&mut self) {
        for e in &mut self.entries {
            let body = &self.physics.bodies[e.handle];
            e.mark = (dvec(body.translation()), body.rotation().angle() as f64);
        }
    }

    /// Mouvement d'un objet mobile ; `None` s'il est fixe ou absent.
    pub fn motion(&self, id: u32) -> Option<Motion> {
        self.entries.iter().find(|e| e.id == id && self.physics.bodies[e.handle].is_dynamic()).map(|e| e.motion)
    }

    /// Un objet au moins est-il en mouvement ?
    pub fn moving(&self) -> bool {
        self.entries.iter().any(|e| e.motion.velocity.length() > REST_SPEED || e.motion.spin.abs() > REST_SPIN)
    }
}

/// Un objet de la scène peut-il bouger ?
pub fn is_mobile(o: &Object) -> bool {
    o.visible && o.body.mobile
}
