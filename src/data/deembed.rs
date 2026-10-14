//! De-embedding of the input / output fixtures (cables, test board, probes…)
//! described by Touchstone `.s2p` files.
//!
//! * **Vector**: when the measurement has the magnitude *and* the phase of the
//!   four S-parameters (`S11db`, `S11deg`, `S21db`, … `S22deg`) and the
//!   fixtures have phase information, the DUT is extracted with transfer
//!   (T) matrices: `T_dut = T_in⁻¹ · T_meas · T_out⁻¹`.
//! * **Scalar**: otherwise the fixture losses (dB) are subtracted, assuming
//!   matched fixtures: `S21 − IL_in − IL_out`, `S11 − 2·IL_in`, `S22 − 2·IL_out`.
//!
//! Other columns can be corrected with the fixture losses (input / output
//! power, gain, noise figure).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::dataset::{Column, Dataset, format_sig};
use super::hash::FxHashMap;

// =============================================================================
// Complex numbers
// =============================================================================

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct C {
    pub re: f64,
    pub im: f64,
}

impl C {
    pub const ONE: C = C { re: 1.0, im: 0.0 };

    pub fn new(re: f64, im: f64) -> C {
        C { re, im }
    }

    /// From a magnitude in dB (20·log10) and a phase in degrees.
    pub fn from_db_deg(db: f64, deg: f64) -> C {
        let m = 10f64.powf(db / 20.0);
        let a = deg.to_radians();
        C::new(m * a.cos(), m * a.sin())
    }

    pub fn abs(self) -> f64 {
        self.re.hypot(self.im)
    }

    pub fn db(self) -> f64 {
        20.0 * self.abs().log10()
    }

    pub fn deg(self) -> f64 {
        self.im.atan2(self.re).to_degrees()
    }

    pub fn inv(self) -> C {
        let d = self.re * self.re + self.im * self.im;
        C::new(self.re / d, -self.im / d)
    }

    pub fn neg(self) -> C {
        C::new(-self.re, -self.im)
    }
}

impl std::ops::Add for C {
    type Output = C;
    fn add(self, o: C) -> C {
        C::new(self.re + o.re, self.im + o.im)
    }
}

impl std::ops::Sub for C {
    type Output = C;
    fn sub(self, o: C) -> C {
        C::new(self.re - o.re, self.im - o.im)
    }
}

impl std::ops::Mul for C {
    type Output = C;
    fn mul(self, o: C) -> C {
        C::new(self.re * o.re - self.im * o.im, self.re * o.im + self.im * o.re)
    }
}

/// Two-port S-parameters, in the order S11, S21, S12, S22.
pub type S2 = [C; 4];
/// 2×2 matrix [[a, b], [c, d]].
type M2 = [C; 4];

/// S → T, with (b1, a1) = T · (a2, b2): cascading = matrix product.
fn s_to_t(s: &S2) -> Option<M2> {
    let [s11, s21, s12, s22] = *s;
    if s21.abs() < 1e-30 {
        return None;
    }
    let k = s21.inv();
    let det = s11 * s22 - s12 * s21;
    Some([det.neg() * k, s11 * k, s22.neg() * k, k])
}

fn t_to_s(t: &M2) -> Option<S2> {
    let [t11, t12, t21, t22] = *t;
    if t22.abs() < 1e-30 {
        return None;
    }
    let k = t22.inv();
    let det = t11 * t22 - t12 * t21;
    Some([t12 * k, k, det * k, t21.neg() * k])
}

fn mul(a: &M2, b: &M2) -> M2 {
    [
        a[0] * b[0] + a[1] * b[2],
        a[0] * b[1] + a[1] * b[3],
        a[2] * b[0] + a[3] * b[2],
        a[2] * b[1] + a[3] * b[3],
    ]
}

fn inv(m: &M2) -> Option<M2> {
    let det = m[0] * m[3] - m[1] * m[2];
    if det.abs() < 1e-30 {
        return None;
    }
    let k = det.inv();
    Some([m[3] * k, m[1].neg() * k, m[2].neg() * k, m[0] * k])
}

/// S-parameters of two cascaded two-ports.
#[cfg(test)]
pub fn cascade(a: &S2, b: &S2) -> Option<S2> {
    t_to_s(&mul(&s_to_t(a)?, &s_to_t(b)?))
}

/// Swaps the ports of a two-port.
#[cfg(test)]
pub fn flip(s: &S2) -> S2 {
    [s[3], s[2], s[1], s[0]]
}

// =============================================================================
// Touchstone files
// =============================================================================

/// A two-port Touchstone file, resampled for interpolation.
#[derive(Clone, Debug)]
pub struct Touchstone {
    pub name: String,
    /// Frequencies (Hz), increasing.
    pub freq: Vec<f64>,
    /// Magnitude (dB) of S11, S21, S12, S22 at every frequency.
    pub db: Vec<[f64; 4]>,
    /// Unwrapped phase (degrees).
    pub deg: Vec<[f64; 4]>,
    /// False when every phase of the file is 0 (scalar loss file).
    pub has_phase: bool,
}

