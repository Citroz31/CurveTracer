//! Static, high-resolution rendering of a plot to PNG (used by the image and
//! PowerPoint exports). Uses the same layout rules as the interactive view.

use ab_glyph::{Font, FontArc, OutlineCurve, PxScale, ScaleFont};
use tiny_skia::{FillRule, LineCap, LineJoin, Mask, Paint, PathBuilder, Pixmap, Rect, Stroke, StrokeDash, Transform};

use super::compute::PlotData;
use super::config::{FontSpec, LineKind, MarkerKind, PlotConfig, Rgba};
use super::geom::{self, Bx, Dock};
use super::style::{LegendMetrics, trace_styles};
use crate::data::format_sig;
use crate::fonts::FontBook;

#[derive(Clone, Debug)]
pub struct RenderOptions {
    pub width: u32,
    pub height: u32,
    /// Scale applied to fonts, line widths and margins (1.0 ≈ screen size).
    pub scale: f32,
    /// Visible data range (xmin, xmax, ymin, ymax); `None` = configured / automatic.
    pub view: Option<[f64; 4]>,
}

impl RenderOptions {
    pub fn new(width: u32, height: u32) -> RenderOptions {
        RenderOptions {
            width,
            height,
            scale: (width.min(height * 16 / 10) as f32 / 1000.0).max(0.5),
            view: None,
        }
    }
}

fn paint(c: Rgba) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color_rgba8(c[0], c[1], c[2], c[3]);
    p.anti_alias = true;
    p
}

struct TextStyle<'a> {
    font: &'a FontArc,
    /// Used for the characters missing in `font`.
    fallback: &'a FontArc,
    px: f32,
    color: Rgba,
    /// CSS font family (SVG output).
    css_family: String,
    bold: bool,
}

/// CSS font-family list for a font of the font book.
fn css_family(family: &str, fonts: &FontBook) -> String {
    match family {
        crate::fonts::PROPORTIONAL => "Ubuntu, 'Segoe UI', Arial, Helvetica, sans-serif".to_owned(),
        crate::fonts::MONOSPACE => "Hack, Consolas, 'Courier New', monospace".to_owned(),
        name if fonts.contains(name) => {
            let base = name.trim_end_matches(" Bold");
            format!("'{base}', Arial, sans-serif")
        }
        _ => "Arial, sans-serif".to_owned(),
    }
}

impl<'a> TextStyle<'a> {
    fn new(fonts: &'a FontBook, spec: &FontSpec, scale: f32) -> TextStyle<'a> {
        TextStyle {
            font: &fonts.get(&spec.family).font,
            fallback: &fonts.get(crate::fonts::MONOSPACE).font,
            px: spec.size * scale,
            color: spec.color,
            css_family: css_family(&spec.family, fonts),
            bold: spec.family.ends_with(" Bold") || spec.family.ends_with("-Bold"),
        }
    }

    fn ascent(&self) -> f32 {
        self.scaled().ascent()
    }

    fn scaled(&self) -> ab_glyph::PxScaleFont<&FontArc> {
        self.font.as_scaled(PxScale::from(self.px))
    }

    /// Font to use for a character.
    fn font_for(&self, c: char) -> &'a FontArc {
        if self.font.glyph_id(c).0 == 0 && self.fallback.glyph_id(c).0 != 0 {
            self.fallback
        } else {
            self.font
        }
    }

    fn width(&self, s: &str) -> f32 {
        let mut w = 0.0;
        let mut prev: Option<(ab_glyph::GlyphId, *const FontArc)> = None;
        for c in s.chars() {
            let font = self.font_for(c);
            let f = font.as_scaled(PxScale::from(self.px));
            let g = f.glyph_id(c);
            if let Some((p, pf)) = prev
                && std::ptr::eq(pf, font)
            {
                w += f.kern(p, g);
            }
            w += f.h_advance(g);
            prev = Some((g, font as *const FontArc));
        }
        w
    }

    fn height(&self) -> f32 {
        let f = self.scaled();
        f.ascent() - f.descent()
    }

