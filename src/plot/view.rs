//! Interactive plot widget: egui_plot for the data area (zoom, pan, hover
//! read-out) + title, a mouse-draggable legend and data markers.

use std::ops::RangeInclusive;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};

use egui::{
    Align2, Color32, CursorIcon, FontFamily, FontId, Id, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Ui,
    UiBuilder, Vec2b,
};
use egui_plot::{
    AxisHints, GridInput, GridMark, HoverPosition, Line, LineStyle, MarkerShape, Plot, PlotBounds, PlotPoint,
    PlotPoints, Points,
};

use super::compute::{PlotData, compute};
use super::config::{FontSpec, LegendPos, LineKind, MarkerKind, PlotConfig, Rgba};
use super::geom::{self, Bx, Dock};
use super::render::resolve_view;
use super::style::{LegendMetrics, trace_styles};
use crate::data::{Dataset, format_sig};
use crate::fonts::{FontBook, MONOSPACE, PROPORTIONAL};

pub fn c32(c: Rgba) -> Color32 {
    Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3])
}

pub fn font_id(fonts: &FontBook, spec: &FontSpec) -> FontId {
    let family = match spec.family.as_str() {
        PROPORTIONAL => FontFamily::Proportional,
        MONOSPACE => FontFamily::Monospace,
        name if fonts.contains(name) => FontFamily::Name(name.into()),
        _ => FontFamily::Proportional,
    };
    FontId::new(spec.size.clamp(4.0, 96.0), family)
}

fn bx(r: Rect) -> Bx {
    Bx::new(r.min.x, r.min.y, r.max.x, r.max.y)
}

fn rect(b: Bx) -> Rect {
    Rect::from_min_max(Pos2::new(b.x0, b.y0), Pos2::new(b.x1, b.y1))
}

/// Per-plot runtime state (not saved).
#[derive(Default)]
pub struct PlotState {
    pub data: Option<Arc<PlotData>>,
    data_key: u64,
    pending: Option<(u64, Receiver<PlotData>)>,
    /// The view must be reset to the configured / automatic range.
    pub reset_view: bool,
    view_key: u64,
    /// Visible data range of the last frame.
    pub view: Option<[f64; 4]>,
    /// The legend is being dragged.
    dragging_legend: bool,
}

impl PlotState {
    pub fn is_computing(&self) -> bool {
        self.pending.is_some()
    }

    /// Recomputes the traces in the background when the configuration or the
    /// data changed.
    pub fn update(&mut self, cfg: &PlotConfig, ds: Option<&Arc<Dataset>>, ctx: &egui::Context) {
        if let Some((key, rx)) = &self.pending
            && let Ok(d) = rx.try_recv()
        {
            let key = *key;
            self.pending = None;
            self.data = Some(Arc::new(d));
            self.data_key = key;
            self.reset_view = true;
        }
        let Some(ds) = ds else {
            if self.data_key != 1 {
                self.data = Some(Arc::new(PlotData::error(format!(
                    "Data source not loaded: {}",
                    if cfg.source.is_empty() { "(none)" } else { &cfg.source }
                ))));
                self.data_key = 1;
            }
            return;
        };
        let key = cfg.data_key() ^ ds.generation.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let vk = cfg.view_key();
        if vk != self.view_key {
            self.view_key = vk;
            self.reset_view = true;
        }
        if key == self.data_key || self.pending.as_ref().is_some_and(|(k, _)| *k == key) {
            return;
        }
        let (tx, rx) = channel();
        let ds = Arc::clone(ds);
        let cfg = cfg.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let d = compute(&ds, &cfg);
            let _ = tx.send(d);
            ctx.request_repaint();
        });
        self.pending = Some((key, rx));
    }

    /// Blocks until the pending computation (if any) is done.
    pub fn wait(&mut self) {
        if let Some((key, rx)) = self.pending.take()
            && let Ok(d) = rx.recv()
        {
            self.data = Some(Arc::new(d));
            self.data_key = key;
            self.reset_view = true;
        }
    }
}

struct LegendRow {
    label: String,
    color: Color32,
    line: LineKind,
    marker: MarkerKind,
    width: f32,
    visible: bool,
    key: String,
}

