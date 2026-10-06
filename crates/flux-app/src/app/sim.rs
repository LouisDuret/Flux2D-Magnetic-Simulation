//! Simulation mécanique et thermique : lecture, pause, pas à pas, et couplage avec le calcul
//! du champ (section 3.8).

use super::App;
use crate::lang::tr;
use eframe::egui;
use flux_core::DVec2;
use flux_core::raster::rasterize;
use flux_core::scene::Scene;
use flux_core::shape::Sdf;
use flux_mech::{DT, Load, World, is_mobile};
use flux_solver::{DEMAG_TOLERANCE, Newton, demagnetize, forces, forces_on};
use std::time::{Duration, Instant};

/// Temps de calcul du champ accordé à une image pendant la lecture, au-delà de celui de
/// l'affichage : quand il est épuisé, la simulation ralentit plutôt que d'avancer avec des
/// forces périmées.
const FIELD_BUDGET: Duration = Duration::from_millis(14);
/// Sous-pas mécaniques entre deux mises à jour thermiques : la thermique, plus lente, avance
/// à 10 Hz environ.
const THERMAL_EVERY: u32 = 24;
/// Écart de température (K) depuis le dernier calcul du champ qui impose de le refaire.
const THERMAL_DRIFT: f64 = 0.25;

/// Pose d'un objet : identifiant, position, angle et rayon englobant.
type Pose = (u32, DVec2, f64, f64);

/// Efforts relevés à un calcul du champ, avec la pose des objets à ce moment.
struct Sample {
    poses: Vec<Pose>,
    loads: Vec<Load>,
}

impl Sample {
    fn pose(&self, id: u32) -> Option<(DVec2, f64)> {
        self.poses.iter().find(|p| p.0 == id).map(|p| (p.1, p.2))
    }

    /// Plus grand déplacement d'un objet entre deux relevés (m), rotation comprise.
    fn distance(&self, other: &Sample) -> f64 {
        if self.poses.len() != other.poses.len() {
            return f64::INFINITY;
        }
        let moved = |(a, b): (&Pose, &Pose)| if a.0 == b.0 { a.1.distance(b.1) + a.3 * (a.2 - b.2).abs() } else { f64::INFINITY };
        self.poses.iter().zip(&other.poses).map(moved).fold(0.0, f64::max)
    }
}

pub(super) struct Sim {
    pub playing: bool,
    /// Rapport voulu entre temps simulé et temps réel.
    pub speed: f64,
    /// Rapport obtenu à la dernière image.
    pub rate: f64,
    world: Option<World>,
    /// Temps restant à simuler (s).
    backlog: f64,
    /// Avancer d'un seul sous-pas à la prochaine image.
    step_once: bool,
    /// Scène au lancement, pour y revenir.
    origin: Option<Scene>,
    /// Sous-pas mécaniques depuis la dernière mise à jour thermique.
    since_thermal: u32,
    /// Températures des objets au dernier calcul du champ.
    temperatures: Vec<f64>,
    /// Les deux derniers relevés d'efforts : entre deux calculs du champ, la force suit la
    /// droite qui les joint.
    previous: Option<Sample>,
    last: Option<Sample>,
}

impl Default for Sim {
    fn default() -> Self {
        Sim {
            playing: false,
            speed: 1.0,
            rate: 0.0,
            world: None,
            backlog: 0.0,
            step_once: false,
            origin: None,
            since_thermal: 0,
            temperatures: Vec::new(),
            previous: None,
            last: None,
        }
    }
}

impl Sim {
    /// Temps simulé depuis le lancement (s).
    pub fn time(&self) -> f64 {
        self.world.as_ref().map_or(0.0, |w| w.time)
    }

    /// La simulation a déjà avancé : on peut revenir à l'état initial.
    pub fn started(&self) -> bool {
        self.origin.is_some()
    }

    pub fn world(&self) -> Option<&World> {
        self.world.as_ref()
    }