impl Touchstone {
    pub fn load(path: &Path) -> Result<Touchstone, String> {
        let text = std::fs::read(path).map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        Touchstone::parse(&String::from_utf8_lossy(&text), &name).map_err(|e| format!("{name}: {e}"))
    }

    /// Parses a two-port Touchstone file (version 1 or 2).
    pub fn parse(text: &str, name: &str) -> Result<Touchstone, String> {
        let mut freq_mult = 1e9;
        let mut format = "MA".to_owned();
        let mut option_seen = false;
        let mut order_12_21 = false;
        let mut in_data = true;
        let mut numbers: Vec<f64> = Vec::new();
        for (ln, raw) in text.lines().enumerate() {
            let line = raw.split('!').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            if let Some(opt) = line.strip_prefix('#') {
                if option_seen {
                    continue;
                }
                option_seen = true;
                let toks: Vec<String> = opt.split_whitespace().map(|t| t.to_ascii_uppercase()).collect();
                let mut k = 0;
                while k < toks.len() {
                    match toks[k].as_str() {
                        "HZ" => freq_mult = 1.0,
                        "KHZ" => freq_mult = 1e3,
                        "MHZ" => freq_mult = 1e6,
                        "GHZ" => freq_mult = 1e9,
                        "S" => {}
                        "Y" | "Z" | "H" | "G" => {
                            return Err(format!("{}-parameters are not supported (S-parameters only)", toks[k]));
                        }
                        "MA" | "DB" | "RI" => format = toks[k].clone(),
                        "R" => k += 1,
                        _ => {}
                    }
                    k += 1;
                }
                continue;
            }
            if line.starts_with('[') {
                let low = line.to_ascii_lowercase();
                if low.starts_with("[number of ports]") {
                    let n = low.trim_start_matches("[number of ports]").trim();
                    if n != "2" {
                        return Err(format!("{n}-port file: a two-port (.s2p) file is expected"));
                    }
                } else if low.starts_with("[two-port data order]") {
                    order_12_21 = low.contains("12_21");
                } else if low.starts_with("[network data]") {
                    in_data = true;
                } else if low.starts_with("[end]") {
                    break;
                } else {
                    // other keywords ([Version], [Reference], [Noise Data]…)
                    in_data = !low.starts_with("[noise data]");
                }
                continue;
            }
            if !in_data {
                continue;
            }
            for tok in line.split_whitespace() {
                match tok.parse::<f64>() {
                    Ok(v) => numbers.push(v),
                    Err(_) => return Err(format!("line {}: invalid number '{tok}'", ln + 1)),
                }
            }
        }
        if numbers.len() < 9 {
            return Err("no two-port data found".to_owned());
        }
        if !numbers.len().is_multiple_of(9) {
            return Err(format!(
                "{} values: not a two-port file (9 values per frequency expected)",
                numbers.len()
            ));
        }
        let mut points: Vec<(f64, S2)> = Vec::with_capacity(numbers.len() / 9);
        for rec in numbers.chunks_exact(9) {
            let f = rec[0] * freq_mult;
            let mut s = [C::ONE; 4];
            for (k, s) in s.iter_mut().enumerate() {
                let (a, b) = (rec[1 + 2 * k], rec[2 + 2 * k]);
                *s = match format.as_str() {
                    "RI" => C::new(a, b),
                    "DB" => C::from_db_deg(a, b),
                    _ => {
                        let r = b.to_radians();
                        C::new(a * r.cos(), a * r.sin())
                    }
                };
            }
            if order_12_21 {
                s.swap(1, 2);
            }
            // noise parameters of v1 files follow with decreasing frequencies
            if points.last().is_some_and(|(pf, _)| f <= *pf) {
                break;
            }
            points.push((f, s));
        }
        Ok(Touchstone::from_points(name, &points))
    }

    pub fn from_points(name: &str, points: &[(f64, S2)]) -> Touchstone {
        let mut freq = Vec::with_capacity(points.len());
        let mut db: Vec<[f64; 4]> = Vec::with_capacity(points.len());
        let mut deg: Vec<[f64; 4]> = Vec::with_capacity(points.len());
        let mut has_phase = false;
        for (f, s) in points {
            freq.push(*f);
            db.push(std::array::from_fn(|k| s[k].db().max(-400.0)));
            let mut d: [f64; 4] = std::array::from_fn(|k| s[k].deg());
            if d.iter().any(|v| v.abs() > 1e-9) {
                has_phase = true;
            }
            // unwrap against the previous point
            if let Some(prev) = deg.last() {
                for k in 0..4 {
                    d[k] += 360.0 * ((prev[k] - d[k]) / 360.0).round();
                }
            }
            deg.push(d);
        }
        Touchstone {
            name: name.to_owned(),
            freq,
            db,
            deg,
            has_phase,
        }
    }

