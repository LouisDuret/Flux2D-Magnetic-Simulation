//! Cœur de Flux2D : matériaux, formes, scène et rastérisation.
//! Aucune dépendance à une fenêtre ou à un GPU.

pub mod material;
pub mod raster;
pub mod scene;
pub mod shape;

pub use glam::{DVec2, DVec3};

/// Perméabilité du vide (H/m).
pub const MU0: f64 = 1.256_637_062_12e-6;

/// Zéro absolu (°C) : aucune température de la scène ne peut descendre en dessous.
pub const ABSOLUTE_ZERO_C: f64 = -273.15;