    /// Draws `s` with its top-left corner at (x, y), optionally rotated by
    /// `angle` degrees around that corner.
    fn draw_pixmap(&self, pm: &mut Pixmap, s: &str, x: f32, y: f32, angle: f32) {
        let baseline = y + self.scaled().ascent();
        let mut pb = PathBuilder::new();
        let mut caret = x;
        let mut prev: Option<(ab_glyph::GlyphId, *const FontArc)> = None;
        for c in s.chars() {
            let font = self.font_for(c);
            let f = font.as_scaled(PxScale::from(self.px));
            let sx = f.h_scale_factor();
            let sy = f.v_scale_factor();
            let g = f.glyph_id(c);
            if let Some((p, pf)) = prev
                && std::ptr::eq(pf, font)
            {
                caret += f.kern(p, g);
            }
            if let Some(outline) = font.outline(g) {
                let mut last: Option<ab_glyph::Point> = None;
                let tp = |p: ab_glyph::Point| (caret + p.x * sx, baseline - p.y * sy);
                for curve in &outline.curves {
                    let (start, end) = match curve {
                        OutlineCurve::Line(a, b) => (*a, *b),
                        OutlineCurve::Quad(a, _, b) => (*a, *b),
                        OutlineCurve::Cubic(a, _, _, b) => (*a, *b),
                    };
                    let continuous = last.is_some_and(|l| (l.x - start.x).abs() < 1e-3 && (l.y - start.y).abs() < 1e-3);
                    if !continuous {
                        if last.is_some() {
                            pb.close();
                        }
                        let (px, py) = tp(start);
                        pb.move_to(px, py);
                    }
                    match curve {
                        OutlineCurve::Line(_, b) => {
                            let (px, py) = tp(*b);
                            pb.line_to(px, py);
                        }
                        OutlineCurve::Quad(_, c1, b) => {
                            let (cx, cy) = tp(*c1);
                            let (px, py) = tp(*b);
                            pb.quad_to(cx, cy, px, py);
                        }
                        OutlineCurve::Cubic(_, c1, c2, b) => {
                            let (c1x, c1y) = tp(*c1);
                            let (c2x, c2y) = tp(*c2);
                            let (px, py) = tp(*b);
                            pb.cubic_to(c1x, c1y, c2x, c2y, px, py);
                        }
                    }
                    last = Some(end);
                }
                if last.is_some() {
                    pb.close();
                }
            }
            caret += f.h_advance(g);
            prev = Some((g, font as *const FontArc));
        }
        if let Some(path) = pb.finish() {
            let ts = if angle != 0.0 {
                Transform::from_rotate_at(angle, x, y)
            } else {
                Transform::identity()
            };
            pm.fill_path(&path, &paint(self.color), FillRule::Winding, ts, None);
        }
    }
}

// =============================================================================
// Drawing back-ends
// =============================================================================

/// Drawing primitives used by the plot renderer (bitmap or vector output).
trait Canvas {
    /// Clip region of the curves (the plot frame).
    fn set_clip(&mut self, frame: Bx);
    fn rect(&mut self, b: Bx, fill: Option<Rgba>, stroke: Option<(Rgba, f32)>);
    fn line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, c: Rgba, w: f32);
    /// Polyline with the style of a trace (`clip`: limited to the frame).
    fn polyline(&mut self, pts: &[(f32, f32)], c: Rgba, w: f32, kind: LineKind, clip: bool);
    fn marker(&mut self, kind: MarkerKind, x: f32, y: f32, r: f32, c: Rgba, clip: bool);
    /// Text with its top-left corner at (x, y), rotated by `angle` degrees.
    fn text(&mut self, ts: &TextStyle, s: &str, x: f32, y: f32, angle: f32);
    /// Starts / ends a group (SVG only, keeps the file organized).
    fn group(&mut self, _name: Option<&str>) {}
}

fn dash_pattern(kind: LineKind, w: f32) -> Option<Vec<f32>> {
    match kind {
        LineKind::Dashed => Some(vec![w * 5.0, w * 3.0]),
        LineKind::Dotted => Some(vec![w * 0.1, w * 2.5]),
        _ => None,
    }
}

struct PixmapCanvas {
    pm: Pixmap,
    mask: Option<Mask>,
}

fn marker_path(kind: MarkerKind, x: f32, y: f32, r: f32) -> Option<tiny_skia::Path> {
    let mut pb = PathBuilder::new();
    match kind {
        MarkerKind::None => return None,
        MarkerKind::Circle => pb.push_circle(x, y, r),
        MarkerKind::Square => pb.push_rect(Rect::from_ltrb(x - r, y - r, x + r, y + r)?),
        MarkerKind::Diamond => {
            pb.move_to(x, y - r * 1.3);
            pb.line_to(x + r * 1.3, y);
            pb.line_to(x, y + r * 1.3);
            pb.line_to(x - r * 1.3, y);
            pb.close();
        }
        MarkerKind::Up => {
            pb.move_to(x, y - r * 1.3);
            pb.line_to(x + r * 1.2, y + r);
            pb.line_to(x - r * 1.2, y + r);
            pb.close();
        }
        MarkerKind::Down => {
            pb.move_to(x, y + r * 1.3);
            pb.line_to(x + r * 1.2, y - r);
            pb.line_to(x - r * 1.2, y - r);
            pb.close();
        }
        MarkerKind::Cross => {
            pb.move_to(x - r, y - r);
            pb.line_to(x + r, y + r);
            pb.move_to(x + r, y - r);
            pb.line_to(x - r, y + r);
        }
        MarkerKind::Plus => {
            pb.move_to(x - r * 1.2, y);
            pb.line_to(x + r * 1.2, y);
            pb.move_to(x, y - r * 1.2);
            pb.line_to(x, y + r * 1.2);
        }
    }
    pb.finish()
}

