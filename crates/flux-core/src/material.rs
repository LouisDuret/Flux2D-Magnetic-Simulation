//! Bibliothèque de matériaux (valeurs par défaut de la section 6 du document).

use crate::{ABSOLUTE_ZERO_C, MU0};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::sync::{Arc, Mutex};

/// Perméabilité relative d'un supraconducteur dans l'état Meissner (χ ≈ −1) :
/// une très forte réluctivité qui annule B à l'intérieur.
pub const MU_R_MEISSNER: f64 = 1e-4;

/// Susceptibilité en dessous de laquelle un matériau est invisible pour le solveur (μr = 1) et
/// ne subit que la force de Kelvin, exacte au premier ordre en χ.
pub const WEAK_CHI: f64 = 0.01;

/// Rapport entre le champ du coude de la courbe de désaimantation d'un aimant et sa
/// coercivité intrinsèque HcJ.
pub const KNEE: f64 = 0.9;

/// Aimantation spontanée réduite Ms(T)/Ms(0) d'un ferromagnétique (loi de Kuz'min) :
/// [1 − s·τ^(3/2) − (1 − s)·τ^(5/2)]^(1/3), τ = T/Tc en kelvins.
pub fn kuzmin(s: f64, tau: f64) -> f64 {
    if tau >= 1.0 {
        return 0.0;
    }
    let tau = tau.max(0.0);
    (1.0 - s * tau.powf(1.5) - (1.0 - s) * tau.powf(2.5)).max(0.0).cbrt()
}

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

    /// Courbe à une autre température : la polarisation à saturation est multipliée par `f` à
    /// perméabilité initiale inchangée, J(H) → f·J(H/f), ce qui revient à multiplier H et B par f.
    pub fn scaled(&self, f: f64) -> BhCurve {
        BhCurve { points: self.points.iter().map(|p| [p[0] * f, p[1] * f]).collect() }
    }

    /// Lit une table de fiche technique : une ligne par point, deux nombres (H en A/m et B en
    /// teslas, dans un ordre ou dans l'autre) séparés par un point-virgule, une tabulation, une
    /// virgule ou des espaces. Les lignes sans deux nombres (titres) sont ignorées.
    pub fn from_text(text: &str) -> Option<BhCurve> {
        let mut rows: Vec<[f64; 2]> = Vec::new();
        for line in text.lines() {
            // Avec un point-virgule ou une tabulation pour séparateur, la virgule est décimale.
            let decimal_comma = line.contains(';') || line.contains('\t');
            let cells: Vec<f64> = line
                .split(|c: char| c == ';' || c == '\t' || (c == ',' && !decimal_comma) || (c == ' ' && !decimal_comma))
                .filter_map(|cell| cell.trim().replace(',', ".").parse().ok())
                .collect();
            if let [a, b] = cells[..] {
                rows.push([a, b]);
            }
        }
        // La colonne des plus grandes valeurs est H (des A/m contre des teslas).
        let max = |k: usize| rows.iter().map(|r| r[k].abs()).fold(0.0, f64::max);
        if max(0) < max(1) {
            rows.iter_mut().for_each(|r| r.swap(0, 1));
        }
        rows.retain(|r| r[0] > 0.0 && r[1] > 0.0);
        rows.sort_by(|a, b| a[0].total_cmp(&b[0]));
        let curve = BhCurve { points: rows };
        (curve.knots().0.len() >= 3).then_some(curve)
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
    /// Susceptibilité volumique SI des para/diamagnétiques (traitée en perturbation), à la
    /// température `t_ref`.
    #[serde(default)]
    pub chi: f64,
    /// Matériau anisotrope (graphite pyrolytique) : susceptibilité le long des feuillets, `chi`
    /// valant alors perpendiculairement à eux. 0 : matériau isotrope.
    #[serde(default)]
    pub chi_par: f64,
    /// Rémanence à 20 °C (T).
    #[serde(default)]
    pub br: f64,
    /// Coefficient de température de Br (%/K).
    #[serde(default)]
    pub alpha_br: f64,
    /// Coercivité intrinsèque HcJ à 20 °C (A/m) ; 0 : l'aimant ne se désaimante jamais.
    #[serde(default)]
    pub hcj: f64,
    /// Coefficient de température de HcJ (%/K).
    #[serde(default)]
    pub beta_hcj: f64,
    /// Température maximale d'emploi (°C), 0 si sans objet.
    #[serde(default)]
    pub t_max: f64,
    /// Température de Curie (°C), 0 si sans objet.
    #[serde(default)]
    pub t_curie: f64,
    /// Paramètre de forme s de la loi de Kuz'min ; 0 : la saturation ne dépend pas de T sous Tc.
    #[serde(default)]
    pub kuzmin_s: f64,
    /// Constante de Curie C (K) : au-dessus de Tc, χ = C/(T − Tc) (loi de Curie–Weiss).
    #[serde(default)]
    pub curie_const: f64,
    /// Température (°C) à laquelle la courbe B(H) et la susceptibilité sont données.
    #[serde(default = "room")]
    pub t_ref: f64,
    /// Température critique d'un supraconducteur (°C).
    #[serde(default)]
    pub t_critical: f64,
    /// Masse volumique (kg/m³).
    pub density: f64,
    /// Capacité thermique massique (J/(kg·K)).
    #[serde(default = "typical_heat_capacity")]
    pub heat_capacity: f64,
    /// Émissivité de la surface (rayonnement thermique), de 0 à 1.
    #[serde(default = "typical_emissivity")]
    pub emissivity: f64,
    /// Résistivité à 20 °C (Ω·m), 0 si sans objet.
    #[serde(default)]
    pub resistivity: f64,
    /// Coefficient de température de la résistivité (1/K).
    #[serde(default)]
    pub alpha_rho: f64,
    /// Courbe B(H) d'un ferromagnétique doux ; sans elle, le matériau est linéaire (μr constant).
    #[serde(default)]
    pub bh: Option<BhCurve>,
}

