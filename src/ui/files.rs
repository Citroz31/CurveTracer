//! "File Select" page: load data sources and manage them.

use egui::{Align, Layout, RichText, Ui};

use super::{ERR, OK_GREEN, WARN, chip, section, thousands};
use crate::app::{App, DataSource};
use crate::data::dataset::FileKind;
use crate::data::deembed::{
    Correction, DeembedMode, ExtraCorrection, Fixture, FreqUnit, find_freq_column, find_sparam_columns,
};
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

/// Picker of a fixture file; returns true when something changed.
fn fixture_ui(
    ui: &mut Ui,
    label: &str,
    hint: &str,
    fx: &mut Fixture,
    last_dir: &mut Option<std::path::PathBuf>,
) -> bool {
    let mut changed = false;
    ui.label(label).on_hover_text(hint);
    ui.horizontal(|ui| {
        let name = fx
            .path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned());
        match &name {
            Some(n) => {
                ui.label(RichText::new(n).strong())
                    .on_hover_text(fx.path.as_ref().map(|p| p.display().to_string()).unwrap_or_default());
            }
            None => {
                ui.label(RichText::new("none").weak());
            }
        }
        if ui.button("📂 .s2p…").on_hover_text(hint).clicked() {
            let mut dlg = rfd::FileDialog::new()
                .set_title(format!("{label}: Touchstone file"))
                .add_filter("Touchstone two-port", &["s2p", "S2P"])
                .add_filter("All files", &["*"]);
            if let Some(d) = last_dir.as_ref() {
                dlg = dlg.set_directory(d);
            }
            if let Some(p) = dlg.pick_file() {
                if let Some(parent) = p.parent() {
                    *last_dir = Some(parent.to_path_buf());
                }
                fx.path = Some(p);
                changed = true;
            }
        }
        if name.is_some() && ui.button("🗙").on_hover_text("No fixture on this side").clicked() {
            fx.path = None;
            changed = true;
        }
        changed |= ui
            .checkbox(&mut fx.flip, "Flip ports")
            .on_hover_text("Swap the ports of the file (S11 ↔ S22, S21 ↔ S12) when it was measured the other way round")
            .changed();
    });
    ui.end_row();
    changed
}

