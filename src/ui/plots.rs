//! "Plots" page: the list of plots (left), the toolbar, the plots stacked
//! vertically (the mouse wheel goes from one plot to the next) and the
//! settings of the current plot (right).

use std::sync::Arc;

use egui::{
    Align, Color32, CornerRadius, Id, Layout, Margin, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Ui, UiBuilder,
    Vec2,
};

use super::{ERR, NAVY, NAVY_LIGHT, WARN, plot_settings, section, thousands};
use crate::app::App;
use crate::plot::render::ImageFormat;
use crate::plot::view::{EditTarget, show_plot};

/// Gap between two stacked plots.
const GAP: f32 = 16.0;

#[derive(Clone, Copy, PartialEq)]
enum Action {
    Delete,
    Csv,
    Png,
    Svg,
    Duplicate,
    Reset,
    ZoomIn,
    ZoomOut,
    FitY,
    Previous,
    Axes,
}

pub fn show(app: &mut App, ui: &mut Ui) {
    if app.plots.is_empty() {
        ui.heading("Plots");
        ui.add_space(6.0);
        section(ui, "Create a plot", |ui| {
            if app.sources.iter().all(|s| s.ds.is_none()) {
                ui.label("Load a data source first (File Select page).");
            }
            for (k, name) in loaded_sources(app) {
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

    egui::Panel::left("ct_plot_strip")
        .resizable(true)
        .default_size(205.0)
        .size_range(150.0..=340.0)
        .show(ui, |ui| strip(app, ui));
    if app.plots.is_empty() {
        return;
    }

    let mut action = None;
    egui::Panel::top("ct_plot_toolbar")
        .frame(egui::Frame::new().inner_margin(Margin::symmetric(4, 4)))
        .show(ui, |ui| action = toolbar(app, ui));
    let i = app.active;
    match action {
        Some(Action::Delete) => {
            app.remove_plot(i);
            return;
        }
        Some(Action::Csv) => app.save_plot_csv_dialog(i),
        Some(Action::Png) => app.save_plot_image_dialog(i, ImageFormat::Png),
        Some(Action::Svg) => app.save_plot_image_dialog(i, ImageFormat::Svg),
        Some(Action::Duplicate) => {
            app.duplicate_plot(i);
            return;
        }
        Some(Action::Reset) => app.plots[i].state.reset_view = true,
        Some(Action::ZoomIn) => app.plots[i].state.zoom(0.7),
        Some(Action::ZoomOut) => app.plots[i].state.zoom(1.0 / 0.7),
        Some(Action::FitY) => {
            let p = &mut app.plots[i];
            p.state.fit_y(&p.cfg);
        }
        Some(Action::Previous) => app.plots[i].state.previous_view(),
        Some(Action::Axes) => {
            let pos = ui.ctx().content_rect().center() - Vec2::new(180.0, 160.0);
            app.plots[i].state.open_editor(EditTarget::All, pos);
        }
        None => {}
    }

    egui::Panel::right("ct_plot_settings")
        .resizable(true)
        .default_size(440.0)
        .size_range(320.0..=900.0)
        .show(ui, |ui| plot_settings::show(app, ui));

    egui::CentralPanel::default()
        .frame(egui::Frame::new().inner_margin(Margin::same(4)))
        .show(ui, |ui| navigator(app, ui));
}

fn loaded_sources(app: &App) -> Vec<(String, String)> {
    app.sources
        .iter()
        .filter(|s| s.ds.is_some())
        .map(|s| (s.key.clone(), s.name()))
        .collect()
}

// =============================================================================
// Left: list of the plots
// =============================================================================

fn strip(app: &mut App, ui: &mut Ui) {
    let keys = loaded_sources(app);
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("Plots ({})", app.plots.len())).strong());
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.menu_button("+ New", |ui| {
                if keys.is_empty() {
                    ui.label("No data source loaded");
                }
                for (k, name) in &keys {
                    if ui.button(name).clicked() {
                        app.add_plot(k);
                        ui.close();
                    }
                }
            })
            .response
            .on_hover_text("New plot");
        });
    });
    ui.separator();

    let fonts = Arc::clone(&app.fonts);
    let w = ui.available_width();
    let thumb_h = (w * 0.62).round();
    let row_h = thumb_h + 30.0;
    let n = app.plots.len();
    // keep the current plot visible when it changes (wheel, keyboard)
    let last_id = Id::new("ct_strip_last_active");
    let last: Option<usize> = ui.data(|d| d.get_temp(last_id));
    let follow = last != Some(app.active);
    ui.data_mut(|d| d.insert_temp(last_id, app.active));

    let mut select = None;
    let mut menu_action: Option<(usize, &str)> = None;
    let mut scroll = egui::ScrollArea::vertical()
        .id_salt("ct_strip_scroll")
        .auto_shrink([false, false]);
    if follow {
        let spacing = ui.spacing().item_spacing.y;
        let target = app.active as f32 * (row_h + spacing) - (ui.available_height() - row_h) * 0.5;
        scroll = scroll.vertical_scroll_offset(target.max(0.0));
    }
    scroll.show_rows(ui, row_h, n, |ui, range| {
        for i in range {
            let active = i == app.active;
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, row_h), Sense::click());
            let painter = ui.painter_at(rect);
            let bg = if active {
                NAVY_LIGHT.gamma_multiply(0.16)
            } else if resp.hovered() {
                ui.visuals().widgets.hovered.weak_bg_fill
            } else {
                Color32::TRANSPARENT
            };
            painter.rect_filled(rect, 6.0, bg);
            if active {
                painter.rect_stroke(rect.shrink(1.0), 6.0, Stroke::new(2.0, NAVY_LIGHT), StrokeKind::Inside);
            }
            let badge = Rect::from_min_size(rect.min + Vec2::new(6.0, 5.0), Vec2::new(22.0, 18.0));
            painter.rect_filled(badge, 4.0, if active { NAVY } else { Color32::from_gray(140) });
            painter.text(
                badge.center(),
                egui::Align2::CENTER_CENTER,
                (i + 1).to_string(),
                egui::FontId::proportional(12.0),
                Color32::WHITE,
            );
            let name = &app.plots[i].cfg.name;
            let galley = painter.layout(
                name.clone(),
                egui::FontId::proportional(13.0),
                ui.visuals().text_color(),
                rect.width() - 40.0,
            );
            painter.galley(Pos2::new(badge.right() + 6.0, rect.top() + 6.0), galley, Color32::BLACK);
            let trect = Rect::from_min_size(rect.min + Vec2::new(4.0, 26.0), Vec2::new(w - 8.0, thumb_h));
            let p = &mut app.plots[i];
            show_plot(ui, trect, &mut p.cfg, &mut p.state, &fonts, false);
            if resp.clicked() {
                select = Some(i);
            }
            resp.on_hover_text("Click to show this plot (or use the mouse wheel on the plot)")
                .context_menu(|ui| {
                    for (label, a) in [
                        ("Copy / Split Plot", "dup"),
                        ("⏶ Move up", "up"),
                        ("⏷ Move down", "down"),
                        ("🗑 Delete", "del"),
                    ] {
                        if ui.button(label).clicked() {
                            menu_action = Some((i, a));
                            ui.close();
                        }
                    }
                });
        }
    });
    if let Some(i) = select {
        app.active = i;
    }
    match menu_action {
        Some((i, "dup")) => app.duplicate_plot(i),
        Some((i, "up")) => app.move_plot(i, -1),
        Some((i, "down")) => app.move_plot(i, 1),
        Some((i, "del")) => app.remove_plot(i),
        _ => {}
    }
}

