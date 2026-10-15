//! Smoothing of the curves (Data → Process Data → Smoothing).
//!
//! Every method works on the points of a curve segment in their order
//! (ascending X for sorted curves); the windows shrink at the ends of the
//! curve (symmetrically, so the ends are not biased) instead of inventing
//! values.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SmoothMethod {
    /// Penalized cubic spline: a smooth continuous curve (best with few points).
    #[default]
    Spline,
    /// Local weighted regression (LOWESS / LOESS).
    Loess,
    /// Mean of the `window` neighbouring points.
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
    pub const ALL: [SmoothMethod; 8] = [
        SmoothMethod::Spline,
        SmoothMethod::Loess,
        SmoothMethod::MovingAverage,
        SmoothMethod::Gaussian,
        SmoothMethod::SavitzkyGolay,
        SmoothMethod::Median,
        SmoothMethod::Exponential,
        SmoothMethod::XWindow,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SmoothMethod::Spline => "Smoothing spline (recommended)",
            SmoothMethod::Loess => "LOESS (local regression)",
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
            SmoothMethod::Spline => "spline",
            SmoothMethod::Loess => "loess",
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
            SmoothMethod::Spline => {
                "Smooth continuous curve that follows the trend of the points (penalized cubic spline). \
                 Works well even with very few points; one setting: the smoothness."
            }
            SmoothMethod::Loess => {
                "Local weighted regression on a fraction (span) of the points around each X: follows \
                 slow trends; the robust option ignores outliers."
            }
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
    /// Spline: smoothness, 0 (follows the points) … 1 (almost a straight line).
    pub smoothness: f64,
    /// LOESS: fraction of the points used around each X.
    pub span: f64,
    /// LOESS: local polynomial degree (1 or 2).
    pub degree: usize,
    /// LOESS: robust fit (outliers ignored).
    pub robust: bool,
    /// Draw a smooth curve between the points (dense resampling) instead of
    /// straight segments: the result looks smooth even with few points.
    pub interpolate: bool,
    /// Number of points of the smooth curve.
    pub display_points: usize,
    /// Keep the raw curves (faded) next to the smoothed ones.
    pub keep_original: bool,
}

impl Default for SmoothConfig {
    fn default() -> Self {
        SmoothConfig {
            enabled: false,
            method: SmoothMethod::Spline,
            window: 11,
            sigma: 2.0,
            order: 2,
            x_width: 0.5,
            passes: 1,
            smoothness: 0.5,
            span: 0.3,
            degree: 2,
            robust: false,
            interpolate: true,
            display_points: 400,
            keep_original: true,
        }
    }
}

impl SmoothConfig {
    /// Short description for the legend ("SG 11 pts, order 2").
    pub fn describe(&self) -> String {
        let base = match self.method {
            SmoothMethod::Spline => format!("spline {:.0} %", self.smoothness * 100.0),
            SmoothMethod::Loess => format!(
                "loess {:.0} %{}",
                self.span * 100.0,
                if self.robust { ", robust" } else { "" }
            ),
            SmoothMethod::Gaussian => format!("gauss σ={}", self.sigma),
            SmoothMethod::SavitzkyGolay => format!("SG {} pts, ord {}", odd(self.window), self.order),
            SmoothMethod::XWindow => format!("X avg {}", self.x_width),
            m => format!("{} {} pts", m.short(), odd(self.window)),
        };
        if self.passes > 1 && !self.is_curve_method() {
            format!("{base} ×{}", self.passes)
        } else {
            base
        }
    }

    /// Spline / LOESS: they build a continuous curve (no passes).
    pub fn is_curve_method(&self) -> bool {
        matches!(self.method, SmoothMethod::Spline | SmoothMethod::Loess)
    }

