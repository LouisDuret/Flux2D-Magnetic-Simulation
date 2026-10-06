//! Simulation mécanique : lecture, pause, pas à pas, et couplage avec le calcul du champ
//! (section 3.8).

use super::App;
use crate::lang::tr;
use eframe::egui;
use flux_core::DVec2;
use flux_core::raster::rasterize;
use flux_core::scene::Scene;
use flux_mech::{DT, Load, World, is_mobile};
use flux_solver::{Newton, forces};
use std::time::{Duration, Instant};

/// Temps de calcul du champ accordé à une image pendant la lecture, au-delà de celui de
/// l'affichage : quand il est épuisé, la simulation ralentit plutôt que d'avancer avec des
/// forces périmées.
const FIELD_BUDGET: Duration = Duration::from_millis(14);

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
}

impl Default for Sim {
    fn default() -> Self {
        Sim { playing: false, speed: 1.0, rate: 0.0, world: None, backlog: 0.0, step_once: false, origin: None }
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

    fn can_simulate(&mut self) -> bool {
        let mobile = self.scene.objects.iter().any(is_mobile);
        if !mobile {
            self.message = tr("Aucun objet mobile : rendez un objet « Mobile » dans l’inspecteur.").into();
        }
        mobile
    }

    /// Replace les objets là où ils étaient au lancement de la simulation.
    pub(super) fn rewind(&mut self) {
        let Some(origin) = self.sim.origin.take() else { return };
        for o in &mut self.scene.objects {
            if let Some(start) = origin.get(o.id) {
                (o.pos, o.angle) = (start.pos, start.angle);
            }
        }
        self.sim.reset();
        self.dirty = true;
    }

    fn loads(&self) -> Vec<Load> {
        let depth = self.scene.depth;
        self.wrenches.iter().map(|w| Load { id: w.id, force: w.force.truncate() * depth, torque: w.torque * depth }).collect()
    }

    /// Recalcule tout de suite le champ et les forces pour la scène actuelle.
    fn solve_now(&mut self, budget: Duration) {
        let mut newton = Newton::start(self.solver.as_mut(), rasterize(&self.scene, self.solved_n));
        self.status = newton.advance(self.solver.as_mut(), budget);
        self.wrenches = forces(self.solver.field(), &self.scene);
        self.newton = Some(newton);
        self.field_version += 1;
    }

    /// Avance la mécanique du temps écoulé depuis la dernière image. Le champ est recalculé
    /// dès qu'un objet s'est déplacé d'un quart de cellule : sans cela, un objet qui accélère
    /// vers un aimant gagnerait une énergie fictive.
    pub(super) fn simulate(&mut self, ctx: &egui::Context) {
        let once = std::mem::take(&mut self.sim.step_once);
        if !(self.sim.playing || once) {
            self.sim.rate = 0.0;
            return;
        }
        if !self.scene.objects.iter().any(is_mobile) {
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
        let mut loads = self.loads();
        let quarter_cell = self.scene.size / self.solved_n.max(1) as f64 / 4.0;
        let (start, mut steps) = (Instant::now(), 0);
        while self.sim.backlog >= DT {
            if world.drift() > quarter_cell {
                if start.elapsed() > FIELD_BUDGET {
                    // Le calcul du champ ne suit plus : le temps simulé en retard est abandonné.
                    self.sim.backlog = 0.0;
                    break;
                }
                world.write(&mut self.scene);
                self.solve_now(FIELD_BUDGET);
                loads = self.loads();
                world.mark();
            }
            world.step(&self.scene, &loads);
            self.sim.backlog -= DT;
            steps += 1;
        }
        world.write(&mut self.scene);
        self.sim.rate = steps as f64 * DT / elapsed.max(1e-6);
        self.sim.world = Some(world);
        if self.scene.objects.iter().map(|o| (o.pos.truncate(), o.angle)).ne(before) {
            self.dirty = true;
        }
        if self.sim.playing {
            ctx.request_repaint();
        }
    }
}