impl Canvas for PixmapCanvas {
    fn set_clip(&mut self, frame: Bx) {
        if let Some(mut mask) = Mask::new(self.pm.width(), self.pm.height()) {
            if let Some(r) = Rect::from_ltrb(frame.x0, frame.y0, frame.x1, frame.y1) {
                mask.fill_path(
                    &PathBuilder::from_rect(r),
                    FillRule::Winding,
                    false,
                    Transform::identity(),
                );
            }
            self.mask = Some(mask);
        }
    }

    fn rect(&mut self, b: Bx, fill: Option<Rgba>, stroke: Option<(Rgba, f32)>) {
        let Some(r) = Rect::from_ltrb(b.x0, b.y0, b.x1, b.y1) else {
            return;
        };
        if let Some(c) = fill {
            self.pm.fill_rect(r, &paint(c), Transform::identity(), None);
        }
        if let Some((c, w)) = stroke {
            let path = PathBuilder::from_rect(r);
            let st = Stroke {
                width: w,
                ..Default::default()
            };
            self.pm.stroke_path(&path, &paint(c), &st, Transform::identity(), None);
        }
    }

    fn line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, c: Rgba, w: f32) {
        let mut pb = PathBuilder::new();
        pb.move_to(x0, y0);
        pb.line_to(x1, y1);
        if let Some(p) = pb.finish() {
            let st = Stroke {
                width: w,
                ..Default::default()
            };
            self.pm.stroke_path(&p, &paint(c), &st, Transform::identity(), None);
        }
    }

    fn polyline(&mut self, pts: &[(f32, f32)], c: Rgba, w: f32, kind: LineKind, clip: bool) {
        let mut pb = PathBuilder::new();
        for (k, &(x, y)) in pts.iter().enumerate() {
            if k == 0 {
                pb.move_to(x, y);
            } else {
                pb.line_to(x, y);
            }
        }
        if pts.len() == 1 {
            pb.push_circle(pts[0].0, pts[0].1, w * 0.6);
        }
        let Some(path) = pb.finish() else {
            return;
        };
        let mut st = Stroke {
            width: w,
            line_cap: LineCap::Round,
            line_join: LineJoin::Round,
            ..Default::default()
        };
        st.dash = dash_pattern(kind, w).and_then(|d| StrokeDash::new(d, 0.0));
        let mask = if clip { self.mask.as_ref() } else { None };
        self.pm.stroke_path(&path, &paint(c), &st, Transform::identity(), mask);
    }

    fn marker(&mut self, kind: MarkerKind, x: f32, y: f32, r: f32, c: Rgba, clip: bool) {
        let Some(p) = marker_path(kind, x, y, r) else {
            return;
        };
        let mask = if clip { self.mask.as_ref() } else { None };
        if matches!(kind, MarkerKind::Cross | MarkerKind::Plus) {
            let st = Stroke {
                width: (r * 0.5).max(1.0),
                ..Default::default()
            };
            self.pm.stroke_path(&p, &paint(c), &st, Transform::identity(), mask);
        } else {
            self.pm
                .fill_path(&p, &paint(c), FillRule::Winding, Transform::identity(), mask);
        }
    }

    fn text(&mut self, ts: &TextStyle, s: &str, x: f32, y: f32, angle: f32) {
        ts.draw_pixmap(&mut self.pm, s, x, y, angle);
    }
}

/// SVG output: vector curves, real (editable) text.
struct SvgCanvas {
    out: String,
    clip_defined: bool,
    in_group: bool,
}

fn svg_color(c: Rgba) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

fn svg_opacity(attr: &str, c: Rgba) -> String {
    if c[3] == 255 {
        String::new()
    } else {
        format!(" {attr}=\"{:.3}\"", c[3] as f32 / 255.0)
    }
}

fn f2(v: f32) -> String {
    let r = (v * 100.0).round() / 100.0;
    if r == r.trunc() {
        format!("{}", r as i64)
    } else {
        format!("{r}")
    }
}

impl SvgCanvas {
    fn new(w: u32, h: u32) -> SvgCanvas {
        SvgCanvas {
            out: format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\">\n"
            ),
            clip_defined: false,
            in_group: false,
        }
    }

    fn finish(mut self) -> String {
        if self.in_group {
            self.out.push_str("</g>\n");
        }
        self.out.push_str("</svg>\n");
        self.out
    }

    fn clip_attr(&self, clip: bool) -> &'static str {
        if clip && self.clip_defined {
            " clip-path=\"url(#frame)\""
        } else {
            ""
        }
    }
}

impl Canvas for SvgCanvas {
    fn set_clip(&mut self, frame: Bx) {
        self.out.push_str(&format!(
            "<defs><clipPath id=\"frame\"><rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"/></clipPath></defs>\n",
            f2(frame.x0),
            f2(frame.y0),
            f2(frame.w()),
            f2(frame.h())
        ));
        self.clip_defined = true;
    }

    fn group(&mut self, name: Option<&str>) {
        if self.in_group {
            self.out.push_str("</g>\n");
            self.in_group = false;
        }
        if let Some(n) = name {
            self.out
                .push_str(&format!("<g id=\"{}\">\n", crate::export::pptx::xml_escape(n)));
            self.in_group = true;
        }
    }

