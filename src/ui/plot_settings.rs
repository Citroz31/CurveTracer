//! Settings panel of the active plot: Data (select / process / view),
//! Layout, Plot Options and Trace Options.

use std::collections::HashSet;
use std::sync::Arc;

use egui::{Align, Color32, Frame, Layout, Margin, RichText, Ui};

use super::{ERR, color_button, font_editor, section, table::data_table, thousands};
use crate::app::{App, DataTab, SettingsTab};
use crate::data::dataset::{Column, Dataset};
use crate::plot::config::{
    AggFunc, AxisLayout, FilterConfig, FilterMode, LegendPos, LineKind, MarkerKind, Modifier, Palette, PlotConfig,
    Scale, TraceOverride,
};
use crate::plot::smooth::{SmoothConfig, SmoothMethod};
use crate::plot::style::trace_styles;

/// Drag & drop payload: a column name.
struct ColumnDrag(String);

pub fn show(app: &mut App, ui: &mut Ui) {
    ui.horizontal(|ui| {
        for (tab, label) in [
            (SettingsTab::Data, "Data"),
            (SettingsTab::Layout, "Layout"),
            (SettingsTab::Options, "Plot Options"),
            (SettingsTab::Traces, "Trace Options"),
        ] {
            let text = if app.settings_tab == tab {
                RichText::new(label).strong()
            } else {
                RichText::new(label)
            };
            ui.selectable_value(&mut app.settings_tab, tab, text);
        }
    });
    ui.separator();
    match app.settings_tab {
        SettingsTab::Data => data_tab(app, ui),
        SettingsTab::Layout => scrolled(ui, "layout", |ui| layout_tab(app, ui)),
        SettingsTab::Options => scrolled(ui, "options", |ui| options_tab(app, ui)),
        SettingsTab::Traces => scrolled(ui, "traces", |ui| traces_tab(app, ui)),
    }
}

fn scrolled(ui: &mut Ui, id: &str, add: impl FnOnce(&mut Ui)) {
    // horizontal scrolling too: a wide table (Trace Options) scrolls instead of
    // being cut
    egui::ScrollArea::both()
        .id_salt(("settings_scroll", id))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
        });
}

// =============================================================================
// Data tab
// =============================================================================

fn data_tab(app: &mut App, ui: &mut Ui) {
    ui.horizontal(|ui| {
        for (tab, label, tip) in [
            (
                DataTab::Select,
                "Select Data",
                "Choose the data source, X, Y, filters and series",
            ),
            (
                DataTab::Process,
                "Process Data",
                "Average / min / max / sum / RMS … of the curves",
            ),
            (DataTab::View, "View Data", "Table of the rows used by this plot"),
        ] {
            ui.selectable_value(&mut app.data_tab, tab, label).on_hover_text(tip);
        }
    });
    ui.add_space(2.0);
    match app.data_tab {
        DataTab::Select => scrolled(ui, "select", |ui| select_data(app, ui)),
        DataTab::Process => scrolled(ui, "process", |ui| process_data(app, ui)),
        DataTab::View => view_data(app, ui),
    }
}

fn drop_zone(ui: &mut Ui, title: &str, add: impl FnOnce(&mut Ui)) -> Option<String> {
    let frame = Frame::group(ui.style()).inner_margin(Margin::same(6));
    let (_, dropped) = ui.dnd_drop_zone::<ColumnDrag, ()>(frame, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(title).strong());
        add(ui);
    });
    dropped.map(|d| d.0.clone())
}

fn modifier_combo(ui: &mut Ui, id: &str, scale: &mut Scale) {
    ui.horizontal(|ui| {
        ui.label("Modifier");
        egui::ComboBox::from_id_salt(("modifier", id))
            .width(150.0)
            .selected_text(scale.modifier.label())
            .show_ui(ui, |ui| {
                for m in Modifier::ALL {
                    ui.selectable_value(&mut scale.modifier, m, m.label());
                }
            });
        if scale.modifier == Modifier::Custom {
            ui.label("×");
            ui.add(egui::DragValue::new(&mut scale.custom).speed(0.01));
        }
    });
}

fn new_filter(col: &Column) -> FilterConfig {
    match col.dict() {
        Some(dict) => FilterConfig {
            column: col.name.clone(),
            mode: FilterMode::Values,
            values: dict.to_vec(),
            ..Default::default()
        },
        None => FilterConfig {
            column: col.name.clone(),
            mode: FilterMode::Range,
            min: col.min.is_finite().then_some(col.min),
            max: col.max.is_finite().then_some(col.max),
            ..Default::default()
        },
    }
}

#[derive(Clone, Copy)]
enum ColAction {
    X,
    Y,
    Filter,
    Series,
}