    pub fn f_min(&self) -> f64 {
        self.freq.first().copied().unwrap_or(0.0)
    }

    pub fn f_max(&self) -> f64 {
        self.freq.last().copied().unwrap_or(0.0)
    }

    /// Magnitude (dB) and phase (deg) at `f` (Hz): linear interpolation,
    /// clamped to the first / last point. The flag is false out of range.
    pub fn at(&self, f: f64) -> ([f64; 4], [f64; 4], bool) {
        let n = self.freq.len();
        if n == 1 || f <= self.freq[0] {
            return (self.db[0], self.deg[0], n == 1 || f >= self.freq[0] * (1.0 - 1e-9));
        }
        if f >= self.freq[n - 1] {
            return (self.db[n - 1], self.deg[n - 1], f <= self.freq[n - 1] * (1.0 + 1e-9));
        }
        let j = self.freq.partition_point(|&x| x <= f);
        let (f0, f1) = (self.freq[j - 1], self.freq[j]);
        let t = (f - f0) / (f1 - f0);
        let lerp = |a: &[f64; 4], b: &[f64; 4]| -> [f64; 4] { std::array::from_fn(|k| a[k] + (b[k] - a[k]) * t) };
        (
            lerp(&self.db[j - 1], &self.db[j]),
            lerp(&self.deg[j - 1], &self.deg[j]),
            true,
        )
    }
}

// =============================================================================
// Configuration
// =============================================================================

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeembedMode {
    /// Vector when the phases are available, scalar otherwise.
    #[default]
    Auto,
    Vector,
    Scalar,
}

impl DeembedMode {
    pub const ALL: [DeembedMode; 3] = [DeembedMode::Auto, DeembedMode::Vector, DeembedMode::Scalar];

    pub fn label(self) -> &'static str {
        match self {
            DeembedMode::Auto => "Auto (vector if phases available)",
            DeembedMode::Vector => "Vector (T-matrices)",
            DeembedMode::Scalar => "Scalar (dB losses)",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum FreqUnit {
    #[default]
    Auto,
    Hz,
    KHz,
    MHz,
    GHz,
}

impl FreqUnit {
    pub const ALL: [FreqUnit; 5] = [
        FreqUnit::Auto,
        FreqUnit::Hz,
        FreqUnit::KHz,
        FreqUnit::MHz,
        FreqUnit::GHz,
    ];

    pub fn label(self) -> &'static str {
        match self {
            FreqUnit::Auto => "Auto",
            FreqUnit::Hz => "Hz",
            FreqUnit::KHz => "kHz",
            FreqUnit::MHz => "MHz",
            FreqUnit::GHz => "GHz",
        }
    }

    fn mult(self) -> f64 {
        match self {
            FreqUnit::Auto | FreqUnit::Hz => 1.0,
            FreqUnit::KHz => 1e3,
            FreqUnit::MHz => 1e6,
            FreqUnit::GHz => 1e9,
        }
    }

    /// Guesses the unit from the largest frequency of the data.
    fn guess(max: f64) -> FreqUnit {
        if max < 1e3 {
            FreqUnit::GHz
        } else if max < 1e6 {
            FreqUnit::MHz
        } else {
            FreqUnit::Hz
        }
    }
}

/// How an extra column is corrected with the fixture losses
/// (IL = insertion loss in dB, positive).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Correction {
    /// Power at the DUT input: P − IL_in.
    #[default]
    InputPower,
    /// Power at the DUT output: P + IL_out.
    OutputPower,
    /// Gain: G + IL_in + IL_out.
    Gain,
    /// Noise figure: NF − IL_in (passive input loss at room temperature).
    NoiseFigure,
    /// Loss / reflection seen through the input fixture: X + 2·IL_in.
    InputReflection,
    /// Loss / reflection seen through the output fixture: X + 2·IL_out.
    OutputReflection,
}

