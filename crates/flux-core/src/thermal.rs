//! Thermique (section 2.5) : chaque objet est un bloc à température uniforme.
//!
//! m·c·dT/dt = P_Joule − h·S·(T − T_amb) − ε·σ·S·(T⁴ − T_amb⁴)

use crate::ABSOLUTE_ZERO_C;
use crate::scene::{Object, Scene};
use serde::{Deserialize, Serialize};

/// Constante de Stefan–Boltzmann (W/(m²·K⁴)).
pub const STEFAN_BOLTZMANN: f64 = 5.670_374_419e-8;
/// Coefficient d'échange du jet d'un pistolet chauffant ou d'une bombe de froid (W/(m²·K)).
pub const JET_EXCHANGE: f64 = 1500.0;
/// Température du jet d'un pistolet chauffant et d'une bombe de froid (°C).
pub const HEAT_GUN: f64 = 500.0;
pub const COLD_SPRAY: f64 = -50.0;

/// Réglages thermiques de la scène.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(default)]
pub struct Thermal {
    /// Le bilan thermique fait évoluer la température des objets pendant la simulation.
    pub enabled: bool,
    /// Coefficient de convection avec l'air ambiant (W/(m²·K)).
    pub convection: f64,
    /// Accélération du temps thermique par rapport au temps mécanique.
    pub speed: f64,
}

impl Default for Thermal {
    fn default() -> Self {
        Thermal { enabled: true, convection: 10.0, speed: 1.0 }
    }
}

impl Scene {
    /// Surface d'échange d'un objet avec l'air (m²) : son pourtour sur la profondeur, plus ses
    /// deux faces.
    pub fn exchange_area(&self, o: &Object) -> f64 {
        o.shape.perimeter() * self.depth + 2.0 * o.shape.area()
    }

    /// Capacité thermique d'un objet (J/K).
    pub fn heat_capacity(&self, o: &Object) -> f64 {
        self.mass(o) * self.material(&o.material).map_or(0.0, |m| m.heat_capacity)
    }

    /// Résistance (Ω) d'un bobinage : N spires en série, longues de la profondeur de la scène,
    /// dont le fil a pour section la part `fill` de celle de l'objet divisée par N. Une bobine
    /// compte ses deux sections : le fil y est deux fois plus long, dans une section moitié
    /// moindre. `None` si l'objet n'est pas un conducteur bobiné.
    pub fn resistance(&self, o: &Object) -> Option<f64> {
        let rho = self.material(&o.material)?.resistivity_at(o.temperature);
        let (passes, copper) = (o.shape.passes(), o.fill.clamp(0.01, 1.0) * o.shape.area());
        (rho > 0.0 && o.turns > 0.0).then(|| rho * o.turns * o.turns * self.depth * passes * passes / copper)
    }

    /// Puissance dissipée par effet Joule (W).
    pub fn joule_power(&self, o: &Object) -> f64 {
        self.resistance(o).map_or(0.0, |r| r * o.current * o.current)
    }

    /// Puissance cédée à l'air par convection et rayonnement à la température `t` (W), et sa
    /// dérivée par rapport à `t` (W/K).
    fn losses(&self, o: &Object, t: f64) -> (f64, f64) {
        let area = self.exchange_area(o);
        let radiation = self.material(&o.material).map_or(0.0, |m| m.emissivity) * STEFAN_BOLTZMANN * area;
        let (k, ka) = (t - ABSOLUTE_ZERO_C, self.ambient - ABSOLUTE_ZERO_C);
        let h = self.thermal.convection * area;
        (h * (t - self.ambient) + radiation * (k.powi(4) - ka.powi(4)), h + 4.0 * radiation * k.powi(3))
    }

    /// Constante de temps du retour à l'équilibre thermique, à la température actuelle (s).
    pub fn time_constant(&self, o: &Object) -> f64 {
        self.heat_capacity(o) / self.losses(o, o.temperature).1.max(1e-12)
    }

    /// Un objet dont la température suit le bilan thermique : visible et sans thermostat.
    fn is_free(o: &Object) -> bool {
        o.visible && !o.thermostat
    }

    /// Le bilan thermique a-t-il encore quelque chose à faire évoluer ?
    pub fn thermally_active(&self) -> bool {
        self.thermal.enabled
            && self.objects.iter().any(|o| Self::is_free(o) && ((o.temperature - self.ambient).abs() > 0.05 || self.joule_power(o) > 0.0))
    }

    /// Avance le bilan thermique de `dt` secondes ; renvoie la plus grande variation de
    /// température (K). Le schéma est implicite linéarisé : il reste stable quand le pas
    /// dépasse la constante de temps d'un petit objet.
    pub fn thermal_step(&mut self, dt: f64) -> f64 {
        if !self.thermal.enabled {
            return 0.0;
        }
        let mut change: f64 = 0.0;
        for k in 0..self.objects.len() {
            let o = &self.objects[k];
            let capacity = self.heat_capacity(o);
            if !Self::is_free(o) || capacity <= 0.0 {
                continue;
            }
            let (loss, slope) = self.losses(o, o.temperature);
            let delta = dt * (self.joule_power(o) - loss) / (capacity + dt * slope);
            self.objects[k].temperature = (o.temperature + delta).max(ABSOLUTE_ZERO_C);
            change = change.max(delta.abs());
        }
        change
    }