fn line_style(kind: LineKind, width: f32) -> LineStyle {
    match kind {
        LineKind::Dashed => LineStyle::Dashed {
            length: (width * 5.0).max(6.0),
        },
        LineKind::Dotted => LineStyle::Dotted {
            spacing: (width * 3.0).max(4.0),
        },
        _ => LineStyle::Solid,
    }
}

fn marker_shape(kind: MarkerKind) -> MarkerShape {
    match kind {
        MarkerKind::Square => MarkerShape::Square,
        MarkerKind::Diamond => MarkerShape::Diamond,
        MarkerKind::Up => MarkerShape::Up,
        MarkerKind::Down => MarkerShape::Down,
        MarkerKind::Cross => MarkerShape::Cross,
        MarkerKind::Plus => MarkerShape::Plus,
        _ => MarkerShape::Circle,
    }
}

fn tick_formatter(
    step: Option<f64>,
    decimals: Option<usize>,
    cats: Option<Arc<Vec<String>>>,
) -> impl Fn(GridMark, &RangeInclusive<f64>) -> String {
    move |mark: GridMark, _range: &RangeInclusive<f64>| -> String {
        if let Some(cats) = &cats {
            let i = mark.value.round();
            if (mark.value - i).abs() > 1e-6 || i < 0.0 || i as usize >= cats.len() {
                return String::new();
            }
            return cats[i as usize].clone();
        }
        if let Some(s) = step
            && !geom::is_multiple(mark.value, s)
        {
            return String::new();
        }
        let dec = decimals.unwrap_or_else(|| geom::decimals_for_step(step.unwrap_or(mark.step_size)));
        geom::format_tick(mark.value, dec)
    }
}

fn step_spacer(step: f64, categorical: bool) -> impl Fn(GridInput) -> Vec<GridMark> {
    move |input: GridInput| -> Vec<GridMark> {
        let (lo, hi) = input.bounds;
        let s = if categorical {
            let mut s = 1.0;
            while (hi - lo) / s > 60.0 {
                s *= 2.0;
            }
            s
        } else {
            step
        };
        let values = geom::ticks(lo, hi, s);
        if values.is_empty() && !categorical {
            return (egui_plot::log_grid_spacer(10))(input);
        }
        values
            .into_iter()
            .map(|v| GridMark { value: v, step_size: s })
            .collect()
    }
}

