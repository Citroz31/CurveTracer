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
    /// Left-drag draws a zoom rectangle (instead of panning).
    pub box_zoom: bool,
    /// View to apply at the next frame (zoom buttons, previous view…).
    pub set_view: Option<[f64; 4]>,
    /// Previous views (for "previous view").
    pub history: Vec<[f64; 4]>,
    stable_view: Option<[f64; 4]>,
    frame_view: Option<[f64; 4]>,
    view_changed_at: f64,
    /// Axis / title editor opened by a click.
    pub edit: Option<EditTarget>,
    edit_pos: Pos2,
}

/// What the axis editor shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditTarget {
    X,
    Y,
    Title,
    All,
}

fn same_view(a: &[f64; 4], b: &[f64; 4]) -> bool {
    let tol = |lo: f64, hi: f64| ((hi - lo).abs() * 1e-9).max(1e-300);
    (a[0] - b[0]).abs() <= tol(a[0], a[1])
        && (a[1] - b[1]).abs() <= tol(a[0], a[1])
        && (a[2] - b[2]).abs() <= tol(a[2], a[3])
        && (a[3] - b[3]).abs() <= tol(a[2], a[3])
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
        if key == self.data_key || self.pending.is_some() {
            // one computation at a time: when settings change quickly (clicking
            // filter values, dragging a range), the next one starts with the
            // latest settings once the running one is done
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

    /// Zooms in (factor < 1) or out (factor > 1) around the center of the view.
    pub fn zoom(&mut self, factor: f64) {
        if let Some(v) = self.view {
            let (cx, cy) = ((v[0] + v[1]) * 0.5, (v[2] + v[3]) * 0.5);
            let (hx, hy) = ((v[1] - v[0]) * 0.5 * factor, (v[3] - v[2]) * 0.5 * factor);
            self.set_view = Some([cx - hx, cx + hx, cy - hy, cy + hy]);
        }
    }

    /// Fits the Y range to the curves visible in the current X range.
    pub fn fit_y(&mut self, cfg: &PlotConfig) {
        let (Some(v), Some(data)) = (self.view, &self.data) else {
            return;
        };
        let styles = trace_styles(cfg, data);
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        for (t, s) in data.traces.iter().zip(&styles) {
            if !s.visible {
                continue;
            }
            for seg in &t.segments {
                for p in seg {
                    if p[0] >= v[0] && p[0] <= v[1] && p[1].is_finite() {
                        lo = lo.min(p[1]);
                        hi = hi.max(p[1]);
                    }
                }
            }
        }
        if lo.is_finite() && hi.is_finite() {
            let pad = if hi > lo {
                (hi - lo) * 0.05
            } else {
                lo.abs().max(1.0) * 0.1
            };
            self.set_view = Some([v[0], v[1], lo - pad, hi + pad]);
        }
    }

    /// Goes back to the previous view.
    pub fn previous_view(&mut self) {
        if let Some(v) = self.history.pop() {
            self.stable_view = Some(v);
            self.set_view = Some(v);
        }
    }

    /// Records the views the user stays on (for "previous view").
    fn track_view(&mut self, v: [f64; 4], now: f64, pointer_down: bool) {
        if self.frame_view.is_none_or(|f| !same_view(&f, &v)) {
            self.frame_view = Some(v);
            self.view_changed_at = now;
            return;
        }
        if pointer_down || now - self.view_changed_at < 0.35 {
            return;
        }
        match self.stable_view {
            Some(s) if !same_view(&s, &v) => {
                self.history.push(s);
                if self.history.len() > 50 {
                    self.history.remove(0);
                }
                self.stable_view = Some(v);
            }
            None => self.stable_view = Some(v),
            _ => {}
        }
    }

    /// Opens the axis / title editor.
    pub fn open_editor(&mut self, target: EditTarget, pos: Pos2) {
        self.edit = Some(target);
        self.edit_pos = pos;
    }

    /// Data up to date with `cfg` (for the exports): waits for the running
    /// computation and computes again if the settings changed meanwhile.
    pub fn wait_current(&mut self, cfg: &PlotConfig, ds: Option<&Arc<Dataset>>) {
        self.wait();
        if let Some(ds) = ds {
            let key = cfg.data_key() ^ ds.generation.wrapping_mul(0x9E37_79B9_7F4A_7C15);
            if key != self.data_key {
                self.data = Some(Arc::new(compute(ds, cfg)));
                self.data_key = key;
                self.reset_view = true;
            }
        }
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
    dash: Vec<f32>,
    marker: MarkerKind,
    width: f32,
    visible: bool,
    key: String,
}

/// Dashed polyline (pattern in multiples of the width) on the screen.
fn dashed_shapes(pts: &[Pos2], stroke: Stroke, pattern: &[f32]) -> Vec<egui::Shape> {
    let w = stroke.width.max(0.5);
    let dashes: Vec<f32> = pattern.iter().step_by(2).map(|d| (d * w).max(0.5)).collect();
    let gaps: Vec<f32> = pattern.iter().skip(1).step_by(2).map(|d| (d * w).max(0.5)).collect();
    if dashes.is_empty() || gaps.is_empty() {
        return vec![egui::Shape::line(pts.to_vec(), stroke)];
    }
    egui::Shape::dashed_line_with_offset(pts, stroke, &dashes, &gaps, 0.0)
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

/// Points, color, width and dash pattern of a curve drawn with a custom pattern.
type CustomPath = (Vec<[f64; 2]>, Color32, f32, Vec<f32>);

/// Point budget of a thumbnail (plot list, Export page).
const THUMB_POINTS: usize = 40_000;

/// Minimum grid spacing given to egui_plot (its `base_step_size` = this many pixels).
const GRID_MIN_PX: f32 = 8.0;

/// Automatic ticks with 1 / 2 / 5 steps: a labelled tick about every
/// `target_px` pixels and fainter grid lines in between (egui_plot's default
/// spacer only uses powers of 10, which can leave a narrow range unlabelled).
fn auto_spacer(target_px: f32) -> impl Fn(GridInput) -> Vec<GridMark> {
    move |input: GridInput| -> Vec<GridMark> {
        let (lo, hi) = input.bounds;
        let per_px = input.base_step_size / GRID_MIN_PX as f64;
        let raw = per_px * target_px as f64;
        let mag = 10f64.powf(raw.max(1e-300).log10().floor());
        let major = [1.0, 2.0, 5.0, 10.0]
            .iter()
            .map(|m| m * mag)
            .find(|s| *s >= raw * 0.999)
            .unwrap_or(10.0 * mag);
        let minor = major / 5.0;
        geom::ticks(lo, hi, minor)
            .into_iter()
            .map(|v| GridMark {
                value: v,
                step_size: if geom::is_multiple(v, major) { major } else { minor },
            })
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
    let mut title_rect = Rect::NOTHING;
    if lay.show_title && !lay.title.trim().is_empty() {
        let mut f = font_id(fonts, &lay.title_font);
        f.size *= thumb;
        let mut galley = painter.layout_no_wrap(lay.title.clone(), f.clone(), c32(lay.title_font.color));
        if !interactive && galley.size().x > fig.width() {
            // thumbnails: shrink a long title to the width
            f.size *= (fig.width() / galley.size().x).max(0.4);
            galley = painter.layout_no_wrap(lay.title.clone(), f, c32(lay.title_font.color));
        }
        let pos = Pos2::new(fig.center().x - galley.size().x * 0.5, fig.top());
        let h = galley.size().y;
        title_rect = Rect::from_min_size(pos, galley.size()).expand(3.0);
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
            dash: s.dash.clone(),
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

    let explicit = if interactive { st.set_view.take() } else { None };
    let target_view = if let Some(v) = explicit {
        v
    } else if interactive || st.reset_view {
        resolve_view(cfg, &data, None)
    } else {
        // thumbnails mirror the view of the interactive plot
        st.view.unwrap_or_else(|| resolve_view(cfg, &data, None))
    };
    let reset = !interactive || st.reset_view || st.view.is_none() || explicit.is_some();
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
        // the mouse wheel goes from one plot to the next: Ctrl + wheel zooms
        .allow_zoom(interactive)
        .allow_drag(interactive && !st.box_zoom)
        .allow_scroll(false)
        .allow_boxed_zoom(interactive)
        .boxed_zoom_pointer_button(if st.box_zoom {
            egui::PointerButton::Primary
        } else {
            egui::PointerButton::Secondary
        })
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
    plot = plot.grid_spacing(egui::Rangef::new(GRID_MIN_PX, 300.0));
    if let Some(s) = x_step {
        plot = plot.x_grid_spacer(step_spacer(s, false));
    } else if cats.is_some() {
        plot = plot.x_grid_spacer(step_spacer(1.0, true));
    } else {
        plot = plot.x_grid_spacer(auto_spacer(95.0 * thumb));
    }
    if let Some(s) = y_step {
        plot = plot.y_grid_spacer(step_spacer(s, false));
    } else {
        plot = plot.y_grid_spacer(auto_spacer(48.0 * thumb));
    }

    let px_width = plot_rect.width().max(64.0) as usize;
    let traces_data = Arc::clone(&data);
    let styles_c = styles.clone();
    let decimate = opts.decimate;
    // thumbnails (200 px wide) do not need every point: keep one point out of
    // `stride` when the plot is big (thousands of curves, millions of points)
    let stride = if interactive {
        1
    } else {
        let total: usize = data
            .traces
            .iter()
            .zip(&styles)
            .filter(|(_, s)| s.visible)
            .map(|(t, _)| t.n_points)
            .sum();
        total.div_ceil(THUMB_POINTS).max(1)
    };
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
                // custom dash patterns are painted after the plot
                let mut custom: Vec<CustomPath> = Vec::new();
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
                        if stride > 1 && buf.len() > 2 {
                            let last = buf[buf.len() - 1];
                            let mut k = 0;
                            buf.retain(|_| {
                                k += 1;
                                (k - 1) % stride == 0
                            });
                            if buf.last() != Some(&last) {
                                buf.push(last);
                            }
                        }
                        if buf.is_empty() {
                            continue;
                        }
                        if s.line == LineKind::Custom {
                            // invisible line: hover read-out only
                            pu.line(
                                Line::new(s.label.clone(), PlotPoints::new(buf.clone()))
                                    .id(Id::new(("ct_line", ti, si)))
                                    // TRANSPARENT would mean "automatic color" for egui_plot
                                    .color(Color32::from_black_alpha(1))
                                    .width(0.0),
                            );
                            custom.push((buf.clone(), color, s.width, s.dash.clone()));
                        } else if s.line != LineKind::None {
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
                custom
            })
        })
        .inner;
    let tf = resp.transform;
    if !resp.inner.is_empty() {
        let cp = ui.painter_at(*tf.frame());
        for (pts, color, w, dash) in &resp.inner {
            let screen: Vec<Pos2> = pts
                .iter()
                .map(|p| {
                    let q = tf.position_from_point(&PlotPoint::new(p[0], p[1]));
                    Pos2::new(q.x.clamp(-1e5, 1e5), q.y.clamp(-1e5, 1e5))
                })
                .collect();
            cp.extend(dashed_shapes(&screen, Stroke::new(*w, *color), dash));
        }
    }
    if interactive {
        st.reset_view = false;
        let b = tf.bounds();
        let v = [b.min()[0], b.max()[0], b.min()[1], b.max()[1]];
        st.view = Some(v);
        let (now, down) = ui.input(|i| (i.time, i.pointer.any_down()));
        st.track_view(v, now, down);
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
            if ui.add_enabled(!st.history.is_empty(), egui::Button::new("⏴ Previous view")).clicked() {
                st.previous_view();
                ui.close();
            }
            if ui.button("📐 Axes & title…").clicked() {
                st.open_editor(EditTarget::All, ui.min_rect().left_top());
                ui.close();
            }
            ui.separator();
            ui.label(RichText::new("Click on a curve: add / remove a marker\nDrag: pan · Ctrl + wheel: zoom · Right-drag: box zoom\nWheel: previous / next plot · Double-click: reset view\nClick on an axis or on the title: edit it").weak());
        });
    }

    // ---- click on the axes / title: editor --------------------------------------
    if interactive {
        let zones = [
            (
                EditTarget::X,
                Rect::from_min_max(
                    Pos2::new(frame.left(), frame.bottom() + 1.0),
                    Pos2::new(frame.right(), plot_rect.bottom()),
                ),
                "X axis",
            ),
            (
                EditTarget::Y,
                Rect::from_min_max(
                    Pos2::new(plot_rect.left(), frame.top()),
                    Pos2::new(frame.left() - 1.0, frame.bottom()),
                ),
                "Y axis",
            ),
            (EditTarget::Title, title_rect, "Title"),
        ];
        for (target, zone, name) in zones {
            if zone.width() < 2.0 || zone.height() < 2.0 {
                continue;
            }
            let r = ui
                .interact(zone, ui.id().with(("ct_axis_edit", cfg.id, name)), Sense::click())
                .on_hover_cursor(CursorIcon::PointingHand)
                .on_hover_text(if target == EditTarget::Title {
                    "Click to edit the title".to_owned()
                } else {
                    format!("Click to edit the {name} (title, range, ticks) · drag to zoom this axis")
                });
            if r.hovered() {
                ui.painter().rect_stroke(
                    zone,
                    3.0,
                    Stroke::new(1.0, Color32::from_rgba_unmultiplied(40, 110, 220, 120)),
                    StrokeKind::Inside,
                );
            }
            if r.clicked() {
                st.open_editor(target, r.interact_pointer_pos().unwrap_or(zone.center()));
            }
        }
        if let Some(target) = st.edit {
            let auto = (data.x_label.clone(), data.y_label.clone());
            let view = st.view;
            let mut open = true;
            let title = match target {
                EditTarget::X => "X axis",
                EditTarget::Y => "Y axis",
                EditTarget::Title => "Title",
                EditTarget::All => "Axes & title",
            };
            let pos = st.edit_pos;
            egui::Window::new(title)
                .id(Id::new(("ct_axis_editor", cfg.id)))
                .default_pos(pos + egui::vec2(10.0, 10.0))
                .collapsible(false)
                .resizable(false)
                .open(&mut open)
                .show(ui.ctx(), |ui| {
                    super::axis_editor::show(ui, cfg, target, &auto, view);
                });
            if !open {
                st.edit = None;
            }
        }
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
                LineKind::Dashed | LineKind::Custom => {
                    lpainter.extend(dashed_shapes(&[a, b], Stroke::new(w, color), &row.dash));
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
