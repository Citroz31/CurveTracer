//! User interface: top bar, navigation, status bar and the pages.

pub mod data_page;
pub mod export_page;
pub mod files;
pub mod functions;
pub mod help;
pub mod plot_settings;
pub mod plots;
pub mod start;
pub mod table;

use egui::{Align, Color32, Frame, Key, KeyboardShortcut, Layout, Margin, Modifiers, RichText, Stroke, Ui};

use crate::app::{App, Page, Switch, setup_style};
use crate::fonts::FontBook;
use crate::plot::config::{FontSpec, Rgba};

pub const NAVY: Color32 = Color32::from_rgb(22, 45, 84);
pub const NAVY_LIGHT: Color32 = Color32::from_rgb(52, 92, 160);
pub const OK_GREEN: Color32 = Color32::from_rgb(40, 140, 70);
pub const WARN: Color32 = Color32::from_rgb(200, 120, 20);
pub const ERR: Color32 = Color32::from_rgb(190, 50, 50);

pub fn show(app: &mut App, ui: &mut Ui) {
    start::pending_switch_dialog(app, ui.ctx());
    if app.show_start {
        start::show(app, ui);
        preview_dropped_files(ui.ctx());
        return;
    }
    shortcuts(app, ui.ctx());
    top_bar(app, ui);
    status_bar(app, ui);
    nav_bar(app, ui);
    egui::CentralPanel::default().show(ui, |ui| match app.page {
        Page::Files => files::show(app, ui),
        Page::Data => data_page::show(app, ui),
        Page::Functions => functions::show(app, ui),
        Page::Plots => plots::show(app, ui),
        Page::Export => export_page::show(app, ui),
        Page::Help => help::show(ui),
    });
    preview_dropped_files(ui.ctx());
}

fn shortcuts(app: &mut App, ctx: &egui::Context) {
    let open = KeyboardShortcut::new(Modifiers::COMMAND, Key::O);
    let save = KeyboardShortcut::new(Modifiers::COMMAND, Key::S);
    if ctx.input_mut(|i| i.consume_shortcut(&open)) {
        app.open_files_dialog(ctx);
    }
    if ctx.input_mut(|i| i.consume_shortcut(&save)) {
        app.save_workspace_dialog(false);
    }
}

fn top_bar(app: &mut App, ui: &mut Ui) {
    egui::Panel::top("ct_top")
        .frame(Frame::new().fill(NAVY).inner_margin(Margin::symmetric(10, 5)))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.visuals_mut().override_text_color = Some(Color32::WHITE);
                ui.visuals_mut().widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
                ui.visuals_mut().widgets.hovered.weak_bg_fill = NAVY_LIGHT;
                logo(ui);
                ui.add_space(12.0);
                let ctx = ui.ctx().clone();
                ui.menu_button("File", |ui| {
                    ui.visuals_mut().override_text_color = None;
                    if ui.button("📂 Open data files…   (Ctrl+O)").clicked() {
                        app.open_files_dialog(&ctx);
                        ui.close();
                    }
                    ui.separator();
                    if ui.button("🔤 Load a font file (.ttf / .otf)…").clicked() {
                        if let Some(p) = rfd::FileDialog::new().add_filter("Fonts", &["ttf", "otf"]).pick_file() {
                            app.add_font_file(&p, &ctx);
                        }
                        ui.close();
                    }
                });
                workspace_menu(app, ui, &ctx);
                ui.menu_button("View", |ui| {
                    ui.visuals_mut().override_text_color = None;
                    if ui.checkbox(&mut app.dark, "Dark theme").changed() {
                        setup_style(&ctx, app.dark);
                    }
                    for (p, label) in [
                        (Page::Files, "File Select"),
                        (Page::Data, "Data"),
                        (Page::Functions, "Functions"),
                        (Page::Plots, "Plots"),
                        (Page::Export, "Export"),
                    ] {
                        if ui.button(label).clicked() {
                            app.page = p;
                            ui.close();
                        }
                    }
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui
                        .button(RichText::new("💾 QuickSave").strong().color(Color32::WHITE))
                        .on_hover_text("Save the workspace (sources, functions, plots)")
                        .clicked()
                    {
                        app.save_workspace_dialog(false);
                    }
                    if app.is_dirty_cached(ui.input(|i| i.time)) {
                        ui.label(
                            RichText::new("● unsaved changes")
                                .small()
                                .color(Color32::from_rgb(255, 200, 120)),
                        );
                    }
                    if let Some(b) = &app.busy {
                        ui.spinner();
                        ui.label(b.clone());
                    }
                });
            });
        });
}

