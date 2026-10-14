//! Pure geometry shared by the interactive view and the PNG renderer:
//! legend placement, decimation, tick generation and formatting.

use super::config::LegendPos;

/// Axis-aligned box in pixels (y grows downwards).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bx {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

impl Bx {
    pub fn new(x0: f32, y0: f32, x1: f32, y1: f32) -> Bx {
        Bx { x0, y0, x1, y1 }
    }
    pub fn w(&self) -> f32 {
        self.x1 - self.x0
    }
    pub fn h(&self) -> f32 {
        self.y1 - self.y0
    }
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x0 && x <= self.x1 && y >= self.y0 && y <= self.y1
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dock {
    None,
    Left,
    Right,
    Top,
    Bottom,
}

/// Gap between a docked legend and the plot.
pub const DOCK_GAP: f32 = 12.0;
/// A free legend closer than this to the figure border is docked outside.
pub const SNAP: f32 = 4.0;
/// Margin of inside legends.
pub const INSIDE_MARGIN: f32 = 8.0;

/// Legend box of a free legend (normalized position inside `area`).
pub fn free_box(area: Bx, w: f32, h: f32, x: f32, y: f32) -> Bx {
    let x0 = area.x0 + x.clamp(0.0, 1.0) * (area.w() - w).max(0.0);
    let y0 = area.y0 + y.clamp(0.0, 1.0) * (area.h() - h).max(0.0);
    Bx::new(x0, y0, x0 + w, y0 + h)
}

/// Converts a legend box back to a free position.
pub fn to_free(area: Bx, b: Bx) -> (f32, f32) {
    let fx = if area.w() - b.w() > 1.0 {
        (b.x0 - area.x0) / (area.w() - b.w())
    } else {
        0.0
    };
    let fy = if area.h() - b.h() > 1.0 {
        (b.y0 - area.y0) / (area.h() - b.h())
    } else {
        0.0
    };
    (fx.clamp(0.0, 1.0), fy.clamp(0.0, 1.0))
}

/// On which side the legend is docked (outside the plot).
pub fn dock_side(pos: LegendPos, area: Bx, w: f32, h: f32) -> Dock {
    match pos {
        LegendPos::OutsideRight => Dock::Right,
        LegendPos::OutsideLeft => Dock::Left,
        LegendPos::OutsideTop => Dock::Top,
        LegendPos::OutsideBottom => Dock::Bottom,
        LegendPos::Free { x, y } => {
            let b = free_box(area, w, h, x, y);
            // never dock when the legend would eat most of the figure
            if b.x1 >= area.x1 - SNAP && w < area.w() * 0.6 {
                Dock::Right
            } else if b.x0 <= area.x0 + SNAP && w < area.w() * 0.6 {
                Dock::Left
            } else if b.y0 <= area.y0 + SNAP && h < area.h() * 0.6 {
                Dock::Top
            } else if b.y1 >= area.y1 - SNAP && h < area.h() * 0.6 {
                Dock::Bottom
            } else {
                Dock::None
            }
        }
        _ => Dock::None,
    }
}

/// Rectangle left for the plot widget (axes included) once the legend is
/// docked outside.
pub fn plot_box(area: Bx, dock: Dock, w: f32, h: f32) -> Bx {
    let mut b = area;
    match dock {
        Dock::None => {}
        Dock::Right => b.x1 = area.x1 - w - DOCK_GAP,
        Dock::Left => b.x0 = area.x0 + w + DOCK_GAP,
        Dock::Top => b.y0 = area.y0 + h + DOCK_GAP,
        Dock::Bottom => b.y1 = area.y1 - h - DOCK_GAP,
    }
    if b.w() < 60.0 {
        b.x1 = b.x0 + 60.0;
    }
    if b.h() < 60.0 {
        b.y1 = b.y0 + 60.0;
    }
    b
}

/// Final legend box once the plot frame (data area) is known.
pub fn legend_box(pos: LegendPos, area: Bx, frame: Bx, w: f32, h: f32) -> Bx {
    let m = INSIDE_MARGIN;
    let at = |x0: f32, y0: f32| Bx::new(x0, y0, x0 + w, y0 + h);
    let center_x = (frame.x0 + frame.x1 - w) * 0.5;
    let center_x = center_x.clamp(area.x0, (area.x1 - w).max(area.x0));
    match pos {
        LegendPos::InsideTopRight => at(frame.x1 - w - m, frame.y0 + m),
        LegendPos::InsideTopLeft => at(frame.x0 + m, frame.y0 + m),
        LegendPos::InsideBottomRight => at(frame.x1 - w - m, frame.y1 - h - m),
        LegendPos::InsideBottomLeft => at(frame.x0 + m, frame.y1 - h - m),
        LegendPos::OutsideRight => at(area.x1 - w, frame.y0.min(area.y1 - h).max(area.y0)),
        LegendPos::OutsideLeft => at(area.x0, frame.y0.min(area.y1 - h).max(area.y0)),
        LegendPos::OutsideTop => at(center_x, area.y0),
        LegendPos::OutsideBottom => at(center_x, area.y1 - h),
        LegendPos::Free { x, y } => free_box(area, w, h, x, y),
    }
}

/// Min/max decimation of an ascending-X polyline for display.
/// Keeps at most ~4 points per bucket (first, min, max, last), plus the
/// points just outside the visible range so that lines reach the borders.
pub fn decimate(seg: &[[f64; 2]], x0: f64, x1: f64, buckets: usize, out: &mut Vec<[f64; 2]>) {
    out.clear();
    if seg.is_empty() {
        return;
    }
    let start = seg.partition_point(|p| p[0] < x0).saturating_sub(1);
    let end = (seg.partition_point(|p| p[0] <= x1) + 1).min(seg.len());
    if start >= end {
        return;
    }
    let vis = &seg[start..end];
    let buckets = buckets.max(16);
    if vis.len() <= buckets * 4 || x1 <= x0 {
        out.extend_from_slice(vis);
        return;
    }
    let width = (x1 - x0) / buckets as f64;
    let bucket_of = |x: f64| ((x - x0) / width).floor() as i64;
    let mut i = 0;
    while i < vis.len() {
        let b = bucket_of(vis[i][0]);
        let mut j = i + 1;
        while j < vis.len() && bucket_of(vis[j][0]) == b {
            j += 1;
        }
        let run = &vis[i..j];
        if run.len() <= 4 {
            out.extend_from_slice(run);
        } else {
            let mut imin = 0;
            let mut imax = 0;
            for (k, p) in run.iter().enumerate() {
                if p[1] < run[imin][1] {
                    imin = k;
                }
                if p[1] > run[imax][1] {
                    imax = k;
                }
            }
            out.push(run[0]);
            let (a, b) = if imin < imax { (imin, imax) } else { (imax, imin) };
            if a != 0 {
                out.push(run[a]);
            }
            if b != a && b != run.len() - 1 {
                out.push(run[b]);
            }
            out.push(run[run.len() - 1]);
        }
        i = j;
    }
}

/// "Nice" step (1, 2, 2.5, 5 × 10^k) close to `raw`.
pub fn nice_step(raw: f64) -> f64 {
    if !(raw.is_finite() && raw > 0.0) {
        return 1.0;
    }
    let mag = 10f64.powf(raw.log10().floor());
    let f = raw / mag;
    let nice = if f <= 1.0 {
        1.0
    } else if f <= 2.0 {
        2.0
    } else if f <= 2.5 {
        2.5
    } else if f <= 5.0 {
        5.0
    } else {
        10.0
    };
    nice * mag
}

/// Tick values (multiples of `step`) inside `[lo, hi]`.
pub fn ticks(lo: f64, hi: f64, step: f64) -> Vec<f64> {
    if step.is_nan() || step <= 0.0 || !lo.is_finite() || !hi.is_finite() || hi < lo {
        return Vec::new();
    }
    if (hi - lo) / step > 5000.0 {
        return Vec::new();
    }
    let k0 = (lo / step - 1e-9).ceil() as i64;
    let k1 = (hi / step + 1e-9).floor() as i64;
    (k0..=k1).map(|k| clean(k as f64 * step, step)).collect()
}

/// Removes floating point noise (0.30000000000000004 → 0.3).
fn clean(v: f64, step: f64) -> f64 {
    let d = decimals_for_step(step) as i32 + 2;
    let p = 10f64.powi(d.min(15));
    let r = (v * p).round() / p;
    if r == 0.0 { 0.0 } else { r }
}

/// Number of decimals needed to print multiples of `step` exactly.
pub fn decimals_for_step(step: f64) -> usize {
    if !(step.is_finite() && step > 0.0) {
        return 0;
    }
    for d in 0..=10usize {
        let s = step * 10f64.powi(d as i32);
        if (s - s.round()).abs() < 1e-6 * s.max(1.0) {
            return d;
        }
    }
    10
}

/// Formats a tick label.
pub fn format_tick(v: f64, decimals: usize) -> String {
    let a = v.abs();
    if a != 0.0 && !(1e-6..1e9).contains(&a) {
        // shortest mantissa that keeps neighbouring ticks distinct (2.5e9, 1.25e10)
        for p in 0..6 {
            let t = format!("{v:.p$e}");
            if t.parse::<f64>().is_ok_and(|r| (r - v).abs() <= a * 1e-9) {
                return t;
            }
        }
        return format!("{v:.6e}");
    }
    let s = format!("{v:.decimals$}");
    if s.starts_with('-') && s[1..].chars().all(|c| c == '0' || c == '.') {
        s[1..].to_owned()
    } else {
        s
    }
}

/// Is `v` (close to) a multiple of `step`?
pub fn is_multiple(v: f64, step: f64) -> bool {
    if step <= 0.0 {
        return false;
    }
    let k = v / step;
    (k - k.round()).abs() < 1e-6
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legend_docking() {
        let area = Bx::new(0.0, 0.0, 800.0, 600.0);
        let pos = LegendPos::Free { x: 1.0, y: 0.3 };
        assert_eq!(dock_side(pos, area, 150.0, 100.0), Dock::Right);
        let pb = plot_box(area, Dock::Right, 150.0, 100.0);
        assert_eq!(pb.x1, 800.0 - 150.0 - DOCK_GAP);
        let pos = LegendPos::Free { x: 0.5, y: 0.5 };
        assert_eq!(dock_side(pos, area, 150.0, 100.0), Dock::None);
        let b = free_box(area, 150.0, 100.0, 0.5, 0.5);
        let (fx, fy) = to_free(area, b);
        assert!((fx - 0.5).abs() < 1e-6 && (fy - 0.5).abs() < 1e-6);
        let frame = Bx::new(60.0, 10.0, 790.0, 560.0);
        let lb = legend_box(LegendPos::InsideTopRight, area, frame, 100.0, 50.0);
        assert_eq!(lb.x1, 790.0 - INSIDE_MARGIN);
    }

    #[test]
    fn decimation() {
        let seg: Vec<[f64; 2]> = (0..100_000).map(|i| [i as f64, (i % 7) as f64]).collect();
        let mut out = Vec::new();
        decimate(&seg, 0.0, 99_999.0, 500, &mut out);
        assert!(out.len() <= 2000 && out.len() >= 500);
        assert!(out.iter().any(|p| p[1] == 6.0) && out.iter().any(|p| p[1] == 0.0));
        assert!(out.windows(2).all(|w| w[0][0] <= w[1][0]));
        decimate(&seg, 10.0, 20.0, 500, &mut out);
        assert_eq!(out.first().unwrap()[0], 9.0);
        assert_eq!(out.last().unwrap()[0], 21.0);
    }

    #[test]
    fn tick_values() {
        assert_eq!(nice_step(0.37), 0.5);
        assert_eq!(nice_step(7.0), 10.0);
        assert_eq!(ticks(0.0, 1.0, 0.1).len(), 11);
        assert_eq!(ticks(0.0, 1.0, 0.1)[3], 0.3);
        assert_eq!(
            ticks(35.0, 70.0, 5.0),
            vec![35.0, 40.0, 45.0, 50.0, 55.0, 60.0, 65.0, 70.0]
        );
        assert_eq!(decimals_for_step(0.25), 2);
        assert_eq!(decimals_for_step(5.0), 0);
        assert_eq!(format_tick(-0.0, 1), "0.0");
        assert_eq!(format_tick(2.5, 2), "2.50");
        assert_eq!(format_tick(1.25e10, 0), "1.25e10");
        assert_eq!(format_tick(1.5e10, 0), "1.5e10");
        assert_eq!(format_tick(2e9, 0), "2e9");
        assert!(is_multiple(45.0, 5.0));
        assert!(!is_multiple(46.0, 5.0));
    }
}