/// Shows a plot in `area`. `interactive = false` for thumbnails.
pub fn show_plot(
    ui: &mut Ui,
    area: Rect,
    cfg: &mut PlotConfig,
    st: &mut PlotState,
    fonts: &FontBook,
    interactive: bool,
) {
    let lay = cfg.layout.clone();
    let bg = c32(lay.background);
    let painter = ui.painter_at(area);
    painter.rect_filled(area, 2.0, bg);
    let Some(data) = st.data.clone() else {
        painter.text(
            area.center(),
            Align2::CENTER_CENTER,
            "Computing…",
            FontId::proportional(16.0),
            Color32::GRAY,
        );
        return;
    };
    if let Some(err) = &data.error {
        painter.text(
            area.center(),
            Align2::CENTER_CENTER,
            err,
            FontId::proportional(if interactive { 15.0 } else { 11.0 }),
            Color32::from_rgb(160, 60, 60),
        );
        return;
    }
    let thumb = if interactive { 1.0 } else { 0.55 };
    let mut fig = area.shrink(6.0 * thumb);

    // ---- title --------------------------------------------------------------
    if lay.show_title && !lay.title.trim().is_empty() {
        let mut f = font_id(fonts, &lay.title_font);
        f.size *= thumb;
        let galley = painter.layout_no_wrap(lay.title.clone(), f, c32(lay.title_font.color));
        let pos = Pos2::new(fig.center().x - galley.size().x * 0.5, fig.top());
        let h = galley.size().y;
        painter.galley(pos, galley, Color32::BLACK);
        fig.min.y += h + 6.0 * thumb;
    }

    // ---- legend measurement ---------------------------------------------------
    let styles = trace_styles(cfg, &data);
    let rows: Vec<LegendRow> = data
        .traces
        .iter()
        .zip(&styles)
        .filter(|(_, s)| interactive || s.visible)
        .map(|(t, s)| LegendRow {
            label: s.label.clone(),
            color: c32(s.color),
            line: s.line,
            marker: s.marker,
            width: s.width,
            visible: s.visible,
            key: t.key.clone(),
        })
        .collect();
    let mut legend_font = font_id(fonts, &lay.legend.font);
    legend_font.size *= thumb;
    let show_legend = lay.legend.show && !rows.is_empty();
    let lm = LegendMetrics::new(legend_font.size, !lay.legend.title.is_empty());
    let galleys: Vec<_> = if show_legend {
        rows.iter()
            .map(|r| {
                let color = if r.visible {
                    c32(lay.legend.font.color)
                } else {
                    Color32::from_gray(170)
                };
                painter.layout_no_wrap(r.label.clone(), legend_font.clone(), color)
            })
            .collect()
    } else {
        Vec::new()
    };
    let title_galley = (show_legend && !lay.legend.title.is_empty()).then(|| {
        painter.layout_no_wrap(
            lay.legend.title.clone(),
            legend_font.clone(),
            c32(lay.legend.font.color),
        )
    });
    let fig_b = bx(fig);
    let max_legend_h = match lay.legend.position {
        LegendPos::OutsideTop | LegendPos::OutsideBottom => fig.height() * 0.35,
        _ => fig.height() * 0.92,
    };
    let grid = {
        let max_w = galleys.iter().map(|g| g.size().x).fold(0.0f32, f32::max);
        lm.grid(
            rows.len(),
            max_w,
            title_galley.as_ref().map_or(0.0, |g| g.size().x),
            max_legend_h,
        )
    };
    let (lw, lh) = if show_legend { (grid.w, grid.h) } else { (0.0, 0.0) };
    let dock = if show_legend {
        geom::dock_side(lay.legend.position, fig_b, lw, lh)
    } else {
        Dock::None
    };
    let plot_rect = rect(geom::plot_box(fig_b, dock, lw, lh));

    // ---- the plot -------------------------------------------------------------
    let mut tick_font = font_id(fonts, &lay.tick_font);
    tick_font.size *= thumb;
    let mut label_font = font_id(fonts, &lay.label_font);
    label_font.size *= thumb;
    let x_label = if lay.x.label.is_empty() {
        data.x_label.clone()
    } else {
        lay.x.label.clone()
    };
    let y_label = if lay.y.label.is_empty() {
        data.y_label.clone()
    } else {
        lay.y.label.clone()
    };
    let cats = data.x_categories.clone().map(Arc::new);
    let x_step = (!lay.x.auto_ticks && lay.x.step > 0.0).then_some(lay.x.step);
    let y_step = (!lay.y.auto_ticks && lay.y.step > 0.0).then_some(lay.y.step);
    let x_dec = (!lay.x.auto_decimals).then_some(lay.x.decimals);
    let y_dec = (!lay.y.auto_decimals).then_some(lay.y.decimals);
    let tick_color = c32(lay.tick_font.color);
    let mut x_axis = AxisHints::new_x()
        .label(
            RichText::new(x_label)
                .font(label_font.clone())
                .color(c32(lay.label_font.color)),
        )
        .tick_label_font(tick_font.clone())
        .tick_label_color(tick_color)
        .formatter(tick_formatter(x_step, x_dec, cats.clone()));
    let mut y_axis = AxisHints::new_y()
        .label(RichText::new(y_label).font(label_font).color(c32(lay.label_font.color)))
        .tick_label_font(tick_font)
        .tick_label_color(tick_color)
        .formatter(tick_formatter(y_step, y_dec, None));
    if x_step.is_some() || cats.is_some() {
        x_axis = x_axis.label_spacing(egui::Rangef::new(1.0, 2.0));
    }
    if y_step.is_some() {
        y_axis = y_axis.label_spacing(egui::Rangef::new(1.0, 2.0));
    }

    let target_view = if interactive || st.reset_view {
        resolve_view(cfg, &data, None)
    } else {
        // thumbnails mirror the view of the interactive plot
        st.view.unwrap_or_else(|| resolve_view(cfg, &data, None))
    };
    let reset = !interactive || st.reset_view || st.view.is_none();
    let opts = cfg.options.clone();
    let x_fmt_dec = x_dec;
    let y_fmt_dec = y_dec;
    let cats_hover = cats.clone();
    let mut plot = Plot::new(("ct_plot", cfg.id, interactive))
        .width(plot_rect.width())
        .height(plot_rect.height())
        .custom_x_axes(vec![x_axis])
        .custom_y_axes(vec![y_axis])
        .allow_double_click_reset(false)
        .allow_zoom(interactive)
        .allow_drag(interactive)
        .allow_scroll(interactive)
        .allow_boxed_zoom(interactive)
        .allow_axis_zoom_drag(interactive)
        .show_x(interactive && opts.hover_readout)
        .show_y(interactive && opts.hover_readout)
        .show_crosshair(interactive && opts.crosshair)
        .show_grid(Vec2b::new(lay.x.grid, lay.y.grid))
        .grid_color(c32(lay.grid_color))
        .label_formatter(move |pos: &HoverPosition<'_>| -> Option<String> {
            let fx = |v: f64| match (&cats_hover, x_fmt_dec) {
                (Some(c), _) => c.get(v.round().max(0.0) as usize).cloned().unwrap_or_default(),
                (None, Some(d)) => format!("{v:.d$}"),
                (None, None) => format_sig(v, 6),
            };
            let fy = |v: f64| match y_fmt_dec {
                Some(d) => format!("{v:.d$}"),
                None => format_sig(v, 6),
            };
            Some(match pos {
                HoverPosition::NearDataPoint {
                    plot_name, position, ..
                } => {
                    format!("{plot_name}\nx = {}\ny = {}", fx(position.x), fy(position.y))
                }
                HoverPosition::Elsewhere { position } => format!("x = {}\ny = {}", fx(position.x), fy(position.y)),
            })
        });
    if let Some(s) = x_step {
        plot = plot.x_grid_spacer(step_spacer(s, false));
    } else if cats.is_some() {
        plot = plot.x_grid_spacer(step_spacer(1.0, true));
    }
    if let Some(s) = y_step {
        plot = plot.y_grid_spacer(step_spacer(s, false));
    }

    let px_width = plot_rect.width().max(64.0) as usize;
    let traces_data = Arc::clone(&data);
    let styles_c = styles.clone();
    let decimate = opts.decimate;
    let resp = ui
        .scope_builder(UiBuilder::new().max_rect(plot_rect), |ui| {
            ui.visuals_mut().extreme_bg_color = bg;
            plot.show(ui, |pu| {
                if reset {
                    pu.set_plot_bounds(PlotBounds::from_min_max(
                        [target_view[0], target_view[2]],
                        [target_view[1], target_view[3]],
                    ));
                }
                let b = if reset {
                    PlotBounds::from_min_max([target_view[0], target_view[2]], [target_view[1], target_view[3]])
                } else {
                    pu.plot_bounds()
                };
                let (x0, x1) = (b.min()[0], b.max()[0]);
                let margin = (x1 - x0) * 0.25;
                let mut buf: Vec<[f64; 2]> = Vec::new();
                for (ti, (t, s)) in traces_data.traces.iter().zip(&styles_c).enumerate() {
                    if !s.visible {
                        continue;
                    }
                    let color = c32(s.color);
                    for (si, seg) in t.segments.iter().enumerate() {
                        if t.sorted && decimate {
                            geom::decimate(seg, x0 - margin, x1 + margin, px_width * 2, &mut buf);
                        } else {
                            buf.clear();
                            buf.extend_from_slice(seg);
                        }
                        if buf.is_empty() {
                            continue;
                        }
                        if s.line != LineKind::None {
                            pu.line(
                                Line::new(s.label.clone(), PlotPoints::new(buf.clone()))
                                    .id(Id::new(("ct_line", ti, si)))
                                    .color(color)
                                    .width(s.width)
                                    .style(line_style(s.line, s.width)),
                            );
                        }
                        if s.marker != MarkerKind::None && buf.len() <= 50_000 {
                            pu.points(
                                Points::new(s.label.clone(), PlotPoints::new(buf.clone()))
                                    .id(Id::new(("ct_pts", ti, si)))
                                    .color(color)
                                    .shape(marker_shape(s.marker))
                                    .radius(s.marker_size)
                                    .filled(true),
                            );
                        }
                    }
                }
            })
        })
        .inner;
    let tf = resp.transform;
    if interactive {
        st.reset_view = false;
        let b = tf.bounds();
        st.view = Some([b.min()[0], b.max()[0], b.min()[1], b.max()[1]]);
    }
    let frame = *tf.frame();

    // ---- interactions with the data area ---------------------------------------
    if interactive {
        if resp.response.double_clicked() {
            st.reset_view = true;
            ui.ctx().request_repaint();
        }
        if resp.response.clicked()
            && let Some(pos) = resp.response.interact_pointer_pos()
        {
            toggle_marker(cfg, &data, &styles, &tf, pos);
        }
        resp.response.context_menu(|ui| {
            if ui.button("⟲ Reset view").clicked() {
                st.reset_view = true;
                ui.close();
            }
            if ui.add_enabled(!cfg.markers.is_empty(), egui::Button::new("✖ Clear markers")).clicked() {
                cfg.markers.clear();
                ui.close();
            }
            ui.separator();
            ui.label(RichText::new("Click on a curve: add / remove a marker\nDrag: pan · Wheel: zoom · Right-drag: box zoom\nDouble-click: reset view").weak());
        });
    }

    // ---- data markers -------------------------------------------------------
    let mpainter = ui.painter_at(frame);
    let marker_font = FontId::proportional(12.0 * thumb);
    for m in &cfg.markers {
        let p = tf.position_from_point(&PlotPoint::new(m.x, m.y));
        if !frame.contains(p) {
            continue;
        }
        let color = data
            .traces
            .iter()
            .position(|t| t.key == m.trace)
            .map(|i| c32(styles[i].color))
            .unwrap_or(Color32::BLACK);
        mpainter.circle_filled(p, 5.0 * thumb, Color32::WHITE);
        mpainter.circle_filled(p, 3.6 * thumb, color);
        let text = format!("x={}  y={}", format_sig(m.x, 6), format_sig(m.y, 5));
        let galley = mpainter.layout_no_wrap(text, marker_font.clone(), Color32::from_gray(20));
        let size = galley.size() + egui::vec2(8.0, 4.0);
        let mut r = Rect::from_min_size(p + egui::vec2(7.0, -size.y - 6.0), size);
        if r.right() > frame.right() {
            r = r.translate(egui::vec2(-size.x - 14.0, 0.0));
        }
        if r.top() < frame.top() {
            r = r.translate(egui::vec2(0.0, size.y + 12.0));
        }
        mpainter.rect_filled(r, 3.0, Color32::from_white_alpha(230));
        mpainter.rect_stroke(r, 3.0, Stroke::new(1.0, color), StrokeKind::Inside);
        mpainter.galley(r.min + egui::vec2(4.0, 2.0), galley, Color32::BLACK);
    }

    // ---- legend ---------------------------------------------------------------
    if !show_legend {
        return;
    }
    let lb = geom::legend_box(lay.legend.position, fig_b, bx(frame), lw, lh);
    let lrect = rect(lb);
    let lpainter = if interactive {
        ui.painter().clone()
    } else {
        ui.painter_at(area)
    };
    if lay.legend.background {
        lpainter.rect_filled(lrect, 3.0, Color32::from_white_alpha(235));
    }
    if lay.legend.border || st.dragging_legend {
        let stroke = if st.dragging_legend {
            Stroke::new(1.5, Color32::from_rgb(40, 110, 220))
        } else {
            Stroke::new(1.0, Color32::from_gray(130))
        };
        lpainter.rect_stroke(lrect, 3.0, stroke, StrokeKind::Inside);
    }
    if let Some(g) = title_galley {
        lpainter.galley(
            Pos2::new(lrect.left() + lm.pad, lrect.top() + lm.pad),
            g,
            Color32::BLACK,
        );
    }
    for (k, (row, g)) in rows.iter().zip(galleys).enumerate() {
        let (ex, ey) = lm.entry_pos(&grid, k);
        let cy = lrect.top() + ey + lm.row_h * 0.5;
        let x0 = lrect.left() + ex;
        let color = if row.visible {
            row.color
        } else {
            row.color.gamma_multiply(0.25)
        };
        if row.line != LineKind::None {
            let w = row.width.min(lm.row_h * 0.4);
            let a = Pos2::new(x0, cy);
            let b = Pos2::new(x0 + lm.swatch, cy);
            match row.line {
                LineKind::Dashed => {
                    lpainter.extend(egui::Shape::dashed_line(&[a, b], Stroke::new(w, color), 6.0, 3.0));
                }
                LineKind::Dotted => {
                    lpainter.extend(egui::Shape::dotted_line(&[a, b], color, 5.0, w * 0.7));
                }
                _ => {
                    lpainter.line_segment([a, b], Stroke::new(w, color));
                }
            }
        }
        if row.marker != MarkerKind::None {
            lpainter.circle_filled(Pos2::new(x0 + lm.swatch * 0.5, cy), 3.5 * thumb, color);
        }
        lpainter.galley(
            Pos2::new(x0 + lm.swatch + lm.gap, cy - g.size().y * 0.5),
            g,
            Color32::BLACK,
        );
    }
    if !interactive {
        return;
    }
    let lresp = ui
        .interact(lrect, ui.id().with(("ct_legend", cfg.id)), Sense::click_and_drag())
        .on_hover_cursor(CursorIcon::Grab)
        .on_hover_text("Drag to move the legend (drop it on an edge to place it outside the plot).\nClick an entry to show / hide the curve.");
    if lresp.drag_started() {
        let (fx, fy) = geom::to_free(fig_b, lb);
        cfg.layout.legend.position = LegendPos::Free { x: fx, y: fy };
        st.dragging_legend = true;
    }
    if lresp.dragged() {
        let d = lresp.drag_delta();
        if let LegendPos::Free { x, y } = cfg.layout.legend.position {
            let nx = x + d.x / (fig.width() - lw).max(1.0);
            let ny = y + d.y / (fig.height() - lh).max(1.0);
            cfg.layout.legend.position = LegendPos::Free {
                x: nx.clamp(0.0, 1.0),
                y: ny.clamp(0.0, 1.0),
            };
        }
        ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
    }
    if lresp.drag_stopped() {
        st.dragging_legend = false;
    }
    if lresp.clicked()
        && let Some(p) = lresp.interact_pointer_pos()
        && let Some(i) = lm.entry_at(&grid, p.x - lrect.left(), p.y - lrect.top(), rows.len())
    {
        let key = rows[i].key.clone();
        let o = cfg.overrides.entry(key.clone()).or_default();
        o.visible = !o.visible;
        if o.is_default() {
            cfg.overrides.remove(&key);
        }
    }
}