    /// Souffle pendant `dt` secondes un jet à la température `jet` (°C) sur un objet : sa
    /// température s'en rapproche avec la constante de temps m·c / (h_jet·S).
    pub fn blow(&mut self, id: u32, jet: f64, dt: f64) {
        let Some(o) = self.get(id) else { return };
        let rate = JET_EXCHANGE * self.exchange_area(o) / self.heat_capacity(o).max(1e-12);
        let t = o.temperature + (jet - o.temperature) * (1.0 - (-rate * dt).exp());
        if let Some(o) = self.get_mut(id) {
            o.temperature = t;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shape::Shape;
    use glam::DVec2;

    /// Un bloc chaud revient à l'ambiante : au début, à la vitesse fixée par sa constante de temps.
    #[test]
    fn hot_block_cools_to_ambient() {
        let mut s = Scene::default();
        let id = s.add("bloc", Shape::Rect { w: 0.016, h: 0.060 }, DVec2::ZERO, "Fer pur (Armco)");
        s.get_mut(id).unwrap().temperature = 60.0;
        let o = s.get(id).unwrap().clone();
        // 75,6 g de fer : 33,9 J/K ; 3,44·10⁻³ m² d'échange.
        assert!((s.heat_capacity(&o) - 7870.0 * 9.6e-4 * 0.01 * 449.0).abs() < 1e-9);
        assert!((s.exchange_area(&o) - (0.152 * 0.01 + 2.0 * 9.6e-4)).abs() < 1e-12);
        let tau = s.time_constant(&o);
        assert!(tau > 500.0 && tau < 1000.0, "τ = {tau} s");
        // Après un dixième de la constante de temps, l'écart a perdu environ 1 − e^(−0,1).
        for _ in 0..100 {
            s.thermal_step(tau / 1000.0);
        }
        let left = (s.get(id).unwrap().temperature - 20.0) / 40.0;
        assert!((left - (-0.1f64).exp()).abs() < 0.01, "{left}");
        // Un pas de dix constantes de temps reste stable et ne dépasse pas l'ambiante.
        for _ in 0..20 {
            s.thermal_step(10.0 * tau);
        }
        let t = s.get(id).unwrap().temperature;
        assert!((20.0..20.5).contains(&t), "{t}");
        assert!(!s.thermally_active());
    }

    /// Une bobine parcourue par un courant s'échauffe jusqu'à l'équilibre P = pertes, et sa
    /// résistance suit ρ(T).
    #[test]
    fn coil_heats_until_losses_balance_joule_power() {
        let mut s = Scene::default();
        let id = s.add("bobine", Shape::Rect { w: 0.004, h: 0.030 }, DVec2::ZERO, "Cuivre (bobinage)");
        let o = s.get_mut(id).unwrap();
        (o.turns, o.current, o.fill) = (100.0, 3.0, 0.6);
        let o = s.get(id).unwrap().clone();
        // R = ρ·N²·profondeur / (remplissage·S) = 1,68·10⁻⁸ × 10⁴ × 0,01 / (0,6 × 1,2·10⁻⁴).
        let r20 = 1.68e-8 * 1e4 * 0.01 / (0.6 * 1.2e-4);
        assert!((s.resistance(&o).unwrap() / r20 - 1.0).abs() < 1e-12);
        assert!((s.joule_power(&o) / (9.0 * r20) - 1.0).abs() < 1e-12);
        assert!(s.thermally_active());
        for _ in 0..4000 {
            s.thermal_step(5.0);
        }
        let o = s.get(id).unwrap().clone();
        let (loss, _) = s.losses(&o, o.temperature);
        assert!(o.temperature > 30.0 && o.temperature < 150.0, "{}", o.temperature);
        assert!((s.joule_power(&o) / loss - 1.0).abs() < 1e-6);
        assert!((s.resistance(&o).unwrap() / r20 - (1.0 + 0.00393 * (o.temperature - 20.0))).abs() < 1e-9);

        // Une bobine dont chaque section a cette forme résiste deux fois plus : le fil fait
        // l'aller et le retour.
        let mut coil = Scene::default();
        let pair = coil.add("bobine", Shape::Coil { w: 0.02, h: 0.030, thick: 0.004 }, DVec2::ZERO, "Cuivre (bobinage)");
        let o = coil.get_mut(pair).unwrap();
        (o.turns, o.current, o.fill) = (100.0, 3.0, 0.6);
        assert!((coil.resistance(&coil.objects[0]).unwrap() / (2.0 * r20) - 1.0).abs() < 1e-12);

        // Avec un thermostat, ou le bilan coupé, la température ne bouge plus.
        s.get_mut(id).unwrap().thermostat = true;
        assert_eq!(s.thermal_step(100.0), 0.0);
        s.get_mut(id).unwrap().thermostat = false;
        s.thermal.enabled = false;
        assert_eq!(s.thermal_step(100.0), 0.0);
    }

    /// Le pistolet chauffant amène vite un petit objet vers 500 °C, la bombe de froid vers −50 °C.
    #[test]
    fn jets_heat_and_cool() {
        let mut s = Scene::default();
        let id = s.add("bille", Shape::Circle { r: 0.005 }, DVec2::ZERO, "Nickel");
        for _ in 0..600 {
            s.blow(id, HEAT_GUN, 1.0 / 60.0);
        }
        let hot = s.get(id).unwrap().temperature;
        assert!(hot > 354.0 && hot < HEAT_GUN, "après 10 s de pistolet : {hot} °C");
        for _ in 0..3600 {
            s.blow(id, COLD_SPRAY, 1.0 / 60.0);
        }
        assert!((s.get(id).unwrap().temperature - COLD_SPRAY).abs() < 1.0);
    }
}
