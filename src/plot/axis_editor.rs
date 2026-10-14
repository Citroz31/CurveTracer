//! Small editor opened by a click on an axis or on the title of a plot:
//! axis title, range, tick step, grid; plot title.

use egui::{RichText, Ui};

use super::config::{AxisLayout, PlotConfig};
use super::view::EditTarget;

fn axis(ui: &mut Ui, id: &str, a: &mut AxisLayout, auto_label: &str, range: Option<(f64, f64)>) {
    egui::Grid::new(("axis_editor", id))
        .num_columns(2)
        .spacing([10.0, 6.0])
        .show(ui, |ui| {
            ui.label("Title");
            let r = ui.add(
                egui::TextEdit::singleline(&mut a.label)
                    .hint_text(auto_label)
                    .desired_width(230.0),
            );
            if a.label.is_empty() && !r.has_focus() {
                r.on_hover_text("Empty = automatic title (column name and unit)");
            }
            ui.end_row();

            ui.label("Range");
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    if ui.checkbox(&mut a.auto_range, "Auto").changed()
                        && !a.auto_range
                        && let Some((lo, hi)) = range
                    {
                        a.min = lo;
                        a.max = hi;
                    }
                    if let Some((lo, hi)) = range
                        && ui
                            .small_button("Use current zoom")
                            .on_hover_text("Keep the range shown now")
                            .clicked()
                    {
                        a.auto_range = false;
                        a.min = lo;
                        a.max = hi;
                    }
                });
                ui.add_enabled_ui(!a.auto_range, |ui| {
                    ui.horizontal(|ui| {
                        let speed = ((a.max - a.min).abs() / 200.0).max(1e-9);
                        ui.label("min");
                        ui.add(egui::DragValue::new(&mut a.min).speed(speed).max_decimals(6));
                        ui.label("max");
                        ui.add(egui::DragValue::new(&mut a.max).speed(speed).max_decimals(6));
                    });
                });
                if !a.auto_range && a.min >= a.max {
                    ui.label(
                        RichText::new("⚠ min must be lower than max")
                            .color(crate::ui::ERR)
                            .small(),
                    );
                }
            });
            ui.end_row();

            ui.label("Tick step");
            ui.horizontal(|ui| {
                ui.checkbox(&mut a.auto_ticks, "Auto");
                let speed = (a.step.abs() * 0.02).max(1e-9);
                ui.add_enabled(
                    !a.auto_ticks,
                    egui::DragValue::new(&mut a.step)
                        .range(1e-12..=1e15)
                        .speed(speed)
                        .max_decimals(6),
                );
            });
            ui.end_row();

            ui.label("Grid");
            ui.checkbox(&mut a.grid, "Show the grid lines");
            ui.end_row();
        });
}

/// Editor content for `target`. `auto` = automatic (x, y) axis titles,
/// `view` = visible range (xmin, xmax, ymin, ymax).
pub fn show(ui: &mut Ui, cfg: &mut PlotConfig, target: EditTarget, auto: &(String, String), view: Option<[f64; 4]>) {
    let lay = &mut cfg.layout;
    let all = target == EditTarget::All;
    if target == EditTarget::Title || all {
        if all {
            ui.label(RichText::new("Title").strong());
        }
        ui.horizontal(|ui| {
            ui.checkbox(&mut lay.show_title, "Show");
            let r = ui.add(
                egui::TextEdit::singleline(&mut lay.title)
                    .hint_text("Plot title")
                    .desired_width(260.0),
            );
            if r.changed() && !lay.title.trim().is_empty() {
                lay.show_title = true;
            }
        });
        ui.horizontal(|ui| {
            ui.label("Size");
            ui.add(
                egui::DragValue::new(&mut lay.title_font.size)
                    .range(6.0..=60.0)
                    .speed(0.2),
            );
        });
    }
    if target == EditTarget::X || all {
        if all {
            ui.separator();
            ui.label(RichText::new("X axis").strong());
        }
        axis(ui, "x", &mut lay.x, &auto.0, view.map(|v| (v[0], v[1])));
    }
    if target == EditTarget::Y || all {
        if all {
            ui.separator();
            ui.label(RichText::new("Y axis").strong());
        }
        axis(ui, "y", &mut lay.y, &auto.1, view.map(|v| (v[2], v[3])));
    }
    if target != EditTarget::Title {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label("Axis titles size");
            ui.add(
                egui::DragValue::new(&mut lay.label_font.size)
                    .range(6.0..=48.0)
                    .speed(0.2),
            );
            ui.label("Ticks size");
            ui.add(
                egui::DragValue::new(&mut lay.tick_font.size)
                    .range(5.0..=40.0)
                    .speed(0.2),
            );
        });
    }
    ui.label(
        RichText::new("More settings (fonts, colors, decimals…) in the Layout tab.")
            .small()
            .weak(),
    );
}