// =============================================================================
// Toolbar
// =============================================================================

fn toolbar(app: &mut App, ui: &mut Ui) -> Option<Action> {
    let mut action = None;
    let i = app.active;
    ui.horizontal(|ui| {
        let p = &mut app.plots[i];
        ui.add(
            egui::TextEdit::singleline(&mut p.cfg.name)
                .desired_width(190.0)
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
            let mut b = |ui: &mut Ui, text: RichText, tip: &str, a: Action| {
                if ui.button(text).on_hover_text(tip).clicked() {
                    action = Some(a);
                }
            };
            b(ui, RichText::new("🗑").color(ERR), "Delete this plot", Action::Delete);
            b(
                ui,
                "⬇ Download Data".into(),
                "Save the curves of this plot as CSV",
                Action::Csv,
            );
            b(
                ui,
                "SVG".into(),
                "Save this plot as a vector image (SVG: editable in Inkscape, PowerPoint, Illustrator…)",
                Action::Svg,
            );
            b(
                ui,
                "🖼 PNG".into(),
                "Save this plot as a high resolution image",
                Action::Png,
            );
            b(ui, "Copy / Split Plot".into(), "Duplicate this plot", Action::Duplicate);
        });
    });
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new("Mouse:").weak());
        ui.selectable_value(&mut app.box_zoom, false, "✋ Pan")
            .on_hover_text("Left-drag moves the view (right-drag: zoom rectangle)");
        ui.selectable_value(&mut app.box_zoom, true, "⬚ Zoom area")
            .on_hover_text("Left-drag draws the rectangle to zoom on");
        ui.separator();
        let st = &app.plots[i].state;
        let has_history = !st.history.is_empty();
        let mut b = |ui: &mut Ui, enabled: bool, text: &str, tip: &str, a: Action| {
            if ui
                .add_enabled(enabled, egui::Button::new(text))
                .on_hover_text(tip)
                .clicked()
            {
                action = Some(a);
            }
        };
        b(
            ui,
            true,
            "🔍 +",
            "Zoom in (or Ctrl + mouse wheel on the plot)",
            Action::ZoomIn,
        );
        b(ui, true, "🔍 −", "Zoom out", Action::ZoomOut);
        b(
            ui,
            true,
            "↕ Fit Y",
            "Fit the Y axis to the curves visible in the current X range",
            Action::FitY,
        );
        b(
            ui,
            has_history,
            "⏴ Previous view",
            "Go back to the previous zoom",
            Action::Previous,
        );
        b(
            ui,
            true,
            "⟲ Original scale",
            "Back to the original range (or double-click on the plot)",
            Action::Reset,
        );
        ui.separator();
        b(
            ui,
            true,
            "📐 Axes & title…",
            "Edit the axis titles, ranges and tick steps (or click directly on an axis / the title)",
            Action::Axes,
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(
                RichText::new("Wheel: next / previous plot · Ctrl + wheel: zoom · Click an axis to edit it")
                    .small()
                    .weak(),
            );
        });
    });
    action
}