    /// Smooths the points of one segment. With `interpolate`, a curve sorted
    /// by X is returned as a dense smooth curve (`display_points` points).
    pub fn apply(&self, pts: &[[f64; 2]]) -> Vec<[f64; 2]> {
        let ascending = pts.windows(2).all(|w| w[0][0] <= w[1][0]);
        if self.is_curve_method() {
            // spline / LOESS work on X: sort the points if needed
            let mut sorted = pts.to_vec();
            if !ascending {
                sorted.sort_by(|a, b| a[0].total_cmp(&b[0]));
            }
            let (x, y, w) = merge_equal_x(&sorted);
            if x.len() < 3 {
                return sorted;
            }
            let grid = self.output_grid(&x);
            let values = match self.method {
                SmoothMethod::Spline => smoothing_spline(&x, &y, &w, self.smoothness, &grid),
                _ => {
                    // LOESS evaluated at the points (on at most 2000 of them for
                    // big curves), then a smooth curve through the fitted values:
                    // no kinks where the neighbourhood changes
                    let n = x.len();
                    let step = n.div_ceil(2000).max(1);
                    let mut idx: Vec<usize> = (0..n).step_by(step).collect();
                    if idx.last() != Some(&(n - 1)) {
                        idx.push(n - 1);
                    }
                    let kx: Vec<f64> = idx.iter().map(|&i| x[i]).collect();
                    let ky = loess(&x, &y, self.span, self.degree, self.robust, &kx);
                    if kx.len() >= 3 { pchip(&kx, &ky, &grid) } else { ky }
                }
            };
            return values.into_iter().zip(grid).map(|(y, x)| [x, y]).collect();
        }
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
                SmoothMethod::Spline | SmoothMethod::Loess => unreachable!(),
            };
        }
        // smooth display between the (smoothed) points: monotone cubic
        // interpolation, which never overshoots (steps stay steps)
        if self.interpolate && ascending && x.len() >= 3 && x.len() < self.display_points {
            let pairs: Vec<[f64; 2]> = x.iter().zip(&y).map(|(&a, &b)| [a, b]).collect();
            let (ux, uy, _) = merge_equal_x(&pairs);
            if ux.len() >= 3 {
                let grid = self.output_grid(&ux);
                return pchip(&ux, &uy, &grid)
                    .into_iter()
                    .zip(grid)
                    .map(|(y, x)| [x, y])
                    .collect();
            }
        }
        x.into_iter().zip(y).map(|(x, y)| [x, y]).collect()
    }

    /// Smoothed values at the points themselves (data export): `x` ascending,
    /// one value per point, so the exported rows keep their X values. Points
    /// with a NaN are ignored by the fit and stay NaN.
    pub fn values_at(&self, x: &[f64], y: &[f64]) -> Vec<f64> {
        let idx: Vec<usize> = (0..x.len().min(y.len()))
            .filter(|&i| x[i].is_finite() && y[i].is_finite())
            .collect();
        let mut out = vec![f64::NAN; y.len()];
        if idx.len() < 3 {
            for &i in &idx {
                out[i] = y[i];
            }
            return out;
        }
        let fx: Vec<f64> = idx.iter().map(|&i| x[i]).collect();
        let fy: Vec<f64> = idx.iter().map(|&i| y[i]).collect();
        let values = if self.is_curve_method() {
            let pairs: Vec<[f64; 2]> = fx.iter().zip(&fy).map(|(&a, &b)| [a, b]).collect();
            let (ux, uy, w) = merge_equal_x(&pairs);
            if ux.len() < 3 {
                fy.clone()
            } else if self.method == SmoothMethod::Spline {
                smoothing_spline(&ux, &uy, &w, self.smoothness, &fx)
            } else {
                let n = ux.len();
                let step = n.div_ceil(2000).max(1);
                let mut knots: Vec<usize> = (0..n).step_by(step).collect();
                if knots.last() != Some(&(n - 1)) {
                    knots.push(n - 1);
                }
                let kx: Vec<f64> = knots.iter().map(|&i| ux[i]).collect();
                let ky = loess(&ux, &uy, self.span, self.degree, self.robust, &kx);
                if kx.len() >= 3 {
                    pchip(&kx, &ky, &fx)
                } else {
                    fy.clone()
                }
            }
        } else {
            let mut plain = self.clone();
            plain.interpolate = false;
            let pts: Vec<[f64; 2]> = fx.iter().zip(&fy).map(|(&a, &b)| [a, b]).collect();
            plain.apply(&pts).into_iter().map(|p| p[1]).collect()
        };
        for (k, &i) in idx.iter().enumerate() {
            out[i] = values.get(k).copied().unwrap_or(f64::NAN);
        }
        out
    }

    /// X values of the output: a dense regular grid (smooth display) or the
    /// X values of the points.
    fn output_grid(&self, x: &[f64]) -> Vec<f64> {
        let n = x.len();
        let m = self.display_points.clamp(50, 5000);
        if !self.interpolate || n >= m {
            return x.to_vec();
        }
        let (a, b) = (x[0], x[n - 1]);
        (0..m).map(|k| a + (b - a) * k as f64 / (m - 1) as f64).collect()
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

/// Points sorted by X → distinct X, mean Y and number of points (weight) of
/// each X.
fn merge_equal_x(pts: &[[f64; 2]]) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    let mut x: Vec<f64> = Vec::with_capacity(pts.len());
    let mut y: Vec<f64> = Vec::with_capacity(pts.len());
    let mut w: Vec<f64> = Vec::with_capacity(pts.len());
    for p in pts {
        if !(p[0].is_finite() && p[1].is_finite()) {
            continue;
        }
        match x.last() {
            Some(&lx) if (p[0] - lx).abs() <= 1e-12 * lx.abs().max(1e-300) => {
                let k = y.len() - 1;
                y[k] = (y[k] * w[k] + p[1]) / (w[k] + 1.0);
                w[k] += 1.0;
            }
            _ => {
                x.push(p[0]);
                y.push(p[1]);
                w.push(1.0);
            }
        }
    }
    (x, y, w)
}