fn select_data(app: &mut App, ui: &mut Ui) {
    let i = app.active;
    let sources: Vec<(String, String, bool)> = app
        .sources
        .iter()
        .map(|s| (s.key.clone(), s.name(), s.ds.is_some()))
        .collect();
    let ds: Option<Arc<Dataset>> = app.dataset(&app.plots[i].cfg.source).cloned();
    let entry = &mut app.plots[i];
    let cfg = &mut entry.cfg;
    let pui = &mut entry.ui;

    // ---- data source ---------------------------------------------------------
    ui.horizontal(|ui| {
        ui.label("Data source");
        let current = sources
            .iter()
            .find(|s| s.0 == cfg.source)
            .map(|s| s.1.clone())
            .unwrap_or_else(|| "(not loaded)".to_owned());
        egui::ComboBox::from_id_salt("plot_source")
            .width(ui.available_width() - 10.0)
            .selected_text(current)
            .show_ui(ui, |ui| {
                for (k, name, loaded) in &sources {
                    let label = if *loaded {
                        name.clone()
                    } else {
                        format!("{name} (loading)")
                    };
                    ui.selectable_value(&mut cfg.source, k.clone(), label);
                }
            });
    });
    let Some(ds) = ds else {
        ui.label(RichText::new("The data source of this plot is not loaded.").color(ERR));
        return;
    };

    // ---- columns list -------------------------------------------------------
    let mut action: Option<(ColAction, String)> = None;
    section(ui, "Columns", |ui| {
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut pui.col_search)
                    .hint_text("search…")
                    .desired_width(140.0),
            );
            ui.label("Sort:");
            ui.selectable_value(&mut pui.sort_by_unique, false, "Name");
            ui.selectable_value(&mut pui.sort_by_unique, true, "Unique vals");
        });
        ui.label(
            RichText::new("Drag a column onto X / Y / Filter / Series, or use the buttons.")
                .small()
                .weak(),
        );
        let search = pui.col_search.to_lowercase();
        let mut base: Vec<&Arc<Column>> = ds
            .columns
            .iter()
            .filter(|c| c.formula.is_none() && (search.is_empty() || c.name.to_lowercase().contains(&search)))
            .collect();
        let funcs: Vec<&Arc<Column>> = ds
            .columns
            .iter()
            .filter(|c| c.formula.is_some() && (search.is_empty() || c.name.to_lowercase().contains(&search)))
            .collect();
        if pui.sort_by_unique {
            base.sort_by_key(|c| c.n_unique);
        }
        egui::ScrollArea::vertical()
            .id_salt("columns_list")
            .max_height(240.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                let mut row = |ui: &mut Ui, c: &Column| {
                    ui.horizontal(|ui| {
                        let id = egui::Id::new(("col_drag", &c.name));
                        ui.dnd_drag_source(id, ColumnDrag(c.name.clone()), |ui| {
                            let color = if c.formula.is_some() {
                                Color32::from_rgb(40, 110, 200)
                            } else {
                                ui.visuals().text_color()
                            };
                            ui.label(RichText::new(format!("{} {}", c.kind.short(), c.name)).color(color));
                        })
                        .response
                        .on_hover_text(format!(
                            "{} unique value(s){}",
                            c.n_unique,
                            if c.min.is_finite() {
                                format!("\nmin {} · max {}", c.min, c.max)
                            } else {
                                String::new()
                            }
                        ));
                        ui.label(RichText::new(format!("({})", c.n_unique)).weak().small());
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui
                                .small_button("S")
                                .on_hover_text("Add as series (one curve per value)")
                                .clicked()
                            {
                                action = Some((ColAction::Series, c.name.clone()));
                            }
                            if ui
                                .small_button("F")
                                .on_hover_text("Add a filter on this column")
                                .clicked()
                            {
                                action = Some((ColAction::Filter, c.name.clone()));
                            }
                            if ui.small_button("Y").on_hover_text("Add to Y").clicked() {
                                action = Some((ColAction::Y, c.name.clone()));
                            }
                            if ui.small_button("X").on_hover_text("Use as X").clicked() {
                                action = Some((ColAction::X, c.name.clone()));
                            }
                        });
                    });
                };
                for c in &base {
                    row(ui, c);
                }
                if !funcs.is_empty() {
                    ui.separator();
                    ui.label(RichText::new("User Functions").strong());
                    for c in &funcs {
                        row(ui, c);
                    }
                }
            });
    });

    // ---- X ---------------------------------------------------------------------
    let mut clear_x = false;
    if let Some(name) = drop_zone(ui, "X-Data (limit 1)", |ui| {
        if cfg.x.is_empty() {
            ui.label(RichText::new("Drop a column here or click X in the list").weak());
        } else {
            ui.horizontal(|ui| {
                ui.label(RichText::new(&cfg.x).strong());
                if ds.column(&cfg.x).is_none() {
                    ui.label(RichText::new("(missing)").color(ERR));
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.small_button("🗙").clicked() {
                        clear_x = true;
                    }
                });
            });
            modifier_combo(ui, "x", &mut cfg.x_scale);
        }
    }) {
        action = Some((ColAction::X, name));
    }
    if clear_x {
        cfg.x.clear();
    }

    // ---- Y ---------------------------------------------------------------------
    let mut remove_y = None;
    if let Some(name) = drop_zone(ui, "Y-Data", |ui| {
        if cfg.ys.is_empty() {
            ui.label(RichText::new("Drop one or several columns here").weak());
        }
        for (k, y) in cfg.ys.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(RichText::new(y).strong());
                if ds.column(y).is_none() {
                    ui.label(RichText::new("(missing)").color(ERR));
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.small_button("🗙").clicked() {
                        remove_y = Some(k);
                    }
                });
            });
        }
        if !cfg.ys.is_empty() {
            modifier_combo(ui, "y", &mut cfg.y_scale);
        }
    }) {
        action = Some((ColAction::Y, name));
    }
    if let Some(k) = remove_y {
        cfg.ys.remove(k);
    }

    // ---- filters ----------------------------------------------------------------
    let mut remove_f = None;
    if let Some(name) = drop_zone(ui, "Filter", |ui| {
        if cfg.filters.is_empty() {
            ui.label(RichText::new("Drop a column to keep only some of its values").weak());
        }
        for (k, f) in cfg.filters.iter_mut().enumerate() {
            let col = ds.column(&f.column);
            let total = col.map_or(0, |c| c.dict().map_or(0, |d| d.len()));
            ui.horizontal(|ui| {
                let open = pui.open_filters.contains(&k);
                if ui.small_button(if open { "▼" } else { "▶" }).clicked() {
                    if open {
                        pui.open_filters.remove(&k);
                    } else {
                        pui.open_filters.insert(k);
                    }
                }
                let summary = match f.mode {
                    FilterMode::Values => format!("{} ({}/{})", f.column, f.values.len(), total),
                    FilterMode::Range => format!(
                        "{} ∈ [{}, {}]",
                        f.column,
                        f.min.map_or("−∞".to_owned(), |v| crate::data::format_sig(v, 5)),
                        f.max.map_or("+∞".to_owned(), |v| crate::data::format_sig(v, 5))
                    ),
                };
                let text = if f.enabled {
                    RichText::new(summary).strong()
                } else {
                    RichText::new(summary).weak()
                };
                if ui.add(egui::Label::new(text).sense(egui::Sense::click())).clicked() {
                    if open {
                        pui.open_filters.remove(&k);
                    } else {
                        pui.open_filters.insert(k);
                    }
                }
                if col.is_none() {
                    ui.label(RichText::new("(missing)").color(ERR));
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.small_button("🗙").clicked() {
                        remove_f = Some(k);
                    }
                    ui.checkbox(&mut f.enabled, "").on_hover_text("Enable this filter");
                });
            });
            if pui.open_filters.contains(&k)
                && let Some(col) = col
            {
                Frame::new()
                    .inner_margin(Margin {
                        left: 18,
                        right: 2,
                        top: 0,
                        bottom: 4,
                    })
                    .show(ui, |ui| {
                        filter_editor(ui, k, f, col, &mut pui.filter_search);
                    });
            }
        }
    }) {
        action = Some((ColAction::Filter, name));
    }
    if let Some(k) = remove_f {
        cfg.filters.remove(k);
        pui.open_filters.clear();
    }

    // ---- series ----------------------------------------------------------------
    let mut remove_s = None;
    let mut up_s = None;
    if let Some(name) = drop_zone(ui, "Series", |ui| {
        if cfg.series.is_empty() {
            ui.label(RichText::new("Drop a column: one curve per value (SN, temperature, part number…)").weak());
        }
        for (k, s) in cfg.series.iter().enumerate() {
            ui.horizontal(|ui| {
                let n = ds.column(s).map_or(0, |c| c.n_unique);
                ui.label(RichText::new(format!("{s} ({n})")).strong());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.small_button("🗙").clicked() {
                        remove_s = Some(k);
                    }
                    if k > 0 && ui.small_button("⬆").on_hover_text("Move up (grouping order)").clicked() {
                        up_s = Some(k);
                    }
                });
            });
        }
    }) {
        action = Some((ColAction::Series, name));
    }
    if let Some(k) = remove_s {
        let name = cfg.series.remove(k);
        cfg.process.collapse.retain(|c| *c != name);
    }
    if let Some(k) = up_s {
        cfg.series.swap(k, k - 1);
    }

    // ---- apply the column action -------------------------------------------------
    if let Some((what, name)) = action {
        apply_column_action(cfg, &ds, what, name, pui);
    }
}

