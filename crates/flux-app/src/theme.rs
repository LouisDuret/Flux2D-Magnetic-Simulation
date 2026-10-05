//! Thème « instrument » : fond presque noir, aplats, angles droits, filets d'un pixel
//! et accent ambre. Valeurs reprises de la maquette de référence.

use eframe::egui::{self, Color32, CornerRadius, FontFamily, FontId, Stroke, TextStyle};
use flux_core::material::MagClass;
use std::sync::Arc;

pub const BG: Color32 = Color32::from_rgb(0x0A, 0x0B, 0x0D);
pub const PANEL: Color32 = Color32::from_rgb(0x0E, 0x0F, 0x12);
/// Filets : séparations de panneaux, de lignes de liste, de contrôles.
pub const LINE: Color32 = Color32::from_rgb(0x1F, 0x21, 0x27);
pub const LINE_SOFT: Color32 = Color32::from_rgb(0x16, 0x17, 0x1C);
pub const LINE_CTRL: Color32 = Color32::from_rgb(0x2A, 0x2C, 0x35);
pub const TICK_MINOR: Color32 = Color32::from_rgb(0x2E, 0x30, 0x38);
pub const TICK_MAJOR: Color32 = Color32::from_rgb(0x4A, 0x4D, 0x58);
/// Texte, du plus lumineux au plus discret.
pub const TEXT_HI: Color32 = Color32::from_rgb(0xF4, 0xF2, 0xEE);
pub const TEXT: Color32 = Color32::from_rgb(0xE8, 0xE6, 0xE3);
pub const TEXT_MID: Color32 = Color32::from_rgb(0xC9, 0xCB, 0xD1);
pub const LABEL: Color32 = Color32::from_rgb(0x9A, 0x9D, 0xA6);
pub const DIM: Color32 = Color32::from_rgb(0x8E, 0x91, 0x9A);
pub const FAINT: Color32 = Color32::from_rgb(0x6E, 0x71, 0x7A);
pub const DISABLED: Color32 = Color32::from_rgb(0x5E, 0x61, 0x6B);
/// Accent ambre et son dégradé orange → jaune.
pub const ACCENT: Color32 = Color32::from_rgb(0xF5, 0xC5, 0x42);
pub const ACCENT_TEXT: Color32 = Color32::from_rgb(0xFF, 0xE7, 0xA8);
pub const ON_ACCENT: Color32 = Color32::from_rgb(0x1A, 0x12, 0x06);
pub const GRAD: [Color32; 3] =
    [Color32::from_rgb(0xF0, 0x8A, 0x24), Color32::from_rgb(0xF5, 0xB5, 0x2E), Color32::from_rgb(0xFF, 0xD8, 0x66)];
pub const WARN: Color32 = GRAD[0];
pub const NORTH: Color32 = Color32::from_rgb(0xF2, 0x67, 0x7A);
pub const SOUTH: Color32 = Color32::from_rgb(0x6F, 0xA8, 0xFF);
pub const FORCE: Color32 = Color32::from_rgb(0x7B, 0xD8, 0x8F);
pub const FORCE_TEXT: Color32 = Color32::from_rgb(0xA8, 0xED, 0xB8);

/// Teinte de fond d'un élément actif.
pub fn tint() -> Color32 {
    Color32::from_rgba_unmultiplied(245, 181, 46, 23)
}

/// Fond translucide des étiquettes et barres posées sur le canevas.
pub fn overlay() -> Color32 {
    Color32::from_rgba_unmultiplied(10, 11, 13, 224)
}

pub fn class_color(class: MagClass) -> Color32 {
    match class {
        MagClass::Magnet => NORTH,
        MagClass::Ferro => TEXT_MID,
        MagClass::Para => Color32::from_rgb(0x4F, 0xC3, 0xB0),
        MagClass::Dia => Color32::from_rgb(0xA7, 0x8B, 0xFA),
        MagClass::Conductor => Color32::from_rgb(0xE0, 0x9A, 0x5A),
        MagClass::Superconductor => Color32::from_rgb(0xF9, 0xA8, 0xD4),
    }
}

pub fn sans(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}

pub fn sans_bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("sans-semibold".into()))
}

pub fn mono(size: f32) -> FontId {
    FontId::new(size, FontFamily::Monospace)
}

pub fn mono_bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("mono-semibold".into()))
}