/// Solves a symmetric positive definite band system (half bandwidth 2) by
/// Gaussian elimination; `a[i]` = row i, columns i-2 ..= i+2.
fn solve_band5(a: &mut [[f64; 5]], b: &mut [f64]) {
    let m = b.len();
    for i in 0..m {
        let p = a[i][2];
        if p.abs() < 1e-300 {
            continue;
        }
        for r in 1..=2 {
            if i + r >= m {
                break;
            }
            let f = a[i + r][2 - r] / p;
            if f == 0.0 {
                continue;
            }
            // row i+r -= f * row i (columns i ..= i+2)
            for c in 0..=2 {
                if i + c < m && 2 - r + c < 5 {
                    a[i + r][2 - r + c] -= f * a[i][2 + c];
                }
            }
            b[i + r] -= f * b[i];
        }
    }
    for i in (0..m).rev() {
        let mut v = b[i];
        for c in 1..=2 {
            if i + c < m {
                v -= a[i][2 + c] * b[i + c];
            }
        }
        b[i] = if a[i][2].abs() < 1e-300 { 0.0 } else { v / a[i][2] };
    }
}

/// Penalized cubic smoothing spline (Reinsch): minimizes
/// Σ wᵢ (yᵢ − f(xᵢ))² + λ ∫ f''², evaluated at `at`. `smoothness` 0…1 sets
/// the width of the equivalent smoothing window, from 0.2 % to the whole X range.
pub fn smoothing_spline(x: &[f64], y: &[f64], w: &[f64], smoothness: f64, at: &[f64]) -> Vec<f64> {
    let n = x.len();
    if n < 3 {
        return at.iter().map(|_| y.iter().sum::<f64>() / n.max(1) as f64).collect();
    }
    // X normalized to [0, 1]: the setting does not depend on the units
    let (x0, span) = (x[0], (x[n - 1] - x[0]).max(1e-300));
    let t: Vec<f64> = x.iter().map(|v| (v - x0) / span).collect();
    let h: Vec<f64> = t.windows(2).map(|p| (p[1] - p[0]).max(1e-12)).collect();
    let bandwidth = 0.002 * 500f64.powf(smoothness.clamp(0.0, 1.0));
    let total_w: f64 = w.iter().sum();
    let lambda = total_w * bandwidth.powi(4);
    // Q (n × m) has 3 non-zero entries per column j (rows j, j+1, j+2)
    let m = n - 2;
    let q = |j: usize| -> [f64; 3] { [1.0 / h[j], -1.0 / h[j] - 1.0 / h[j + 1], 1.0 / h[j + 1]] };
    let mut a = vec![[0.0f64; 5]; m];
    let mut rhs = vec![0.0; m];
    for j in 0..m {
        let qj = q(j);
        // R (tridiagonal)
        a[j][2] += (h[j] + h[j + 1]) / 3.0;
        if j + 1 < m {
            a[j][3] += h[j + 1] / 6.0;
            a[j + 1][1] += h[j + 1] / 6.0;
        }
        // λ Qᵀ W⁻¹ Q: column j against columns j, j+1, j+2
        for d in 0..=2 {
            if j + d >= m {
                break;
            }
            let qk = q(j + d);
            // common rows of columns j and j+d: rows j+d ..= j+2
            let mut v = 0.0;
            for r in (j + d)..=(j + 2) {
                v += qj[r - j] * qk[r - j - d] / w[r];
            }
            a[j][2 + d] += lambda * v;
            if d > 0 {
                a[j + d][2 - d] += lambda * v;
            }
        }
        rhs[j] = qj[0] * y[j] + qj[1] * y[j + 1] + qj[2] * y[j + 2];
    }
    solve_band5(&mut a, &mut rhs);
    // second derivatives (0 at the ends) and fitted values f = y − λ W⁻¹ Q γ
    let mut g = vec![0.0; n];
    g[1..=m].copy_from_slice(&rhs);
    let mut f = y.to_vec();
    for j in 0..m {
        let qj = q(j);
        for (k, qv) in qj.iter().enumerate() {
            f[j + k] -= lambda * qv * rhs[j] / w[j + k];
        }
    }
    // natural cubic spline through (t, f) with second derivatives g
    at.iter()
        .map(|&v| {
            let u = ((v - x0) / span).clamp(0.0, 1.0);
            let i = t.partition_point(|&tt| tt <= u).clamp(1, n - 1) - 1;
            let hi = h[i];
            let (a, b) = (t[i + 1] - u, u - t[i]);
            g[i] * a.powi(3) / (6.0 * hi)
                + g[i + 1] * b.powi(3) / (6.0 * hi)
                + (f[i] / hi - g[i] * hi / 6.0) * a
                + (f[i + 1] / hi - g[i + 1] * hi / 6.0) * b
        })
        .collect()
}