/// "Workspace ▾" menu of the top bar: new / open / recent / save / close.
fn workspace_menu(app: &mut App, ui: &mut Ui, ctx: &egui::Context) {
    let name = app.workspace_name().unwrap_or_else(|| "No workspace".to_owned());
    ui.menu_button(
        RichText::new(format!("📁 {name}")).strong().color(Color32::WHITE),
        |ui| {
            ui.visuals_mut().override_text_color = None;
            if ui.button("+ New workspace…").clicked() {
                app.request_switch(Switch::Close, ctx);
                ui.close();
            }
            if ui.button("📂 Open workspace…").clicked() {
                app.request_switch(Switch::OpenDialog, ctx);
                ui.close();
            }
            let recent: Vec<_> = app.recent.iter().filter(|r| r.exists()).take(10).cloned().collect();
            ui.add_enabled_ui(!recent.is_empty(), |ui| {
                ui.menu_button("🕘 Recent workspaces", |ui| {
                    for r in recent {
                        if ui
                            .button(r.name())
                            .on_hover_text(format!("{}\n{}", r.path.display(), crate::recent::ago(r.last)))
                            .clicked()
                        {
                            app.request_switch(Switch::Open(r.path.clone()), ctx);
                            ui.close();
                        }
                    }
                });
            });
            ui.separator();
            if ui.button("💾 Save   (Ctrl+S)").clicked() {
                app.save_workspace_dialog(false);
                ui.close();
            }
            if ui.button("💾 Save as…").clicked() {
                app.save_workspace_dialog(true);
                ui.close();
            }
            ui.separator();
            if ui.button("🏠 Start screen (close the workspace)").clicked() {
                app.request_switch(Switch::Close, ctx);
                ui.close();
            }
        },
    )
    .response
    .on_hover_text("Current workspace: create, open or switch workspaces");
}

pub const ACCENT: Color32 = Color32::from_rgb(110, 180, 255);

/// Application logo: a small curve badge and the name, readable on the navy bar.
pub fn logo(ui: &mut Ui) {
    let h = 24.0;
    let (r, _) = ui.allocate_exact_size(egui::vec2(h, h), egui::Sense::hover());
    let p = ui.painter();
    p.rect_filled(r, 5.0, Color32::WHITE);
    let pts: Vec<egui::Pos2> = (0..=24)
        .map(|k| {
            let t = k as f32 / 24.0;
            let y = 0.5 - 0.32 * (t * 6.0).sin() * (1.0 - 0.5 * t);
            egui::pos2(r.left() + 3.0 + t * (h - 6.0), r.top() + y * h)
        })
        .collect();
    p.add(egui::Shape::line(pts, Stroke::new(2.2, NAVY_LIGHT)));
    ui.add_space(4.0);
    let mut job = egui::text::LayoutJob::default();
    let font = egui::FontId::proportional(19.0);
    job.append("Curve", 0.0, egui::TextFormat::simple(font.clone(), Color32::WHITE));
    job.append("Tracer", 0.0, egui::TextFormat::simple(font, ACCENT));
    ui.label(job);
}

