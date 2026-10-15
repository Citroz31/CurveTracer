//! "Export Data" page: choose the data (columns, frequency band, criteria),
//! the smoothing of every column, the formats (CSV / Touchstone .s2p) and the
//! sorting of the files into folders; preview of the files and of the
//! smoothing; export in the background.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, channel};

use egui::{Color32, RichText, Ui};
use egui_plot::{Legend, Line, Plot, PlotPoints, Points, VLine};

use super::plot_settings::{filter_editor, new_filter, smoothing_editor};
use super::{ERR, NAVY, OK_GREEN, WARN, section, thousands};
use crate::app::App;
use crate::data::dataset::{Dataset, format_sig};
use crate::data::deembed::{FreqUnit, find_freq_column, find_sparam_columns};
use crate::data::with_functions;
use crate::export::data::{
    self as dx, CsvSep, DataExportConfig, ExportSummary, Plan, Progress, SPARAM_NAMES, SmoothChoice, SplitLevel,
    TsFormat, is_phase,
};

struct Job {
    progress: Arc<Progress>,
    rx: Receiver<Result<ExportSummary, String>>,
    dir: PathBuf,
}

/// State of the page (not saved).
#[derive(Default)]
pub struct DataExportState {
    plan: Option<Arc<Plan>>,
    plan_error: Option<String>,
    /// Key of `plan` / `plan_error`.
    plan_key: u64,
    pending: Option<(u64, Receiver<Result<Plan, String>>)>,
    job: Option<Job>,
    last: Option<(Result<ExportSummary, String>, PathBuf)>,
    /// Source the default settings were chosen for.
    inited_for: String,
    /// (settings before, after) the automatic defaults: not a change of the
    /// user (no "unsaved changes" for just opening the page).
    pub auto_defaults: Option<(DataExportConfig, DataExportConfig)>,
    /// Raw dataset (no de-embedding) with the user functions: (base generation, dataset).
    raw: Option<(u64, Arc<Dataset>)>,
    col_search: String,
    filter_search: Vec<String>,
    preview_col: String,
    preview_curve: usize,
    /// Column whose custom smoothing is being edited.
    editing: Option<String>,
    show_sparams: bool,
    /// (key, file list) of the preview.
    files: Option<(u64, Vec<(usize, String)>)>,
    /// (key, curve preview): the smoothing is computed once per change.
    curve_preview: Option<(u64, Arc<dx::Preview>)>,
}

impl DataExportState {
    pub fn is_busy(&self) -> bool {
        self.pending.is_some() || self.job.is_some()
    }
}

/// Receives the result of a finished export.
pub fn poll(app: &mut App) {
    let Some(job) = &app.dx.job else {
        return;
    };
    let Ok(res) = job.rx.try_recv() else {
        return;
    };
    let dir = job.dir.clone();
    app.dx.job = None;
    match &res {
        Ok(s) => app.set_status(
            format!(
                "{} file(s) written to {} in {:.1} s",
                s.csv_files + s.s2p_files,
                dir.display(),
                s.ms / 1000.0
            ),
            false,
        ),
        Err(e) => app.set_status(e.clone(), true),
    }
    app.dx.last = Some((res, dir));
}

/// Dataset of the export and the description of the source.
fn export_dataset(app: &mut App) -> Option<(Arc<Dataset>, Vec<String>)> {
    let s = app.sources.iter().find(|s| s.key == app.data_export.source)?;
    let mut info = vec![format!("Source: {}", s.path.display())];
    let deembedded = s.deembedded.is_some();
    let ds = if app.data_export.raw && deembedded {
        info.push("De-embedding: not applied (raw data)".to_owned());
        let base = s.base.clone()?;
        match &app.dx.raw {
            Some((g, ds)) if *g == base.generation => ds.clone(),
            _ => {
                let (ds, _) = with_functions(&base, &app.functions);
                let ds = Arc::new(ds);
                app.dx.raw = Some((base.generation, ds.clone()));
                ds
            }
        }
    } else {
        match &s.deembed_report {
            Some(Ok(r)) if deembedded => {
                let mut line = format!("De-embedding: {}", r.summary());
                if !r.fixtures.is_empty() {
                    line.push_str(&format!(" ({})", r.fixtures.join("; ")));
                }
                info.push(line);
            }
            _ => info.push("De-embedding: none".to_owned()),
        }
        s.ds.clone()?
    };
    Some((ds, info))
}

/// Default settings for a data source: frequency X, S-parameter columns.
fn init_for(cfg: &mut DataExportConfig, ds: &Dataset) {
    let numeric = |i: usize| ds.columns[i].has_numbers();
    let x = find_freq_column(ds, "").or_else(|| (0..ds.columns.len()).find(|&i| numeric(i)));
    cfg.x = x.map(|i| ds.columns[i].name.clone()).unwrap_or_default();
    let (db, deg) = find_sparam_columns(ds);
    let sp: Vec<usize> = (0..4).flat_map(|k| [db[k], deg[k]]).flatten().collect();
    cfg.columns = if sp.is_empty() {
        (0..ds.columns.len())
            .filter(|&i| Some(i) != x && numeric(i))
            .map(|i| ds.columns[i].name.clone())
            .collect()
    } else {
        sp.iter().map(|&i| ds.columns[i].name.clone()).collect()
    };
    cfg.s2p = !sp.is_empty();
    cfg.x_min = None;
    cfg.x_max = None;
    cfg.filters.clear();
    cfg.split.clear();
    cfg.curve_keys.clear();
    cfg.auto_keys = true;
    cfg.sparams = Default::default();
}

