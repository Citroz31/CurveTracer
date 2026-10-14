//! "Functions" page: user functions = computed columns.

use egui::{Color32, RichText, Ui};

use super::{ERR, OK_GREEN, WARN, section};
use crate::app::App;
use crate::data::UserFunction;
use crate::data::expr::FUNCTIONS;

const PRESETS: &[(&str, &str)] = &[
    ("Frequency (GHz)", "[Freq] / 1e9"),
    ("Gain - Thru Loss", "[S21db] - 1.5"),
    ("|S21| (linear)", "lin20([S21db])"),
    ("DC power (W)", "[VDD Supply Voltage [V]] * [VDD Supply Current [A]]"),
];

pub fn show(app: &mut App, ui: &mut Ui) {
    egui::ScrollArea::vertical()
        .id_salt("functions_page")
        .show(ui, |ui| page(app, ui));
}

fn page(app: &mut App, ui: &mut Ui) {
    ui.heading("User Functions");
    ui.label(
        "A user function creates a new column computed from the others. It is available in every data \
         source containing the referenced columns, and can be used for X, Y, filters and series.",
    );
    ui.add_space(6.0);

    let mut remove: Option<usize> = None;
    let mut changed = false;
    let mut dirty = false;
    section(ui, "Functions", |ui| {
        if app.functions.is_empty() {
            ui.label(RichText::new("No function yet – add one below.").weak());
        }
        egui::Grid::new("functions_grid")
            .num_columns(5)
            .spacing([8.0, 6.0])
            .striped(true)
            .show(ui, |ui| {
                if !app.functions.is_empty() {
                    ui.label(RichText::new("On").strong());
                    ui.label(RichText::new("Name").strong());
                    ui.label(RichText::new("Expression").strong());
                    ui.label(RichText::new("Status").strong());
                    ui.label("");
                    ui.end_row();
                }
                for i in 0..app.functions.len() {
                    let f = &mut app.functions[i];
                    changed |= ui.checkbox(&mut f.enabled, "").changed();
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut f.name)
                            .desired_width(170.0)
                            .hint_text("Column name"),
                    );
                    dirty |= r.changed();
                    changed |= r.lost_focus();
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut f.expr)
                            .desired_width(380.0)
                            .font(egui::TextStyle::Monospace)
                            .hint_text("e.g. [Freq] / 1e9"),
                    );
                    dirty |= r.changed();
                    changed |= r.lost_focus();
                    // status over all sources
                    let mut ok = 0;
                    let mut errors: Vec<String> = Vec::new();
                    for s in &app.sources {
                        match s.func_errors.get(i) {
                            Some(None) if s.ds.is_some() => ok += 1,
                            Some(Some(e)) => errors.push(format!("{}: {e}", s.name())),
                            _ => {}
                        }
                    }
                    if app.func_dirty {
                        ui.label(RichText::new("● not applied").color(WARN));
                    } else if !f.enabled {
                        ui.label(RichText::new("disabled").weak());
                    } else if errors.is_empty() {
                        ui.label(RichText::new(format!("✔ {ok} source(s)")).color(OK_GREEN));
                    } else {
                        ui.label(RichText::new(format!("⚠ {} error(s)", errors.len())).color(if ok > 0 {
                            WARN
                        } else {
                            ERR
                        }))
                        .on_hover_text(errors.join("\n"));
                    }
                    if ui.button("🗑").on_hover_text("Delete").clicked() {
                        remove = Some(i);
                    }
                    ui.end_row();
                }
            });
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            if ui.button("+ New function").clicked() {
                app.functions.push(UserFunction {
                    name: format!("Function {}", app.functions.len() + 1),
                    ..Default::default()
                });
            }
            ui.label("Presets:");
            for (name, expr) in PRESETS {
                if ui.small_button(*name).on_hover_text(*expr).clicked() {
                    app.functions.push(UserFunction {
                        name: (*name).to_owned(),
                        expr: (*expr).to_owned(),
                        enabled: true,
                    });
                    changed = true;
                }
            }
        });
        ui.add_space(4.0);
        if ui
            .add(
                egui::Button::new(RichText::new("✔ Apply").strong()).fill(if app.func_dirty {
                    Color32::from_rgb(255, 225, 160)
                } else {
                    ui.visuals().widgets.inactive.weak_bg_fill
                }),
            )
            .on_hover_text("Recompute the user function columns of every data source")
            .clicked()
        {
            changed = true;
        }
    });
    if dirty {
        app.func_dirty = true;
    }
    if let Some(i) = remove {
        app.functions.remove(i);
        changed = true;
    }
    if changed {
        app.apply_functions();
    }

    ui.add_space(8.0);
    ui.columns(2, |cols| {
        section(&mut cols[0], "Syntax", |ui| {
            ui.label("• Columns: [Column name] – simple names (S21db, Freq) can be written without brackets.");
            ui.label("• Operators: +  −  *  /  ^ (power), parentheses, constants pi and e.");
            ui.label("• Numbers: 1.5, 1e9, -3.2e-3");
            ui.label("• Example: db20(sqrt(lin20([S21db])^2 + 1)) − [Thru Loss]");
            ui.add_space(6.0);
            egui::Grid::new("fn_ref").num_columns(2).striped(true).show(ui, |ui| {
                for (name, arity, desc) in FUNCTIONS {
                    let args = if *arity == 2 { "(a, b)" } else { "(x)" };
                    ui.label(RichText::new(format!("{name}{args}")).monospace());
                    ui.label(*desc);
                    ui.end_row();
                }
            });
        });
        let ui = &mut cols[1];
        section(ui, "Available columns", |ui| {
            if app.sources.is_empty() {
                ui.label(RichText::new("Load a data source to see its columns.").weak());
            }
            egui::ScrollArea::vertical()
                .id_salt("fn_cols")
                .max_height(420.0)
                .show(ui, |ui| {
                    for s in &app.sources {
                        if let Some(ds) = &s.base {
                            egui::CollapsingHeader::new(s.name())
                                .id_salt(("fn_src", &s.key))
                                .default_open(true)
                                .show(ui, |ui| {
                                    ui.horizontal_wrapped(|ui| {
                                        for c in &ds.columns {
                                            if ui
                                                .small_button(&c.name)
                                                .on_hover_text("Click to copy [name] to the clipboard")
                                                .clicked()
                                            {
                                                ui.ctx().copy_text(format!("[{}]", c.name));
                                            }
                                        }
                                    });
                                });
                        }
                    }
                });
        });
    });
}