    fn rect(&mut self, b: Bx, fill: Option<Rgba>, stroke: Option<(Rgba, f32)>) {
        let fill_attr = match fill {
            Some(c) => format!("fill=\"{}\"{}", svg_color(c), svg_opacity("fill-opacity", c)),
            None => "fill=\"none\"".to_owned(),
        };
        let stroke_attr = match stroke {
            Some((c, w)) => format!(
                " stroke=\"{}\" stroke-width=\"{}\"{}",
                svg_color(c),
                f2(w),
                svg_opacity("stroke-opacity", c)
            ),
            None => String::new(),
        };
        self.out.push_str(&format!(
            "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" {fill_attr}{stroke_attr}/>\n",
            f2(b.x0),
            f2(b.y0),
            f2(b.w()),
            f2(b.h())
        ));
    }

    fn line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, c: Rgba, w: f32) {
        self.out.push_str(&format!(
            "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"{}\" stroke-width=\"{}\"{}/>\n",
            f2(x0),
            f2(y0),
            f2(x1),
            f2(y1),
            svg_color(c),
            f2(w),
            svg_opacity("stroke-opacity", c)
        ));
    }

    fn polyline(&mut self, pts: &[(f32, f32)], c: Rgba, w: f32, kind: LineKind, clip: bool) {
        if pts.is_empty() {
            return;
        }
        if pts.len() == 1 {
            self.marker(MarkerKind::Circle, pts[0].0, pts[0].1, w * 0.6, c, clip);
            return;
        }
        let mut p = String::with_capacity(pts.len() * 14);
        for &(x, y) in pts {
            p.push_str(&f2(x));
            p.push(',');
            p.push_str(&f2(y));
            p.push(' ');
        }
        let dash = dash_pattern(kind, w)
            .map(|d| format!(" stroke-dasharray=\"{} {}\"", f2(d[0]), f2(d[1])))
            .unwrap_or_default();
        self.out.push_str(&format!(
            "<polyline points=\"{}\" fill=\"none\" stroke=\"{}\" stroke-width=\"{}\" stroke-linecap=\"round\" stroke-linejoin=\"round\"{dash}{}{}/>\n",
            p.trim_end(),
            svg_color(c),
            f2(w),
            svg_opacity("stroke-opacity", c),
            self.clip_attr(clip)
        ));
    }

    fn marker(&mut self, kind: MarkerKind, x: f32, y: f32, r: f32, c: Rgba, clip: bool) {
        let fill = format!(
            "fill=\"{}\"{}{}",
            svg_color(c),
            svg_opacity("fill-opacity", c),
            self.clip_attr(clip)
        );
        let poly = |pts: &[(f32, f32)]| -> String {
            pts.iter()
                .map(|(a, b)| format!("{},{}", f2(*a), f2(*b)))
                .collect::<Vec<_>>()
                .join(" ")
        };
        let e = match kind {
            MarkerKind::None => return,
            MarkerKind::Circle => format!("<circle cx=\"{}\" cy=\"{}\" r=\"{}\" {fill}/>", f2(x), f2(y), f2(r)),
            MarkerKind::Square => format!(
                "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" {fill}/>",
                f2(x - r),
                f2(y - r),
                f2(2.0 * r),
                f2(2.0 * r)
            ),
            MarkerKind::Diamond => format!(
                "<polygon points=\"{}\" {fill}/>",
                poly(&[(x, y - r * 1.3), (x + r * 1.3, y), (x, y + r * 1.3), (x - r * 1.3, y)])
            ),
            MarkerKind::Up => format!(
                "<polygon points=\"{}\" {fill}/>",
                poly(&[(x, y - r * 1.3), (x + r * 1.2, y + r), (x - r * 1.2, y + r)])
            ),
            MarkerKind::Down => format!(
                "<polygon points=\"{}\" {fill}/>",
                poly(&[(x, y + r * 1.3), (x + r * 1.2, y - r), (x - r * 1.2, y - r)])
            ),
            MarkerKind::Cross | MarkerKind::Plus => {
                let (a, b) = if kind == MarkerKind::Cross {
                    ([(x - r, y - r), (x + r, y + r)], [(x + r, y - r), (x - r, y + r)])
                } else {
                    (
                        [(x - r * 1.2, y), (x + r * 1.2, y)],
                        [(x, y - r * 1.2), (x, y + r * 1.2)],
                    )
                };
                format!(
                    "<path d=\"M{},{} L{},{} M{},{} L{},{}\" stroke=\"{}\" stroke-width=\"{}\" fill=\"none\"{}/>",
                    f2(a[0].0),
                    f2(a[0].1),
                    f2(a[1].0),
                    f2(a[1].1),
                    f2(b[0].0),
                    f2(b[0].1),
                    f2(b[1].0),
                    f2(b[1].1),
                    svg_color(c),
                    f2((r * 0.5).max(1.0)),
                    self.clip_attr(clip)
                )
            }
        };
        self.out.push_str(&e);
        self.out.push('\n');
    }

    fn text(&mut self, ts: &TextStyle, s: &str, x: f32, y: f32, angle: f32) {
        if s.is_empty() {
            return;
        }
        let base_y = y + ts.ascent();
        let rot = if angle != 0.0 {
            format!(" transform=\"rotate({} {} {})\"", f2(angle), f2(x), f2(y))
        } else {
            String::new()
        };
        // fixes the width computed with the export font: the layout holds even
        // when the viewer substitutes another font
        let fit = if s.chars().count() > 1 {
            format!(" textLength=\"{}\" lengthAdjust=\"spacingAndGlyphs\"", f2(ts.width(s)))
        } else {
            String::new()
        };
        self.out.push_str(&format!(
            "<text x=\"{}\" y=\"{}\" font-family=\"{}\" font-size=\"{}\"{} fill=\"{}\"{}{rot}{fit} xml:space=\"preserve\">{}</text>\n",
            f2(x),
            f2(base_y),
            ts.css_family,
            f2(ts.px),
            if ts.bold { " font-weight=\"bold\"" } else { "" },
            svg_color(ts.color),
            svg_opacity("fill-opacity", ts.color),
            crate::export::pptx::xml_escape(s)
        ));
    }
}