fn room() -> f64 {
    20.0
}

fn typical_heat_capacity() -> f64 {
    500.0
}

fn typical_emissivity() -> f64 {
    0.5
}

impl Material {
    /// Matériau neutre (μr = 1) de la classe donnée, à compléter champ par champ.
    pub fn new(name: &str, class: MagClass) -> Material {
        Material {
            name: name.into(),
            class,
            mu_r: 1.0,
            chi: 0.0,
            chi_par: 0.0,
            br: 0.0,
            alpha_br: 0.0,
            hcj: 0.0,
            beta_hcj: 0.0,
            t_max: 0.0,
            t_curie: 0.0,
            kuzmin_s: 0.0,
            curie_const: 0.0,
            t_ref: room(),
            t_critical: 0.0,
            density: 1000.0,
            heat_capacity: typical_heat_capacity(),
            emissivity: typical_emissivity(),
            resistivity: 0.0,
            alpha_rho: 0.0,
            bh: None,
        }
    }

    /// Rémanence à la température `t` (°C) : Br(20)·[1 + α(T − 20)], nulle au-delà de Tc.
    pub fn br_at(&self, t: f64) -> f64 {
        if self.t_curie > 0.0 && t >= self.t_curie {
            return 0.0;
        }
        (self.br * (1.0 + self.alpha_br / 100.0 * (t - 20.0))).max(0.0)
    }

    /// Coercivité intrinsèque à la température `t` (°C) : HcJ(20)·[1 + β(T − 20)].
    pub fn hcj_at(&self, t: f64) -> f64 {
        (self.hcj * (1.0 + self.beta_hcj / 100.0 * (t - 20.0))).max(0.0)
    }