pub fn show(app: &mut App, ui: &mut Ui) {
    let loaded: Vec<(String, String)> = app
        .sources
        .iter()
        .filter(|s| s.ds.is_some())
        .map(|s| (s.key.clone(), s.name()))
        .collect();
    if loaded.is_empty() {
        ui.heading("Export Data");
        ui.add_space(6.0);
        ui.label("Load a data source first (File Select page).");
        return;
    }
    if !loaded.iter().any(|(k, _)| *k == app.data_export.source) {
        app.data_export.source = loaded[0].0.clone();
    }
    let Some((ds, info)) = export_dataset(app) else {
        ui.spinner();
        return;
    };
    if app.dx.inited_for != app.data_export.source {
        // keep the settings of a workspace that were made for this source
        if app.data_export.x.is_empty() || ds.column(&app.data_export.x).is_none() {
            let before = app.data_export.clone();
            init_for(&mut app.data_export, &ds);
            app.dx.auto_defaults = Some((before, app.data_export.clone()));
        }
        app.dx.inited_for = app.data_export.source.clone();
        app.dx.preview_col.clear();
    }
    update_plan(app, &ds, ui.ctx());

    egui::Panel::left("ct_dx_settings")
        .resizable(true)
        .default_size(560.0)
        .size_range(420.0..=900.0)
        .show(ui, |ui| {
            // export button and result always visible under the settings
            egui::Panel::bottom("ct_dx_export_bar")
                .frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(0, 6)))
                .show(ui, |ui| {
                    let plan = current_plan(app, &ds);
                    export_button(app, ui, &ds, plan.as_deref());
                });
            super::fixed_area(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("dx_settings_scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width() - 6.0);
                        settings(app, ui, &ds, &loaded);
                    });
            })
        });
    super::fixed_area(ui, |ui| {
        egui::ScrollArea::vertical()
            .id_salt("dx_preview_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| preview_panel(app, ui, &ds, &info));
    });
}

/// (Re)computes the curves and files in the background when the settings change.
fn update_plan(app: &mut App, ds: &Arc<Dataset>, ctx: &egui::Context) {
    let key = app.data_export.plan_key(ds.generation);
    if let Some((k, rx)) = &app.dx.pending {
        match rx.try_recv() {
            Ok(res) => {
                let k = *k;
                app.dx.pending = None;
                app.dx.plan_key = k;
                match res {
                    Ok(p) => {
                        app.dx.plan = Some(Arc::new(p));
                        app.dx.plan_error = None;
                    }
                    Err(e) => {
                        app.dx.plan = None;
                        app.dx.plan_error = Some(e);
                    }
                }
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(_) => app.dx.pending = None,
        }
    }
    if app.dx.plan_key == key && (app.dx.plan.is_some() || app.dx.plan_error.is_some()) {
        return;
    }
    let (tx, rx) = channel();
    let ds = ds.clone();
    let cfg = app.data_export.clone();
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let res = std::panic::catch_unwind(|| dx::build_plan(&ds, &cfg))
            .unwrap_or_else(|_| Err("Unexpected error while preparing the export".to_owned()));
        let _ = tx.send(res);
        ctx.request_repaint();
    });
    app.dx.pending = Some((key, rx));
}

/// The current plan, if it matches the settings.
fn current_plan(app: &App, ds: &Dataset) -> Option<Arc<Plan>> {
    let p = app.dx.plan.as_ref()?;
    (p.key == app.data_export.plan_key(ds.generation)).then(|| p.clone())
}

fn column_combo(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    value: &mut String,
    names: &[String],
    width: f32,
) -> bool {
    let mut changed = false;
    egui::ComboBox::from_id_salt(id)
        .selected_text(if value.is_empty() { "—" } else { value.as_str() })
        .width(width)
        .height(400.0)
        .show_ui(ui, |ui| {
            for n in names {
                if ui.selectable_label(value == n, n).clicked() && value != n {
                    *value = n.clone();
                    changed = true;
                }
            }
        });
    changed
}

/// Display factor and unit of the X values (frequencies shown in GHz…).
fn x_display(cfg: &DataExportConfig, ds: &Dataset) -> (f64, String) {
    let Some(xi) = ds.column_index(&cfg.x) else {
        return (1.0, String::new());
    };
    if find_freq_column(ds, "") != Some(xi) && !cfg.x.to_lowercase().contains("freq") {
        return (1.0, String::new());
    }
    let out = if cfg.ts_unit == FreqUnit::Auto {
        FreqUnit::GHz
    } else {
        cfg.ts_unit
    };
    (
        cfg.data_unit(&ds.columns[xi]).mult() / out.mult(),
        out.label().to_owned(),
    )
}