/// Visible data range: explicit view, else configured range, else data + 5 %.
pub fn resolve_view(cfg: &PlotConfig, data: &PlotData, view: Option<[f64; 4]>) -> [f64; 4] {
    if let Some(v) = view
        && v[0] < v[1]
        && v[2] < v[3]
    {
        return v;
    }
    let b = data.bounds.unwrap_or([0.0, 1.0, 0.0, 1.0]);
    let pad = |lo: f64, hi: f64| {
        let span = hi - lo;
        if span > 0.0 {
            (lo - span * 0.05, hi + span * 0.05)
        } else {
            let d = if lo == 0.0 { 1.0 } else { lo.abs() * 0.1 };
            (lo - d, hi + d)
        }
    };
    let (mut x0, mut x1) = pad(b[0], b[1]);
    let (mut y0, mut y1) = pad(b[2], b[3]);
    if data.x_categories.is_some() {
        x0 = b[0] - 0.5;
        x1 = b[1] + 0.5;
    }
    let lx = &cfg.layout.x;
    if !lx.auto_range && lx.min < lx.max {
        x0 = lx.min;
        x1 = lx.max;
    }
    let ly = &cfg.layout.y;
    if !ly.auto_range && ly.min < ly.max {
        y0 = ly.min;
        y1 = ly.max;
    }
    [x0, x1, y0, y1]
}

struct AxisTicks {
    values: Vec<f64>,
    labels: Vec<String>,
}

#[allow(clippy::too_many_arguments)]
fn axis_ticks(
    lo: f64,
    hi: f64,
    len_px: f32,
    target_px: f32,
    auto: bool,
    step: f64,
    auto_dec: bool,
    dec: usize,
    cats: Option<&[String]>,
) -> AxisTicks {
    if let Some(cats) = cats {
        let mut values = Vec::new();
        let mut labels = Vec::new();
        let every = ((cats.len() as f32 * target_px * 0.6 / len_px.max(1.0)).ceil() as usize).max(1);
        for (i, c) in cats.iter().enumerate() {
            let v = i as f64;
            if v >= lo && v <= hi && i % every == 0 {
                values.push(v);
                labels.push(c.clone());
            }
        }
        return AxisTicks { values, labels };
    }
    let step = if auto || step <= 0.0 {
        geom::nice_step((hi - lo) / (len_px / target_px).max(1.0) as f64)
    } else {
        step
    };
    let mut values = geom::ticks(lo, hi, step);
    if values.len() > 400 {
        values.clear();
    }
    let decimals = if auto_dec { geom::decimals_for_step(step) } else { dec };
    let labels = values.iter().map(|v| geom::format_tick(*v, decimals)).collect();
    AxisTicks { values, labels }
}

