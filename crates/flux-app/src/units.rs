//! Unités d'affichage des grandeurs magnétiques : SI (T, N) ou CGS (G, dyn).
//! Le calcul reste en SI ; seule la présentation change.

use crate::lang::decimal;
use serde::{Deserialize, Serialize};
use std::cell::Cell;

#[derive(Clone, Copy, PartialEq, Debug, Default, Serialize, Deserialize)]
pub enum Units {
    #[default]
    Si,
    Cgs,
}

thread_local! {
    static UNITS: Cell<Units> = const { Cell::new(Units::Si) };
}

pub fn set(units: Units) {
    UNITS.with(|u| u.set(units));
}

pub fn get() -> Units {
    UNITS.with(Cell::get)
}

fn cgs() -> bool {
    get() == Units::Cgs
}

/// Valeur et unité préfixée d'une induction : T, mT, µT ou kG, G, mG (1 T = 10⁴ G).
fn scaled_b(tesla: f64) -> (f64, &'static str) {
    // Valeur dans l'unité de base (tesla ou gauss), et taille de la plus grande unité affichée.
    let (v, top, units) = if cgs() { (tesla * 1e4, 1e3, ["kG", "G", "mG"]) } else { (tesla, 1.0, ["T", "mT", "µT"]) };
    match v.abs() {
        a if a >= top => (v / top, units[0]),
        a if a >= top * 1e-3 => (v / top * 1e3, units[1]),
        _ => (v / top * 1e6, units[2]),
    }
}

/// Induction avec préfixe automatique.
pub fn b(tesla: f64) -> String {
    let (v, unit) = scaled_b(tesla);
    format!("{} {unit}", decimal(v, if unit == "T" || unit == "kG" { 3 } else { 2 }))
}

/// Induction sur deux chiffres significatifs, pour la légende.
pub fn b_short(tesla: f64) -> String {
    let (v, unit) = scaled_b(tesla);
    format!("{} {unit}", decimal(v, if v.abs() >= 10.0 { 0 } else { 1 }))
}

/// Induction d'un aimant, sans changement de préfixe : en teslas ou en kilogauss.
pub fn remanence(tesla: f64) -> (f64, &'static str) {
    if cgs() { (tesla * 10.0, "kG") } else { (tesla, "T") }
}

/// Champ coercitif : kA/m, ou kOe en CGS (1 kA/m = 4π·10⁻³ kOe).
pub fn coercivity(amps_per_meter: f64) -> (String, &'static str) {
    if cgs() { (decimal(amps_per_meter * 4.0 * std::f64::consts::PI * 1e-6, 2), "kOe") } else { (decimal(amps_per_meter / 1e3, 0), "kA/m") }
}

/// Nombre à trois décimales, ou en notation scientifique hors de [10⁻², 10⁴[.
pub fn number(v: f64) -> String {
    if v == 0.0 || (1e-2..1e4).contains(&v.abs()) {
        decimal(v, 3)
    } else {
        let text = format!("{v:.2e}");
        if crate::lang::get() == crate::lang::Lang::Fr { text.replace('.', ",") } else { text }
    }
}

/// Force par mètre de profondeur : N/m ou dyn/cm.
pub fn force_per_length(newton_per_meter: f64) -> (String, &'static str) {
    if cgs() { (number(newton_per_meter * 1e3), "dyn/cm") } else { (number(newton_per_meter), "N/m") }
}

/// Force : N ou dyn.
pub fn force(newton: f64) -> (String, &'static str) {
    if cgs() { (number(newton * 1e5), "dyn") } else { (number(newton), "N") }
}

/// Couple par mètre de profondeur : N·m/m ou dyn·cm/cm.
pub fn torque_per_length(newton: f64) -> (String, &'static str) {
    if cgs() { (number(newton * 1e5), "dyn·cm/cm") } else { (number(newton), "N·m/m") }
}

/// Susceptibilité volumique : sans dimension en SI, divisée par 4π en CGS (emu/cm³).
pub fn susceptibility(chi: f64) -> f64 {
    if cgs() { chi / (4.0 * std::f64::consts::PI) } else { chi }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn si_and_cgs() {
        assert_eq!((b(1.3), b(0.07408), b(2.5e-5)), ("1,300 T".into(), "74,08 mT".into(), "25,00 µT".into()));
        assert_eq!((b_short(0.959), b_short(0.0123)), ("959 mT".into(), "12 mT".into()));
        assert_eq!(force_per_length(209.491), ("209,491".into(), "N/m"));
        assert_eq!(number(-9.48e-3), "-9,48e-3");
        set(Units::Cgs);
        // 1,3 T = 13 kG ; 74,08 mT = 740,8 G ; 25 µT = 250 mG.
        assert_eq!((b(1.3), b(0.07408), b(2.5e-5)), ("13,000 kG".into(), "740,80 G".into(), "250,00 mG".into()));
        assert_eq!(b_short(0.959), "9,6 kG");
        assert_eq!(remanence(1.3), (13.0, "kG"));
        assert_eq!(force_per_length(209.491), ("2,09e5".into(), "dyn/cm"));
        assert_eq!(force(2.0e-3), ("200,000".into(), "dyn"));
        assert!((susceptibility(-1.0) + 0.0796).abs() < 1e-4);
    }
}