fn apply_column_action(
    cfg: &mut PlotConfig,
    ds: &Dataset,
    what: ColAction,
    name: String,
    pui: &mut crate::app::PlotUiState,
) {
    match what {
        ColAction::X => {
            if cfg.x != name {
                cfg.x = name.clone();
                // frequencies in Hz → GHz by default
                if let Some(c) = ds.column(&name) {
                    if name.to_ascii_lowercase().starts_with("freq") && c.max.abs() >= 1e8 {
                        cfg.x_scale.modifier = Modifier::Giga;
                    } else {
                        cfg.x_scale.modifier = Modifier::None;
                    }
                }
            }
        }
        ColAction::Y => {
            if !cfg.ys.contains(&name) {
                cfg.ys.push(name);
            }
        }
        ColAction::Filter => {
            if let Some(c) = ds.column(&name) {
                cfg.filters.push(new_filter(c));
                pui.open_filters.insert(cfg.filters.len() - 1);
            }
        }
        ColAction::Series => {
            if !cfg.series.contains(&name) {
                cfg.series.push(name);
            }
        }
    }
}

fn filter_editor(ui: &mut Ui, k: usize, f: &mut FilterConfig, col: &Column, search: &mut String) {
    ui.horizontal(|ui| {
        if col.is_dict() {
            ui.radio_value(&mut f.mode, FilterMode::Values, "Values");
        }
        if col.has_numbers() && ui.radio_value(&mut f.mode, FilterMode::Range, "Range").clicked() {
            if f.min.is_none() && col.min.is_finite() {
                f.min = Some(col.min);
            }
            if f.max.is_none() && col.max.is_finite() {
                f.max = Some(col.max);
            }
        }
    });
    match f.mode {
        FilterMode::Values => {
            let (Some(dict), Some(sorted)) = (col.dict(), col.sorted_codes()) else {
                ui.label("Too many unique values: use a range filter.");
                return;
            };
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(search)
                        .hint_text("search values…")
                        .desired_width(120.0),
                );
                if ui.small_button("All").clicked() {
                    f.values = dict.to_vec();
                }
                if ui.small_button("None").clicked() {
                    f.values.clear();
                }
                if ui.small_button("Invert").clicked() {
                    let sel: HashSet<&String> = f.values.iter().collect();
                    f.values = dict.iter().filter(|v| !sel.contains(v)).cloned().collect();
                }
            });
            let s = search.to_lowercase();
            // codes shown (search applied), in natural order
            let visible: Vec<u32> = if s.is_empty() {
                sorted.to_vec()
            } else {
                sorted
                    .iter()
                    .copied()
                    .filter(|&c| dict[c as usize].to_lowercase().contains(&s))
                    .collect()
            };
            let mut selected: HashSet<&str> = f.values.iter().map(String::as_str).collect();
            let mut changed = false;
            let mut only: Option<String> = None;
            let row_h = ui.spacing().interact_size.y + ui.spacing().item_spacing.y;
            // only the visible rows are built (thousands of states stay fluid)
            egui::ScrollArea::vertical()
                .id_salt(("filter_values", k))
                .max_height(220.0)
                .auto_shrink([false, true])
                .show_rows(ui, row_h - ui.spacing().item_spacing.y, visible.len(), |ui, range| {
                    for &c in &visible[range] {
                        let v = dict[c as usize].as_str();
                        let label = if v.is_empty() { "(empty)" } else { v };
                        ui.horizontal(|ui| {
                            let mut on = selected.contains(v);
                            if ui.checkbox(&mut on, label).changed() {
                                changed = true;
                                if on {
                                    selected.insert(v);
                                } else {
                                    selected.remove(v);
                                }
                            }
                            if ui.small_button("only").on_hover_text("Keep only this value").clicked() {
                                only = Some(v.to_owned());
                            }
                        });
                    }
                });
            if !s.is_empty() {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{} match(es)", visible.len())).weak().small());
                    if ui.small_button("Select matches").clicked() {
                        selected.extend(visible.iter().map(|&c| dict[c as usize].as_str()));
                        changed = true;
                    }
                    if ui.small_button("Unselect matches").clicked() {
                        for &c in &visible {
                            selected.remove(dict[c as usize].as_str());
                        }
                        changed = true;
                    }
                });
            }
            if let Some(v) = only {
                f.values = vec![v];
            } else if changed {
                // keep the natural order of the dictionary
                f.values = sorted
                    .iter()
                    .map(|&c| &dict[c as usize])
                    .filter(|v| selected.contains(v.as_str()))
                    .cloned()
                    .collect();
            }
        }
        FilterMode::Range => {
            let span = (col.max - col.min).abs().max(1e-12);
            let speed = span / 500.0;
            for (label, v, default) in [("min", &mut f.min, col.min), ("max", &mut f.max, col.max)] {
                ui.horizontal(|ui| {
                    let mut on = v.is_some();
                    if ui.checkbox(&mut on, label).changed() {
                        *v = if on {
                            Some(if default.is_finite() { default } else { 0.0 })
                        } else {
                            None
                        };
                    }
                    if let Some(x) = v {
                        ui.add(egui::DragValue::new(x).speed(speed));
                    } else {
                        ui.label(RichText::new("unbounded").weak());
                    }
                });
            }
            if col.min.is_finite() {
                ui.label(
                    RichText::new(format!(
                        "data: {} … {}",
                        crate::data::format_sig(col.min, 6),
                        crate::data::format_sig(col.max, 6)
                    ))
                    .weak()
                    .small(),
                );
            }
        }
    }
}