/// LOESS: at each X of `at`, weighted polynomial fit (degree 1 or 2) on the
/// `span` fraction of nearest points, tricube weights; `robust`: two extra
/// passes down-weighting the outliers (bisquare).
pub fn loess(x: &[f64], y: &[f64], span: f64, degree: usize, robust: bool, at: &[f64]) -> Vec<f64> {
    let n = x.len();
    let degree = degree.clamp(1, 2);
    let k = ((span.clamp(0.02, 1.0) * n as f64).ceil() as usize).clamp(degree + 2, n);
    let mut rw = vec![1.0; n];
    let fit = |z: f64, rw: &[f64]| -> f64 {
        // the k nearest points: window around the insertion point
        let mut lo = x.partition_point(|&v| v < z).min(n - 1);
        let mut hi = lo;
        while hi - lo + 1 < k {
            if lo == 0 {
                hi += 1;
            } else if hi == n - 1 || z - x[lo - 1] <= x[hi + 1] - z {
                lo -= 1;
            } else {
                hi += 1;
            }
        }
        let dmax = (z - x[lo]).abs().max((x[hi] - z).abs()).max(1e-300) * 1.0001;
        // weighted normal equations in (x - z)
        let p = degree + 1;
        let mut m = [[0.0f64; 4]; 3];
        for i in lo..=hi {
            let d = x[i] - z;
            let u = (d.abs() / dmax).min(1.0);
            let wt = (1.0 - u * u * u).powi(3) * rw[i];
            if wt <= 0.0 {
                continue;
            }
            let pw = [1.0, d, d * d, d * d * d, d * d * d * d];
            for r in 0..p {
                for c in 0..p {
                    m[r][c] += wt * pw[r + c];
                }
                m[r][3] += wt * pw[r] * y[i];
            }
        }
        // solve for the constant term (Gauss-Jordan on p × p)
        for col in 0..p {
            let piv = (col..p)
                .max_by(|&a, &b| m[a][col].abs().total_cmp(&m[b][col].abs()))
                .unwrap_or(col);
            m.swap(col, piv);
            let d = m[col][col];
            if d.abs() < 1e-300 {
                continue;
            }
            for v in &mut m[col][col..p] {
                *v /= d;
            }
            m[col][3] /= d;
            let pivot = m[col];
            for (r, row) in m.iter_mut().enumerate().take(p) {
                if r != col {
                    let f = row[col];
                    for c in col..p {
                        row[c] -= f * pivot[c];
                    }
                    row[3] -= f * pivot[3];
                }
            }
        }
        let v = m[0][3];
        if v.is_finite() {
            v
        } else {
            // degenerate neighbourhood: weighted mean
            let (s, c) = (lo..=hi).fold((0.0, 0.0), |(s, c), i| (s + y[i] * rw[i], c + rw[i]));
            s / c.max(1e-300)
        }
    };
    if robust && (n as f64) * (k as f64) <= 5e7 {
        for _ in 0..2 {
            let res: Vec<f64> = (0..n).map(|i| y[i] - fit(x[i], &rw)).collect();
            let mut abs: Vec<f64> = res.iter().map(|r| r.abs()).collect();
            abs.sort_by(|a, b| a.total_cmp(b));
            let s = 6.0 * abs[n / 2].max(1e-300);
            for i in 0..n {
                let u = (res[i] / s).abs();
                rw[i] = if u < 1.0 { (1.0 - u * u).powi(2) } else { 0.0 };
            }
        }
    }
    at.iter().map(|&z| fit(z, &rw)).collect()
}