/// Draws a plot of `w` × `h` pixels on a canvas.
fn draw_plot(
    c: &mut dyn Canvas,
    cfg: &PlotConfig,
    data: &PlotData,
    fonts: &FontBook,
    opts: &RenderOptions,
    w: u32,
    h: u32,
) {
    let s = opts.scale;
    let lay = &cfg.layout;
    let bg = lay.background;
    c.rect(
        Bx::new(0.0, 0.0, w as f32, h as f32),
        Some([bg[0], bg[1], bg[2], 255]),
        None,
    );

    let margin = 14.0 * s;
    let mut area = Bx::new(margin, margin, w as f32 - margin, h as f32 - margin);

    // ---- title --------------------------------------------------------------
    if lay.show_title && !lay.title.trim().is_empty() {
        let ts = TextStyle::new(fonts, &lay.title_font, s);
        let tw = ts.width(&lay.title);
        c.text(&ts, &lay.title, (w as f32 - tw) * 0.5, area.y0, 0.0);
        area.y0 += ts.height() + 8.0 * s;
    }

    if let Some(err) = &data.error {
        let ts = TextStyle::new(fonts, &FontSpec::new(16.0), s);
        c.text(&ts, err, area.x0, area.y0, 0.0);
        return;
    }

    let styles = trace_styles(cfg, data);

    // ---- legend size ------------------------------------------------------
    let legend_font = TextStyle::new(fonts, &lay.legend.font, s);
    let entries: Vec<usize> = (0..data.traces.len()).filter(|&i| styles[i].visible).collect();
    let show_legend = lay.legend.show && !entries.is_empty();
    let lm = LegendMetrics::new(legend_font.px, !lay.legend.title.is_empty());
    let max_legend_h = match lay.legend.position {
        super::config::LegendPos::OutsideTop | super::config::LegendPos::OutsideBottom => area.h() * 0.35,
        _ => area.h() * 0.92,
    };
    let lgrid = {
        let max_w = entries
            .iter()
            .map(|&i| legend_font.width(&styles[i].label))
            .fold(0.0f32, f32::max);
        let title_w = legend_font.width(&lay.legend.title);
        lm.grid(entries.len(), max_w, title_w, max_legend_h)
    };
    let (lw, lh) = if show_legend { (lgrid.w, lgrid.h) } else { (0.0, 0.0) };
    let dock = if show_legend {
        geom::dock_side(lay.legend.position, area, lw, lh)
    } else {
        Dock::None
    };
    // the interactive gap is in screen pixels: scale it
    let mut pbox = geom::plot_box(area, dock, lw, lh);
    if dock != Dock::None {
        let extra = geom::DOCK_GAP * (s - 1.0);
        match dock {
            Dock::Right => pbox.x1 -= extra,
            Dock::Left => pbox.x0 += extra,
            Dock::Top => pbox.y0 += extra,
            Dock::Bottom => pbox.y1 -= extra,
            Dock::None => {}
        }
    }

    // ---- axes geometry ------------------------------------------------------
    let view = resolve_view(cfg, data, opts.view);
    let tick_font = TextStyle::new(fonts, &lay.tick_font, s);
    let label_font = TextStyle::new(fonts, &lay.label_font, s);
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
    let tick_h = tick_font.height();
    let label_h = label_font.height();
    let tick_len = 5.0 * s;
    let x_axis_h = tick_len + 3.0 * s + tick_h + if x_label.is_empty() { 0.0 } else { label_h + 6.0 * s };
    let top_pad = tick_h * 0.6;
    let frame_h_est = (pbox.h() - x_axis_h - top_pad).max(20.0);
    let yt = axis_ticks(
        view[2],
        view[3],
        frame_h_est,
        55.0 * s,
        lay.y.auto_ticks,
        lay.y.step,
        lay.y.auto_decimals,
        lay.y.decimals,
        None,
    );
    let ytick_w = yt.labels.iter().map(|l| tick_font.width(l)).fold(0.0f32, f32::max);
    let y_axis_w = tick_len + 4.0 * s + ytick_w + if y_label.is_empty() { 0.0 } else { label_h + 8.0 * s };
    let frame_w_est = (pbox.w() - y_axis_w - 10.0 * s).max(20.0);
    let xt = axis_ticks(
        view[0],
        view[1],
        frame_w_est,
        95.0 * s,
        lay.x.auto_ticks,
        lay.x.step,
        lay.x.auto_decimals,
        lay.x.decimals,
        data.x_categories.as_deref(),
    );
    let last_half = xt.labels.last().map(|l| tick_font.width(l) * 0.5).unwrap_or(0.0);
    let right_pad = (last_half - 2.0 * s).max(6.0 * s);
    let frame = Bx::new(
        pbox.x0 + y_axis_w,
        pbox.y0 + top_pad,
        pbox.x1 - right_pad,
        pbox.y1 - x_axis_h,
    );
    if frame.w() < 10.0 || frame.h() < 10.0 {
        return;
    }
    let sx = |x: f64| frame.x0 + ((x - view[0]) / (view[1] - view[0])) as f32 * frame.w();
    let sy = |y: f64| frame.y1 - ((y - view[2]) / (view[3] - view[2])) as f32 * frame.h();

    // ---- grid, frame, ticks -------------------------------------------------
    let mut grid = lay.grid_color;
    grid[3] = 70;
    if lay.x.grid {
        for &v in &xt.values {
            let x = sx(v);
            c.line(x, frame.y0, x, frame.y1, grid, 1.0 * s);
        }
    }
    if lay.y.grid {
        for &v in &yt.values {
            let y = sy(v);
            c.line(frame.x0, y, frame.x1, y, grid, 1.0 * s);
        }
    }
    let axis_color: Rgba = [60, 60, 60, 255];
    for (v, l) in xt.values.iter().zip(&xt.labels) {
        let x = sx(*v);
        c.line(x, frame.y1, x, frame.y1 + tick_len, axis_color, 1.2 * s);
        let tw = tick_font.width(l);
        c.text(&tick_font, l, x - tw * 0.5, frame.y1 + tick_len + 3.0 * s, 0.0);
    }
    for (v, l) in yt.values.iter().zip(&yt.labels) {
        let y = sy(*v);
        c.line(frame.x0 - tick_len, y, frame.x0, y, axis_color, 1.2 * s);
        let tw = tick_font.width(l);
        c.text(&tick_font, l, frame.x0 - tick_len - 4.0 * s - tw, y - tick_h * 0.5, 0.0);
    }
    if !x_label.is_empty() {
        let lw_ = label_font.width(&x_label);
        let y = frame.y1 + tick_len + 3.0 * s + tick_h + 6.0 * s;
        c.text(&label_font, &x_label, (frame.x0 + frame.x1 - lw_) * 0.5, y, 0.0);
    }
    if !y_label.is_empty() {
        let lw_ = label_font.width(&y_label);
        let x = frame.x0 - tick_len - 4.0 * s - ytick_w - 8.0 * s - label_h;
        let y = (frame.y0 + frame.y1 + lw_) * 0.5;
        c.text(&label_font, &y_label, x, y, -90.0);
    }

    // ---- traces (clipped to the frame) -----------------------------------
    c.group(Some("traces"));
    c.set_clip(frame);
    let clamp = |v: f32| v.clamp(-1.0e5, 1.0e5);
    let buckets = (frame.w() as usize).max(64);
    let mut buf = Vec::new();
    let mut pts: Vec<(f32, f32)> = Vec::new();
    for (t, st) in data.traces.iter().zip(&styles) {
        if !st.visible {
            continue;
        }
        for seg in &t.segments {
            if t.sorted && cfg.options.decimate {
                geom::decimate(seg, view[0], view[1], buckets, &mut buf);
            } else {
                buf.clear();
                buf.extend_from_slice(seg);
            }
            if buf.is_empty() {
                continue;
            }
            if st.line != LineKind::None {
                pts.clear();
                pts.extend(buf.iter().map(|p| (clamp(sx(p[0])), clamp(sy(p[1])))));
                c.polyline(&pts, st.color, st.width * s, st.line, true);
            }
            if st.marker != MarkerKind::None && buf.len() <= 20_000 {
                for p in &buf {
                    let (x, y) = (sx(p[0]), sy(p[1]));
                    if frame.contains(x, y) {
                        c.marker(st.marker, x, y, st.marker_size * s, st.color, true);
                    }
                }
            }
        }
    }
    c.group(None);
    c.rect(frame, None, Some(([70, 70, 70, 255], 1.2 * s)));

    // ---- data markers --------------------------------------------------------
    let marker_font = TextStyle::new(fonts, &FontSpec::new(lay.tick_font.size * 0.95), s);
    for m in &cfg.markers {
        let (x, y) = (sx(m.x), sy(m.y));
        if !frame.contains(x, y) {
            continue;
        }
        let color = data
            .traces
            .iter()
            .position(|t| t.key == m.trace)
            .map(|i| styles[i].color)
            .unwrap_or([0, 0, 0, 255]);
        c.marker(MarkerKind::Circle, x, y, 4.5 * s, [255, 255, 255, 255], false);
        c.marker(MarkerKind::Circle, x, y, 3.2 * s, color, false);
        let text = format!("x={}  y={}", format_sig(m.x, 6), format_sig(m.y, 5));
        let tw = marker_font.width(&text);
        let th = marker_font.height();
        let mut bx = Bx::new(x + 6.0 * s, y - th - 8.0 * s, x + 6.0 * s + tw + 8.0 * s, y - 4.0 * s);
        if bx.x1 > frame.x1 {
            let d = bx.w() + 12.0 * s;
            bx.x0 -= d;
            bx.x1 -= d;
        }
        if bx.y0 < frame.y0 {
            let d = th + 12.0 * s;
            bx.y0 += d;
            bx.y1 += d;
        }
        c.rect(bx, Some([255, 255, 255, 225]), Some((color, 1.0 * s)));
        c.text(&marker_font, &text, bx.x0 + 4.0 * s, bx.y0 + 2.0 * s, 0.0);
    }

    // ---- legend ------------------------------------------------------------
    if show_legend {
        c.group(Some("legend"));
        let lb = geom::legend_box(lay.legend.position, area, frame, lw, lh);
        if lay.legend.background {
            c.rect(lb, Some([255, 255, 255, 235]), None);
        }
        if lay.legend.border {
            c.rect(lb, None, Some(([120, 120, 120, 255], 1.0 * s)));
        }
        if !lay.legend.title.is_empty() {
            c.text(&legend_font, &lay.legend.title, lb.x0 + lm.pad, lb.y0 + lm.pad, 0.0);
        }
        for (k, &i) in entries.iter().enumerate() {
            let st = &styles[i];
            let (ex, ey) = lm.entry_pos(&lgrid, k);
            let cy = lb.y0 + ey + lm.row_h * 0.5;
            let x0 = lb.x0 + ex;
            if st.line != LineKind::None {
                let lw = (st.width * s).min(lm.row_h * 0.5);
                c.polyline(&[(x0, cy), (x0 + lm.swatch, cy)], st.color, lw, st.line, false);
            }
            if st.marker != MarkerKind::None {
                c.marker(st.marker, x0 + lm.swatch * 0.5, cy, st.marker_size * s, st.color, false);
            }
            let th = legend_font.height();
            c.text(&legend_font, &st.label, x0 + lm.swatch + lm.gap, cy - th * 0.5, 0.0);
        }
    }
    c.group(None);
}