// =============================================================================
// Stacked plots, the mouse wheel goes from one to the next
// =============================================================================

fn navigator(app: &mut App, ui: &mut Ui) {
    let area = ui.available_rect_before_wrap();
    let n = app.plots.len();
    let nav_id = Id::new("ct_plot_nav");

    // ---- wheel / keyboard: previous / next plot ---------------------------------
    if n > 1 && ui.rect_contains_pointer(area) {
        let (dy, now) = ui.input(|i| {
            let mut dy = 0.0;
            for e in &i.events {
                if let egui::Event::MouseWheel {
                    unit, delta, modifiers, ..
                } = e
                {
                    if modifiers.command || modifiers.ctrl {
                        continue; // zoom
                    }
                    dy += match unit {
                        egui::MouseWheelUnit::Point => delta.y,
                        egui::MouseWheelUnit::Line => delta.y * 40.0,
                        egui::MouseWheelUnit::Page => delta.y * area.height(),
                    };
                }
            }
            (dy, i.time)
        });
        if dy != 0.0 {
            let (mut acc, last): (f32, f64) = ui.data(|d| d.get_temp(nav_id)).unwrap_or((0.0, f64::NEG_INFINITY));
            if now - last < 0.28 {
                acc = 0.0; // inertia of the previous move
            } else {
                acc += dy;
            }
            let mut last = last;
            if acc.abs() >= 25.0 {
                if acc < 0.0 && app.active + 1 < n {
                    app.active += 1;
                } else if acc > 0.0 && app.active > 0 {
                    app.active -= 1;
                }
                acc = 0.0;
                last = now;
            }
            ui.data_mut(|d| d.insert_temp(nav_id, (acc, last)));
        }
    }
    let typing = ui.ctx().memory(|m| m.focused().is_some());
    if !typing {
        let (down, up) = ui.input(|i| (i.key_pressed(egui::Key::PageDown), i.key_pressed(egui::Key::PageUp)));
        if down && app.active + 1 < n {
            app.active += 1;
        }
        if up && app.active > 0 {
            app.active -= 1;
        }
    }

    // ---- the plots ------------------------------------------------------------
    let pos = ui
        .ctx()
        .animate_value_with_time(nav_id.with("pos"), app.active as f32, 0.28);
    let step = area.height() + GAP;
    let first = (pos.floor().max(0.0) as usize).min(n - 1);
    let last = (pos.ceil().max(0.0) as usize).min(n - 1);
    let fonts = Arc::clone(&app.fonts);
    let box_zoom = app.box_zoom;
    ui.scope_builder(UiBuilder::new().max_rect(area), |ui| {
        ui.set_clip_rect(area);
        for k in first..=last {
            let r = area.translate(Vec2::new(0.0, (k as f32 - pos) * step));
            if !r.intersects(area) {
                continue;
            }
            let p = &mut app.plots[k];
            p.state.box_zoom = box_zoom;
            show_plot(ui, r, &mut p.cfg, &mut p.state, &fonts, true);
            if k < last {
                let y = r.bottom() + GAP * 0.5;
                ui.painter().line_segment(
                    [Pos2::new(area.left() + 40.0, y), Pos2::new(area.right() - 40.0, y)],
                    Stroke::new(1.0, Color32::from_gray(160)),
                );
            }
        }
    });
    ui.allocate_rect(area, Sense::hover());

    // ---- position indicator (right edge) ----------------------------------------
    if n > 1 {
        indicator(app, ui, area);
    }
}