fn settings(app: &mut App, ui: &mut Ui, ds: &Arc<Dataset>, loaded: &[(String, String)]) {
    let plan = current_plan(app, ds);
    let names: Vec<String> = ds.columns.iter().map(|c| c.name.clone()).collect();
    let numeric: Vec<String> = ds
        .columns
        .iter()
        .filter(|c| c.has_numbers())
        .map(|c| c.name.clone())
        .collect();
    let deembed_state = app
        .sources
        .iter()
        .find(|s| s.key == app.data_export.source)
        .map(|s| (s.deembed.is_active(), s.deembedded.is_some(), s.deembed_report.clone()));
    let cfg = &mut app.data_export;
    let st = &mut app.dx;

    ui.heading("Export Data");
    ui.label(
        RichText::new("Write the data of your choice as CSV and / or Touchstone (.s2p) files, smoothed or not.").weak(),
    );
    ui.add_space(4.0);

    // ---- 1. data -------------------------------------------------------------
    section(ui, "1 · Data", |ui| {
        egui::Grid::new("dx_data")
            .num_columns(2)
            .spacing([10.0, 6.0])
            .show(ui, |ui| {
                ui.label("Data source");
                let current = loaded
                    .iter()
                    .find(|(k, _)| *k == cfg.source)
                    .map(|(_, n)| n.clone())
                    .unwrap_or_default();
                egui::ComboBox::from_id_salt("dx_source")
                    .selected_text(current)
                    .width(300.0)
                    .show_ui(ui, |ui| {
                        for (k, n) in loaded {
                            ui.selectable_value(&mut cfg.source, k.clone(), n);
                        }
                    });
                ui.end_row();
                ui.label("De-embedding");
                ui.vertical(|ui| match &deembed_state {
                    Some((true, true, Some(Ok(r)))) => {
                        ui.label(RichText::new(format!("✔ {}", r.summary())).color(OK_GREEN));
                        ui.checkbox(&mut cfg.raw, "Export the raw data instead (without the de-embedding)");
                    }
                    Some((true, _, Some(Err(e)))) => {
                        ui.label(RichText::new(format!("⚠ {e}: raw data exported")).color(WARN));
                    }
                    Some((true, _, _)) => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label("updating…");
                        });
                    }
                    _ => {
                        ui.label(RichText::new("None (File Select → De-embedding)").weak());
                    }
                });
                ui.end_row();
                ui.label("X (frequency…)");
                column_combo(ui, "dx_x", &mut cfg.x, &numeric, 300.0);
                ui.end_row();
            });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Columns to export").strong());
            ui.label(RichText::new(format!("({} selected)", cfg.columns.len())).weak());
        });
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut st.col_search)
                    .hint_text("search…")
                    .desired_width(140.0),
            );
            let (db, deg) = find_sparam_columns(ds);
            let sp: Vec<String> = (0..4)
                .flat_map(|k| [db[k], deg[k]])
                .flatten()
                .map(|i| ds.columns[i].name.clone())
                .collect();
            if !sp.is_empty()
                && ui
                    .small_button("S-parameters")
                    .on_hover_text("S11, S21, S12, S22 (dB and phase)")
                    .clicked()
            {
                cfg.columns = sp;
            }
            if ui.small_button("All numbers").clicked() {
                cfg.columns = numeric.iter().filter(|n| **n != cfg.x).cloned().collect();
            }
            if ui.small_button("None").clicked() {
                cfg.columns.clear();
            }
        });
        let s = st.col_search.to_lowercase();
        let shown: Vec<&String> = names
            .iter()
            .filter(|n| **n != cfg.x && (s.is_empty() || n.to_lowercase().contains(&s)))
            .collect();
        let row_h = ui.spacing().interact_size.y;
        egui::ScrollArea::vertical()
            .id_salt("dx_columns")
            .max_height(170.0)
            .auto_shrink([false, true])
            .show_rows(ui, row_h, shown.len(), |ui, range| {
                for n in &shown[range] {
                    let mut on = cfg.columns.contains(n);
                    let col = ds.column(n);
                    let kind = col.map_or("", |c| c.kind.short());
                    ui.horizontal(|ui| {
                        if ui.checkbox(&mut on, n.as_str()).changed() {
                            if on {
                                // keep the order of the data
                                cfg.columns.push((*n).clone());
                                let order = |c: &String| ds.column_index(c).unwrap_or(usize::MAX);
                                cfg.columns.sort_by_key(order);
                            } else {
                                cfg.columns.retain(|c| c != *n);
                            }
                        }
                        ui.label(RichText::new(kind).weak().small());
                    });
                }
            });
    });
    ui.add_space(6.0);

    // ---- 2. selection --------------------------------------------------------
    let (xf, xunit) = x_display(cfg, ds);
    section(ui, "2 · Selection (band, criteria)", |ui| {
        let xc = ds.column(&cfg.x);
        ui.horizontal(|ui| {
            ui.label(RichText::new("X band").strong()).on_hover_text(
                "Only the points in this band are written. Smoothing uses the whole curve, so the band \
                 limits are not distorted.",
            );
            for (label, v, default) in [
                ("from", &mut cfg.x_min, xc.map_or(0.0, |c| c.min)),
                ("to", &mut cfg.x_max, xc.map_or(0.0, |c| c.max)),
            ] {
                let mut on = v.is_some();
                if ui.checkbox(&mut on, label).changed() {
                    *v = on.then_some(if default.is_finite() { default } else { 0.0 });
                }
                match v {
                    Some(x) => {
                        let mut shown = *x * xf;
                        let speed = xc.map_or(0.01, |c| ((c.max - c.min).abs() * xf / 500.0).max(1e-9));
                        if ui
                            .add(egui::DragValue::new(&mut shown).speed(speed).max_decimals(6))
                            .changed()
                        {
                            *x = shown / xf;
                        }
                    }
                    None => {
                        ui.label(RichText::new("—").weak());
                    }
                }
            }
            ui.label(&xunit);
        });
        if let Some(c) = xc.filter(|c| c.min.is_finite()) {
            ui.label(
                RichText::new(format!(
                    "data: {} … {} {xunit}",
                    format_sig(c.min * xf, 6),
                    format_sig(c.max * xf, 6)
                ))
                .weak()
                .small(),
            );
        }
        ui.add_space(4.0);
        ui.label(RichText::new("Criteria").strong())
            .on_hover_text("Keep only some values (SN, temperature…) or a range (input power…)");
        st.filter_search.resize(cfg.filters.len(), String::new());
        let mut remove = None;
        for (k, f) in cfg.filters.iter_mut().enumerate() {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.checkbox(&mut f.enabled, RichText::new(&f.column).strong());
                    if ui.small_button("🗙").on_hover_text("Remove this criterion").clicked() {
                        remove = Some(k);
                    }
                });
                match ds.column(&f.column) {
                    Some(col) if f.enabled => filter_editor(ui, ("dx", k), f, col, &mut st.filter_search[k]),
                    Some(_) => {}
                    None => {
                        ui.label(RichText::new("column not found").color(ERR));
                    }
                }
            });
        }
        if let Some(k) = remove {
            cfg.filters.remove(k);
            st.filter_search.remove(k);
        }
        let mut add = String::new();
        if column_combo(ui, "dx_add_filter", &mut add, &names, 220.0)
            && let Some(col) = ds.column(&add)
        {
            cfg.filters.push(new_filter(col));
        }
        ui.label(RichText::new("↑ add a criterion (choose a column)").weak().small());
    });
    ui.add_space(6.0);

    // ---- 3. files and curves ---------------------------------------------------
    section(ui, "3 · Sorting of the files", |ui| {
        ui.label(
            RichText::new(
                "One folder or one file per value of each column, in this order (e.g. a folder per SN, a file \
                 per temperature).",
            )
            .weak(),
        );
        let mut action: Option<(usize, i32)> = None;
        let n = cfg.split.len();
        for (k, lvl) in cfg.split.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.label(format!("{}.", k + 1));
                column_combo(ui, ("dx_split", k), &mut lvl.column, &names, 200.0);
                ui.selectable_value(&mut lvl.folder, true, "📁 Folder");
                ui.selectable_value(&mut lvl.folder, false, "📄 File name");
                if ui.add_enabled(k > 0, egui::Button::new("⏶").small()).clicked() {
                    action = Some((k, -1));
                }
                if ui.add_enabled(k + 1 < n, egui::Button::new("⏷").small()).clicked() {
                    action = Some((k, 1));
                }
                if ui.small_button("🗙").clicked() {
                    action = Some((k, 0));
                }
            });
        }
        match action {
            Some((k, 0)) => {
                cfg.split.remove(k);
            }
            Some((k, d)) => cfg.split.swap(k, (k as i32 + d) as usize),
            None => {}
        }
        ui.horizontal(|ui| {
            let mut add = String::new();
            if column_combo(ui, "dx_add_split", &mut add, &names, 200.0) {
                let folder = cfg.split.is_empty();
                cfg.split.push(SplitLevel { column: add, folder });
            }
            ui.label(RichText::new("← add a level").weak().small());
        });
        ui.checkbox(
            &mut cfg.with_names,
            "Column names in the folder / file names (SN_3008260)",
        );
        ui.horizontal(|ui| {
            ui.label("File names start with");
            let hint = std::path::Path::new(&ds.name)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            ui.add(
                egui::TextEdit::singleline(&mut cfg.prefix)
                    .hint_text(hint)
                    .desired_width(200.0),
            );
        });
        ui.add_space(4.0);
        ui.label(RichText::new("Curves").strong()).on_hover_text(
            "A curve = the points sharing the same values of these columns, sorted by X. Each curve is \
             smoothed separately and written as one .s2p file.",
        );
        let split_names: Vec<String> = cfg.split.iter().map(|s| s.column.clone()).collect();
        if ui
            .checkbox(&mut cfg.auto_keys, "Find the columns defining a curve automatically")
            .changed()
            && !cfg.auto_keys
        {
            cfg.curve_keys = plan
                .as_ref()
                .map(|p| {
                    p.key_cols[p.n_split..]
                        .iter()
                        .map(|&c| ds.columns[c].name.clone())
                        .collect()
                })
                .unwrap_or_default();
        }
        ui.horizontal_wrapped(|ui| {
            ui.label("A curve =");
            for s in &split_names {
                ui.label(RichText::new(s).strong());
            }
            if cfg.auto_keys {
                match &plan {
                    Some(p) => {
                        for &c in &p.auto_found {
                            ui.label(RichText::new(&ds.columns[c].name).strong().color(NAVY_TEXT));
                        }
                        if p.key_cols.is_empty() {
                            ui.label(RichText::new("all the points").weak());
                        }
                    }
                    None => {
                        ui.spinner();
                    }
                }
            } else {
                let mut remove = None;
                for (k, c) in cfg.curve_keys.iter().enumerate() {
                    if ui
                        .button(RichText::new(format!("{c}  🗙")).strong())
                        .on_hover_text("Remove")
                        .clicked()
                    {
                        remove = Some(k);
                    }
                }
                if let Some(k) = remove {
                    cfg.curve_keys.remove(k);
                }
                let mut add = String::new();
                if column_combo(ui, "dx_add_key", &mut add, &names, 160.0) && !cfg.curve_keys.contains(&add) {
                    cfg.curve_keys.push(add);
                }
            }
        });
        if let Some(p) = &plan
            && p.duplicates > 0
        {
            ui.label(
                RichText::new(format!(
                    "⚠ {} curve(s) have repeated X values: add a column to the curve definition \
                     (or a criterion).",
                    p.duplicates
                ))
                .color(WARN),
            );
        }
    });
    ui.add_space(6.0);

    // ---- 4. smoothing ----------------------------------------------------------
    section(ui, "4 · Smoothing", |ui| {
        ui.checkbox(
            &mut cfg.smooth_enabled,
            RichText::new("Smooth the exported values").strong(),
        )
        .on_hover_text(
            "Each curve is smoothed on its whole X range, then cut to the band. Phases are unwrapped \
                 before smoothing (no jump at ±180°).",
        );
        ui.add_enabled_ui(cfg.smooth_enabled, |ui| {
            ui.label(RichText::new("Default smoothing (columns set to “Default”)").strong());
            smoothing_editor(ui, "dx_default", &mut cfg.smooth, true);
            ui.add_space(4.0);
            ui.label(RichText::new("Per column").strong());
            let mut cols: Vec<String> = cfg.columns.clone();
            if cfg.s2p
                && let Some(p) = &plan
            {
                for c in p.sparams.iter().flatten() {
                    let n = &ds.columns[*c].name;
                    if !cols.contains(n) {
                        cols.push(n.clone());
                    }
                }
            }
            egui::Grid::new("dx_cols")
                .num_columns(3)
                .striped(true)
                .spacing([10.0, 4.0])
                .show(ui, |ui| {
                    for name in &cols {
                        let numeric = ds.column(name).is_some_and(|c| c.has_numbers());
                        let label = if is_phase(name) {
                            format!("{name}  ∠")
                        } else {
                            name.clone()
                        };
                        if ui
                            .selectable_label(st.preview_col == *name, label)
                            .on_hover_text(if is_phase(name) {
                                "Phase: unwrapped before smoothing. Click to preview."
                            } else {
                                "Click to preview"
                            })
                            .clicked()
                        {
                            st.preview_col = name.clone();
                        }
                        if !numeric {
                            ui.label(RichText::new("text: not smoothed").weak());
                            ui.end_row();
                            continue;
                        }
                        let mut choice = cfg.choice_for(name);
                        egui::ComboBox::from_id_salt(("dx_choice", name))
                            .selected_text(choice.label())
                            .width(90.0)
                            .show_ui(ui, |ui| {
                                for c in SmoothChoice::ALL {
                                    ui.selectable_value(&mut choice, c, c.label());
                                }
                            });
                        if choice != cfg.choice_for(name) {
                            cfg.column_smooth_mut(name).choice = choice;
                            if choice == SmoothChoice::Custom {
                                st.editing = Some(name.clone());
                                st.preview_col = name.clone();
                            }
                        }
                        ui.horizontal(|ui| {
                            match cfg.smoothing_for(name) {
                                Some(sm) => ui.label(RichText::new(sm.describe()).weak()),
                                None => ui.label(RichText::new("not smoothed").weak()),
                            };
                            if choice == SmoothChoice::Custom {
                                let open = st.editing.as_deref() == Some(name.as_str());
                                if ui.small_button(if open { "Close" } else { "✏ Edit" }).clicked() {
                                    st.editing = if open { None } else { Some(name.clone()) };
                                    st.preview_col = name.clone();
                                }
                            }
                        });
                        ui.end_row();
                    }
                });
            if let Some(name) = st.editing.clone()
                && cfg.choice_for(&name) == SmoothChoice::Custom
            {
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("Smoothing of {name}")).strong());
                        if ui.small_button("Copy the default").clicked() {
                            cfg.column_smooth_mut(&name).custom = cfg.smooth.clone();
                        }
                    });
                    let sm = &mut cfg.column_smooth_mut(&name).custom;
                    smoothing_editor(ui, ("dx_custom", name.as_str()), sm, true);
                });
            }
            ui.checkbox(
                &mut cfg.write_raw,
                "CSV: also write the unsmoothed values (columns “… (unsmoothed)”)",
            );
        });
    });
    ui.add_space(6.0);

    // ---- 5. output -------------------------------------------------------------
    section(ui, "5 · Formats and folder", |ui| {
        ui.horizontal(|ui| {
            ui.checkbox(&mut cfg.csv, RichText::new("CSV").strong())
                .on_hover_text("One file per value of the sorting columns (one file if there is no sorting)");
            ui.add_enabled_ui(cfg.csv, |ui| {
                ui.label("separator");
                egui::ComboBox::from_id_salt("dx_sep")
                    .selected_text(cfg.csv_sep.label())
                    .show_ui(ui, |ui| {
                        for s in CsvSep::ALL {
                            ui.selectable_value(&mut cfg.csv_sep, s, s.label());
                        }
                    });
                ui.checkbox(&mut cfg.decimal_comma, "decimal comma (Excel FR)");
            });
        });
        if cfg.decimal_comma && cfg.csv_sep == CsvSep::Comma {
            ui.label(
                RichText::new("Tip: use the ; separator with the decimal comma.")
                    .color(WARN)
                    .small(),
            );
        }
        ui.horizontal(|ui| {
            ui.checkbox(&mut cfg.s2p, RichText::new("Touchstone .s2p").strong())
                .on_hover_text("One file per curve, with S11, S21, S12, S22");
            ui.add_enabled_ui(cfg.s2p, |ui| {
                egui::ComboBox::from_id_salt("dx_ts_format")
                    .selected_text(cfg.ts_format.label())
                    .show_ui(ui, |ui| {
                        for f in TsFormat::ALL {
                            ui.selectable_value(&mut cfg.ts_format, f, f.label());
                        }
                    });
                egui::ComboBox::from_id_salt("dx_ts_unit")
                    .selected_text(cfg.ts_unit.label())
                    .width(60.0)
                    .show_ui(ui, |ui| {
                        for u in [FreqUnit::Hz, FreqUnit::KHz, FreqUnit::MHz, FreqUnit::GHz] {
                            ui.selectable_value(&mut cfg.ts_unit, u, u.label());
                        }
                    });
                ui.label("R");
                ui.add(egui::DragValue::new(&mut cfg.r_ref).range(0.001..=1e6).suffix(" Ω"));
            });
        });
        if cfg.s2p {
            ui.indent("dx_s2p", |ui| {
                let xc = ds.column(&cfg.x);
                ui.horizontal(|ui| {
                    ui.label("Frequency unit of the data");
                    let auto = xc.map_or("Auto".to_owned(), |c| {
                        format!("Auto ({})", FreqUnit::guess(c.max.abs().max(c.min.abs())).label())
                    });
                    egui::ComboBox::from_id_salt("dx_data_unit")
                        .selected_text(if cfg.freq_unit == FreqUnit::Auto {
                            auto.clone()
                        } else {
                            cfg.freq_unit.label().to_owned()
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut cfg.freq_unit, FreqUnit::Auto, auto);
                            for u in [FreqUnit::Hz, FreqUnit::KHz, FreqUnit::MHz, FreqUnit::GHz] {
                                ui.selectable_value(&mut cfg.freq_unit, u, u.label());
                            }
                        });
                });
                let found = plan.as_ref().map(|p| p.sparams);
                let summary: Vec<String> = (0..4)
                    .map(|p| {
                        let ok = found.is_some_and(|f| f[2 * p].is_some());
                        format!("{}{}", &SPARAM_NAMES[2 * p][..3], if ok { " ✔" } else { " —" })
                    })
                    .collect();
                ui.horizontal(|ui| {
                    ui.label(RichText::new(summary.join("   ")).weak());
                    ui.toggle_value(&mut st.show_sparams, "Columns…");
                });
                if st.show_sparams {
                    let mut opts = vec![String::new()];
                    opts.extend(numeric.iter().cloned());
                    egui::Grid::new("dx_sparams")
                        .num_columns(2)
                        .spacing([10.0, 4.0])
                        .show(ui, |ui| {
                            for k in 0..8 {
                                ui.label(SPARAM_NAMES[k]);
                                let auto = found
                                    .and_then(|f| f[k])
                                    .filter(|_| cfg.sparams[k].is_empty())
                                    .map(|c| format!("auto: {}", ds.columns[c].name))
                                    .unwrap_or_else(|| "auto: not found".to_owned());
                                let shown = if cfg.sparams[k].is_empty() {
                                    auto
                                } else {
                                    cfg.sparams[k].clone()
                                };
                                egui::ComboBox::from_id_salt(("dx_sp", k))
                                    .selected_text(shown)
                                    .width(220.0)
                                    .height(300.0)
                                    .show_ui(ui, |ui| {
                                        for o in &opts {
                                            let label = if o.is_empty() { "auto (from the names)" } else { o };
                                            ui.selectable_value(&mut cfg.sparams[k], o.clone(), label);
                                        }
                                    });
                                ui.end_row();
                            }
                        });
                }
            });
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Folder").strong());
            if ui.button("📂 Choose…").clicked() {
                let mut dlg = rfd::FileDialog::new().set_title("Folder of the exported data");
                if let Some(d) = cfg.folder.as_ref().filter(|d| d.is_dir()) {
                    dlg = dlg.set_directory(d);
                }
                if let Some(d) = dlg.pick_folder() {
                    cfg.folder = Some(d);
                }
            }
            match &cfg.folder {
                Some(d) => ui.add(egui::Label::new(RichText::new(d.display().to_string()).small()).truncate()),
                None => ui.label(RichText::new("not chosen").color(WARN)),
            };
        });
    });
    ui.add_space(8.0);
}

