//! Resolution of the final style (color, width, label…) of every trace.

use super::compute::PlotData;
use super::config::{LineKind, MarkerKind, PlotConfig, Rgba};

#[derive(Clone, Debug, PartialEq)]
pub struct TraceStyle {
    pub color: Rgba,
    pub width: f32,
    pub line: LineKind,
    pub marker: MarkerKind,
    pub marker_size: f32,
    pub visible: bool,
    pub label: String,
}

pub fn trace_styles(cfg: &PlotConfig, data: &PlotData) -> Vec<TraceStyle> {
    let d = &cfg.traces;
    let n_orig = data.traces.iter().filter(|t| !t.aggregate).count();
    let n_agg = data.traces.len() - n_orig;
    let both = data.has_aggregates_and_originals;
    let mut i_orig = 0;
    let mut i_agg = 0;
    data.traces
        .iter()
        .map(|t| {
            let (mut color, mut width) = if both && t.aggregate {
                let c = d.palette.color(i_agg, n_agg);
                i_agg += 1;
                (c, d.width * 1.8 + 0.5)
            } else if both {
                let mut c = d.palette.color(i_orig, n_orig);
                c[3] = 90;
                i_orig += 1;
                (c, d.width)
            } else {
                let i = i_orig + i_agg;
                let c = d.palette.color(i, data.traces.len());
                if t.aggregate {
                    i_agg += 1;
                } else {
                    i_orig += 1;
                }
                (c, d.width)
            };
            let mut line = d.line;
            let mut marker = d.marker;
            let mut visible = true;
            let mut label = t.label.clone();
            if let Some(o) = cfg.overrides.get(&t.key) {
                if let Some(c) = o.color {
                    color = c;
                }
                if let Some(w) = o.width {
                    width = w;
                }
                if let Some(l) = o.line {
                    line = l;
                }
                if let Some(m) = o.marker {
                    marker = m;
                }
                visible = o.visible;
                if !o.label.trim().is_empty() {
                    label = o.label.clone();
                }
            }
            if line == LineKind::None && marker == MarkerKind::None {
                marker = MarkerKind::Circle;
            }
            TraceStyle {
                color,
                width,
                line,
                marker,
                marker_size: d.marker_size,
                visible,
                label,
            }
        })
        .collect()
}

/// Legend row metrics (in pixels, before any font measurement).
pub struct LegendMetrics {
    pub pad: f32,
    pub swatch: f32,
    pub gap: f32,
    pub row_h: f32,
    pub title_h: f32,
}

/// Arrangement of the legend entries in columns.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LegendGrid {
    pub cols: usize,
    pub rows: usize,
    pub col_w: f32,
    pub w: f32,
    pub h: f32,
}

impl LegendMetrics {
    pub fn new(font_px: f32, has_title: bool) -> LegendMetrics {
        LegendMetrics {
            pad: (font_px * 0.5).max(4.0),
            swatch: font_px * 2.0,
            gap: font_px * 0.5,
            row_h: font_px * 1.35,
            title_h: if has_title { font_px * 1.5 } else { 0.0 },
        }
    }

    fn col_gap(&self) -> f32 {
        self.gap * 2.0
    }

    /// Splits `n` entries in as many columns as needed to fit in `max_h`.
    pub fn grid(&self, n: usize, max_text_w: f32, title_w: f32, max_h: f32) -> LegendGrid {
        let avail = (max_h - self.pad * 2.0 - self.title_h).max(self.row_h);
        let max_rows = ((avail / self.row_h).floor() as usize).max(1);
        let cols = n.div_ceil(max_rows).max(1);
        let rows = n.div_ceil(cols);
        let col_w = self.swatch + self.gap + max_text_w;
        let w =
            (self.pad * 2.0 + cols as f32 * col_w + (cols - 1) as f32 * self.col_gap()).max(title_w + self.pad * 2.0);
        let h = self.pad * 2.0 + self.title_h + rows as f32 * self.row_h;
        LegendGrid {
            cols,
            rows,
            col_w,
            w,
            h,
        }
    }

    /// Top-left corner of entry `k`, relative to the legend box.
    pub fn entry_pos(&self, g: &LegendGrid, k: usize) -> (f32, f32) {
        let (col, row) = (k / g.rows.max(1), k % g.rows.max(1));
        (
            self.pad + col as f32 * (g.col_w + self.col_gap()),
            self.pad + self.title_h + row as f32 * self.row_h,
        )
    }

    /// Entry under a point (relative to the legend box).
    pub fn entry_at(&self, g: &LegendGrid, dx: f32, dy: f32, n: usize) -> Option<usize> {
        let y = dy - self.pad - self.title_h;
        let x = dx - self.pad;
        if x < 0.0 || y < 0.0 {
            return None;
        }
        let row = (y / self.row_h) as usize;
        let col = (x / (g.col_w + self.col_gap())) as usize;
        if row >= g.rows || col >= g.cols {
            return None;
        }
        let k = col * g.rows + row;
        (k < n).then_some(k)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legend_columns() {
        let m = LegendMetrics::new(10.0, false);
        let g = m.grid(30, 100.0, 0.0, 200.0);
        assert!(g.cols >= 2);
        assert!(g.h <= 200.0);
        assert_eq!(g.cols * g.rows >= 30, true);
        let (x, y) = m.entry_pos(&g, g.rows + 1);
        assert_eq!(m.entry_at(&g, x + 1.0, y + 1.0, 30), Some(g.rows + 1));
        let g1 = m.grid(3, 100.0, 0.0, 500.0);
        assert_eq!((g1.cols, g1.rows), (1, 3));
    }
}