    /// Désaimantation irréversible (section 2.4). L'aimant suit la droite de recul
    /// B = μ0·μrec·H + `remanence` tant que le champ le long de son axe reste au-dessus du coude,
    /// en −`KNEE`·HcJ. Au-delà, la rémanence de recul tombe en ligne droite jusqu'à annuler la
    /// polarisation en −HcJ, et s'inverse ensuite. Renvoie la rémanence (T) de la droite de
    /// recul passant par le point de cette courbe atteint à l'induction `b_axis` (T) mesurée le
    /// long de l'axe : jamais plus que `remanence`.
    pub fn demagnetized(&self, t: f64, b_axis: f64, remanence: f64) -> f64 {
        let (br, hcj) = (self.br_at(t), self.hcj_at(t).max(1.0));
        if self.class != MagClass::Magnet || self.hcj <= 0.0 || br <= 0.0 {
            return remanence;
        }
        let mu = MU0 * self.mu_r;
        let knee = KNEE * hcj;
        let slope = (br - MU0 * (self.mu_r - 1.0) * hcj) / (hcj - knee);
        if (b_axis - remanence) / mu >= -knee || slope <= 0.0 {
            return remanence;
        }
        // Intersection de la courbe avec la droite de recul d'induction b_axis :
        // x = Br + ((b_axis − x)/μ + Hk)·pente.
        let x = (br + (b_axis / mu + knee) * slope) / (1.0 + slope / mu);
        x.clamp(-br, remanence)
    }

    /// Rapport Js(T)/Js(t_ref) d'un ferromagnétique (loi de Kuz'min) : 0 au-delà de Tc.
    pub fn js_factor(&self, t: f64) -> f64 {
        if self.t_curie <= 0.0 {
            return 1.0;
        }
        if t >= self.t_curie {
            return 0.0;
        }
        if self.kuzmin_s == 0.0 || t == self.t_ref {
            return 1.0;
        }
        let tc = self.t_curie - ABSOLUTE_ZERO_C;
        let reduced = |t: f64| kuzmin(self.kuzmin_s, (t - ABSOLUTE_ZERO_C) / tc);
        let reference = reduced(self.t_ref);
        // Arrondi au millième : la table ν(B) n'est pas recalculée pour un centième de degré.
        if reference > 0.0 { (reduced(t) / reference * 1000.0).round() / 1000.0 } else { 0.0 }
    }

    /// Un ferromagnétique l'est-il encore à la température `t` (°C) ?
    pub fn is_ferromagnetic(&self, t: f64) -> bool {
        self.class == MagClass::Ferro && self.js_factor(t) > 0.0
    }

    /// Perméabilité relative vue par le solveur à champ faible. Les para/dia y valent 1 :
    /// en f32, μr = 1,00002 se noierait dans le résidu (section 2.4).
    pub fn mu_r_solver(&self, t: f64) -> f64 {
        match self.class {
            MagClass::Magnet => self.mu_r,
            MagClass::Ferro if self.is_ferromagnetic(t) => self.mu_r,
            // Au-dessus de Tc, le fer n'est plus que paramagnétique (Curie–Weiss).
            MagClass::Ferro => {
                let chi = self.chi_at(t);
                if chi >= WEAK_CHI { 1.0 + chi } else { 1.0 }
            }
            MagClass::Superconductor if self.is_superconducting(t) => MU_R_MEISSNER,
            _ => 1.0,
        }
    }

