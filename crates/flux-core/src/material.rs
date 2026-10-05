//! Bibliothèque de matériaux (valeurs par défaut de la section 6 du document).

use crate::ABSOLUTE_ZERO_C;
use serde::{Deserialize, Serialize};

/// Perméabilité relative d'un supraconducteur dans l'état Meissner (χ ≈ −1) :
/// une très forte réluctivité qui annule B à l'intérieur.
pub const MU_R_MEISSNER: f64 = 1e-4;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum MagClass {
    Magnet,
    Ferro,
    Para,
    Dia,
    Conductor,
    Superconductor,
}

impl MagClass {
    pub const ALL: [MagClass; 6] =
        [MagClass::Magnet, MagClass::Ferro, MagClass::Para, MagClass::Dia, MagClass::Conductor, MagClass::Superconductor];

    pub fn label(self) -> &'static str {
        match self {
            MagClass::Magnet => "Aimants",
            MagClass::Ferro => "Ferro",
            MagClass::Para => "Para",
            MagClass::Dia => "Dia",
            MagClass::Conductor => "Conducteurs",
            MagClass::Superconductor => "Supra",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Material {
    pub name: String,
    pub class: MagClass,
    /// Perméabilité relative (μrec pour un aimant, μr linéaire pour un ferro).
    pub mu_r: f64,
    /// Susceptibilité volumique SI des para/diamagnétiques (traitée en perturbation).
    #[serde(default)]
    pub chi: f64,
    /// Rémanence à 20 °C (T).
    #[serde(default)]
    pub br: f64,
    /// Coefficient de température de Br (%/K).
    #[serde(default)]
    pub alpha_br: f64,
    /// Température de Curie (°C), 0 si sans objet.
    #[serde(default)]
    pub t_curie: f64,
    /// Température critique d'un supraconducteur (°C).
    #[serde(default)]
    pub t_critical: f64,
    /// Masse volumique (kg/m³).
    pub density: f64,
}

impl Material {
    /// Rémanence à la température `t` (°C) : Br(20)·[1 + α(T − 20)], nulle au-delà de Tc.
    pub fn br_at(&self, t: f64) -> f64 {
        if self.t_curie > 0.0 && t >= self.t_curie {
            return 0.0;
        }
        (self.br * (1.0 + self.alpha_br / 100.0 * (t - 20.0))).max(0.0)
    }

    /// Perméabilité relative vue par le solveur. Les para/dia y valent 1 :
    /// en f32, μr = 1,00002 se noierait dans le résidu (section 2.4).
    pub fn mu_r_solver(&self, t: f64) -> f64 {
        match self.class {
            MagClass::Magnet => self.mu_r,
            MagClass::Ferro if self.t_curie > 0.0 && t >= self.t_curie => 1.0,
            MagClass::Ferro => self.mu_r,
            MagClass::Superconductor if self.is_superconducting(t) => MU_R_MEISSNER,
            _ => 1.0,
        }
    }

    /// Sous sa température critique, un supraconducteur expulse le champ (effet Meissner).
    pub fn is_superconducting(&self, t: f64) -> bool {
        self.class == MagClass::Superconductor && t < self.t_critical
    }

    /// Susceptibilité à la température `t` (°C) : loi de Curie χ·T0/T pour les
    /// paramagnétiques, constante pour les diamagnétiques.
    pub fn chi_at(&self, t: f64) -> f64 {
        match self.class {
            MagClass::Para => self.chi * 293.15 / (t - ABSOLUTE_ZERO_C).max(1.0),
            _ => self.chi,
        }
    }
}

fn magnet(name: &str, br: f64, alpha: f64, tc: f64, rho: f64, mu_rec: f64) -> Material {
    Material {
        name: name.into(),
        class: MagClass::Magnet,
        mu_r: mu_rec,
        chi: 0.0,
        br,
        alpha_br: alpha,
        t_curie: tc,
        t_critical: 0.0,
        density: rho * 1000.0,
    }
}

fn ferro(name: &str, mu_r: f64, tc: f64, rho: f64) -> Material {
    Material {
        name: name.into(),
        class: MagClass::Ferro,
        mu_r,
        chi: 0.0,
        br: 0.0,
        alpha_br: 0.0,
        t_curie: tc,
        t_critical: 0.0,
        density: rho * 1000.0,
    }
}

fn weak(name: &str, class: MagClass, chi: f64, rho: f64) -> Material {
    Material {
        name: name.into(),
        class,
        mu_r: 1.0 + chi,
        chi,
        br: 0.0,
        alpha_br: 0.0,
        t_curie: 0.0,
        t_critical: 0.0,
        density: rho * 1000.0,
    }
}

/// Supraconducteur de température critique `tc_kelvin`.
fn superconductor(name: &str, tc_kelvin: f64, rho: f64) -> Material {
    Material {
        name: name.into(),
        class: MagClass::Superconductor,
        mu_r: MU_R_MEISSNER,
        chi: -1.0,
        br: 0.0,
        alpha_br: 0.0,
        t_curie: 0.0,
        t_critical: tc_kelvin + ABSOLUTE_ZERO_C,
        density: rho * 1000.0,
    }
}

/// Bibliothèque par défaut.
pub fn library() -> Vec<Material> {
    use MagClass::*;
    vec![
        magnet("NdFeB N35", 1.19, -0.12, 310.0, 7.5, 1.05),
        magnet("NdFeB N42", 1.30, -0.12, 310.0, 7.5, 1.05),
        magnet("NdFeB N52", 1.45, -0.12, 310.0, 7.5, 1.05),
        magnet("NdFeB N42SH", 1.30, -0.12, 340.0, 7.5, 1.05),
        magnet("SmCo 2:17", 1.07, -0.035, 800.0, 8.4, 1.05),
        magnet("Ferrite Sr (Y30)", 0.385, -0.20, 450.0, 4.9, 1.1),
        magnet("AlNiCo 5", 1.26, -0.02, 860.0, 7.3, 3.5),
        ferro("Fer pur (Armco)", 5000.0, 770.0, 7.87),
        ferro("Acier doux (S235)", 1500.0, 770.0, 7.85),
        ferro("Acier électrique Fe-3%Si", 7000.0, 740.0, 7.65),
        ferro("Mu-métal", 80000.0, 400.0, 8.7),
        ferro("Permalloy 50% Ni", 50000.0, 480.0, 8.2),
        ferro("Fer-cobalt (Permendur)", 10000.0, 940.0, 8.1),
        ferro("Ferrite douce MnZn", 5000.0, 200.0, 4.8),
        ferro("Nickel", 300.0, 354.0, 8.9),
        ferro("Cobalt", 150.0, 1115.0, 8.9),
        ferro("Gadolinium", 50.0, 20.0, 7.9),
        weak("Aluminium", Para, 2.2e-5, 2.70),
        weak("Platine", Para, 2.7e-4, 21.45),
        weak("Titane", Para, 1.8e-4, 4.51),
        weak("Tungstène", Para, 7.8e-5, 19.3),
        weak("Graphite pyrolytique", Dia, -4.5e-4, 2.2),
        weak("Bismuth", Dia, -1.66e-4, 9.78),
        weak("Eau", Dia, -9.0e-6, 1.0),
        weak("Diamant", Dia, -2.2e-5, 3.51),
        weak("Argent", Dia, -2.4e-5, 10.5),
        weak("Or", Dia, -3.4e-5, 19.3),
        weak("Cuivre (bobinage)", Conductor, -9.6e-6, 8.96),
        superconductor("YBCO", 92.0, 6.3),
        superconductor("BSCCO-2223", 110.0, 6.5),
        superconductor("MgB2", 39.0, 2.57),
        superconductor("Niobium", 9.25, 8.57),
        superconductor("Plomb", 7.2, 11.34),
    ]
}