/// Darker blue readable on the light panels.
const NAVY_TEXT: Color32 = Color32::from_rgb(52, 92, 160);

fn export_button(app: &mut App, ui: &mut Ui, ds: &Arc<Dataset>, plan: Option<&Plan>) {
    if let Some(job) = &app.dx.job {
        let done = job.progress.done.load(Ordering::Relaxed);
        let total = job.progress.total.load(Ordering::Relaxed).max(1);
        ui.horizontal(|ui| {
            ui.add(
                egui::ProgressBar::new(done as f32 / total as f32)
                    .text(format!("Exporting… {done} / {total} curves"))
                    .desired_width(ui.available_width() - 90.0),
            );
            if ui.button("Cancel").clicked() {
                job.progress.cancel.store(true, Ordering::Relaxed);
            }
        });
        return;
    }
    let cfg = &app.data_export;
    let n_files = plan.map_or(0, |p| {
        usize::from(cfg.csv) * p.groups.len() + usize::from(cfg.s2p) * p.curves.len()
    });
    let ready = plan.is_some_and(|p| !p.curves.is_empty()) && cfg.folder.is_some() && (cfg.csv || cfg.s2p);
    let text = if cfg.folder.is_none() {
        "💾 Export (choose a folder first)".to_owned()
    } else {
        format!("💾 Export {} file(s)", thousands(n_files))
    };
    let b = egui::Button::new(RichText::new(text).size(15.0).color(Color32::WHITE).strong())
        .fill(NAVY)
        .min_size(egui::vec2(ui.available_width(), 36.0));
    if ui.add_enabled(ready, b).clicked() {
        start_export(app, ds);
    }
    if let Some((res, dir)) = &app.dx.last {
        match res {
            Ok(s) => {
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new(format!(
                            "✔ {} CSV + {} .s2p file(s), {} points, in {:.1} s",
                            s.csv_files,
                            s.s2p_files,
                            thousands(s.points),
                            s.ms / 1000.0
                        ))
                        .color(OK_GREEN),
                    );
                    if ui.small_button("Open the folder").clicked() {
                        open_folder(dir);
                    }
                });
                for n in &s.notes {
                    ui.label(RichText::new(format!("ⓘ {n}")).weak().small());
                }
            }
            Err(e) => {
                ui.label(RichText::new(format!("⚠ {e}")).color(ERR));
            }
        }
    }
}

