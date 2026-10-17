//! Style presets: menu of the top bar and "Presets" window (save, apply,
//! default preset, import / export of `.ctp` files).

use egui::{Color32, RichText, Ui};

use super::{ERR, NAVY, OK_GREEN};
use crate::app::App;
use crate::preset::{PRESET_EXT, Preset};

/// State of the "Presets" window.
pub struct PresetUi {
    pub open: bool,
    pub name: String,
    pub description: String,
    /// What a saved preset contains.
    pub with_style: bool,
    pub with_export: bool,
    /// What applying a preset changes.
    pub apply_style: bool,
    pub apply_export: bool,
    pub message: Option<(String, bool)>,
}

impl Default for PresetUi {
    fn default() -> Self {
        PresetUi {
            open: false,
            name: String::new(),
            description: String::new(),
            with_style: true,
            with_export: true,
            apply_style: true,
            apply_export: true,
            message: None,
        }
    }
}

/// "🎨 Presets" menu of the top bar.
pub fn menu(app: &mut App, ui: &mut Ui) {
    ui.menu_button("🎨 Presets", |ui| {
        ui.visuals_mut().override_text_color = None;
        ui.label(
            RichText::new("Style templates: fonts, colors, lines, legend, export size.\nNo data, unlike a workspace.")
                .weak()
                .small(),
        );
        ui.separator();
        if app.presets.presets.is_empty() {
            ui.label(RichText::new("No preset yet").weak());
        }
        let names: Vec<(String, String)> = app
            .presets
            .presets
            .iter()
            .map(|(_, p)| (p.name.clone(), p.contents()))
            .collect();
        let has_plot = !app.plots.is_empty();
        for (name, contents) in names {
            let is_default = app.presets.default.as_deref() == Some(name.as_str());
            let label = if is_default {
                format!("★ {name}")
            } else {
                name.clone()
            };
            ui.menu_button(label, |ui| {
                ui.label(RichText::new(&contents).weak().small());
                if ui
                    .add_enabled(has_plot, egui::Button::new("Apply to the current plot"))
                    .clicked()
                {
                    app.apply_preset(&name, false, true, true);
                    ui.close();
                }
                if ui
                    .add_enabled(has_plot, egui::Button::new("Apply to all the plots"))
                    .clicked()
                {
                    app.apply_preset(&name, true, true, true);
                    ui.close();
                }
                ui.separator();
                let text = if is_default {
                    "★ Default for new plots (click to remove)"
                } else {
                    "☆ Use for new plots and workspaces"
                };
                if ui.button(text).clicked() {
                    app.presets.set_default((!is_default).then(|| name.clone()));
                    ui.close();
                }
            });
        }
        ui.separator();
        if ui.button("💾 Save the current style as a preset…").clicked() {
            app.preset_ui.open = true;
            ui.close();
        }
        if ui.button("⚙ Manage the presets…").clicked() {
            app.preset_ui.open = true;
            ui.close();
        }
    });
}

/// "Presets" window.
pub fn window(app: &mut App, ctx: &egui::Context) {
    if !app.preset_ui.open {
        return;
    }
    let mut open = true;
    egui::Window::new("🎨 Style presets")
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_width(600.0)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .show(ctx, |ui| contents(app, ui));
    app.preset_ui.open = open;
}

