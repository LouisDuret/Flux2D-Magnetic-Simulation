//! Bibliothèque de matériaux (valeurs par défaut de la section 6 du document).

use crate::{ABSOLUTE_ZERO_C, MU0};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

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

/// Courbe d'aimantation B(H) d'un ferromagnétique doux (section 2.4) : points (H en A/m,
/// B en T) strictement croissants, origine exclue. Elle est interpolée par une spline cubique
/// monotone (Fritsch–Carlson) et prolongée après le dernier point par B = Bn + μ0·(H − Hn).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct BhCurve {
    pub points: Vec<[f64; 2]>,
}

impl BhCurve {
    /// Courbe modèle calée sur la perméabilité initiale et la polarisation à saturation `js` (T) :
    /// J = Js·x/√(1 + x²), x = (μr − 1)·μ0·H/Js. Elle tient lieu de fiche technique tant que
    /// des points mesurés ne la remplacent pas.
    pub fn model(mu_r: f64, js: f64) -> BhCurve {
        let points = (0..26)
            .map(|i| {
                let x = 0.05 * 1.33f64.powi(i);
                let h = x * js / ((mu_r - 1.0).max(1e-6) * MU0);
                [h, MU0 * h + js * x / (1.0 + x * x).sqrt()]
            })
            .collect();
        BhCurve { points }
    }

    /// Polarisation à saturation μ0·Ms (T), lue sur le prolongement de la courbe.
    pub fn js(&self) -> f64 {
        self.points.last().map_or(0.0, |p| p[1] - MU0 * p[0])
    }

    /// Nœuds de la spline (origine comprise) et pentes de Fritsch–Carlson.
    fn knots(&self) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
        let (mut hs, mut bs) = (vec![0.0], vec![0.0]);
        for p in &self.points {
            // Les points qui ne croissent pas strictement sont ignorés.
            if p[0] > *hs.last().unwrap() && p[1] > *bs.last().unwrap() {
                hs.push(p[0]);
                bs.push(p[1]);
            }
        }
        let n = hs.len();
        if n < 2 {
            return (vec![0.0, 1.0], vec![0.0, MU0], vec![MU0, MU0]);
        }
        let step: Vec<f64> = hs.windows(2).map(|w| w[1] - w[0]).collect();
        let secant: Vec<f64> = (0..n - 1).map(|k| (bs[k + 1] - bs[k]) / step[k]).collect();
        let mut slope = vec![0.0; n];
        for k in 1..n - 1 {
            // Moyenne harmonique pondérée des pentes voisines : la spline reste monotone.
            let (w1, w2) = (2.0 * step[k] + step[k - 1], step[k] + 2.0 * step[k - 1]);
            slope[k] = (w1 + w2) / (w1 / secant[k - 1] + w2 / secant[k]);
        }
        slope[0] = if n > 2 {
            let m = ((2.0 * step[0] + step[1]) * secant[0] - step[0] * secant[1]) / (step[0] + step[1]);
            m.clamp(0.0, 3.0 * secant[0])
        } else {
            secant[0]
        };
        // Raccord de pente avec le prolongement saturé.
        slope[n - 1] = MU0.min(3.0 * secant[n - 2]);
        (hs, bs, slope)
    }

    /// Perméabilité relative initiale (pente à l'origine).
    pub fn mu_r_initial(&self) -> f64 {
        self.knots().2[0] / MU0
    }

    /// Table ν(B) pour le solveur. Les dernières tables calculées sont gardées : la scène est
    /// rastérisée à chaque image pendant un geste, avec les mêmes courbes.
    pub fn table(&self) -> Arc<NuTable> {
        static CACHE: Mutex<Vec<(BhCurve, Arc<NuTable>)>> = Mutex::new(Vec::new());
        let mut cache = CACHE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some((_, table)) = cache.iter().find(|(curve, _)| curve == self) {
            return table.clone();
        }
        let table = Arc::new(self.build_table());
        if cache.len() >= 64 {
            cache.remove(0);
        }
        cache.push((self.clone(), table.clone()));
        table
    }

    fn build_table(&self) -> NuTable {
        const SAMPLES: usize = 1024;
        let (hs, bs, slope) = self.knots();
        let n = hs.len();
        // B et dB/dH en H, par interpolation d'Hermite.
        let eval = |h: f64| -> (f64, f64) {
            if h >= hs[n - 1] {
                return (bs[n - 1] + MU0 * (h - hs[n - 1]), MU0);
            }
            let k = hs.partition_point(|&x| x <= h).clamp(1, n - 1) - 1;
            let d = hs[k + 1] - hs[k];
            let t = (h - hs[k]) / d;
            let (t2, t3) = (t * t, t * t * t);
            let b = (2.0 * t3 - 3.0 * t2 + 1.0) * bs[k]
                + (t3 - 2.0 * t2 + t) * d * slope[k]
                + (-2.0 * t3 + 3.0 * t2) * bs[k + 1]
                + (t3 - t2) * d * slope[k + 1];
            let db = (6.0 * t2 - 6.0 * t) * (bs[k] - bs[k + 1]) / d
                + (3.0 * t2 - 4.0 * t + 1.0) * slope[k]
                + (3.0 * t2 - 2.0 * t) * slope[k + 1];
            (b, db)
        };
        let (h_max, b_max) = (hs[n - 1], bs[n - 1]);
        let (mut nu, mut nu_d) = (Vec::with_capacity(SAMPLES + 1), Vec::with_capacity(SAMPLES + 1));
        for i in 0..=SAMPLES {
            let b = b_max * i as f64 / SAMPLES as f64;
            // H(B) par dichotomie sur la spline, monotone par construction.
            let (mut lo, mut hi) = (0.0, h_max);
            for _ in 0..60 {
                let mid = 0.5 * (lo + hi);
                if eval(mid).0 < b { lo = mid } else { hi = mid }
            }
            let h = 0.5 * (lo + hi);
            let db = eval(h).1.max(MU0);
            nu.push(if i == 0 { MU0 / slope[0].max(MU0) } else { MU0 * h / b });
            nu_d.push(MU0 / db);
        }
        NuTable { b_max, h_max, nu, nu_d }
    }
}