/// De-embedding settings of a data source.
/// Returns (settings changed, apply to every source).
fn deembed_ui(ui: &mut Ui, s: &mut DataSource, last_dir: &mut Option<std::path::PathBuf>) -> (bool, bool) {
    let mut changed = false;
    let mut apply_all = false;
    let id = s.key.clone();
    let cfg = &mut s.deembed;
    ui.horizontal(|ui| {
        ui.label(RichText::new("De-embedding of the fixtures").strong());
        changed |= ui.checkbox(&mut cfg.enabled, "Enabled").changed();
    });
    egui::Grid::new(("deembed", &id))
        .num_columns(2)
        .spacing([10.0, 4.0])
        .show(ui, |ui| {
            changed |= fixture_ui(
                ui,
                "Input fixture",
                "Losses between the instrument and the DUT input: port 1 = instrument, port 2 = DUT",
                &mut cfg.input,
                last_dir,
            );
            changed |= fixture_ui(
                ui,
                "Output fixture",
                "Losses between the DUT output and the instrument: port 1 = DUT, port 2 = instrument",
                &mut cfg.output,
                last_dir,
            );

            ui.label("Mode");
            egui::ComboBox::from_id_salt(("deemb_mode", &id))
                .selected_text(cfg.mode.label())
                .width(260.0)
                .show_ui(ui, |ui| {
                    for m in DeembedMode::ALL {
                        changed |= ui.selectable_value(&mut cfg.mode, m, m.label()).changed();
                    }
                });
            ui.end_row();

            let cols: Vec<String> = s
                .base
                .as_ref()
                .map(|b| {
                    b.columns
                        .iter()
                        .filter(|c| c.has_numbers())
                        .map(|c| c.name.clone())
                        .collect()
                })
                .unwrap_or_default();
            ui.label("Frequency");
            ui.horizontal(|ui| {
                let auto_name = s
                    .base
                    .as_ref()
                    .and_then(|b| find_freq_column(b, "").map(|i| b.columns[i].name.clone()));
                let shown = if cfg.freq_column.is_empty() {
                    format!("Auto ({})", auto_name.unwrap_or_else(|| "not found".to_owned()))
                } else {
                    cfg.freq_column.clone()
                };
                egui::ComboBox::from_id_salt(("deemb_freq", &id))
                    .selected_text(shown)
                    .width(170.0)
                    .show_ui(ui, |ui| {
                        changed |= ui
                            .selectable_value(&mut cfg.freq_column, String::new(), "Auto")
                            .changed();
                        for c in &cols {
                            changed |= ui.selectable_value(&mut cfg.freq_column, c.clone(), c).changed();
                        }
                    });
                egui::ComboBox::from_id_salt(("deemb_unit", &id))
                    .selected_text(cfg.freq_unit.label())
                    .width(60.0)
                    .show_ui(ui, |ui| {
                        for u in FreqUnit::ALL {
                            changed |= ui.selectable_value(&mut cfg.freq_unit, u, u.label()).changed();
                        }
                    });
            });
            ui.end_row();

            ui.label("Other columns").on_hover_text(
                "Columns corrected with the fixture insertion losses (IL, dB, read from S21 of the files)",
            );
            ui.vertical(|ui| {
                let mut remove = None;
                for (k, e) in cfg.extra.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        egui::ComboBox::from_id_salt(("deemb_xcol", &id, k))
                            .selected_text(if e.column.is_empty() {
                                "column…"
                            } else {
                                e.column.as_str()
                            })
                            .width(170.0)
                            .show_ui(ui, |ui| {
                                for c in &cols {
                                    changed |= ui.selectable_value(&mut e.column, c.clone(), c).changed();
                                }
                            });
                        egui::ComboBox::from_id_salt(("deemb_xkind", &id, k))
                            .selected_text(e.kind.label())
                            .width(190.0)
                            .show_ui(ui, |ui| {
                                for c in Correction::ALL {
                                    changed |= ui.selectable_value(&mut e.kind, c, c.label()).changed();
                                }
                            });
                        if ui.small_button("🗙").clicked() {
                            remove = Some(k);
                        }
                    });
                }
                if let Some(k) = remove {
                    cfg.extra.remove(k);
                    changed = true;
                }
                if ui.small_button("+ Add a column (power, gain, NF…)").clicked() {
                    cfg.extra.push(ExtraCorrection::default());
                }
            });
            ui.end_row();
        });
    changed |= ui
        .checkbox(&mut cfg.keep_raw, "Keep the raw columns (“S21db raw”…)")
        .changed();

    // detected S-parameter columns
    if let Some(b) = &s.base {
        let (db, deg) = find_sparam_columns(b);
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Detected:").small());
            for (k, p) in ["S11", "S21", "S12", "S22"].iter().enumerate() {
                let txt = match (db[k], deg[k]) {
                    (Some(_), Some(_)) => format!("{p} dB+deg"),
                    (Some(_), None) => format!("{p} dB"),
                    (None, Some(_)) => format!("{p} deg"),
                    (None, None) => format!("{p} –"),
                };
                let color = match (db[k], deg[k]) {
                    (Some(_), Some(_)) => OK_GREEN,
                    (None, None) => ui.visuals().weak_text_color(),
                    _ => WARN,
                };
                ui.label(RichText::new(txt).small().color(color));
            }
        });
    }
    match &s.deembed_report {
        Some(Ok(r)) => {
            ui.label(RichText::new(format!("✔ {} · frequency {}", r.summary(), r.freq_column)).color(OK_GREEN));
            for f in &r.fixtures {
                ui.label(RichText::new(f).small());
            }
            ui.label(
                RichText::new(format!("Corrected: {}", r.corrected.join(", ")))
                    .small()
                    .weak(),
            );
            for n in &r.notes {
                ui.label(RichText::new(format!("⚠ {n}")).small().color(WARN));
            }
        }
        Some(Err(e)) => {
            ui.label(RichText::new(format!("⚠ {e}")).color(ERR));
        }
        None => {}
    }
    ui.horizontal(|ui| {
        if ui
            .button("⟳ Re-apply")
            .on_hover_text("Read the fixture files again")
            .clicked()
        {
            changed = true;
        }
        if ui
            .button("Use these fixtures for every data source")
            .on_hover_text("Copy these de-embedding settings to all the other data sources")
            .clicked()
        {
            apply_all = true;
        }
    });
    (changed, apply_all)
}