/// Renders a plot to a pixmap.
pub fn render(cfg: &PlotConfig, data: &PlotData, fonts: &FontBook, opts: &RenderOptions) -> Result<Pixmap, String> {
    let w = opts.width.clamp(64, 8000);
    let h = opts.height.clamp(64, 8000);
    let pm = Pixmap::new(w, h).ok_or("Cannot allocate the image")?;
    let mut c = PixmapCanvas { pm, mask: None };
    draw_plot(&mut c, cfg, data, fonts, opts, w, h);
    Ok(c.pm)
}

/// Renders a plot to an SVG document (vector curves, editable text).
pub fn render_svg(cfg: &PlotConfig, data: &PlotData, fonts: &FontBook, opts: &RenderOptions) -> String {
    let w = opts.width.clamp(64, 8000);
    let h = opts.height.clamp(64, 8000);
    let mut c = SvgCanvas::new(w, h);
    draw_plot(&mut c, cfg, data, fonts, opts, w, h);
    c.finish()
}

/// Renders a plot to PNG bytes.
pub fn render_png(
    cfg: &PlotConfig,
    data: &PlotData,
    fonts: &FontBook,
    opts: &RenderOptions,
) -> Result<Vec<u8>, String> {
    let pm = render(cfg, data, fonts, opts)?;
    pm.encode_png().map_err(|e| format!("PNG encoding failed: {e}"))
}