    /// Efforts à appliquer dans l'état actuel du monde. Tenir la force constante entre deux
    /// calculs du champ la mettrait en retard sur le mouvement : un objet qui oscille dans un
    /// puits magnétique y gagnerait de l'énergie à chaque aller-retour. Elle est donc prolongée
    /// le long du déplacement, sur la droite qui joint les deux derniers relevés. `reach` est
    /// l'écart (m) au-delà duquel deux relevés ne disent plus rien de la pente.
    fn loads_at(&self, world: &World, reach: f64) -> Vec<Load> {
        let Some(last) = &self.last else { return Vec::new() };
        let turn = std::f64::consts::TAU;
        let wrap = |a: f64| a - turn * (a / turn).round();
        let extend = |load: &Load| -> Option<Load> {
            let previous = self.previous.as_ref()?;
            let old = previous.loads.iter().find(|l| l.id == load.id)?;
            let ((p0, a0), (p1, a1), (p, a, radius)) = (previous.pose(load.id)?, last.pose(load.id)?, world.pose(load.id)?);
            let step = (p1 - p0).extend(radius * wrap(a1 - a0));
            let moved = (p - p1).extend(radius * wrap(a - a1));
            let span = step.length_squared();
            if span < 1e-20 || span > reach * reach || moved.length_squared() > reach * reach {
                return None;
            }
            let t = (moved.dot(step) / span).clamp(-40.0, 40.0);
            Some(Load {
                id: load.id,
                force: load.force + (load.force - old.force) * t,
                torque: load.torque + (load.torque - old.torque) * t,
            })
        };
        last.loads.iter().map(|load| extend(load).unwrap_or(*load)).collect()
    }

    /// Oublie le monde mécanique (nouvelle scène, annulation).
    pub fn reset(&mut self) {
        *self = Sim { speed: self.speed, ..Sim::default() };
    }
}

impl App {
    /// Lance la simulation ou la met en pause.
    pub(super) fn toggle_play(&mut self) {
        if !self.sim.playing && !self.can_simulate() {
            return;
        }
        self.sim.playing = !self.sim.playing;
    }

    /// Avance d'un sous-pas, simulation en pause.
    pub(super) fn step_simulation(&mut self) {
        if self.can_simulate() {
            (self.sim.playing, self.sim.step_once) = (false, true);
        }
    }

    /// Y a-t-il quelque chose à simuler : un objet mobile, ou un écart de température à résorber ?
    fn has_dynamics(&self) -> bool {
        self.scene.objects.iter().any(is_mobile) || self.scene.thermally_active()
    }

    fn can_simulate(&mut self) -> bool {
        let active = self.has_dynamics();
        if !active {
            self.message = tr("Rien à simuler : rendez un objet « Mobile » dans l’inspecteur, ou chauffez-en un.").into();
        }
        active
    }

    /// Remet les objets dans l'état où ils étaient au lancement de la simulation : position,
    /// température et aimantation.
    pub(super) fn rewind(&mut self) {
        let Some(origin) = self.sim.origin.take() else { return };
        for o in &mut self.scene.objects {
            if let Some(start) = origin.get(o.id) {
                (o.pos, o.angle, o.temperature) = (start.pos, start.angle, start.temperature);
                o.demag.clone_from(&start.demag);
            }
        }
        self.sim.reset();
        self.dirty = true;
    }

    fn loads(&self) -> Vec<Load> {
        let depth = self.scene.depth;
        self.wrenches.iter().map(|w| Load { id: w.id, force: w.force.truncate() * depth, torque: w.torque * depth }).collect()
    }

    /// Note les efforts du dernier calcul du champ et la pose des objets à laquelle ils valent.
    /// Deux relevés distants de moins de `min_span` (m) ne donneraient qu'une pente bruitée :
    /// le nouveau remplace alors le dernier au lieu de s'y ajouter.
    fn record_sample(&mut self, min_span: f64) {
        let poses = self.scene.objects.iter().map(|o| (o.id, o.pos.truncate(), o.angle, o.shape.bounding_radius())).collect();
        let sample = Sample { poses, loads: self.loads() };
        match &mut self.sim.last {
            Some(last) if last.distance(&sample) < min_span => *last = sample,
            last => self.sim.previous = last.replace(sample),
        }
    }

    /// Le champ dépend-il de la position des objets mobiles ? Un para ou un diamagnétique n'y
    /// change rien : quand lui seul bouge, il suffit de recalculer les forces.
    fn movers_shape_the_field(&self) -> bool {
        let weak = |o: &flux_core::scene::Object| self.scene.material(&o.material).is_some_and(|m| m.weak_chi(o.temperature).is_some());
        self.scene.objects.iter().any(|o| is_mobile(o) && !weak(o))
    }

