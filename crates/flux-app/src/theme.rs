//! Thème sombre (section 7.5 du document).

use eframe::egui::{self, Color32, Stroke};
use flux_core::material::MagClass;

pub const BG: Color32 = Color32::from_rgb(0x0E, 0x11, 0x16);
pub const SURFACE: Color32 = Color32::from_rgb(0x16, 0x1B, 0x22);
pub const RAISED: Color32 = Color32::from_rgb(0x1C, 0x22, 0x30);
pub const BORDER: Color32 = Color32::from_rgb(0x2A, 0x31, 0x40);
pub const TEXT: Color32 = Color32::from_rgb(0xE6, 0xEA, 0xF2);
pub const TEXT_DIM: Color32 = Color32::from_rgb(0x9A, 0xA4, 0xB2);
pub const ACCENT: Color32 = Color32::from_rgb(0x38, 0xBD, 0xF8);
pub const NORTH: Color32 = Color32::from_rgb(0xEF, 0x44, 0x44);
pub const SOUTH: Color32 = Color32::from_rgb(0x3B, 0x82, 0xF6);
pub const FORCE: Color32 = Color32::from_rgb(0x22, 0xC5, 0x5E);
pub const WARN: Color32 = Color32::from_rgb(0xF5, 0x9E, 0x0B);
const COPPER: Color32 = Color32::from_rgb(0xD0, 0x8A, 0x4E);

pub fn class_color(class: MagClass) -> Color32 {
    match class {
        MagClass::Magnet => NORTH,
        MagClass::Ferro => Color32::from_rgb(0x94, 0xA3, 0xB8),
        MagClass::Para => Color32::from_rgb(0x2D, 0xD4, 0xBF),
        MagClass::Dia => Color32::from_rgb(0xA7, 0x8B, 0xFA),
        MagClass::Conductor => COPPER,
        MagClass::Superconductor => Color32::from_rgb(0xF9, 0xA8, 0xD4),
    }
}

pub fn apply(ctx: &egui::Context) {
    let mut v = egui::Visuals::dark();
    v.panel_fill = SURFACE;
    v.window_fill = RAISED;
    v.extreme_bg_color = BG;
    v.faint_bg_color = RAISED;
    v.override_text_color = Some(TEXT);
    v.hyperlink_color = ACCENT;
    v.window_stroke = Stroke::new(1.0, BORDER);
    v.selection.bg_fill = ACCENT.gamma_multiply(0.35);
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    v.widgets.inactive.bg_fill = RAISED;
    v.widgets.inactive.weak_bg_fill = RAISED;
    v.widgets.hovered.weak_bg_fill = BORDER;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, ACCENT);
    v.widgets.active.weak_bg_fill = BORDER;
    ctx.set_visuals(v);
}