/// Cherche le premier fichier de police disponible : d'abord IBM Plex dans `assets/fonts`
/// (à côté de l'exécutable ou du répertoire courant), sinon une police du système.
fn load_font(files: &[&str]) -> Option<Vec<u8>> {
    let mut dirs = vec![std::path::PathBuf::from("assets/fonts")];
    if let Ok(exe) = std::env::current_exe() {
        dirs.extend(exe.parent().map(|d| d.join("assets/fonts")));
    }
    dirs.extend(std::env::var_os("LOCALAPPDATA").map(|d| std::path::Path::new(&d).join("Microsoft/Windows/Fonts")));
    dirs.extend(std::env::var_os("WINDIR").map(|d| std::path::Path::new(&d).join("Fonts")));
    files.iter().find_map(|file| dirs.iter().find_map(|dir| std::fs::read(dir.join(file)).ok()))
}

fn install_fonts(ctx: &egui::Context) {
    let mut defs = egui::FontDefinitions::default();
    let families = [
        (FontFamily::Proportional, FontFamily::Proportional, ["IBMPlexSans-Regular.ttf", "segoeui.ttf"]),
        (FontFamily::Name("sans-semibold".into()), FontFamily::Proportional, ["IBMPlexSans-SemiBold.ttf", "seguisb.ttf"]),
        (FontFamily::Monospace, FontFamily::Monospace, ["IBMPlexMono-Regular.ttf", "consola.ttf"]),
        (FontFamily::Name("mono-semibold".into()), FontFamily::Monospace, ["IBMPlexMono-SemiBold.ttf", "consolab.ttf"]),
    ];
    for (i, (family, fallback, files)) in families.into_iter().enumerate() {
        // Les polices intégrées d'egui restent en repli pour les glyphes manquants.
        let mut list = defs.families.get(&fallback).cloned().unwrap_or_default();
        if let Some(bytes) = load_font(&files) {
            let key = format!("flux-{i}");
            defs.font_data.insert(key.clone(), Arc::new(egui::FontData::from_owned(bytes)));
            list.insert(0, key);
        }
        defs.families.insert(family, list);
    }
    ctx.set_fonts(defs);
}

pub fn apply(ctx: &egui::Context) {
    install_fonts(ctx);
    let mut v = egui::Visuals::dark();
    v.panel_fill = PANEL;
    v.window_fill = PANEL;
    v.window_stroke = Stroke::new(1.0, LINE_CTRL);
    v.extreme_bg_color = BG;
    v.faint_bg_color = LINE_SOFT;
    v.override_text_color = Some(TEXT);
    v.hyperlink_color = ACCENT;
    v.window_corner_radius = CornerRadius::ZERO;
    v.menu_corner_radius = CornerRadius::ZERO;
    v.window_shadow = egui::Shadow::NONE;
    v.popup_shadow = egui::Shadow::NONE;
    v.selection.bg_fill = ACCENT.gamma_multiply(0.28);
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    v.text_cursor.stroke = Stroke::new(1.5, ACCENT);
    let w = &mut v.widgets;
    for state in [&mut w.noninteractive, &mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
        state.corner_radius = CornerRadius::ZERO;
        state.expansion = 0.0;
        state.bg_fill = LINE_CTRL;
        state.weak_bg_fill = Color32::TRANSPARENT;
        state.bg_stroke = Stroke::NONE;
        state.fg_stroke = Stroke::new(1.0, TEXT_HI);
    }
    w.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
    w.noninteractive.fg_stroke = Stroke::new(1.0, LABEL);
    w.hovered.weak_bg_fill = tint();
    w.hovered.bg_stroke = Stroke::new(1.0, LINE_CTRL);
    w.active.weak_bg_fill = tint();
    w.active.bg_stroke = Stroke::new(1.0, ACCENT);
    w.open.weak_bg_fill = tint();
    ctx.set_visuals(v);
    ctx.all_styles_mut(|style| {
        style.text_styles = [
            (TextStyle::Body, sans(12.5)),
            (TextStyle::Button, sans(12.5)),
            (TextStyle::Small, sans(10.5)),
            (TextStyle::Heading, sans_bold(13.0)),
            (TextStyle::Monospace, mono(12.0)),
        ]
        .into();
        style.drag_value_text_style = TextStyle::Monospace;
        style.spacing.button_padding = egui::vec2(6.0, 2.0);
    });
}