    /// Courbe B(H) en vigueur à la température `t` (°C) : sa saturation suit la loi de Kuz'min,
    /// et il n'y en a plus au-delà de Tc.
    pub fn curve_at(&self, t: f64) -> Option<Cow<'_, BhCurve>> {
        let bh = self.bh.as_ref().filter(|_| self.class == MagClass::Ferro)?;
        let factor = self.js_factor(t);
        if factor <= 0.0 {
            None
        } else if factor == 1.0 {
            Some(Cow::Borrowed(bh))
        } else {
            Some(Cow::Owned(bh.scaled(factor)))
        }
    }

    /// Sous sa température critique, un supraconducteur expulse le champ (effet Meissner).
    pub fn is_superconducting(&self, t: f64) -> bool {
        self.class == MagClass::Superconductor && t < self.t_critical
    }

    /// Susceptibilité à la température `t` (°C) : loi de Curie χ·T0/T pour les
    /// paramagnétiques, loi de Curie–Weiss C/(T − Tc) pour un ferromagnétique au-dessus de Tc
    /// (bornée à 1 K de la transition, où elle divergerait), constante pour les diamagnétiques.
    pub fn chi_at(&self, t: f64) -> f64 {
        match self.class {
            MagClass::Para => self.chi * (self.t_ref - ABSOLUTE_ZERO_C) / (t - ABSOLUTE_ZERO_C).max(1.0),
            MagClass::Ferro if !self.is_ferromagnetic(t) => {
                (self.curie_const / (t - self.t_curie).max(1.0)).min((self.mu_r - 1.0).max(0.0))
            }
            _ => self.chi,
        }
    }

    /// Susceptibilités [χ⊥, χ∥] d'un matériau trop faiblement magnétique pour le solveur, qui
    /// ne subit que la force de Kelvin : perpendiculairement aux feuillets (axe Y de l'objet)
    /// et le long d'eux. `None` si le matériau entre dans le calcul du champ.
    pub fn weak_chi(&self, t: f64) -> Option<[f64; 2]> {
        let chi = self.chi_at(t);
        match self.class {
            MagClass::Para | MagClass::Dia if self.chi_par != 0.0 && self.chi != 0.0 => Some([chi, self.chi_par * chi / self.chi]),
            MagClass::Para | MagClass::Dia => Some([chi, chi]),
            MagClass::Ferro if !self.is_ferromagnetic(t) && chi < WEAK_CHI => Some([chi, chi]),
            _ => None,
        }
    }

    /// Résistivité à la température `t` (°C) : ρ20·[1 + α(T − 20)] (Ω·m).
    pub fn resistivity_at(&self, t: f64) -> f64 {
        (self.resistivity * (1.0 + self.alpha_rho * (t - 20.0))).max(0.0)
    }
}

/// Aimant : Br (T), α (%/K), HcJ (kA/m), β (%/K), température maximale d'emploi et Tc (°C),
/// masse volumique (g/cm³), μrec, capacité thermique (J/(kg·K)).
#[allow(clippy::too_many_arguments)]
fn magnet(name: &str, br: f64, alpha: f64, hcj: f64, beta: f64, t_max: f64, tc: f64, rho: f64, mu_rec: f64, c: f64) -> Material {
    Material {
        mu_r: mu_rec,
        br,
        alpha_br: alpha,
        hcj: hcj * 1e3,
        beta_hcj: beta,
        t_max,
        t_curie: tc,
        density: rho * 1000.0,
        heat_capacity: c,
        ..Material::new(name, MagClass::Magnet)
    }
}

/// Ferromagnétique doux : perméabilité initiale, polarisation à saturation `js` (T) à 20 °C,
/// Tc (°C), masse volumique (g/cm³), capacité thermique (J/(kg·K)), paramètre s de Kuz'min
/// et constante de Curie (K).
#[allow(clippy::too_many_arguments)]
fn ferro(name: &str, mu_r: f64, js: f64, tc: f64, rho: f64, c: f64, s: f64, curie: f64) -> Material {
    Material {
        mu_r,
        t_curie: tc,
        kuzmin_s: s,
        curie_const: curie,
        density: rho * 1000.0,
        heat_capacity: c,
        emissivity: 0.6,
        bh: Some(BhCurve::model(mu_r, js)),
        ..Material::new(name, MagClass::Ferro)
    }
}

fn weak(name: &str, class: MagClass, chi: f64, rho: f64, c: f64) -> Material {
    Material { mu_r: 1.0 + chi, chi, density: rho * 1000.0, heat_capacity: c, emissivity: 0.3, ..Material::new(name, class) }
}