impl Correction {
    pub const ALL: [Correction; 6] = [
        Correction::InputPower,
        Correction::OutputPower,
        Correction::Gain,
        Correction::NoiseFigure,
        Correction::InputReflection,
        Correction::OutputReflection,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Correction::InputPower => "Input power (− IL in)",
            Correction::OutputPower => "Output power (+ IL out)",
            Correction::Gain => "Gain (+ IL in + IL out)",
            Correction::NoiseFigure => "Noise figure (− IL in)",
            Correction::InputReflection => "Input return loss (+ 2·IL in)",
            Correction::OutputReflection => "Output return loss (+ 2·IL out)",
        }
    }

    /// Value added to the column for the fixture losses `il_in`, `il_out` (dB, > 0).
    fn delta(self, il_in: f64, il_out: f64) -> f64 {
        match self {
            Correction::InputPower => -il_in,
            Correction::OutputPower => il_out,
            Correction::Gain => il_in + il_out,
            Correction::NoiseFigure => -il_in,
            Correction::InputReflection => 2.0 * il_in,
            Correction::OutputReflection => 2.0 * il_out,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExtraCorrection {
    pub column: String,
    pub kind: Correction,
}

impl Default for ExtraCorrection {
    fn default() -> Self {
        ExtraCorrection {
            column: String::new(),
            kind: Correction::InputPower,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Fixture {
    pub path: Option<PathBuf>,
    /// The file was measured with port 1 on the DUT side: swap its ports.
    pub flip: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DeembedConfig {
    pub enabled: bool,
    /// Between the instrument (port 1 of the file) and the DUT input (port 2).
    pub input: Fixture,
    /// Between the DUT output (port 1 of the file) and the instrument (port 2).
    pub output: Fixture,
    pub mode: DeembedMode,
    /// Frequency column (empty = automatic).
    pub freq_column: String,
    pub freq_unit: FreqUnit,
    /// Keep a copy of the raw columns ("S21db raw").
    pub keep_raw: bool,
    pub extra: Vec<ExtraCorrection>,
}

impl Default for DeembedConfig {
    fn default() -> Self {
        DeembedConfig {
            enabled: false,
            input: Fixture::default(),
            output: Fixture::default(),
            mode: DeembedMode::Auto,
            freq_column: String::new(),
            freq_unit: FreqUnit::Auto,
            keep_raw: true,
            extra: Vec::new(),
        }
    }
}

impl DeembedConfig {
    pub fn is_active(&self) -> bool {
        self.enabled && (self.input.path.is_some() || self.output.path.is_some())
    }
}

/// What was done.
#[derive(Clone, Debug, Default)]
pub struct DeembedReport {
    pub vector: bool,
    /// Description of the fixture files.
    pub fixtures: Vec<String>,
    pub freq_column: String,
    pub corrected: Vec<String>,
    pub notes: Vec<String>,
}

impl DeembedReport {
    pub fn summary(&self) -> String {
        format!(
            "{} de-embedding · {} column(s)",
            if self.vector { "Vector" } else { "Scalar" },
            self.corrected.len()
        )
    }
}

// =============================================================================
// Column detection
// =============================================================================

/// Lower-case alphanumeric form of a column name ("S21 [dB]" → "s21db").
fn norm(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

const PARAMS: [&str; 4] = ["s11", "s21", "s12", "s22"];

/// Index of the magnitude (dB) and phase (deg) columns of S11, S21, S12, S22.
pub fn find_sparam_columns(ds: &Dataset) -> ([Option<usize>; 4], [Option<usize>; 4]) {
    let names: Vec<String> = ds.columns.iter().map(|c| norm(&c.name)).collect();
    let find = |cands: &[String]| names.iter().position(|n| cands.iter().any(|c| c == n));
    let mut db = [None; 4];
    let mut deg = [None; 4];
    for (k, p) in PARAMS.iter().enumerate() {
        db[k] = find(&[
            format!("{p}db"),
            format!("db{p}"),
            format!("{p}mag"),
            format!("{p}magdb"),
        ]);
        deg[k] = find(&[
            format!("{p}deg"),
            format!("{p}ang"),
            format!("{p}angle"),
            format!("{p}phase"),
            format!("{p}phasedeg"),
            format!("{p}phdeg"),
        ]);
    }
    (db, deg)
}

/// Frequency column: the configured one, else the first named like "freq".
pub fn find_freq_column(ds: &Dataset, configured: &str) -> Option<usize> {
    if !configured.is_empty() {
        return ds.column_index(configured);
    }
    let names: Vec<String> = ds.columns.iter().map(|c| norm(&c.name)).collect();
    names
        .iter()
        .position(|n| n == "freq" || n == "frequency" || n.starts_with("freqhz") || n.starts_with("frequencyhz"))
        .or_else(|| names.iter().position(|n| n.contains("freq")))
}

// =============================================================================
// De-embedding
// =============================================================================

/// Fixture S-parameters at a frequency, as (dB, deg) and complex values.
#[derive(Clone, Copy)]
struct FixAt {
    db: [f64; 4],
    deg: [f64; 4],
    s: S2,
}

fn fixture_at(t: &Touchstone, flip_ports: bool, f: f64) -> (FixAt, bool) {
    let (mut db, mut deg, ok) = t.at(f);
    if flip_ports {
        db = [db[3], db[2], db[1], db[0]];
        deg = [deg[3], deg[2], deg[1], deg[0]];
    }
    let s = std::array::from_fn(|k| C::from_db_deg(db[k], deg[k]));
    (FixAt { db, deg, s }, ok)
}

/// Thru (no fixture).
const THRU: FixAt = FixAt {
    db: [-400.0, 0.0, 0.0, -400.0],
    deg: [0.0; 4],
    s: [C { re: 0.0, im: 0.0 }, C::ONE, C::ONE, C { re: 0.0, im: 0.0 }],
};

fn wrap_deg(d: f64) -> f64 {
    let r = (d + 180.0).rem_euclid(360.0) - 180.0;
    if r == -180.0 { 180.0 } else { r }
}

/// Applies the de-embedding to a dataset.
pub fn apply(base: &Dataset, cfg: &DeembedConfig) -> Result<(Dataset, DeembedReport), String> {
    let load = |fx: &Fixture| -> Result<Option<Arc<Touchstone>>, String> {
        match &fx.path {
            Some(p) => Touchstone::load(p).map(|t| Some(Arc::new(t))),
            None => Ok(None),
        }
    };
    let fin = load(&cfg.input)?;
    let fout = load(&cfg.output)?;
    apply_with(base, cfg, fin.as_deref(), fout.as_deref())
}

pub fn apply_with(
    base: &Dataset,
    cfg: &DeembedConfig,
    fin: Option<&Touchstone>,
    fout: Option<&Touchstone>,
) -> Result<(Dataset, DeembedReport), String> {
    if fin.is_none() && fout.is_none() {
        return Err("Choose an input and / or an output fixture file (.s2p)".to_owned());
    }
    let fcol = find_freq_column(base, &cfg.freq_column).ok_or_else(|| {
        if cfg.freq_column.is_empty() {
            "No frequency column found: choose it in the de-embedding settings".to_owned()
        } else {
            format!("Frequency column '{}' not found", cfg.freq_column)
        }
    })?;
    let freq_col = &base.columns[fcol];
    let unit = if cfg.freq_unit == FreqUnit::Auto {
        FreqUnit::guess(freq_col.max.abs().max(freq_col.min.abs()))
    } else {
        cfg.freq_unit
    };
    let fmult = unit.mult();

    let (db_cols, deg_cols) = find_sparam_columns(base);
    let mut report = DeembedReport {
        freq_column: format!("{} ({})", freq_col.name, unit.label()),
        ..Default::default()
    };
    for (side, t, fx) in [("Input", fin, &cfg.input), ("Output", fout, &cfg.output)] {
        if let Some(t) = t {
            report.fixtures.push(format!(
                "{side}: {} – {} points, {} – {} GHz, {}{}",
                t.name,
                t.freq.len(),
                format_sig(t.f_min() / 1e9, 4),
                format_sig(t.f_max() / 1e9, 4),
                if t.has_phase { "with phase" } else { "no phase" },
                if fx.flip { ", ports flipped" } else { "" }
            ));
        }
    }

    let meas_full = db_cols.iter().all(Option::is_some) && deg_cols.iter().all(Option::is_some);
    let fix_phase = fin.is_none_or(|t| t.has_phase) && fout.is_none_or(|t| t.has_phase);
    let vector = match cfg.mode {
        DeembedMode::Scalar => false,
        DeembedMode::Vector | DeembedMode::Auto => {
            let possible = meas_full && fix_phase;
            if !possible {
                let why = if !meas_full {
                    "the data does not contain the dB and phase columns of S11, S21, S12 and S22"
                } else {
                    "a fixture file has no phase information"
                };
                if cfg.mode == DeembedMode::Vector {
                    return Err(format!("Vector de-embedding impossible: {why}"));
                }
                report.notes.push(format!("Scalar de-embedding: {why}."));
            }
            possible
        }
    };
    report.vector = vector;

    // fixture values per distinct frequency
    let n = base.n_rows;
    let mut cache: FxHashMap<u64, (FixAt, FixAt)> = FxHashMap::default();
    let mut out_of_range = 0usize;
    let mut fix_rows: Vec<Option<(FixAt, FixAt)>> = Vec::with_capacity(n);
    for r in 0..n {
        let f = freq_col.value(r) * fmult;
        if !f.is_finite() {
            fix_rows.push(None);
            continue;
        }
        let key = f.to_bits();
        let v = *cache.entry(key).or_insert_with(|| {
            let mut ok = true;
            let a = match fin {
                Some(t) => {
                    let (v, o) = fixture_at(t, cfg.input.flip, f);
                    ok &= o;
                    v
                }
                None => THRU,
            };
            let b = match fout {
                Some(t) => {
                    let (v, o) = fixture_at(t, cfg.output.flip, f);
                    ok &= o;
                    v
                }
                None => THRU,
            };
            if !ok {
                out_of_range += 1;
            }
            (a, b)
        });
        fix_rows.push(Some(v));
    }
    if out_of_range > 0 {
        let span = |t: Option<&Touchstone>| {
            t.map(|t| {
                format!(
                    "{} [{} – {} GHz]",
                    t.name,
                    format_sig(t.f_min() / 1e9, 4),
                    format_sig(t.f_max() / 1e9, 4)
                )
            })
        };
        let files: Vec<String> = [span(fin), span(fout)].into_iter().flatten().collect();
        report.notes.push(format!(
            "{out_of_range} frequenc{} outside the fixture files ({}): the nearest point is used. Check the frequency unit.",
            if out_of_range > 1 { "ies" } else { "y" },
            files.join(", ")
        ));
    }

    let mut ds = base.clone();
    let mut new_cols: Vec<(usize, Vec<f64>)> = Vec::new();
    let col_vals = |i: usize| -> Vec<f64> {
        let c = &base.columns[i];
        (0..n).map(|r| c.value(r)).collect()
    };

    if vector {
        let dbi: [usize; 4] = std::array::from_fn(|k| db_cols[k].unwrap_or(0));
        let dgi: [usize; 4] = std::array::from_fn(|k| deg_cols[k].unwrap_or(0));
        let mut out_db: [Vec<f64>; 4] = std::array::from_fn(|_| vec![f64::NAN; n]);
        let mut out_deg: [Vec<f64>; 4] = std::array::from_fn(|_| vec![f64::NAN; n]);
        let mut failed = 0usize;
        // inverse T-matrices of the fixtures, per frequency
        let mut tinv: FxHashMap<u64, Option<(M2, M2)>> = FxHashMap::default();
        for r in 0..n {
            let Some((a, b)) = fix_rows[r] else { continue };
            let key = (freq_col.value(r) * fmult).to_bits();
            let Some((ta, tb)) = *tinv.entry(key).or_insert_with(|| {
                let ta = inv(&s_to_t(&a.s)?)?;
                let tb = inv(&s_to_t(&b.s)?)?;
                Some((ta, tb))
            }) else {
                failed += 1;
                continue;
            };
            let m: S2 =
                std::array::from_fn(|k| C::from_db_deg(base.columns[dbi[k]].value(r), base.columns[dgi[k]].value(r)));
            if m.iter().any(|c| !c.re.is_finite() || !c.im.is_finite()) {
                continue;
            }
            let Some(s) = s_to_t(&m).and_then(|tm| t_to_s(&mul(&mul(&ta, &tm), &tb))) else {
                failed += 1;
                continue;
            };
            for k in 0..4 {
                out_db[k][r] = s[k].db();
                out_deg[k][r] = s[k].deg();
            }
        }
        if failed > 0 {
            report
                .notes
                .push(format!("{failed} row(s) could not be de-embedded (S21 = 0)."));
        }
        for k in 0..4 {
            new_cols.push((dbi[k], std::mem::take(&mut out_db[k])));
            new_cols.push((dgi[k], std::mem::take(&mut out_deg[k])));
        }
    } else {
        // scalar: matched fixtures, losses (and phases) subtracted
        for k in 0..4 {
            // contributions of the fixtures to the parameter k
            let terms = |a: &FixAt, b: &FixAt, v: &dyn Fn(&FixAt, usize) -> f64| -> f64 {
                match k {
                    0 => v(a, 1) + v(a, 2),
                    1 => v(a, 1) + v(b, 1),
                    2 => v(a, 2) + v(b, 2),
                    _ => v(b, 1) + v(b, 2),
                }
            };
            if let Some(i) = db_cols[k] {
                let raw = col_vals(i);
                let vals = (0..n)
                    .map(|r| match &fix_rows[r] {
                        Some((a, b)) => raw[r] - terms(a, b, &|x, j| x.db[j]),
                        None => f64::NAN,
                    })
                    .collect();
                new_cols.push((i, vals));
            }
            if let Some(i) = deg_cols[k]
                && fix_phase
            {
                let raw = col_vals(i);
                let vals = (0..n)
                    .map(|r| match &fix_rows[r] {
                        Some((a, b)) => wrap_deg(raw[r] - terms(a, b, &|x, j| x.deg[j])),
                        None => f64::NAN,
                    })
                    .collect();
                new_cols.push((i, vals));
            }
        }
        if deg_cols.iter().any(Option::is_some) && !fix_phase {
            report
                .notes
                .push("The phase columns are not corrected (no phase in the fixture files).".to_owned());
        }
    }

    // extra columns
    for e in &cfg.extra {
        let Some(i) = base.column_index(&e.column) else {
            if !e.column.is_empty() {
                report.notes.push(format!("Column '{}' not found.", e.column));
            }
            continue;
        };
        if new_cols.iter().any(|(j, _)| *j == i) {
            continue;
        }
        let raw = col_vals(i);
        let vals = (0..n)
            .map(|r| match &fix_rows[r] {
                Some((a, b)) => raw[r] + e.kind.delta(-a.db[1], -b.db[1]),
                None => f64::NAN,
            })
            .collect();
        new_cols.push((i, vals));
    }

    if new_cols.is_empty() {
        return Err("No column to de-embed: S-parameter columns (S21db, S11db, S21deg…) not found".to_owned());
    }
    for (i, vals) in new_cols {
        let name = base.columns[i].name.clone();
        if cfg.keep_raw {
            let raw_name = format!("{name} raw");
            if ds.column_index(&raw_name).is_none() {
                ds.columns
                    .push(Arc::new(Column::from_values(raw_name, col_vals(i), None)));
            }
        }
        ds.columns[i] = Arc::new(Column::from_values(name.clone(), vals, None));
        report.corrected.push(name);
    }
    ds.generation = super::next_generation();
    Ok((ds, report))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::loader::{ImportOptions, parse_csv};

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    /// A lossy, slightly mismatched line with a frequency dependent delay.
    fn line(f: f64, loss_db: f64, delay_ps: f64, gamma: f64) -> S2 {
        let ph = -360.0 * f * delay_ps * 1e-12;
        let t = C::from_db_deg(-loss_db * (f / 10e9).sqrt(), ph);
        let g = C::new(gamma, 0.0);
        [g, t, t, g]
    }

    fn dut(f: f64) -> S2 {
        [
            C::from_db_deg(-12.0 - f / 1e10, 40.0),
            C::from_db_deg(15.0 - f / 1e10, -100.0 + f / 1e9),
            C::from_db_deg(-35.0, 60.0),
            C::from_db_deg(-9.0, -150.0),
        ]
    }

    fn touchstone(name: &str, freqs: &[f64], s: impl Fn(f64) -> S2) -> Touchstone {
        let pts: Vec<(f64, S2)> = freqs.iter().map(|&f| (f, s(f))).collect();
        Touchstone::from_points(name, &pts)
    }

    fn measurement(freqs: &[f64], fin: &dyn Fn(f64) -> S2, fout: &dyn Fn(f64) -> S2) -> Dataset {
        let mut text = String::from("Freq,S11db,S11deg,S21db,S21deg,S12db,S12deg,S22db,S22deg,Pout\n");
        for &f in freqs {
            let m = cascade(&cascade(&fin(f), &dut(f)).unwrap(), &fout(f)).unwrap();
            text.push_str(&format!("{f}"));
            for c in m {
                text.push_str(&format!(",{},{}", c.db(), c.deg()));
            }
            text.push_str(",10\n");
        }
        parse_csv(text.as_bytes(), &ImportOptions::default()).unwrap()
    }

    #[test]
    fn t_matrix_round_trip() {
        let s = dut(20e9);
        let back = t_to_s(&s_to_t(&s).unwrap()).unwrap();
        for k in 0..4 {
            assert!((back[k] - s[k]).abs() < 1e-12);
        }
        // thru cascade is neutral
        let thru = [C::new(0.0, 0.0), C::ONE, C::ONE, C::new(0.0, 0.0)];
        let c = cascade(&thru, &s).unwrap();
        assert!((c[1] - s[1]).abs() < 1e-12);
    }

    #[test]
    fn parse_v1_and_v2() {
        let v1 = "! cable\n# GHz S DB R 50\n1 -30 0 -1 -10 -1 -10 -30 0\n2 -28 5 -1.5 -20\n -1.5 -20 -28 5\n";
        let t = Touchstone::parse(v1, "c.s2p").unwrap();
        assert_eq!(t.freq, vec![1e9, 2e9]);
        assert!(close(t.db[1][1], -1.5, 1e-9));
        assert!(close(t.deg[1][1], -20.0, 1e-9));
        assert!(t.has_phase);

        let ma = "# MHz S MA R 50\n100 0.1 0 0.9 0 0.9 0 0.1 0\n";
        let t = Touchstone::parse(ma, "x").unwrap();
        assert_eq!(t.freq, vec![100e6]);
        assert!(close(t.db[0][1], 20.0 * 0.9f64.log10(), 1e-9));
        assert!(!t.has_phase);

        let v2 = "[Version] 2.0\n# Hz S RI R 50\n[Number of Ports] 2\n[Two-Port Data Order] 12_21\n[Number of Frequencies] 1\n[Network Data]\n1e9 0 0 0.5 0 0.8 0 0 0\n[End]\n";
        let t = Touchstone::parse(v2, "x").unwrap();
        // 12_21: S12 = 0.5 written first, S21 = 0.8
        assert!(close(t.db[0][1], 20.0 * 0.8f64.log10(), 1e-9));
        assert!(close(t.db[0][2], 20.0 * 0.5f64.log10(), 1e-9));

        assert!(Touchstone::parse("# GHz S DB R 50\n1 2 3\n", "x").is_err());
        assert!(Touchstone::parse("# GHz Z RI R 50\n1 0 0 0 0 0 0 0 0\n", "x").is_err());
    }

    #[test]
    fn unwrap_and_interpolate() {
        let freqs: Vec<f64> = (0..=10).map(|k| k as f64 * 1e9 + 1e9).collect();
        let t = touchstone("x", &freqs, |f| line(f, 1.0, 300.0, 0.0));
        // linear phase is interpolated exactly despite the wrapping
        let (_, deg, ok) = t.at(4.5e9);
        assert!(ok);
        assert!(close(wrap_deg(deg[1]), wrap_deg(-360.0 * 4.5e9 * 300e-12), 1e-6));
        let (db, _, ok) = t.at(50e9);
        assert!(!ok);
        assert_eq!(db, t.db[10]);
    }

    #[test]
    fn vector_recovers_the_dut() {
        let freqs: Vec<f64> = (0..=40).map(|k| 1e9 + k as f64 * 0.5e9).collect();
        let fin = |f: f64| line(f, 1.2, 150.0, 0.08);
        let fout = |f: f64| line(f, 2.0, 230.0, 0.05);
        let ds = measurement(&freqs, &fin, &fout);
        let tin = touchstone("in.s2p", &freqs, fin);
        // output file measured the other way round: flipped back by the option
        let tout = touchstone("out.s2p", &freqs, |f| flip(&fout(f)));
        let cfg = DeembedConfig {
            enabled: true,
            output: Fixture { path: None, flip: true },
            ..Default::default()
        };
        let (out, rep) = apply_with(&ds, &cfg, Some(&tin), Some(&tout)).unwrap();
        assert!(rep.vector, "{:?}", rep.notes);
        assert_eq!(rep.corrected.len(), 8);
        for (r, &f) in freqs.iter().enumerate() {
            let d = dut(f);
            for (k, p) in ["S11", "S21", "S12", "S22"].iter().enumerate() {
                let db = out.column(&format!("{p}db")).unwrap().value(r);
                let deg = out.column(&format!("{p}deg")).unwrap().value(r);
                assert!(close(db, d[k].db(), 1e-6), "{p} at {f}: {db} vs {}", d[k].db());
                assert!(close(wrap_deg(deg - d[k].deg()), 0.0, 1e-5), "{p} phase at {f}");
            }
        }
        // raw columns kept
        let raw = out.column("S21db raw").unwrap().value(0);
        assert!(close(raw, ds.column("S21db").unwrap().value(0), 1e-12));
    }

    #[test]
    fn scalar_for_matched_fixtures() {
        let freqs: Vec<f64> = (1..=20).map(|k| k as f64 * 1e9).collect();
        let fin = |f: f64| line(f, 1.0, 100.0, 0.0);
        let fout = |f: f64| line(f, 3.0, 100.0, 0.0);
        let ds = measurement(&freqs, &fin, &fout);
        // scalar loss files: magnitude only
        let tin = touchstone("in", &freqs, |f| fin(f).map(|c| C::new(c.abs(), 0.0)));
        let tout = touchstone("out", &freqs, |f| fout(f).map(|c| C::new(c.abs(), 0.0)));
        let cfg = DeembedConfig {
            enabled: true,
            keep_raw: false,
            extra: vec![ExtraCorrection {
                column: "Pout".into(),
                kind: Correction::OutputPower,
            }],
            ..Default::default()
        };
        let (out, rep) = apply_with(&ds, &cfg, Some(&tin), Some(&tout)).unwrap();
        assert!(!rep.vector);
        assert!(out.column("S21db raw").is_none());
        for (r, &f) in freqs.iter().enumerate() {
            let d = dut(f);
            for (k, p) in ["S11", "S21", "S12", "S22"].iter().enumerate() {
                let db = out.column(&format!("{p}db")).unwrap().value(r);
                assert!(close(db, d[k].db(), 1e-6), "{p} at {f}");
            }
            let il_out = -fout(f)[1].db();
            assert!(close(out.column("Pout").unwrap().value(r), 10.0 + il_out, 1e-9));
        }
        // the phase is not corrected without fixture phase
        assert!(rep.notes.iter().any(|n| n.contains("phase")));
    }

    #[test]
    fn frequency_unit_and_errors() {
        let text = "Freq (GHz),S21db\n1,10\n2,10\n";
        let ds = parse_csv(text.as_bytes(), &ImportOptions::default()).unwrap();
        let t = touchstone("in", &[1e9, 2e9], |_| {
            [
                C::new(0.0, 0.0),
                C::from_db_deg(-2.0, 0.0),
                C::from_db_deg(-2.0, 0.0),
                C::new(0.0, 0.0),
            ]
        });
        let cfg = DeembedConfig {
            enabled: true,
            ..Default::default()
        };
        let (out, rep) = apply_with(&ds, &cfg, Some(&t), None).unwrap();
        assert!(rep.freq_column.contains("GHz"));
        assert!(!rep.vector);
        assert!(!rep.notes.iter().any(|n| n.contains("outside")), "{:?}", rep.notes);
        assert!(close(out.column("S21db").unwrap().value(1), 12.0, 1e-9));
        let forced = DeembedConfig {
            mode: DeembedMode::Vector,
            ..cfg.clone()
        };
        assert!(apply_with(&ds, &forced, Some(&t), None).is_err());
        assert!(apply_with(&ds, &cfg, None, None).is_err());
    }
}