fn open_folder(dir: &std::path::Path) {
    let cmd = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(cmd).arg(dir).spawn();
}

fn start_export(app: &mut App, ds: &Arc<Dataset>) {
    let Some(dir) = app.data_export.folder.clone() else {
        return;
    };
    let Some((_, info)) = export_dataset(app) else {
        return;
    };
    let plan = current_plan(app, ds);
    let progress = Arc::new(Progress::default());
    let (tx, rx) = channel();
    let (ds, cfg, p, d, ctx) = (
        ds.clone(),
        app.data_export.clone(),
        progress.clone(),
        dir.clone(),
        app.repaint_ctx(),
    );
    std::thread::spawn(move || {
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| dx::run(&ds, &cfg, &d, &info, &p, plan)))
            .unwrap_or_else(|_| Err("Unexpected error while exporting".to_owned()));
        let _ = tx.send(res);
        ctx.request_repaint();
    });
    app.dx.last = None;
    app.dx.job = Some(Job { progress, rx, dir });
}

// =============================================================================
// Preview
// =============================================================================

fn preview_panel(app: &mut App, ui: &mut Ui, ds: &Arc<Dataset>, info: &[String]) {
    ui.heading("Preview");
    for line in info {
        ui.label(RichText::new(line).weak().small());
    }
    if let Some(e) = &app.dx.plan_error {
        ui.label(RichText::new(format!("⚠ {e}")).color(ERR));
    }
    let Some(plan) = current_plan(app, ds) else {
        if app.dx.pending.is_some() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Preparing…");
            });
        }
        return;
    };
    let cfg = &app.data_export;
    ui.label(
        RichText::new(format!(
            "{} rows selected · {} curve(s) · {} points in the band",
            thousands(plan.n_rows),
            thousands(plan.curves.len()),
            thousands(plan.n_points)
        ))
        .strong(),
    );
    for w in &plan.warnings {
        ui.label(RichText::new(format!("⚠ {w}")).color(WARN).small());
    }
    if plan.curves.is_empty() {
        ui.label(RichText::new("No point to export: check the band and the criteria.").color(WARN));
        return;
    }

    // ---- files ------------------------------------------------------------------
    let files_key = plan.key ^ (u64::from(cfg.csv) << 1) ^ (u64::from(cfg.s2p) << 2);
    if app.dx.files.as_ref().is_none_or(|f| f.0 != files_key) {
        app.dx.files = Some((files_key, file_list(&plan, cfg.csv, cfg.s2p)));
    }
    let cfg = &app.data_export;
    let n_files = usize::from(cfg.csv) * plan.groups.len() + usize::from(cfg.s2p) * plan.curves.len();
    ui.add_space(4.0);
    section(ui, &format!("Files ({})", thousands(n_files)), |ui| {
        if let Some(d) = &cfg.folder {
            ui.label(RichText::new(format!("📂 {}", d.display())).small());
        }
        let list = &app.dx.files.as_ref().expect("file list").1;
        let row_h = ui.text_style_height(&egui::TextStyle::Small);
        egui::ScrollArea::vertical()
            .id_salt("dx_files")
            .max_height(230.0)
            .auto_shrink([false, true])
            .show_rows(ui, row_h, list.len(), |ui, range| {
                for (indent, text) in &list[range] {
                    let t = format!("{}{text}", "    ".repeat(*indent));
                    ui.add(egui::Label::new(RichText::new(t).small()).extend());
                }
            });
    });

    // ---- smoothing preview ----------------------------------------------------
    ui.add_space(6.0);
    let mut cols: Vec<String> = cfg.columns.clone();
    if cfg.s2p {
        for c in plan.sparams.iter().flatten() {
            let n = &ds.columns[*c].name;
            if !cols.contains(n) {
                cols.push(n.clone());
            }
        }
    }
    cols.retain(|c| ds.column(c).is_some_and(|c| c.has_numbers()));
    if cols.is_empty() {
        return;
    }
    let st = &mut app.dx;
    if !cols.contains(&st.preview_col) {
        st.preview_col = cols[0].clone();
    }
    st.preview_curve = st.preview_curve.min(plan.curves.len() - 1);
    section(ui, "Curve preview", |ui| {
        ui.horizontal(|ui| {
            column_combo(ui, "dx_prev_col", &mut st.preview_col, &cols, 180.0);
            if ui.small_button("◀").clicked() {
                st.preview_curve = st.preview_curve.saturating_sub(1);
            }
            let mut n = st.preview_curve + 1;
            ui.add(egui::DragValue::new(&mut n).range(1..=plan.curves.len()));
            st.preview_curve = n - 1;
            if ui.small_button("▶").clicked() {
                st.preview_curve = (st.preview_curve + 1).min(plan.curves.len() - 1);
            }
            ui.label(format!("/ {}", plan.curves.len()));
        });
        ui.label(RichText::new(plan.curve_label(ds, st.preview_curve)).weak());
        let Some(col) = ds.column_index(&st.preview_col) else {
            return;
        };
        let key = crate::data::hash::hash_one(&(
            plan.key,
            st.preview_curve,
            col,
            serde_json::to_string(&cfg.smoothing_for(&st.preview_col)).unwrap_or_default(),
        ));
        if st.curve_preview.as_ref().is_none_or(|c| c.0 != key) {
            st.curve_preview = dx::preview(ds, &plan, cfg, st.preview_curve, col).map(|p| (key, Arc::new(p)));
        }
        let Some(p) = st.curve_preview.as_ref().map(|c| c.1.clone()) else {
            return;
        };
        let (xf, xunit) = x_display(cfg, ds);
        let scale = |pts: &[[f64; 2]]| -> Vec<[f64; 2]> {
            let step = pts.len().div_ceil(6000).max(1);
            pts.iter().step_by(step).map(|q| [q[0] * xf, q[1]]).collect()
        };
        let h = (ui.available_height() - 10.0).clamp(240.0, 520.0);
        let band = plan.band;
        Plot::new(("dx_preview_plot", &st.preview_col))
            .height(h)
            .legend(Legend::default())
            .x_axis_label(format!(
                "{} {}",
                ds.columns[plan.x].name,
                if xunit.is_empty() {
                    String::new()
                } else {
                    format!("[{xunit}]")
                }
            ))
            .y_axis_label(st.preview_col.clone())
            .allow_scroll(false)
            .show(ui, |pui| {
                let raw_color = Color32::from_gray(150);
                pui.line(
                    Line::new("data", PlotPoints::new(scale(&p.raw)))
                        .color(raw_color)
                        .width(1.0),
                );
                pui.points(
                    Points::new("data", PlotPoints::new(scale(&p.raw)))
                        .color(raw_color)
                        .radius(2.0),
                );
                if p.smoothed {
                    pui.line(
                        Line::new("smoothed (exported)", PlotPoints::new(scale(&p.exported)))
                            .color(Color32::from_rgb(200, 60, 40))
                            .width(2.0),
                    );
                }
                for v in [band.0, band.1] {
                    if v.is_finite() {
                        pui.vline(VLine::new("X band", v * xf).color(NAVY_TEXT).width(1.0));
                    }
                }
            });
        if !p.smoothed {
            ui.label(
                RichText::new("This column is not smoothed: the data is written as is.")
                    .weak()
                    .small(),
            );
        }
    });
}

