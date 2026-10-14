#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // no console window on Windows (release)

mod app;
mod data;
mod export;
mod fonts;
mod plot;
mod ui;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("CurveTracer")
            .with_inner_size([1500.0, 920.0])
            .with_min_inner_size([960.0, 600.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native(
        "CurveTracer",
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(&cc.egui_ctx)))),
    )
}