/// Réluctivité relative ν = μ0·H/B et réluctivité différentielle νd = μ0·dH/dB, tabulées à pas
/// constant en B jusqu'au dernier point de la courbe, analytiques au-delà.
#[derive(Clone, Debug, PartialEq)]
pub struct NuTable {
    b_max: f64,
    h_max: f64,
    nu: Vec<f64>,
    nu_d: Vec<f64>,
}

impl NuTable {
    /// (ν, νd) relatifs à l'induction `b` (T).
    pub fn eval(&self, b: f64) -> (f64, f64) {
        if b >= self.b_max {
            return ((MU0 * self.h_max + b - self.b_max) / b, 1.0);
        }
        let x = b.max(0.0) / self.b_max * (self.nu.len() - 1) as f64;
        let i = (x as usize).min(self.nu.len() - 2);
        let t = x - i as f64;
        (self.nu[i] + t * (self.nu[i + 1] - self.nu[i]), self.nu_d[i] + t * (self.nu_d[i + 1] - self.nu_d[i]))
    }

    /// Champ H (A/m) à l'induction `b` (T).
    pub fn h(&self, b: f64) -> f64 {
        self.eval(b).0 * b / MU0
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Material {
    pub name: String,
    pub class: MagClass,
    /// Perméabilité relative (μrec pour un aimant, μr initiale pour un ferro).
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
    /// Courbe B(H) d'un ferromagnétique doux ; sans elle, le matériau est linéaire (μr constant).
    #[serde(default)]
    pub bh: Option<BhCurve>,
}

impl Material {
    /// Rémanence à la température `t` (°C) : Br(20)·[1 + α(T − 20)], nulle au-delà de Tc.
    pub fn br_at(&self, t: f64) -> f64 {
        if self.t_curie > 0.0 && t >= self.t_curie {
            return 0.0;
        }
        (self.br * (1.0 + self.alpha_br / 100.0 * (t - 20.0))).max(0.0)
    }

    /// Perméabilité relative vue par le solveur à champ faible. Les para/dia y valent 1 :
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

    /// Courbe B(H) en vigueur à la température `t` (°C) : aucune au-delà de Tc.
    pub fn curve_at(&self, t: f64) -> Option<&BhCurve> {
        self.bh.as_ref().filter(|_| self.class == MagClass::Ferro && !(self.t_curie > 0.0 && t >= self.t_curie))
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
        bh: None,
    }
}

/// Ferromagnétique doux : perméabilité initiale, polarisation à saturation `js` (T), Tc (°C).
fn ferro(name: &str, mu_r: f64, js: f64, tc: f64, rho: f64) -> Material {
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
        bh: Some(BhCurve::model(mu_r, js)),
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
        bh: None,
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
        bh: None,
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
        ferro("Fer pur (Armco)", 5000.0, 2.15, 770.0, 7.87),
        ferro("Acier doux (S235)", 1500.0, 2.05, 770.0, 7.85),
        ferro("Acier électrique Fe-3%Si", 7000.0, 2.03, 740.0, 7.65),
        ferro("Mu-métal", 80000.0, 0.75, 400.0, 8.7),
        ferro("Permalloy 50% Ni", 50000.0, 1.55, 480.0, 8.2),
        ferro("Fer-cobalt (Permendur)", 10000.0, 2.35, 940.0, 8.1),
        ferro("Ferrite douce MnZn", 5000.0, 0.45, 200.0, 4.8),
        ferro("Nickel", 300.0, 0.61, 354.0, 8.9),
        ferro("Cobalt", 150.0, 1.79, 1115.0, 8.9),
        ferro("Gadolinium", 50.0, 2.0, 20.0, 7.9),
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

#[cfg(test)]
mod tests {
    use super::*;

    /// La spline passe par les points, reste monotone et se prolonge par la pente du vide.
    #[test]
    fn bh_curve_is_monotone_and_saturates() {
        let curve = BhCurve::model(5000.0, 2.15);
        let table = curve.table();
        assert!((curve.js() - 2.15).abs() < 2e-3, "{}", curve.js());
        assert!((curve.mu_r_initial() / 5000.0 - 1.0).abs() < 0.02, "{}", curve.mu_r_initial());
        // Chaque point de la courbe est retrouvé : H(B) est l'inverse de B(H).
        for p in &curve.points {
            assert!((table.h(p[1]) / p[0] - 1.0).abs() < 5e-3, "H({}) = {} au lieu de {}", p[1], table.h(p[1]), p[0]);
        }
        // ν croît avec B (le fer sature) et tend vers 1 ; νd reste positif.
        let mut last = 0.0;
        for i in 0..=400 {
            let b = i as f64 * 0.01;
            let (nu, nu_d) = table.eval(b);
            assert!(nu >= last - 1e-9 && nu <= 1.0 && nu_d > 0.0 && nu_d <= 1.0 + 1e-12, "B = {b} : ν = {nu}, νd = {nu_d}");
            last = nu;
        }
        let (nu, _) = table.eval(4.0);
        assert!((nu - (4.0 - curve.js()) / 4.0).abs() < 1e-3);
    }

    /// Une table de fiche technique quelconque est respectée, points mal ordonnés compris.
    #[test]
    fn bh_curve_from_datasheet_points() {
        let curve = BhCurve { points: vec![[100.0, 0.5], [50.0, 0.4], [300.0, 1.2], [1000.0, 1.5], [10000.0, 1.8], [100000.0, 2.1]] };
        let table = curve.table();
        for p in [[100.0, 0.5], [300.0, 1.2], [1000.0, 1.5], [10000.0, 1.8]] {
            assert!((table.h(p[1]) / p[0] - 1.0).abs() < 1e-2, "{p:?} : {}", table.h(p[1]));
        }
        assert!((curve.js() - (2.1 - MU0 * 1e5)).abs() < 1e-12);
    }
}