/// Supraconducteur de température critique `tc_kelvin`.
fn superconductor(name: &str, tc_kelvin: f64, rho: f64, c: f64) -> Material {
    Material {
        mu_r: MU_R_MEISSNER,
        chi: -1.0,
        t_critical: tc_kelvin + ABSOLUTE_ZERO_C,
        density: rho * 1000.0,
        heat_capacity: c,
        emissivity: 0.9,
        ..Material::new(name, MagClass::Superconductor)
    }
}

/// Bibliothèque par défaut. Les paramètres de Kuz'min et les constantes de Curie du fer, du
/// nickel, du cobalt et du gadolinium viennent de la littérature ; ceux des alliages sont des
/// estimations.
pub fn library() -> Vec<Material> {
    use MagClass::*;
    vec![
        magnet("NdFeB N35", 1.19, -0.12, 955.0, -0.6, 80.0, 310.0, 7.5, 1.05, 440.0),
        magnet("NdFeB N42", 1.30, -0.12, 955.0, -0.6, 80.0, 310.0, 7.5, 1.05, 440.0),
        magnet("NdFeB N52", 1.45, -0.12, 876.0, -0.6, 60.0, 310.0, 7.5, 1.05, 440.0),
        magnet("NdFeB N42SH", 1.30, -0.12, 1592.0, -0.55, 150.0, 340.0, 7.5, 1.05, 440.0),
        magnet("SmCo 2:17", 1.07, -0.035, 1433.0, -0.2, 300.0, 800.0, 8.4, 1.05, 370.0),
        Material { emissivity: 0.9, ..magnet("Ferrite Sr (Y30)", 0.385, -0.20, 200.0, 0.3, 250.0, 450.0, 4.9, 1.1, 800.0) },
        magnet("AlNiCo 5", 1.26, -0.02, 50.0, 0.0, 525.0, 860.0, 7.3, 3.5, 460.0),
        ferro("Fer pur (Armco)", 5000.0, 2.15, 770.0, 7.87, 449.0, 0.35, 2.2),
        ferro("Acier doux (S235)", 1500.0, 2.05, 770.0, 7.85, 470.0, 0.35, 2.1),
        ferro("Acier électrique Fe-3%Si", 7000.0, 2.03, 740.0, 7.65, 460.0, 0.35, 2.0),
        ferro("Mu-métal", 80000.0, 0.75, 400.0, 8.7, 460.0, 0.25, 0.8),
        ferro("Permalloy 50% Ni", 50000.0, 1.55, 480.0, 8.2, 480.0, 0.25, 1.5),
        ferro("Fer-cobalt (Permendur)", 10000.0, 2.35, 940.0, 8.1, 420.0, 0.25, 2.4),
        Material { emissivity: 0.9, ..ferro("Ferrite douce MnZn", 5000.0, 0.45, 200.0, 4.8, 750.0, 0.5, 0.5) },
        ferro("Nickel", 300.0, 0.61, 354.0, 8.9, 444.0, 0.15, 0.61),
        ferro("Cobalt", 150.0, 1.79, 1115.0, 8.9, 421.0, 0.11, 2.3),
        // Le gadolinium n'est ferromagnétique que sous 20 °C : sa courbe est donnée à 0 K.
        Material { t_ref: ABSOLUTE_ZERO_C, ..ferro("Gadolinium", 50.0, 2.6, 20.0, 7.9, 236.0, 1.3, 5.0) },
        Material { emissivity: 0.9, ..ferro("Magnétite (Fe3O4)", 10.0, 0.6, 585.0, 5.2, 650.0, 0.5, 0.6) },
        weak("Aluminium", Para, 2.2e-5, 2.70, 897.0),
        weak("Platine", Para, 2.7e-4, 21.45, 133.0),
        weak("Titane", Para, 1.8e-4, 4.51, 523.0),
        weak("Tungstène", Para, 7.8e-5, 19.3, 134.0),
        weak("Inox austénitique 304", Para, 4.0e-3, 8.0, 500.0),
        // Susceptibilité donnée à son point d'ébullition : la loi de Curie la ramène à 20 °C.
        Material { t_ref: -183.0, emissivity: 0.9, ..weak("Oxygène liquide", Para, 3.5e-3, 1.14, 1700.0) },
        Material { chi_par: -8.5e-5, emissivity: 0.8, ..weak("Graphite pyrolytique", Dia, -4.5e-4, 2.2, 710.0) },
        weak("Bismuth", Dia, -1.66e-4, 9.78, 122.0),
        Material { emissivity: 0.95, ..weak("Eau", Dia, -9.0e-6, 1.0, 4186.0) },
        weak("Diamant", Dia, -2.2e-5, 3.51, 509.0),
        weak("Argent", Dia, -2.4e-5, 10.5, 235.0),
        weak("Or", Dia, -3.4e-5, 19.3, 129.0),
        // Fil émaillé : résistivité du cuivre recuit, ρ(T) = ρ20·[1 + 0,00393·(T − 20)].
        Material {
            resistivity: 1.68e-8,
            alpha_rho: 0.00393,
            emissivity: 0.8,
            ..weak("Cuivre (bobinage)", Conductor, -9.6e-6, 8.96, 385.0)
        },
        superconductor("YBCO", 92.0, 6.3, 430.0),
        superconductor("BSCCO-2223", 110.0, 6.5, 400.0),
        superconductor("MgB2", 39.0, 2.57, 600.0),
        superconductor("Niobium", 9.25, 8.57, 265.0),
        superconductor("Plomb", 7.2, 11.34, 128.0),
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

    /// Loi de Kuz'min : la saturation du fer baisse peu jusqu'à 400 °C, s'effondre près de Tc,
    /// et le gadolinium n'est ferromagnétique que sous 20 °C.
    #[test]
    fn saturation_follows_kuzmin() {
        let lib = library();
        let get = |name: &str| lib.iter().find(|m| m.name == name).unwrap();
        let iron = get("Fer pur (Armco)");
        assert_eq!(iron.js_factor(20.0), 1.0);
        // Fer, s = 0,35 : Ms(T)/Ms(0) vaut 0,973 à 20 °C et 0,844 à 400 °C.
        assert!((kuzmin(0.35, 293.15 / 1043.15) - 0.9728).abs() < 1e-3);
        assert!((iron.js_factor(400.0) - 0.868).abs() < 2e-3, "{}", iron.js_factor(400.0));
        assert!(iron.js_factor(760.0) < 0.35 && iron.js_factor(769.9) < 0.15 && iron.js_factor(770.0) == 0.0);
        let hot = iron.curve_at(400.0).unwrap();
        assert!((hot.js() / 2.15 - iron.js_factor(400.0)).abs() < 2e-3);
        assert!((hot.mu_r_initial() / iron.curve_at(20.0).unwrap().mu_r_initial() - 1.0).abs() < 1e-9);
        assert!(iron.curve_at(800.0).is_none() && !iron.is_ferromagnetic(800.0));

        let gd = get("Gadolinium");
        assert!(gd.curve_at(20.0).is_none() && gd.curve_at(25.0).is_none());
        let cold = gd.curve_at(0.0).unwrap().js();
        assert!(cold > 0.9 && cold < 1.4, "Js du gadolinium à 0 °C : {cold} T");
        assert!((gd.curve_at(ABSOLUTE_ZERO_C).unwrap().js() - 2.6).abs() < 5e-3);
    }

    /// Curie–Weiss au-dessus de Tc, Curie pour les paramagnétiques, ρ(T) du cuivre.
    #[test]
    fn susceptibility_and_resistivity_follow_temperature() {
        let lib = library();
        let get = |name: &str| lib.iter().find(|m| m.name == name).unwrap();
        let nickel = get("Nickel");
        // 100 K au-dessus de Tc : χ = C/100, trop faible pour le solveur, donc force de Kelvin.
        assert!((nickel.chi_at(454.0) - 0.0061).abs() < 1e-12);
        assert_eq!((nickel.mu_r_solver(454.0), nickel.weak_chi(454.0)), (1.0, Some([nickel.chi_at(454.0); 2])));
        // À 10 K de Tc, χ = 0,061 : le nickel entre dans le calcul du champ, sans courbe B(H).
        assert!((nickel.mu_r_solver(364.0) - 1.061).abs() < 1e-9 && nickel.weak_chi(364.0).is_none());
        assert!(nickel.weak_chi(20.0).is_none() && nickel.mu_r_solver(20.0) == 300.0);

        let oxygen = get("Oxygène liquide");
        assert!((oxygen.chi_at(-183.0) - 3.5e-3).abs() < 1e-12);
        assert!((oxygen.chi_at(20.0) - 3.5e-3 * 90.15 / 293.15).abs() < 1e-9);
        let graphite = get("Graphite pyrolytique");
        assert_eq!(graphite.weak_chi(300.0), Some([-4.5e-4, -8.5e-5]));
        let copper = get("Cuivre (bobinage)");
        assert!((copper.resistivity_at(120.0) / copper.resistivity_at(20.0) - 1.393).abs() < 1e-9);
    }

    /// Désaimantation : rien au-dessus du coude, chute jusqu'à la polarisation nulle en −HcJ.
    #[test]
    fn magnet_follows_the_knee() {
        let lib = library();
        let n42 = lib.iter().find(|m| m.name == "NdFeB N42").unwrap();
        let mu = MU0 * n42.mu_r;
        let b_at = |h: f64, remanence: f64| mu * h + remanence;
        // Champ inverse de 600 kA/m : en deçà du coude (859 kA/m), l'aimant est intact.
        assert_eq!(n42.demagnetized(20.0, b_at(-600e3, 1.3), 1.3), 1.3);
        // Point de la courbe en −HcJ : la polarisation J = B − μ0·H y est nulle.
        let x = MU0 * (n42.mu_r - 1.0) * 955e3;
        let got = n42.demagnetized(20.0, b_at(-955e3, x), 1.3);
        assert!((got - x).abs() < 1e-9, "{got} au lieu de {x}");
        // Une fois désaimanté, l'aimant ne se ré-aimante pas quand le champ inverse disparaît.
        assert_eq!(n42.demagnetized(20.0, 0.6, got), got);
        // À 150 °C, HcJ a perdu 78 % : la même induction place l'aimant entre le coude et HcJ.
        assert!((n42.hcj_at(150.0) / 955e3 - 0.22).abs() < 1e-9);
        let hot = n42.br_at(150.0);
        let b = b_at(-600e3, hot);
        let left = n42.demagnetized(150.0, b, hot);
        let h = (b - left) / mu;
        assert!(left < 0.6 * hot && h < -KNEE * n42.hcj_at(150.0) && h > -n42.hcj_at(150.0), "Br' = {left} T, H = {h} A/m");
        // Ce point est stable : un second passage ne retire plus rien.
        assert!((n42.demagnetized(150.0, b, left) - left).abs() < 1e-12);
        // Sans coercivité renseignée, l'aimant est idéal.
        let ideal = Material { br: 1.0, ..Material::new("idéal", MagClass::Magnet) };
        assert_eq!(ideal.demagnetized(20.0, -5.0, 1.0), 1.0);
    }

    /// Une table de fiche technique se lit dans les deux ordres de colonnes, virgule décimale comprise.
    #[test]
    fn bh_curve_from_text() {
        let text = "H (A/m);B (T)\n100;0,5\n300;1,2\n1000;1,5\n10000;1,8\n";
        let curve = BhCurve::from_text(text).unwrap();
        assert_eq!(curve.points, vec![[100.0, 0.5], [300.0, 1.2], [1000.0, 1.5], [10000.0, 1.8]]);
        let swapped = BhCurve::from_text("0.5, 100\n1.2, 300\n1.5, 1000\n1.8, 10000").unwrap();
        assert_eq!(swapped, curve);
        assert!(BhCurve::from_text("rien d'utile\n1;2;3").is_none());
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