fn contents(app: &mut App, ui: &mut Ui) {
    ui.label(
        RichText::new(
            "A preset is a template of the look of the plots and of the export settings. It holds no data, \
             no file and no text (titles, axis names): apply it to any plot of any workspace.",
        )
        .weak(),
    );
    ui.add_space(4.0);

    // ---- save ----------------------------------------------------------------
    super::section(ui, "Save the current style", |ui| {
        let st = &mut app.preset_ui;
        egui::Grid::new("preset_save")
            .num_columns(2)
            .spacing([10.0, 6.0])
            .show(ui, |ui| {
                ui.label("Name");
                ui.add(
                    egui::TextEdit::singleline(&mut st.name)
                        .hint_text("e.g. Test report")
                        .desired_width(260.0),
                );
                ui.end_row();
                ui.label("Description");
                ui.add(
                    egui::TextEdit::singleline(&mut st.description)
                        .hint_text("optional")
                        .desired_width(260.0),
                );
                ui.end_row();
                ui.label("Contains");
                ui.vertical(|ui| {
                    ui.checkbox(&mut st.with_style, "Plot style (of the current plot)")
                        .on_hover_text(
                            "Fonts of the title, axis names, tick labels and legend; palette, line width and style, \
                     markers, temperature styles; legend position, background, border; grid and colors",
                        );
                    ui.checkbox(&mut st.with_export, "Export settings").on_hover_text(
                        "Image size (6 × 4.2 in…), resolution, slide format, PNG / SVG, SVG in the PowerPoint",
                    );
                });
                ui.end_row();
            });
        let exists = app.presets.get(app.preset_ui.name.trim()).is_some();
        ui.horizontal(|ui| {
            let text = if exists {
                "💾 Replace the preset"
            } else {
                "💾 Save the preset"
            };
            if ui
                .add(egui::Button::new(RichText::new(text).color(Color32::WHITE)).fill(NAVY))
                .clicked()
            {
                let st = &app.preset_ui;
                let (name, desc, s, e) = (st.name.clone(), st.description.clone(), st.with_style, st.with_export);
                app.preset_ui.message = Some(match app.save_preset(&name, &desc, s, e) {
                    Ok(()) => (format!("Preset “{}” saved", name.trim()), false),
                    Err(e) => (e, true),
                });
            }
            if app.plots.is_empty() && app.preset_ui.with_style {
                ui.label(RichText::new("No plot: the default style is saved").weak().small());
            }
        });
    });
    ui.add_space(6.0);

    // ---- library ---------------------------------------------------------------
    super::section(ui, "Presets", |ui| {
        ui.horizontal(|ui| {
            ui.label("Apply:");
            ui.checkbox(&mut app.preset_ui.apply_style, "plot style");
            ui.checkbox(&mut app.preset_ui.apply_export, "export settings");
        });
        if app.presets.presets.is_empty() {
            ui.label(RichText::new("No preset yet: save one above, or import a .ctp file.").weak());
        }
        let list: Vec<Preset> = app.presets.presets.iter().map(|(_, p)| p.clone()).collect();
        let has_plot = !app.plots.is_empty();
        let (a_style, a_export) = (app.preset_ui.apply_style, app.preset_ui.apply_export);
        let mut delete = None;
        egui::Grid::new("preset_list")
            .num_columns(3)
            .striped(true)
            .spacing([10.0, 6.0])
            .show(ui, |ui| {
                for p in &list {
                    let is_default = app.presets.default.as_deref() == Some(p.name.as_str());
                    ui.vertical(|ui| {
                        ui.set_min_width(150.0);
                        ui.label(RichText::new(&p.name).strong());
                        let mut info = p.contents();
                        if !p.description.is_empty() {
                            info = format!("{info} · {}", p.description);
                        }
                        ui.label(RichText::new(info).weak().small());
                    });
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(has_plot || a_export, egui::Button::new("Current plot"))
                            .on_hover_text("Apply to the plot shown in the Plots page")
                            .clicked()
                        {
                            app.apply_preset(&p.name, false, a_style, a_export);
                        }
                        if ui
                            .add_enabled(has_plot, egui::Button::new("All plots"))
                            .on_hover_text("Apply to every plot of the workspace")
                            .clicked()
                        {
                            app.apply_preset(&p.name, true, a_style, a_export);
                        }
                        if ui
                            .selectable_label(is_default, if is_default { "★ Default" } else { "☆ Default" })
                            .on_hover_text("Style of the new plots and export settings of the new workspaces")
                            .clicked()
                        {
                            app.presets.set_default((!is_default).then(|| p.name.clone()));
                        }
                    });
                    ui.horizontal(|ui| {
                        if ui
                            .small_button("Export…")
                            .on_hover_text("Save as a .ctp file to share it")
                            .clicked()
                        {
                            export_dialog(app, p);
                        }
                        if ui.small_button("🗑").on_hover_text("Delete this preset").clicked() {
                            delete = Some(p.name.clone());
                        }
                    });
                    ui.end_row();
                }
            });
        if let Some(name) = delete {
            app.presets.delete(&name);
            app.preset_ui.message = Some((format!("Preset “{name}” deleted"), false));
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui.button("📂 Import a preset file…").clicked() {
                let mut dlg = rfd::FileDialog::new()
                    .set_title("Import a CurveTracer preset")
                    .add_filter("CurveTracer preset", &[PRESET_EXT]);
                if let Some(d) = &app.export.last_dir {
                    dlg = dlg.set_directory(d);
                }
                if let Some(p) = dlg.pick_file() {
                    app.preset_ui.message = Some(match app.presets.import(&p) {
                        Ok(name) => (format!("Preset “{name}” imported"), false),
                        Err(e) => (e, true),
                    });
                }
            }
            if let Some(d) = crate::preset::dir() {
                ui.label(RichText::new(d.display().to_string()).weak().small())
                    .on_hover_text("Folder of the presets");
            }
        });
    });
    if let Some((text, err)) = &app.preset_ui.message {
        ui.label(RichText::new(text).color(if *err { ERR } else { OK_GREEN }));
    }
}

fn export_dialog(app: &mut App, p: &Preset) {
    let mut dlg = rfd::FileDialog::new()
        .set_title("Export the preset")
        .add_filter("CurveTracer preset", &[PRESET_EXT])
        .set_file_name(format!("{}.{PRESET_EXT}", crate::app::sanitize(&p.name)));
    if let Some(d) = &app.export.last_dir {
        dlg = dlg.set_directory(d);
    }
    if let Some(mut path) = dlg.save_file() {
        if path.extension().is_none() {
            path.set_extension(PRESET_EXT);
        }
        app.preset_ui.message = Some(match p.save(&path) {
            Ok(()) => (format!("Preset exported to {}", path.display()), false),
            Err(e) => (e, true),
        });
    }
}