/// Monotone cubic interpolation (Fritsch–Carlson / PCHIP): smooth curve
/// through the points without overshoot.
pub fn pchip(x: &[f64], y: &[f64], at: &[f64]) -> Vec<f64> {
    let n = x.len();
    let h: Vec<f64> = x.windows(2).map(|p| p[1] - p[0]).collect();
    let delta: Vec<f64> = (0..n - 1).map(|i| (y[i + 1] - y[i]) / h[i]).collect();
    let mut d = vec![0.0; n];
    for i in 1..n - 1 {
        if delta[i - 1] * delta[i] > 0.0 {
            let w1 = 2.0 * h[i] + h[i - 1];
            let w2 = h[i] + 2.0 * h[i - 1];
            d[i] = (w1 + w2) / (w1 / delta[i - 1] + w2 / delta[i]);
        }
    }
    // ends: one-sided shape-preserving estimates
    let end = |h0: f64, h1: f64, d0: f64, d1: f64| -> f64 {
        let v = ((2.0 * h0 + h1) * d0 - h0 * d1) / (h0 + h1);
        if v * d0 <= 0.0 {
            0.0
        } else if d0 * d1 <= 0.0 && v.abs() > 3.0 * d0.abs() {
            3.0 * d0
        } else {
            v
        }
    };
    if n >= 3 {
        d[0] = end(h[0], h[1], delta[0], delta[1]);
        d[n - 1] = end(h[n - 2], h[n - 3], delta[n - 2], delta[n - 3]);
    } else {
        d[0] = delta[0];
        d[n - 1] = delta[0];
    }
    at.iter()
        .map(|&v| {
            let i = x.partition_point(|&xx| xx <= v).clamp(1, n - 1) - 1;
            let t = ((v - x[i]) / h[i]).clamp(0.0, 1.0);
            let (t2, t3) = (t * t, t * t * t);
            (2.0 * t3 - 3.0 * t2 + 1.0) * y[i]
                + (t3 - 2.0 * t2 + t) * h[i] * d[i]
                + (-2.0 * t3 + 3.0 * t2) * y[i + 1]
                + (t3 - t2) * h[i] * d[i + 1]
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
                interpolate: false,
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
    fn spline_and_loess() {
        let xs: Vec<f64> = (0..30).map(|i| i as f64 * 0.7 + 35.0).collect();
        let line: Vec<f64> = xs.iter().map(|x| 2.0 * x - 3.0).collect();
        let w = vec![1.0; xs.len()];
        // straight lines are kept exactly, at any smoothness
        for s in [0.0, 0.5, 1.0] {
            let f = smoothing_spline(&xs, &line, &w, s, &xs);
            assert!(close(&f, &line, 1e-6), "spline {s}");
        }
        assert!(close(&loess(&xs, &line, 0.3, 1, false, &xs), &line, 1e-6));
        assert!(close(&loess(&xs, &line, 0.5, 2, true, &xs), &line, 1e-6));
        // almost no smoothing = goes through the points
        let wavy: Vec<f64> = xs.iter().map(|x| (x * 0.9).sin()).collect();
        let f = smoothing_spline(&xs, &wavy, &w, 0.0, &xs);
        assert!(close(&f, &wavy, 2e-3));
        // strong smoothing ≈ least-squares straight line
        let f = smoothing_spline(&xs, &wavy, &w, 1.0, &xs);
        let slope = (f[29] - f[0]) / (xs[29] - xs[0]);
        let mid = (f[14] + f[15]) / 2.0 - (f[0] + slope * (xs[14] + xs[15]) / 2.0 - slope * xs[0]);
        assert!(mid.abs() < 0.05, "{mid}");
    }

    #[test]
    fn few_noisy_points_give_a_smooth_curve() {
        // 15 noisy points of a parabola: the result is dense and close to it
        let xs: Vec<f64> = (0..15).map(|i| i as f64).collect();
        let truth = |x: f64| 0.05 * (x - 7.0).powi(2);
        let noise = [
            0.3, -0.2, 0.25, -0.3, 0.1, -0.15, 0.2, -0.25, 0.3, -0.1, 0.15, -0.3, 0.2, -0.2, 0.1,
        ];
        let pts: Vec<[f64; 2]> = xs.iter().zip(noise).map(|(&x, e)| [x, truth(x) + e]).collect();
        for method in [SmoothMethod::Spline, SmoothMethod::Loess, SmoothMethod::SavitzkyGolay] {
            let cfg = SmoothConfig {
                method,
                window: 7,
                smoothness: 0.6,
                span: 0.6,
                ..Default::default()
            };
            let out = cfg.apply(&pts);
            assert_eq!(out.len(), 400, "{method:?}: dense smooth display");
            let err = out.iter().map(|p| (p[1] - truth(p[0])).powi(2)).sum::<f64>() / out.len() as f64;
            let raw = pts.iter().map(|p| (p[1] - truth(p[0])).powi(2)).sum::<f64>() / 15.0;
            assert!(err < raw * 0.6, "{method:?}: {err} vs {raw}");
            // no corners: small second differences
            let max_d2 = out
                .windows(3)
                .map(|w| (w[0][1] - 2.0 * w[1][1] + w[2][1]).abs())
                .fold(0.0, f64::max);
            assert!(max_d2 < 0.01, "{method:?}: {max_d2}");
        }
        // duplicated X and unsorted points do not break the spline
        let mut dup = pts.clone();
        dup.push([3.0, 0.9]);
        dup.reverse();
        let out = SmoothConfig::default().apply(&dup);
        assert!(out.iter().all(|p| p[1].is_finite()));
        assert!(out.windows(2).all(|w| w[0][0] <= w[1][0]));
    }

    #[test]
    fn pchip_does_not_overshoot() {
        let x = [0.0, 1.0, 2.0, 3.0, 4.0];
        let y = [0.0, 0.0, 1.0, 1.0, 1.0];
        let at: Vec<f64> = (0..=40).map(|i| i as f64 * 0.1).collect();
        let v = pchip(&x, &y, &at);
        assert!(v.iter().all(|&a| (-1e-12..=1.0 + 1e-12).contains(&a)));
        assert!((v[20] - 1.0).abs() < 1e-12 && v[10].abs() < 1e-12);
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