/// File format of the exported images.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ImageFormat {
    #[default]
    Png,
    Svg,
}

impl ImageFormat {
    pub const ALL: [ImageFormat; 2] = [ImageFormat::Png, ImageFormat::Svg];

    pub fn ext(self) -> &'static str {
        match self {
            ImageFormat::Png => "png",
            ImageFormat::Svg => "svg",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ImageFormat::Png => "PNG (bitmap)",
            ImageFormat::Svg => "SVG (vector)",
        }
    }
}

/// Renders a plot to the bytes of an image file.
pub fn render_image(
    cfg: &PlotConfig,
    data: &PlotData,
    fonts: &FontBook,
    opts: &RenderOptions,
    format: ImageFormat,
) -> Result<Vec<u8>, String> {
    match format {
        ImageFormat::Png => render_png(cfg, data, fonts, opts),
        ImageFormat::Svg => Ok(render_svg(cfg, data, fonts, opts).into_bytes()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::loader::{ImportOptions, parse_csv};
    use crate::plot::compute::compute;

    fn plot() -> (PlotConfig, PlotData, FontBook) {
        let mut s = String::from("Temp,Freq,S21\n");
        for t in [-40, 25, 85] {
            for f in 0..20 {
                s.push_str(&format!("{t},{f}e9,{}\n", f as f64 * 0.5 - t as f64 / 20.0));
            }
        }
        let ds = parse_csv(s.as_bytes(), &ImportOptions::default()).unwrap();
        let mut cfg = PlotConfig {
            x: "Freq".into(),
            ys: vec!["S21".into()],
            series: vec!["Temp".into()],
            ..Default::default()
        };
        cfg.layout.title = "S21 <dB> & co".into();
        cfg.layout.show_title = true;
        let data = compute(&ds, &cfg);
        let fonts = FontBook::new(epaint_default_fonts::UBUNTU_LIGHT, epaint_default_fonts::HACK_REGULAR);
        (cfg, data, fonts)
    }

    #[test]
    fn svg_output() {
        let (cfg, data, fonts) = plot();
        let svg = render_svg(&cfg, &data, &fonts, &RenderOptions::new(800, 500));
        if let Ok(path) = std::env::var("CT_SVG_OUT") {
            std::fs::write(
                format!("{path}.png"),
                render_png(&cfg, &data, &fonts, &RenderOptions::new(800, 500)).unwrap(),
            )
            .unwrap();
            std::fs::write(path, &svg).unwrap();
        }
        assert!(svg.starts_with("<?xml"));
        assert!(svg.trim_end().ends_with("</svg>"));
        assert_eq!(svg.matches("<polyline").count(), 3 + 3, "3 curves + 3 legend swatches");
        assert!(svg.contains("S21 &lt;dB&gt; &amp; co"));
        // temperature colors
        assert!(svg.contains("#1450dc") && svg.contains("#149632") && svg.contains("#d71e1e"));
        assert!(svg.contains("stroke-width=\"1.6\""), "default width 2 px × scale 0.8");
        assert!(svg.contains("clip-path=\"url(#frame)\""));
    }

    #[test]
    fn png_output() {
        let (cfg, data, fonts) = plot();
        let png = render_image(&cfg, &data, &fonts, &RenderOptions::new(400, 300), ImageFormat::Png).unwrap();
        assert_eq!(&png[1..4], b"PNG");
    }
}