fn status_bar(app: &mut App, ui: &mut Ui) {
    egui::Panel::bottom("ct_status")
        .frame(Frame::new().fill(NAVY).inner_margin(Margin::symmetric(10, 3)))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.visuals_mut().override_text_color = Some(Color32::from_gray(225));
                if let Some(s) = &app.status {
                    let age = s.at.elapsed().as_secs_f32();
                    if age < 12.0 || s.error {
                        let color = if s.error {
                            Color32::from_rgb(255, 150, 150)
                        } else {
                            Color32::WHITE
                        };
                        ui.label(RichText::new(&s.text).color(color));
                    }
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let mb = app.memory_bytes() as f64 / 1e6;
                    let rows: usize = app.sources.iter().filter_map(|s| s.ds.as_ref()).map(|d| d.n_rows).sum();
                    ui.label(format!(
                        "Memory: {mb:.1} MB  ·  {} source(s), {rows} rows  ·  {} plot(s)",
                        app.sources.len(),
                        app.plots.len()
                    ));
                });
            });
        });
}

fn nav_bar(app: &mut App, ui: &mut Ui) {
    egui::Panel::right("ct_nav")
        .exact_size(150.0)
        .resizable(false)
        .show(ui, |ui| {
            ui.add_space(8.0);
            let fn_count = app.functions.len();
            let plot_count = app.plots.len();
            let items = [
                (Page::Files, "📂  File Select".to_owned()),
                (Page::Data, "▦  Data".to_owned()),
                (Page::Functions, format!("Σ  Functions ({fn_count})")),
                (Page::Plots, format!("📈  Plots ({plot_count})")),
                (Page::Export, "📤  Export".to_owned()),
                (Page::Help, "❓  Help".to_owned()),
            ];
            for (page, label) in items {
                let selected = app.page == page;
                let text = if selected {
                    RichText::new(label).strong().color(Color32::WHITE)
                } else {
                    RichText::new(label)
                };
                let mut b = egui::Button::new(text).min_size(egui::vec2(134.0, 34.0));
                if selected {
                    b = b.fill(NAVY_LIGHT);
                }
                if ui.add(b).clicked() {
                    app.page = page;
                }
                ui.add_space(2.0);
            }
        });
}

fn preview_dropped_files(ctx: &egui::Context) {
    if ctx.input(|i| i.raw.hovered_files.is_empty()) {
        return;
    }
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("ct_drop")));
    let rect = ctx.content_rect();
    painter.rect_filled(rect, 0.0, Color32::from_black_alpha(160));
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        "Drop CSV / Excel files (or a .ctw workspace) to load them",
        egui::FontId::proportional(24.0),
        Color32::WHITE,
    );
}

// ---- small reusable widgets ---------------------------------------------------

/// A titled box.
pub fn section<R>(ui: &mut Ui, title: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    Frame::group(ui.style())
        .fill(ui.visuals().extreme_bg_color)
        .inner_margin(Margin::same(8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(title).strong().color(ui.visuals().strong_text_color()));
            ui.add_space(2.0);
            add(ui)
        })
        .inner
}

/// A small "chip" label.
pub fn chip(ui: &mut Ui, text: impl Into<String>) {
    Frame::new()
        .stroke(Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color))
        .corner_radius(4)
        .inner_margin(Margin::symmetric(5, 1))
        .show(ui, |ui| {
            ui.label(RichText::new(text.into()).small());
        });
}

/// Color button for an unmultiplied RGBA color.
pub fn color_button(ui: &mut Ui, c: &mut Rgba) -> egui::Response {
    ui.color_edit_button_srgba_unmultiplied(c)
}

/// Family / size / color editor for a font.
pub fn font_editor(ui: &mut Ui, id: &str, spec: &mut FontSpec, fonts: &FontBook) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt(("font_family", id))
            .width(130.0)
            .selected_text(spec.family.clone())
            .show_ui(ui, |ui| {
                for name in fonts.names() {
                    changed |= ui.selectable_value(&mut spec.family, name.clone(), name).changed();
                }
            });
        changed |= ui
            .add(
                egui::DragValue::new(&mut spec.size)
                    .range(5.0..=72.0)
                    .speed(0.2)
                    .suffix(" pt"),
            )
            .changed();
        changed |= color_button(ui, &mut spec.color).changed();
    });
    changed
}

/// Pretty number of rows: 20 119.
pub fn thousands(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push('\u{202F}');
        }
        out.push(c);
    }
    out
}