fn smoothing_section(ui: &mut Ui, sm: &mut SmoothConfig) {
    section(ui, "Smoothing", |ui| {
        ui.checkbox(&mut sm.enabled, RichText::new("Smooth the curves").strong())
            .on_hover_text("Applied to every curve of this plot (after the processing above, if any)");
        ui.add_enabled_ui(sm.enabled, |ui| {
            egui::Grid::new("smooth_grid")
                .num_columns(2)
                .spacing([10.0, 6.0])
                .show(ui, |ui| {
                    ui.label("Method");
                    egui::ComboBox::from_id_salt("smooth_method")
                        .selected_text(sm.method.label())
                        .width(210.0)
                        .show_ui(ui, |ui| {
                            for m in SmoothMethod::ALL {
                                ui.selectable_value(&mut sm.method, m, m.label())
                                    .on_hover_text(m.help());
                            }
                        });
                    ui.end_row();

                    match sm.method {
                        SmoothMethod::Gaussian => {
                            ui.label("σ (points)")
                                .on_hover_text("Standard deviation of the bell, in points: bigger = smoother");
                            ui.add(
                                egui::Slider::new(&mut sm.sigma, 0.5..=50.0)
                                    .logarithmic(true)
                                    .max_decimals(1),
                            );
                            ui.end_row();
                        }
                        SmoothMethod::XWindow => {
                            ui.label("Width (X units)")
                                .on_hover_text("Points closer than ± width / 2 are averaged (e.g. 0.5 = ±0.25 GHz)");
                            let speed = (sm.x_width.abs() * 0.01).max(1e-9);
                            ui.add(
                                egui::DragValue::new(&mut sm.x_width)
                                    .range(1e-12..=1e15)
                                    .speed(speed)
                                    .max_decimals(6),
                            );
                            ui.end_row();
                        }
                        _ => {
                            let label = if sm.method == SmoothMethod::Exponential {
                                "Span (points)"
                            } else {
                                "Window (points)"
                            };
                            ui.label(label)
                                .on_hover_text("Number of points used for each value: bigger = smoother");
                            let mut w = sm.window as u32;
                            if ui
                                .add(egui::Slider::new(&mut w, 3..=301).logarithmic(true).step_by(2.0))
                                .changed()
                            {
                                sm.window = (w as usize) | 1;
                            }
                            ui.end_row();
                        }
                    }
                    if sm.method == SmoothMethod::SavitzkyGolay {
                        ui.label("Polynomial order")
                            .on_hover_text("2 or 3 for most curves; higher keeps sharper details but smooths less");
                        let max = sm.window.saturating_sub(1).clamp(1, 6);
                        ui.add(egui::Slider::new(&mut sm.order, 1..=max));
                        ui.end_row();
                    }
                    ui.label("Passes")
                        .on_hover_text("Apply the smoothing several times (smoother)");
                    ui.add(egui::Slider::new(&mut sm.passes, 1..=5));
                    ui.end_row();
                });
            ui.label(RichText::new(sm.method.help()).weak().small());
            ui.checkbox(&mut sm.keep_original, "Keep the raw curves (shown lighter)");
            ui.horizontal(|ui| {
                ui.label(RichText::new("Presets:").weak());
                for (name, m, w, o) in [
                    ("Light", SmoothMethod::SavitzkyGolay, 7, 2),
                    ("Medium", SmoothMethod::SavitzkyGolay, 15, 2),
                    ("Strong", SmoothMethod::SavitzkyGolay, 41, 3),
                    ("Remove spikes", SmoothMethod::Median, 5, 2),
                ] {
                    if ui.small_button(name).clicked() {
                        sm.method = m;
                        sm.window = w;
                        sm.order = o;
                        sm.passes = 1;
                    }
                }
            });
        });
    });
}