    /// Recalcule les forces subies par les objets mobiles, le champ restant le même.
    fn refresh_forces(&mut self) {
        let mobile: Vec<u32> = self.scene.objects.iter().filter(|o| is_mobile(o)).map(|o| o.id).collect();
        for wrench in forces_on(self.solver.field(), &self.scene, Some(&mobile)) {
            match self.wrenches.iter_mut().find(|w| w.id == wrench.id) {
                Some(known) => *known = wrench,
                None => self.wrenches.push(wrench),
            }
        }
    }

    /// Recalcule tout de suite le champ et les forces pour la scène actuelle.
    fn solve_now(&mut self, budget: Duration) {
        let mut newton = Newton::start(self.solver.as_mut(), rasterize(&self.scene, self.solved_n));
        self.status = newton.advance(self.solver.as_mut(), budget);
        // Un aimant poussé au-delà de son coude pendant le mouvement se désaimante pour de bon.
        if self.status.converged && demagnetize(self.solver.field(), &mut self.scene) >= DEMAG_TOLERANCE {
            self.dirty = true;
        }
        self.wrenches = forces(self.solver.field(), &self.scene);
        self.newton = Some(newton);
        self.field_version += 1;
    }

    /// Avance la mécanique et la thermique du temps écoulé depuis la dernière image. Le champ est recalculé
    /// dès qu'un objet s'est déplacé d'un quart de cellule : sans cela, un objet qui accélère
    /// vers un aimant gagnerait une énergie fictive.
    pub(super) fn simulate(&mut self, ctx: &egui::Context) {
        let once = std::mem::take(&mut self.sim.step_once);
        if !(self.sim.playing || once) {
            self.sim.rate = 0.0;
            return;
        }
        if !self.has_dynamics() {
            self.sim.playing = false;
            return;
        }
        if self.sim.origin.is_none() {
            self.sim.origin = Some(self.scene.clone());
            // Annuler ramène avant la simulation.
            self.undo.push(self.scene.clone());
            self.redo.clear();
        }
        let elapsed = ctx.input(|i| i.stable_dt as f64).min(1.0 / 30.0);
        self.sim.backlog = if once { DT } else { (self.sim.backlog + elapsed * self.sim.speed).min(0.1) };
        let mut world = self.sim.world.take().unwrap_or_else(|| World::new(&self.scene));
        world.sync(&self.scene);
        world.mark();
        let before: Vec<(DVec2, f64)> = self.scene.objects.iter().map(|o| (o.pos.truncate(), o.angle)).collect();
        let quarter_cell = self.scene.size / self.solved_n.max(1) as f64 / 4.0;
        let shaping = self.movers_shape_the_field();
        if !shaping {
            self.refresh_forces();
        }
        self.record_sample(0.05 * quarter_cell);
        let (start, mut steps) = (Instant::now(), 0);
        while self.sim.backlog >= DT {
            if world.drift() > quarter_cell {
                if start.elapsed() > FIELD_BUDGET {
                    // Le calcul du champ ne suit plus : le temps simulé en retard est abandonné.
                    self.sim.backlog = 0.0;
                    break;
                }
                world.write(&mut self.scene);
                if shaping {
                    self.solve_now(FIELD_BUDGET);
                } else {
                    self.refresh_forces();
                }
                self.record_sample(0.05 * quarter_cell);
                world.mark();
            }
            world.step(&self.scene, &self.sim.loads_at(&world, 16.0 * quarter_cell));
            self.sim.backlog -= DT;
            steps += 1;
            self.sim.since_thermal += 1;
            if self.sim.since_thermal >= THERMAL_EVERY {
                self.scene.thermal_step(self.sim.since_thermal as f64 * DT * self.scene.thermal.speed);
                self.sim.since_thermal = 0;
            }
        }
        world.write(&mut self.scene);
        // Les propriétés magnétiques suivent la température : au-delà d'un quart de degré
        // d'écart, le champ est recalculé.
        let temperatures: Vec<f64> = self.scene.objects.iter().map(|o| o.temperature).collect();
        let drifted = temperatures.len() != self.sim.temperatures.len()
            || temperatures.iter().zip(&self.sim.temperatures).any(|(now, then)| (now - then).abs() > THERMAL_DRIFT);
        if drifted {
            (self.sim.temperatures, self.dirty) = (temperatures, true);
        }
        self.sim.rate = steps as f64 * DT / elapsed.max(1e-6);
        self.sim.world = Some(world);
        if shaping && self.scene.objects.iter().map(|o| (o.pos.truncate(), o.angle)).ne(before) {
            self.dirty = true;
        }
        if self.sim.playing {
            ctx.request_repaint();
        }
    }
}
