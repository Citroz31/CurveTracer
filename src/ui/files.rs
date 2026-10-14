//! "File Select" page: load data sources and manage them.

use egui::{Align, Layout, RichText, Ui};

use super::{ERR, chip, section, thousands};
use crate::app::App;
use crate::data::dataset::FileKind;
use crate::data::loader::ImportOptions;

fn delimiter_label(d: Option<char>) -> String {
    match d {
        None => "Auto".to_owned(),
        Some(',') => "Comma ,".to_owned(),
        Some(';') => "Semicolon ;".to_owned(),
        Some('\t') => "Tab".to_owned(),
        Some('|') => "Pipe |".to_owned(),
        Some(c) => format!("'{c}'"),
    }
}

/// Editor of the import options; returns true when something changed.
fn import_options_ui(ui: &mut Ui, id: &str, o: &mut ImportOptions, sheets: &[String]) -> bool {
    let mut changed = false;
    egui::Grid::new(("import_opts", id))
        .num_columns(2)
        .spacing([10.0, 4.0])
        .show(ui, |ui| {
            ui.label("Delimiter");
            egui::ComboBox::from_id_salt(("delim", id))
                .selected_text(delimiter_label(o.delimiter))
                .show_ui(ui, |ui| {
                    for d in [None, Some(','), Some(';'), Some('\t'), Some('|')] {
                        changed |= ui.selectable_value(&mut o.delimiter, d, delimiter_label(d)).changed();
                    }
                });
            ui.end_row();

            ui.label("Header line");
            ui.horizontal(|ui| {
                let mut auto = o.header_line.is_none();
                if ui.checkbox(&mut auto, "Auto").changed() {
                    o.header_line = if auto { None } else { Some(1) };
                    changed = true;
                }
                if let Some(h) = &mut o.header_line {
                    changed |= ui
                        .add(egui::DragValue::new(h).range(0..=100_000).speed(0.2))
                        .on_hover_text("1-based line number of the column names (0 = no header line)")
                        .changed();
                }
            });
            ui.end_row();

            ui.label("Decimal comma");
            egui::ComboBox::from_id_salt(("dec", id))
                .selected_text(match o.decimal_comma {
                    None => "Auto",
                    Some(true) => "Yes (1,5)",
                    Some(false) => "No (1.5)",
                })
                .show_ui(ui, |ui| {
                    changed |= ui.selectable_value(&mut o.decimal_comma, None, "Auto").changed();
                    changed |= ui
                        .selectable_value(&mut o.decimal_comma, Some(true), "Yes (1,5)")
                        .changed();
                    changed |= ui
                        .selectable_value(&mut o.decimal_comma, Some(false), "No (1.5)")
                        .changed();
                });
            ui.end_row();

            if !sheets.is_empty() {
                ui.label("Sheet");
                egui::ComboBox::from_id_salt(("sheet", id))
                    .selected_text(o.sheet.clone().unwrap_or_else(|| "First sheet".to_owned()))
                    .show_ui(ui, |ui| {
                        for s in sheets {
                            changed |= ui.selectable_value(&mut o.sheet, Some(s.clone()), s).changed();
                        }
                    });
                ui.end_row();
            }
        });
    changed
}