fn process_data(app: &mut App, ui: &mut Ui) {
    let cfg = &mut app.plots[app.active].cfg;
    section(ui, "Processing", |ui| {
        ui.checkbox(
            &mut cfg.process.enabled,
            RichText::new("Apply a processing to the curves").strong(),
        );
        ui.add_enabled_ui(cfg.process.enabled, |ui| {
            ui.horizontal(|ui| {
                ui.label("Function");
                egui::ComboBox::from_id_salt("agg_func")
                    .selected_text(cfg.process.func.label())
                    .show_ui(ui, |ui| {
                        for f in AggFunc::ALL {
                            ui.selectable_value(&mut cfg.process.func, f, f.label());
                        }
                    });
            });
            if cfg.series.is_empty() {
                ui.label("Points sharing the same X value inside each curve are combined.");
            } else {
                ui.label("Combine the curves across (one value per X):");
                for s in cfg.series.clone() {
                    let mut on = cfg.process.collapse.contains(&s);
                    if ui.checkbox(&mut on, &s).changed() {
                        if on {
                            cfg.process.collapse.push(s);
                        } else {
                            cfg.process.collapse.retain(|c| *c != s);
                        }
                    }
                }
                if cfg.process.collapse.is_empty() {
                    ui.label(
                        RichText::new(
                            "Nothing checked: only the points with the same X inside each curve are combined.",
                        )
                        .weak()
                        .small(),
                    );
                } else {
                    ui.label(
                        RichText::new(
                            "Example: series = SN + temperature, combine across SN → one mean curve per temperature.",
                        )
                        .weak()
                        .small(),
                    );
                }
                ui.checkbox(
                    &mut cfg.process.keep_original,
                    "Keep the original curves (shown lighter)",
                );
            }
        });
    });
    smoothing_section(ui, &mut cfg.process.smooth);
    section(ui, "Functions", |ui| {
        ui.label("Mean, median, min, max, sum, RMS (√mean(y²)), standard deviation, range (max − min), count.");
        ui.label(
            RichText::new("For computed columns (dB ↔ linear, Hz → GHz, differences…), use the Functions page.").weak(),
        );
    });
}

