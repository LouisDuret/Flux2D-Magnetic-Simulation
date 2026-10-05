//! Flux2D — simulateur interactif de champs magnétiques (2D plan).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod render;
mod theme;
mod visuals;

use eframe::egui;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title("Flux2D").with_inner_size([1440.0, 880.0]),
        ..Default::default()
    };
    eframe::run_native("Flux2D", options, Box::new(|cc| Ok(Box::new(app::App::new(cc)))))
}