/// Adds a marker on the curve point closest to `pos`, or removes the marker
/// under the pointer.
fn toggle_marker(
    cfg: &mut PlotConfig,
    data: &PlotData,
    styles: &[super::style::TraceStyle],
    tf: &egui_plot::PlotTransform,
    pos: Pos2,
) {
    // remove an existing marker?
    if let Some(i) = cfg.markers.iter().position(|m| {
        let p = tf.position_from_point(&PlotPoint::new(m.x, m.y));
        p.distance(pos) < 9.0
    }) {
        cfg.markers.remove(i);
        return;
    }
    let dx = tf.dvalue_dpos()[0].abs() * 14.0;
    let v = tf.value_from_position(pos);
    let mut best: Option<(f32, usize, [f64; 2])> = None;
    for (ti, t) in data.traces.iter().enumerate() {
        if !styles[ti].visible {
            continue;
        }
        for seg in &t.segments {
            let candidates: &[[f64; 2]] = if t.sorted {
                let a = seg.partition_point(|p| p[0] < v.x - dx);
                let b = seg.partition_point(|p| p[0] <= v.x + dx);
                &seg[a..b]
            } else {
                seg
            };
            for p in candidates {
                let sp = tf.position_from_point(&PlotPoint::new(p[0], p[1]));
                let d = sp.distance(pos);
                if d < 15.0 && best.is_none_or(|(bd, _, _)| d < bd) {
                    best = Some((d, ti, *p));
                }
            }
        }
    }
    if let Some((_, ti, p)) = best {
        cfg.markers.push(super::config::DataMarker {
            trace: data.traces[ti].key.clone(),
            x: p[0],
            y: p[1],
        });
    }
}