fn view_data(app: &mut App, ui: &mut Ui) {
    let i = app.active;
    let ds = app.dataset(&app.plots[i].cfg.source).cloned();
    let entry = &mut app.plots[i];
    let (Some(ds), Some(data)) = (ds, entry.state.data.clone()) else {
        ui.label("No data");
        return;
    };
    ui.horizontal(|ui| {
        ui.label(format!("{} rows match the filters", thousands(data.rows.len())));
        ui.checkbox(&mut entry.ui.view_all_columns, "All columns");
    });
    let cfg = &entry.cfg;
    let cols: Vec<usize> = if entry.ui.view_all_columns {
        (0..ds.columns.len()).collect()
    } else {
        let mut names: Vec<&String> = Vec::new();
        names.extend(cfg.series.iter());
        names.push(&cfg.x);
        names.extend(cfg.ys.iter());
        names.extend(cfg.filters.iter().map(|f| &f.column));
        let mut out = Vec::new();
        for n in names {
            if let Some(c) = ds.column_index(n)
                && !out.contains(&c)
            {
                out.push(c);
            }
        }
        out
    };
    data_table(ui, egui::Id::new(("view_data", cfg.id)), &ds, Some(&data.rows), &cols);
}

// =============================================================================
// Layout tab
// =============================================================================

fn axis_editor(ui: &mut Ui, id: &str, ax: &mut AxisLayout, view: Option<(f64, f64)>) {
    ui.horizontal(|ui| {
        ui.checkbox(&mut ax.auto_range, "Automatic range");
        if let Some((lo, hi)) = view
            && ui.small_button("Use current zoom").clicked()
        {
            ax.auto_range = false;
            ax.min = lo;
            ax.max = hi;
        }
    });
    if !ax.auto_range {
        let speed = ((ax.max - ax.min).abs() / 200.0).max(1e-9);
        ui.horizontal(|ui| {
            ui.label("from");
            ui.add(egui::DragValue::new(&mut ax.min).speed(speed));
            ui.label("to");
            ui.add(egui::DragValue::new(&mut ax.max).speed(speed));
        });
        if ax.min >= ax.max {
            ui.label(RichText::new("min must be smaller than max").color(ERR));
        }
    }
    ui.horizontal(|ui| {
        ui.checkbox(&mut ax.auto_ticks, "Automatic ticks");
        if !ax.auto_ticks {
            ui.label("step");
            ui.add(egui::DragValue::new(&mut ax.step).speed(0.05).range(1e-12..=1e12));
        }
    });
    ui.horizontal(|ui| {
        ui.checkbox(&mut ax.auto_decimals, "Automatic decimals");
        if !ax.auto_decimals {
            ui.add(egui::DragValue::new(&mut ax.decimals).range(0..=10));
        }
        ui.checkbox(&mut ax.grid, "Grid");
    });
    let _ = id;
}

fn layout_tab(app: &mut App, ui: &mut Ui) {
    let fonts = Arc::clone(&app.fonts);
    let i = app.active;
    let view = app.plots[i].state.view;
    let (auto_x, auto_y) = app.plots[i]
        .state
        .data
        .as_ref()
        .map(|d| (d.x_label.clone(), d.y_label.clone()))
        .unwrap_or_default();
    let mut apply_all = false;
    {
        let lay = &mut app.plots[i].cfg.layout;
        section(ui, "Title", |ui| {
            ui.checkbox(&mut lay.show_title, "Show a title");
            ui.add(
                egui::TextEdit::singleline(&mut lay.title)
                    .hint_text("Plot title")
                    .desired_width(f32::INFINITY),
            );
            font_editor(ui, "title", &mut lay.title_font, &fonts);
        });
        section(ui, "Axis labels", |ui| {
            egui::Grid::new("labels_grid").num_columns(2).show(ui, |ui| {
                ui.label("X label");
                ui.add(
                    egui::TextEdit::singleline(&mut lay.x.label)
                        .hint_text(auto_x)
                        .desired_width(240.0),
                );
                ui.end_row();
                ui.label("Y label");
                ui.add(
                    egui::TextEdit::singleline(&mut lay.y.label)
                        .hint_text(auto_y)
                        .desired_width(240.0),
                );
                ui.end_row();
            });
            ui.label("Font");
            font_editor(ui, "labels", &mut lay.label_font, &fonts);
            ui.label("Tick labels font");
            font_editor(ui, "ticks", &mut lay.tick_font, &fonts);
        });
        section(ui, "X axis", |ui| {
            axis_editor(ui, "x", &mut lay.x, view.map(|v| (v[0], v[1])))
        });
        section(ui, "Y axis", |ui| {
            axis_editor(ui, "y", &mut lay.y, view.map(|v| (v[2], v[3])))
        });
        section(ui, "Legend", |ui| {
            ui.checkbox(&mut lay.legend.show, "Show the legend");
            ui.horizontal(|ui| {
                ui.label("Position");
                egui::ComboBox::from_id_salt("legend_pos")
                    .selected_text(lay.legend.position.label())
                    .show_ui(ui, |ui| {
                        for p in LegendPos::PRESETS {
                            ui.selectable_value(&mut lay.legend.position, p, p.label());
                        }
                    });
            });
            ui.label(
                RichText::new(
                    "Tip: drag the legend with the mouse; drop it on an edge of the graph to put it outside.",
                )
                .weak()
                .small(),
            );
            ui.horizontal(|ui| {
                ui.label("Title");
                ui.add(
                    egui::TextEdit::singleline(&mut lay.legend.title)
                        .hint_text("(none)")
                        .desired_width(200.0),
                );
            });
            font_editor(ui, "legend", &mut lay.legend.font, &fonts);
            ui.horizontal(|ui| {
                ui.checkbox(&mut lay.legend.background, "Background");
                ui.checkbox(&mut lay.legend.border, "Border");
            });
        });
        section(ui, "Colors", |ui| {
            ui.horizontal(|ui| {
                ui.label("Background");
                color_button(ui, &mut lay.background);
                ui.label("Grid");
                color_button(ui, &mut lay.grid_color);
            });
        });
        ui.add_space(4.0);
        if ui
            .button("Apply this layout to all plots")
            .on_hover_text("Copy fonts, legend, grid and colors to every plot (titles, labels and ranges are kept)")
            .clicked()
        {
            apply_all = true;
        }
    }
    if apply_all {
        let src = app.plots[i].cfg.layout.clone();
        for (k, p) in app.plots.iter_mut().enumerate() {
            if k == i {
                continue;
            }
            let l = &mut p.cfg.layout;
            l.title_font = src.title_font.clone();
            l.label_font = src.label_font.clone();
            l.tick_font = src.tick_font.clone();
            l.legend = src.legend.clone();
            l.background = src.background;
            l.grid_color = src.grid_color;
            l.show_title = src.show_title;
            l.x.grid = src.x.grid;
            l.y.grid = src.y.grid;
        }
        app.set_status("Layout applied to all plots", false);
    }
}

