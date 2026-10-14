//! "Plots" page: plot tabs, toolbar, the interactive plot and its settings.

use std::sync::Arc;

use egui::{Align, Layout, RichText, Ui};

use super::{ERR, WARN, plot_settings, section, thousands};
use crate::app::App;
use crate::plot::render::ImageFormat;
use crate::plot::view::show_plot;

pub fn show(app: &mut App, ui: &mut Ui) {
    if app.plots.is_empty() {
        ui.heading("Plots");
        ui.add_space(6.0);
        section(ui, "Create a plot", |ui| {
            if app.sources.iter().all(|s| s.ds.is_none()) {
                ui.label("Load a data source first (File Select page).");
            }
            let keys: Vec<(String, String)> = app
                .sources
                .iter()
                .filter(|s| s.ds.is_some())
                .map(|s| (s.key.clone(), s.name()))
                .collect();
            for (k, name) in keys {
                if ui.button(format!("📈 New plot on {name}")).clicked() {
                    app.add_plot(&k);
                }
            }
        });
        return;
    }
    if app.active >= app.plots.len() {
        app.active = app.plots.len() - 1;
    }

    // ---- tabs ----------------------------------------------------------------
    let mut duplicate = None;
    let mut remove = None;
    ui.horizontal_wrapped(|ui| {
        for i in 0..app.plots.len() {
            let name = app.plots[i].cfg.name.clone();
            let r = ui.selectable_label(app.active == i, RichText::new(format!("{}. {name}", i + 1)).strong());
            if r.clicked() {
                app.active = i;
            }
            r.context_menu(|ui| {
                if ui.button("Copy / Split Plot").clicked() {
                    duplicate = Some(i);
                    ui.close();
                }
                if ui.button("🗑 Delete").clicked() {
                    remove = Some(i);
                    ui.close();
                }
            });
        }
        let keys: Vec<(String, String)> = app
            .sources
            .iter()
            .filter(|s| s.ds.is_some())
            .map(|s| (s.key.clone(), s.name()))
            .collect();
        ui.menu_button("+ New plot", |ui| {
            if keys.is_empty() {
                ui.label("No data source loaded");
            }
            for (k, name) in keys {
                if ui.button(name).clicked() {
                    app.add_plot(&k);
                    ui.close();
                }
            }
        });
    });
    if let Some(i) = duplicate {
        app.duplicate_plot(i);
    }
    if let Some(i) = remove {
        app.remove_plot(i);
        return;
    }
    if app.plots.is_empty() {
        return;
    }
    ui.separator();

    // ---- toolbar ---------------------------------------------------------------
    let i = app.active;
    let mut action: Option<&str> = None;
    ui.horizontal(|ui| {
        let p = &mut app.plots[i];
        ui.add(
            egui::TextEdit::singleline(&mut p.cfg.name)
                .desired_width(170.0)
                .hint_text("Plot name"),
        );
        if let Some(d) = &p.state.data {
            if p.state.is_computing() {
                ui.spinner();
            }
            if d.error.is_none() {
                ui.label(
                    RichText::new(format!(
                        "{} curve(s) · {} points · {} rows · {:.0} ms",
                        d.traces.len(),
                        thousands(d.total_points),
                        thousands(d.rows.len()),
                        d.compute_ms
                    ))
                    .weak(),
                );
            }
            if !d.warnings.is_empty() {
                ui.label(RichText::new(format!("⚠ {}", d.warnings.len())).color(WARN))
                    .on_hover_text(d.warnings.join("\n"));
            }
        } else if p.state.is_computing() {
            ui.spinner();
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui
                .button(RichText::new("🗑").color(ERR))
                .on_hover_text("Delete this plot")
                .clicked()
            {
                action = Some("delete");
            }
            if ui
                .button("⬇ Download Data")
                .on_hover_text("Save the curves of this plot as CSV")
                .clicked()
            {
                action = Some("csv");
            }
            if ui
                .button("SVG")
                .on_hover_text("Save this plot as a vector image (SVG: editable in Inkscape, PowerPoint, Illustrator…)")
                .clicked()
            {
                action = Some("svg");
            }
            if ui
                .button("🖼 PNG")
                .on_hover_text("Save this plot as a high resolution image")
                .clicked()
            {
                action = Some("png");
            }
            if ui
                .button("Copy / Split Plot")
                .on_hover_text("Duplicate this plot")
                .clicked()
            {
                action = Some("dup");
            }
            if ui
                .button("⟲")
                .on_hover_text("Reset the view (or double-click on the plot)")
                .clicked()
            {
                action = Some("reset");
            }
        });
    });
    match action {
        Some("delete") => {
            app.remove_plot(i);
            return;
        }
        Some("csv") => app.save_plot_csv_dialog(i),
        Some("png") => app.save_plot_image_dialog(i, ImageFormat::Png),
        Some("svg") => app.save_plot_image_dialog(i, ImageFormat::Svg),
        Some("dup") => {
            app.duplicate_plot(i);
            return;
        }
        Some("reset") => app.plots[i].state.reset_view = true,
        _ => {}
    }

    // ---- settings panel + plot -------------------------------------------------
    egui::Panel::right("ct_plot_settings")
        .resizable(true)
        .default_size(460.0)
        .size_range(340.0..=900.0)
        .show(ui, |ui| plot_settings::show(app, ui));

    egui::CentralPanel::default()
        .frame(egui::Frame::new().inner_margin(egui::Margin::same(4)))
        .show(ui, |ui| {
            let fonts = Arc::clone(&app.fonts);
            let rect = ui.available_rect_before_wrap();
            let p = &mut app.plots[app.active];
            show_plot(ui, rect, &mut p.cfg, &mut p.state, &fonts, true);
        });
}