/// Column of dots on the right edge: current plot, click to jump.
fn indicator(app: &mut App, ui: &mut Ui, area: Rect) {
    let n = app.plots.len();
    let spacing = (area.height() * 0.6 / n as f32).clamp(4.0, 14.0);
    let h = spacing * (n - 1) as f32;
    let x = area.right() - 9.0;
    let y0 = area.center().y - h * 0.5;
    let bar = Rect::from_min_max(Pos2::new(x - 7.0, y0 - 10.0), Pos2::new(x + 7.0, y0 + h + 10.0));
    let painter = ui.painter();
    painter.rect_filled(bar, CornerRadius::same(7), Color32::from_white_alpha(200));
    for k in 0..n {
        let c = Pos2::new(x, y0 + k as f32 * spacing);
        let active = k == app.active;
        painter.circle_filled(
            c,
            if active { 4.0 } else { (spacing * 0.28).clamp(1.5, 2.8) },
            if active { NAVY_LIGHT } else { Color32::from_gray(150) },
        );
    }
    let resp = ui
        .interact(bar, Id::new("ct_plot_indicator"), Sense::click())
        .on_hover_text(format!(
            "Plot {} / {n} — click a dot to jump, wheel to scroll",
            app.active + 1
        ));
    if resp.clicked()
        && let Some(p) = resp.interact_pointer_pos()
    {
        let k = ((p.y - y0) / spacing.max(1.0)).round().clamp(0.0, (n - 1) as f32) as usize;
        app.active = k;
    }
}