// =============================================================================
// Plot options tab
// =============================================================================

fn options_tab(app: &mut App, ui: &mut Ui) {
    let entry = &mut app.plots[app.active];
    let cfg = &mut entry.cfg;
    section(ui, "Curves", |ui| {
        ui.checkbox(&mut cfg.options.sort_x, "Sort the points by X");
        ui.add_enabled(
            !cfg.options.sort_x,
            egui::Checkbox::new(
                &mut cfg.options.break_on_reversal,
                "Start a new line when X goes backwards (new sweep)",
            ),
        );
        ui.checkbox(
            &mut cfg.options.decimate,
            "Fast drawing of huge curves (min/max decimation)",
        );
    });
    section(ui, "Interaction", |ui| {
        ui.checkbox(&mut cfg.options.hover_readout, "Show the values under the mouse");
        ui.checkbox(&mut cfg.options.crosshair, "Crosshair");
        ui.label(
            RichText::new("Click on a curve to pin a marker with its values; click the marker again to remove it.")
                .weak()
                .small(),
        );
    });
    section(ui, "Markers", |ui| {
        if cfg.markers.is_empty() {
            ui.label(RichText::new("No marker").weak());
        }
        let mut remove = None;
        for (k, m) in cfg.markers.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(format!(
                    "x = {}   y = {}",
                    crate::data::format_sig(m.x, 7),
                    crate::data::format_sig(m.y, 6)
                ))
                .on_hover_text(&m.trace);
                if ui.small_button("🗙").clicked() {
                    remove = Some(k);
                }
            });
        }
        if let Some(k) = remove {
            cfg.markers.remove(k);
        }
        if !cfg.markers.is_empty() && ui.button("Clear all markers").clicked() {
            cfg.markers.clear();
        }
    });
    section(ui, "Export", |ui| {
        ui.checkbox(&mut cfg.export, "Include this plot in the exports");
        let hint = if cfg.layout.show_title && !cfg.layout.title.is_empty() {
            cfg.layout.title.clone()
        } else {
            cfg.name.clone()
        };
        ui.horizontal(|ui| {
            ui.label("Slide title");
            ui.add(
                egui::TextEdit::singleline(&mut cfg.slide_title)
                    .hint_text(hint)
                    .desired_width(220.0),
            );
        });
    });
}

// =============================================================================
// Trace options tab
// =============================================================================

