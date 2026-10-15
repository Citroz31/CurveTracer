//! Smoothing of the curves (Data → Process Data → Smoothing).
//!
//! Every method works on the points of a curve segment in their order
//! (ascending X for sorted curves); the windows shrink at the ends of the
//! curve (symmetrically, so the ends are not biased) instead of inventing
//! values.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SmoothMethod {
    /// Mean of the `window` neighbouring points.
    #[default]
    MovingAverage,
    /// Gaussian weighted mean (standard deviation `sigma` points).
    Gaussian,
    /// Local polynomial fit (keeps the height and width of the peaks).
    SavitzkyGolay,
    /// Median of the `window` neighbouring points (removes isolated spikes).
    Median,
    /// Exponential average, forward then backward (no delay).
    Exponential,
    /// Mean of the points within ± width / 2 on the X axis (irregular sampling).
    XWindow,
}

impl SmoothMethod {
    pub const ALL: [SmoothMethod; 6] = [
        SmoothMethod::MovingAverage,
        SmoothMethod::Gaussian,
        SmoothMethod::SavitzkyGolay,
        SmoothMethod::Median,
        SmoothMethod::Exponential,
        SmoothMethod::XWindow,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SmoothMethod::MovingAverage => "Moving average",
            SmoothMethod::Gaussian => "Gaussian",
            SmoothMethod::SavitzkyGolay => "Savitzky–Golay",
            SmoothMethod::Median => "Median",
            SmoothMethod::Exponential => "Exponential (zero-phase)",
            SmoothMethod::XWindow => "Moving average (X width)",
        }
    }

    pub fn short(self) -> &'static str {
        match self {
            SmoothMethod::MovingAverage => "avg",
            SmoothMethod::Gaussian => "gauss",
            SmoothMethod::SavitzkyGolay => "SG",
            SmoothMethod::Median => "median",
            SmoothMethod::Exponential => "exp",
            SmoothMethod::XWindow => "X avg",
        }
    }

    pub fn help(self) -> &'static str {
        match self {
            SmoothMethod::MovingAverage => {
                "Mean of the N neighbouring points. Simple and strong, but flattens the peaks."
            }
            SmoothMethod::Gaussian => {
                "Weighted mean with a bell-shaped weight (σ in points): smoother result than the moving average."
            }
            SmoothMethod::SavitzkyGolay => {
                "Fits a polynomial on N points around each point: removes the noise while keeping the \
                 peaks, slopes and ripples (ideal for S-parameters)."
            }
            SmoothMethod::Median => {
                "Median of the N neighbouring points: removes isolated spikes and glitches, keeps the steps."
            }
            SmoothMethod::Exponential => {
                "Exponential average over about N points, applied forward then backward so the curve is not shifted."
            }
            SmoothMethod::XWindow => {
                "Mean of the points closer than ± width / 2 on the X axis (X units): for irregular X steps."
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SmoothConfig {
    pub enabled: bool,
    pub method: SmoothMethod,
    /// Number of points of the window (odd).
    pub window: usize,
    /// Gaussian: standard deviation, in points.
    pub sigma: f64,
    /// Savitzky–Golay: polynomial order.
    pub order: usize,
    /// X window: width in X units (after the modifier, e.g. GHz).
    pub x_width: f64,
    /// Number of passes (repeating smooths more).
    pub passes: usize,
    /// Keep the raw curves (faded) next to the smoothed ones.
    pub keep_original: bool,
}

impl Default for SmoothConfig {
    fn default() -> Self {
        SmoothConfig {
            enabled: false,
            method: SmoothMethod::SavitzkyGolay,
            window: 11,
            sigma: 2.0,
            order: 2,
            x_width: 0.5,
            passes: 1,
            keep_original: true,
        }
    }
}

impl SmoothConfig {
    /// Short description for the legend ("SG 11 pts, order 2").
    pub fn describe(&self) -> String {
        let base = match self.method {
            SmoothMethod::Gaussian => format!("gauss σ={}", self.sigma),
            SmoothMethod::SavitzkyGolay => format!("SG {} pts, ord {}", odd(self.window), self.order),
            SmoothMethod::XWindow => format!("X avg {}", self.x_width),
            m => format!("{} {} pts", m.short(), odd(self.window)),
        };
        if self.passes > 1 {
            format!("{base} ×{}", self.passes)
        } else {
            base
        }
    }

    /// Smooths the points of one segment (X kept, Y replaced).
    pub fn apply(&self, pts: &[[f64; 2]]) -> Vec<[f64; 2]> {
        let mut y: Vec<f64> = pts.iter().map(|p| p[1]).collect();
        let x: Vec<f64> = pts.iter().map(|p| p[0]).collect();
        for _ in 0..self.passes.clamp(1, 10) {
            y = match self.method {
                SmoothMethod::MovingAverage => moving_average(&y, odd(self.window)),
                SmoothMethod::Gaussian => gaussian(&y, self.sigma),
                SmoothMethod::SavitzkyGolay => savitzky_golay(&y, odd(self.window), self.order),
                SmoothMethod::Median => median(&y, odd(self.window)),
                SmoothMethod::Exponential => exponential(&y, odd(self.window)),
                SmoothMethod::XWindow => x_window(&x, &y, self.x_width),
            };
        }
        x.into_iter().zip(y).map(|(x, y)| [x, y]).collect()
    }
}

/// Nearest odd window ≥ 1.
fn odd(n: usize) -> usize {
    let n = n.max(1);
    if n.is_multiple_of(2) { n + 1 } else { n }
}

/// Half-width of the centered window at `i`: reduced near the ends so the
/// window stays symmetric.
#[inline]
fn half(i: usize, n: usize, h: usize) -> usize {
    h.min(i).min(n - 1 - i)
}

/// Mean over a centered window that shrinks at the ends (prefix sums: O(n)).
pub fn moving_average(y: &[f64], window: usize) -> Vec<f64> {
    let n = y.len();
    let h = window / 2;
    let mut prefix = Vec::with_capacity(n + 1);
    prefix.push(0.0);
    let mut acc = 0.0;
    for v in y {
        acc += v;
        prefix.push(acc);
    }
    (0..n)
        .map(|i| {
            let k = half(i, n, h);
            (prefix[i + k + 1] - prefix[i - k]) / (2 * k + 1) as f64
        })
        .collect()
}

pub fn gaussian(y: &[f64], sigma: f64) -> Vec<f64> {
    let sigma = sigma.clamp(0.1, 1000.0);
    let h = (3.0 * sigma).ceil() as usize;
    let w: Vec<f64> = (0..=h)
        .map(|k| (-(k as f64).powi(2) / (2.0 * sigma * sigma)).exp())
        .collect();
    let n = y.len();
    (0..n)
        .map(|i| {
            let mut s = 0.0;
            let mut ws = 0.0;
            let k = half(i, n, h);
            for (j, v) in y.iter().enumerate().take(i + k + 1).skip(i - k) {
                let wk = w[i.abs_diff(j)];
                s += wk * v;
                ws += wk;
            }
            s / ws
        })
        .collect()
}

pub fn median(y: &[f64], window: usize) -> Vec<f64> {
    let n = y.len();
    let h = window / 2;
    let mut buf = Vec::with_capacity(window);
    (0..n)
        .map(|i| {
            buf.clear();
            let k = half(i, n, h);
            buf.extend_from_slice(&y[i - k..=i + k]);
            buf.sort_unstable_by(|a, b| a.total_cmp(b));
            let m = buf.len();
            if m % 2 == 1 {
                buf[m / 2]
            } else {
                (buf[m / 2 - 1] + buf[m / 2]) * 0.5
            }
        })
        .collect()
}

/// Exponential smoothing with α = 2 / (span + 1), forward then backward.
pub fn exponential(y: &[f64], span: usize) -> Vec<f64> {
    let n = y.len();
    if n == 0 {
        return Vec::new();
    }
    let a = 2.0 / (span.max(1) as f64 + 1.0);
    let mut f = vec![0.0; n];
    f[0] = y[0];
    for i in 1..n {
        f[i] = a * y[i] + (1.0 - a) * f[i - 1];
    }
    let mut b = vec![0.0; n];
    b[n - 1] = f[n - 1];
    for i in (0..n - 1).rev() {
        b[i] = a * f[i] + (1.0 - a) * b[i + 1];
    }
    b
}

/// Mean of the points with |x - x_i| ≤ width / 2 (X ascending: two pointers;
/// otherwise a direct scan).
pub fn x_window(x: &[f64], y: &[f64], width: f64) -> Vec<f64> {
    let n = y.len();
    let half = width.abs() * 0.5;
    let ascending = x.windows(2).all(|w| w[0] <= w[1]);
    if !ascending {
        return (0..n)
            .map(|i| {
                let (s, c) = (0..n)
                    .filter(|&j| (x[j] - x[i]).abs() <= half)
                    .fold((0.0, 0usize), |(s, c), j| (s + y[j], c + 1));
                s / c.max(1) as f64
            })
            .collect();
    }
    let (mut lo, mut hi) = (0usize, 0usize);
    let mut sum = 0.0;
    (0..n)
        .map(|i| {
            while hi < n && x[hi] <= x[i] + half {
                sum += y[hi];
                hi += 1;
            }
            while x[lo] < x[i] - half {
                sum -= y[lo];
                lo += 1;
            }
            sum / (hi - lo) as f64
        })
        .collect()
}

/// Least-squares weights giving the value at `t` of the polynomial of degree
/// `order` fitted on the points at offsets `offsets`.
fn poly_weights(offsets: &[f64], order: usize, t: f64) -> Vec<f64> {
    let p = order + 1;
    // normal matrix AᵀA (p × p) and the right-hand side e(t)
    let mut m = vec![vec![0.0; p + 1]; p];
    for (r, row) in m.iter_mut().enumerate() {
        for (c, cell) in row.iter_mut().enumerate().take(p) {
            *cell = offsets.iter().map(|o| o.powi((r + c) as i32)).sum();
        }
        row[p] = t.powi(r as i32);
    }
    // solve (AᵀA) z = e(t) (Gauss-Jordan with partial pivoting)
    for col in 0..p {
        let piv = (col..p)
            .max_by(|&a, &b| m[a][col].abs().total_cmp(&m[b][col].abs()))
            .unwrap_or(col);
        m.swap(col, piv);
        let d = m[col][col];
        if d.abs() < 1e-300 {
            continue;
        }
        for v in &mut m[col][col..=p] {
            *v /= d;
        }
        let pivot = m[col].clone();
        for (r, row) in m.iter_mut().enumerate() {
            let f = row[col];
            if r != col && f != 0.0 {
                for (v, pv) in row[col..=p].iter_mut().zip(&pivot[col..=p]) {
                    *v -= f * pv;
                }
            }
        }
    }
    let z: Vec<f64> = (0..p).map(|r| m[r][p]).collect();
    // weight of each point: a_kᵀ z
    offsets
        .iter()
        .map(|o| (0..p).map(|j| o.powi(j as i32) * z[j]).sum())
        .collect()
}

pub fn savitzky_golay(y: &[f64], window: usize, order: usize) -> Vec<f64> {
    let n = y.len();
    let window = window.min(if n % 2 == 1 { n } else { n.saturating_sub(1) }).max(1);
    let order = order.min(window.saturating_sub(1)).min(6);
    if window <= order + 1 || n < 3 {
        return y.to_vec();
    }
    let h = window / 2;
    let offsets: Vec<f64> = (0..window).map(|k| k as f64 - h as f64).collect();
    let center = poly_weights(&offsets, order, 0.0);
    let mut out = vec![0.0; n];
    for i in h..n - h {
        out[i] = center.iter().zip(&y[i - h..=i + h]).map(|(w, v)| w * v).sum();
    }
    // ends: the polynomial of the first / last window, evaluated at the point
    for i in (0..h).chain(n - h..n) {
        let start = if i < h { 0 } else { n - window };
        let t = i as f64 - (start + h) as f64;
        let w = poly_weights(&offsets, order, t);
        out[i] = w.iter().zip(&y[start..start + window]).map(|(w, v)| w * v).sum();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: &[f64], b: &[f64], tol: f64) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() <= tol)
    }

    #[test]
    fn constants_and_lines_are_kept() {
        let flat = vec![3.0; 50];
        let line: Vec<f64> = (0..50).map(|i| 2.0 * i as f64 - 7.0).collect();
        for m in SmoothMethod::ALL {
            let cfg = SmoothConfig {
                method: m,
                window: 9,
                ..Default::default()
            };
            let pts: Vec<[f64; 2]> = flat.iter().enumerate().map(|(i, &v)| [i as f64, v]).collect();
            let out: Vec<f64> = cfg.apply(&pts).iter().map(|p| p[1]).collect();
            assert!(close(&out, &flat, 1e-9), "{m:?}");
        }
        // Savitzky–Golay (order ≥ 1) and the median keep a straight line exactly,
        // including the ends
        assert!(close(&savitzky_golay(&line, 9, 2), &line, 1e-9));
        assert!(close(&savitzky_golay(&line, 7, 1), &line, 1e-9));
        assert!(close(&median(&line, 5), &line, 1e-9));
        // symmetric windows: the moving average and the Gaussian keep it too
        assert!(close(&moving_average(&line, 5), &line, 1e-9));
        assert!(close(&gaussian(&line, 2.0), &line, 1e-9));
    }

    #[test]
    fn savitzky_golay_keeps_parabolas() {
        let p: Vec<f64> = (0..40).map(|i| 0.1 * (i as f64 - 20.0).powi(2) + 1.0).collect();
        assert!(close(&savitzky_golay(&p, 11, 2), &p, 1e-8));
        // standard 5-point quadratic coefficients: (-3, 12, 17, 12, -3) / 35
        let w = poly_weights(&[-2.0, -1.0, 0.0, 1.0, 2.0], 2, 0.0);
        let expected = [-3.0, 12.0, 17.0, 12.0, -3.0].map(|v| v / 35.0);
        assert!(close(&w, &expected, 1e-12));
    }

    #[test]
    fn noise_is_reduced() {
        let mut x: u64 = 0x2545_F491_4F6C_DD1D;
        let clean: Vec<f64> = (0..2000).map(|i| (i as f64 * 0.01).sin()).collect();
        let noisy: Vec<f64> = clean
            .iter()
            .map(|v| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                v + ((x % 1000) as f64 / 1000.0 - 0.5) * 0.2
            })
            .collect();
        let err = |s: &[f64]| s.iter().zip(&clean).map(|(a, b)| (a - b).powi(2)).sum::<f64>();
        let e0 = err(&noisy);
        assert!(err(&moving_average(&noisy, 15)) < e0 * 0.2);
        assert!(err(&gaussian(&noisy, 4.0)) < e0 * 0.2);
        assert!(err(&savitzky_golay(&noisy, 31, 3)) < e0 * 0.2);
        assert!(err(&median(&noisy, 15)) < e0 * 0.3);
        assert!(err(&exponential(&noisy, 15)) < e0 * 0.2);
        let xs: Vec<f64> = (0..2000).map(|i| i as f64 * 0.01).collect();
        assert!(err(&x_window(&xs, &noisy, 0.15)) < e0 * 0.2);
    }

    #[test]
    fn spikes_and_edges() {
        let mut y = vec![1.0; 21];
        y[10] = 100.0;
        assert_eq!(median(&y, 5)[10], 1.0);
        // tiny curves do not panic
        for n in 0..4 {
            let v: Vec<f64> = (0..n).map(|i| i as f64).collect();
            assert_eq!(savitzky_golay(&v, 11, 3).len(), n);
            assert_eq!(moving_average(&v, 11).len(), n);
            assert_eq!(gaussian(&v, 3.0).len(), n);
            assert_eq!(median(&v, 11).len(), n);
            assert_eq!(exponential(&v, 11).len(), n);
            assert_eq!(x_window(&v, &v, 1.0).len(), n);
        }
        // X window with irregular steps
        let xs = [0.0, 0.1, 0.2, 5.0, 5.1];
        let ys = [1.0, 2.0, 3.0, 10.0, 20.0];
        assert_eq!(x_window(&xs, &ys, 0.25), vec![1.5, 2.0, 2.5, 15.0, 15.0]);
    }
}