pub fn show(app: &mut App, ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    ui.columns(2, |cols| {
        // ---- left: add data -------------------------------------------------
        let ui = &mut cols[0];
        ui.heading("Add Data Source");
        ui.add_space(4.0);
        section(ui, "Open files", |ui| {
            ui.label("CSV / TXT (any delimiter) and Excel files (xlsx, xlsm, xlsb, xls, ods).");
            ui.add_space(4.0);
            if ui
                .add(
                    egui::Button::new(RichText::new("📂  Open data files…").size(16.0))
                        .min_size(egui::vec2(220.0, 36.0)),
                )
                .clicked()
            {
                app.open_files_dialog(&ctx);
            }
            ui.add_space(4.0);
            ui.label(RichText::new("…or drag & drop files anywhere on the window.").weak());
        });
        ui.add_space(6.0);
        section(ui, "Import options (for the next files)", |ui| {
            import_options_ui(ui, "defaults", &mut app.import_defaults, &[]);
            ui.label(
                RichText::new(
                    "Auto-detection finds the delimiter and the header line: metadata lines \
                     written above the column names are skipped and kept as file information.",
                )
                .weak()
                .small(),
            );
        });
        ui.add_space(6.0);
        section(ui, "Workspace", |ui| {
            ui.horizontal(|ui| {
                if ui.button("📁 Open workspace…").clicked() {
                    app.open_workspace_dialog(&ctx);
                }
                if ui.button("💾 Save workspace").clicked() {
                    app.save_workspace_dialog(false);
                }
            });
            ui.label(
                RichText::new("A workspace (.ctw) stores the data sources, user functions and every plot setting.")
                    .weak()
                    .small(),
            );
        });

        // ---- right: project data sources -------------------------------------
        let ui = &mut cols[1];
        ui.heading("Project Data Sources");
        ui.add_space(4.0);
        if app.sources.is_empty() {
            ui.label(RichText::new("No data source yet. Open or drop a file.").weak());
        }
        let mut add_plot_for: Option<String> = None;
        let mut remove: Option<usize> = None;
        let mut reload: Option<usize> = None;
        let mut download: Option<usize> = None;
        let mut view_data: Option<usize> = None;
        egui::ScrollArea::vertical().id_salt("sources_scroll").show(ui, |ui| {
            for i in 0..app.sources.len() {
                let n_plots = app.plots_using(&app.sources[i].key);
                let s = &mut app.sources[i];
                section(ui, &s.name(), |ui| {
                    ui.label(RichText::new(s.path.display().to_string()).small().weak());
                    ui.horizontal_wrapped(|ui| {
                        if s.loading {
                            ui.spinner();
                            ui.label("Loading…");
                        }
                        if let Some(ds) = &s.ds {
                            chip(ui, format!("{} rows", thousands(ds.n_rows)));
                            chip(ui, format!("{} columns", ds.columns.len()));
                            chip(ui, format!("{n_plots} plot(s)"));
                            match ds.info.kind {
                                FileKind::Csv => chip(
                                    ui,
                                    format!(
                                        "CSV '{}'",
                                        ds.info
                                            .delimiter
                                            .map(|c| if c == '\t' { "tab".to_owned() } else { c.to_string() })
                                            .unwrap_or_default()
                                    ),
                                ),
                                FileKind::Excel => {
                                    chip(ui, format!("Excel · {}", ds.info.sheet.clone().unwrap_or_default()))
                                }
                            }
                            chip(
                                ui,
                                match ds.info.header_line {
                                    Some(h) => format!("header line {h}"),
                                    None => "no header".to_owned(),
                                },
                            );
                            if ds.info.decimal_comma {
                                chip(ui, "decimal comma");
                            }
                            chip(ui, format!("{:.0} ms", ds.info.load_ms));
                        }
                    });
                    if let Some(e) = &s.error {
                        ui.label(RichText::new(format!("⚠ {e}")).color(ERR));
                    }
                    ui.horizontal(|ui| {
                        let loaded = s.ds.is_some();
                        if ui.add_enabled(loaded, egui::Button::new("📈 Add Plot")).clicked() {
                            add_plot_for = Some(s.key.clone());
                        }
                        if ui.add_enabled(loaded, egui::Button::new("▦ View data")).clicked() {
                            view_data = Some(i);
                        }
                        if ui.add_enabled(loaded, egui::Button::new("⬇ Download Data")).clicked() {
                            download = Some(i);
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui.button("🗑").on_hover_text("Remove this data source").clicked() {
                                remove = Some(i);
                            }
                            if ui.button("⟳").on_hover_text("Reload the file").clicked() {
                                reload = Some(i);
                            }
                            ui.toggle_value(&mut s.show_options, "⚙ Options");
                        });
                    });
                    if s.show_options {
                        let sheets = s.ds.as_ref().map(|d| d.info.sheets.clone()).unwrap_or_default();
                        let key = s.key.clone();
                        ui.separator();
                        import_options_ui(ui, &key, &mut s.options, &sheets);
                        if ui.button("Apply & reload").clicked() {
                            reload = Some(i);
                        }
                    }
                    if let Some(ds) = &s.ds
                        && !ds.preamble.is_empty()
                    {
                        egui::CollapsingHeader::new(format!("File information ({} lines)", ds.preamble.len()))
                            .id_salt(("preamble", i))
                            .show(ui, |ui| {
                                for l in ds.preamble.iter().take(200) {
                                    ui.label(RichText::new(l).monospace().small());
                                }
                            });
                    }
                });
                ui.add_space(6.0);
            }
        });
        if let Some(k) = add_plot_for {
            app.add_plot(&k);
        }
        if let Some(i) = reload {
            app.reload_source(i, &ctx);
        }
        if let Some(i) = download {
            app.save_dataset_csv_dialog(i);
        }
        if let Some(i) = view_data {
            app.data_page.source = i;
            app.page = crate::app::Page::Data;
        }
        if let Some(i) = remove {
            app.remove_source(i);
        }
    });
}