/// Indented list of the files of the preview.
fn file_list(plan: &Plan, csv: bool, s2p: bool) -> Vec<(usize, String)> {
    const MAX: usize = 20_000;
    let mut out = Vec::new();
    let mut last_dir: Option<PathBuf> = None;
    for g in &plan.groups {
        if out.len() >= MAX {
            out.push((0, "…".to_owned()));
            break;
        }
        let dir = g.csv.parent().map(|p| p.to_path_buf()).unwrap_or_default();
        let depth = dir.components().count();
        if last_dir.as_ref() != Some(&dir) {
            // show the folders that change
            let parts: Vec<String> = dir
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect();
            let prev: Vec<String> = last_dir
                .as_ref()
                .map(|d| {
                    d.components()
                        .map(|c| c.as_os_str().to_string_lossy().into_owned())
                        .collect()
                })
                .unwrap_or_default();
            let common = parts.iter().zip(&prev).take_while(|(a, b)| a == b).count();
            for (k, p) in parts.iter().enumerate().skip(common) {
                out.push((k, format!("📁 {p}/")));
            }
            last_dir = Some(dir.clone());
        }
        if csv {
            let name = g
                .csv
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            out.push((
                depth,
                format!(
                    "📄 {name}   ({} curve(s), {} points)",
                    g.curves.len(),
                    thousands(g.points)
                ),
            ));
        }
        if s2p {
            for i in g.curves.clone() {
                let name = plan.s2p[i]
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                out.push((depth, format!("📶 {name}   ({} points)", plan.curves[i].in_band)));
            }
        }
    }
    out
}