fn traces_tab(app: &mut App, ui: &mut Ui) {
    let i = app.active;
    let mut apply_all = false;
    {
        let entry = &mut app.plots[i];
        let cfg = &mut entry.cfg;
        section(ui, "Defaults", |ui| {
            egui::Grid::new("trace_defaults").num_columns(2).show(ui, |ui| {
                ui.label("Palette");
                egui::ComboBox::from_id_salt("palette")
                    .selected_text(cfg.traces.palette.label())
                    .show_ui(ui, |ui| {
                        for p in Palette::ALL {
                            ui.selectable_value(&mut cfg.traces.palette, p, p.label());
                        }
                    });
                ui.end_row();
                ui.label("Line width");
                ui.add(
                    egui::DragValue::new(&mut cfg.traces.width)
                        .range(0.3..=12.0)
                        .speed(0.05),
                );
                ui.end_row();
                ui.label("Line style");
                line_combo(ui, "default_line", &mut cfg.traces.line);
                ui.end_row();
                ui.label("Custom pattern").on_hover_text(
                    "Dashes of the “Custom” line style: dash gap dash gap… in multiples of the line width \
                     (6 2.5 1.5 2.5 = dash-dot)",
                );
                ui.horizontal(|ui| {
                    let id = ui.id().with("custom_dash_text");
                    let mut text: String = ui
                        .data(|d| d.get_temp(id))
                        .unwrap_or_else(|| crate::plot::style::format_dash(&cfg.traces.custom_dash));
                    let r = ui.add(egui::TextEdit::singleline(&mut text).desired_width(110.0));
                    let ok = crate::plot::style::parse_dash(&text);
                    if r.changed()
                        && let Some(d) = &ok
                    {
                        cfg.traces.custom_dash = d.clone();
                    }
                    if r.has_focus() {
                        ui.data_mut(|d| d.insert_temp(id, text.clone()));
                    } else {
                        ui.data_mut(|d| d.remove::<String>(id));
                    }
                    if ok.is_none() {
                        ui.label(RichText::new("⚠ numbers > 0").color(super::ERR).small());
                    }
                    if ui.small_button("Reset").clicked() {
                        cfg.traces.custom_dash = crate::plot::config::DEFAULT_CUSTOM_DASH.to_vec();
                    }
                });
                ui.end_row();
                ui.label("Markers");
                marker_combo(ui, "default_marker", &mut cfg.traces.marker);
                ui.end_row();
                ui.label("Marker size");
                ui.add(
                    egui::DragValue::new(&mut cfg.traces.marker_size)
                        .range(1.0..=15.0)
                        .speed(0.05),
                );
                ui.end_row();
            });
            ui.checkbox(
                &mut cfg.traces.temperature_colors,
                "Temperature styles: −40 °C blue custom dashes, 25 °C green solid, 85 °C red dashed",
            )
            .on_hover_text(
                "Used when a temperature column (e.g. Set_TempC) is the series of the plot. \
                 With several Y columns, the 2nd and 3rd ones are drawn in lighter colors.",
            );
            if ui.button("Apply these defaults to all plots").clicked() {
                apply_all = true;
            }
        });

        let Some(data) = entry.state.data.clone() else {
            return;
        };
        let styles = trace_styles(cfg, &data);
        section(ui, &format!("Curves ({})", data.traces.len()), |ui| {
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut entry.ui.trace_search)
                        .hint_text("search…")
                        .desired_width(140.0),
                );
                if ui.small_button("Show all").clicked() {
                    for t in &data.traces {
                        if let Some(o) = cfg.overrides.get_mut(&t.key) {
                            o.visible = true;
                        }
                    }
                }
                if ui.small_button("Hide all").clicked() {
                    for t in &data.traces {
                        cfg.overrides.entry(t.key.clone()).or_default().visible = false;
                    }
                }
                if ui
                    .small_button("Reset")
                    .on_hover_text("Remove every per-curve customization")
                    .clicked()
                {
                    cfg.overrides.clear();
                }
            });
            let search = entry.ui.trace_search.to_lowercase();
            egui::Grid::new("traces_grid")
                .num_columns(6)
                .striped(true)
                .spacing([6.0, 4.0])
                .show(ui, |ui| {
                    ui.label("");
                    ui.label("Color");
                    ui.label("Width");
                    ui.label("Line");
                    ui.label("Marker");
                    ui.label("Legend label");
                    ui.end_row();
                    for (t, st) in data.traces.iter().zip(&styles).take(500) {
                        if !search.is_empty() && !t.label.to_lowercase().contains(&search) {
                            continue;
                        }
                        let o = cfg.overrides.entry(t.key.clone()).or_default();
                        ui.checkbox(&mut o.visible, "");
                        let mut color = st.color;
                        if color_button(ui, &mut color).changed() {
                            o.color = Some(color);
                        }
                        let mut w = st.width;
                        if ui
                            .add(egui::DragValue::new(&mut w).range(0.3..=12.0).speed(0.05))
                            .changed()
                        {
                            o.width = Some(w);
                        }
                        let mut line = st.line;
                        if line_combo(ui, &format!("line_{}", t.key), &mut line) {
                            o.line = Some(line);
                        }
                        let mut marker = st.marker;
                        if marker_combo(ui, &format!("marker_{}", t.key), &mut marker) {
                            o.marker = Some(marker);
                        }
                        ui.add(
                            egui::TextEdit::singleline(&mut o.label)
                                .hint_text(t.label.clone())
                                .desired_width(170.0),
                        );
                        ui.end_row();
                    }
                });
            cfg.overrides.retain(|_, o| !o.is_default());
            if data.traces.len() > 500 {
                ui.label(RichText::new("Only the first 500 curves are listed.").weak());
            }
        });
    }
    if apply_all {
        let src = app.plots[i].cfg.traces.clone();
        for p in &mut app.plots {
            p.cfg.traces = src.clone();
        }
        app.set_status("Trace defaults applied to all plots", false);
    }
    let _ = TraceOverride::default;
}

fn line_combo(ui: &mut Ui, id: &str, v: &mut LineKind) -> bool {
    let mut changed = false;
    egui::ComboBox::from_id_salt(("line_kind", id))
        .width(90.0)
        .selected_text(v.label())
        .show_ui(ui, |ui| {
            for k in LineKind::ALL {
                changed |= ui.selectable_value(v, k, k.label()).changed();
            }
        });
    changed
}

fn marker_combo(ui: &mut Ui, id: &str, v: &mut MarkerKind) -> bool {
    let mut changed = false;
    egui::ComboBox::from_id_salt(("marker_kind", id))
        .width(110.0)
        .selected_text(v.label())
        .show_ui(ui, |ui| {
            for k in MarkerKind::ALL {
                changed |= ui.selectable_value(v, k, k.label()).changed();
            }
        });
    changed
}