pub fn show(app: &mut App, ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    ui.columns(2, |cols| {
        // ---- left: add data -------------------------------------------------
        let ui = &mut cols[0];
        ui.heading("Add Data Source");
        ui.add_space(4.0);
        section(ui, "Open files", |ui| {
            ui.label("CSV / TXT (any delimiter), Excel files (xlsx, xlsm, xlsb, xls, ods) and Touchstone files (.s1p … .snp).");
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
                    app.request_switch(crate::app::Switch::OpenDialog, &ctx);
                }
                if ui.button("💾 Save workspace").clicked() {
                    app.save_workspace_dialog(false);
                }
                if ui
                    .button("🏠 Start screen")
                    .on_hover_text("Create or open another workspace")
                    .clicked()
                {
                    app.request_switch(crate::app::Switch::Close, &ctx);
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
        let mut deembed_changed: Option<usize> = None;
        let mut deembed_all: Option<usize> = None;
        let mut last_dir = app.export.last_dir.clone();
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
                        } else if s.rebuilding {
                            ui.spinner();
                            ui.label("Updating…");
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
                                FileKind::Touchstone(n) => chip(ui, format!("Touchstone · {n} port(s)")),
                            }
                            if !matches!(ds.info.kind, FileKind::Touchstone(_)) {
                                chip(
                                    ui,
                                    match ds.info.header_line {
                                        Some(h) => format!("header line {h}"),
                                        None => "no header".to_owned(),
                                    },
                                );
                            }
                            if ds.info.decimal_comma {
                                chip(ui, "decimal comma");
                            }
                            chip(ui, format!("{:.0} ms", ds.info.load_ms));
                        }
                        match &s.deembed_report {
                            Some(Ok(r)) => {
                                ui.label(
                                    RichText::new(if r.vector {
                                        "De-embedded · vector"
                                    } else {
                                        "De-embedded · scalar"
                                    })
                                    .small()
                                    .color(OK_GREEN),
                                )
                                .on_hover_text(r.fixtures.join("\n"));
                            }
                            Some(Err(_)) => {
                                ui.label(RichText::new("De-embedding error").small().color(ERR));
                            }
                            None => {}
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
                            ui.toggle_value(&mut s.show_deembed, "🔌 De-embedding")
                                .on_hover_text("Remove the losses of the input / output fixtures (.s2p files)");
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
                    if s.show_deembed {
                        ui.separator();
                        let (changed, all) = deembed_ui(ui, s, &mut last_dir);
                        if changed {
                            deembed_changed = Some(i);
                        }
                        if all {
                            deembed_all = Some(i);
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
        if last_dir != app.export.last_dir {
            app.export.last_dir = last_dir;
        }
        if let Some(i) = deembed_changed {
            app.apply_deembed(i);
        }
        if let Some(i) = deembed_all {
            let cfg = app.sources[i].deembed.clone();
            for j in 0..app.sources.len() {
                if j != i {
                    app.sources[j].deembed = cfg.clone();
                    app.apply_deembed(j);
                }
            }
        }
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
