//! "Export Data" page: chosen columns of a data source (after the
//! de-embedding and the user functions), optionally smoothed column by
//! column, written as CSV and / or Touchstone (.s2p) files, sorted into
//! folders / files by the values of chosen columns (SN, temperature…).
//!
//! Pipeline: filters → curves (the points sharing the same values of the
//! curve columns, sorted by X) → smoothing of every curve on its whole X
//! range → X band → files (written in parallel).

use std::collections::BTreeSet;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use serde::{Deserialize, Serialize};

use crate::data::dataset::{Column, Dataset, format_sig};
use crate::data::deembed::{FreqUnit, find_sparam_columns};
use crate::data::hash::{FxHashMap, hash_one};
use crate::plot::compute::filter_rows;
use crate::plot::config::FilterConfig;
use crate::plot::smooth::{SmoothConfig, SmoothMethod};

// =============================================================================
// Settings (saved in the workspace)
// =============================================================================

/// Smoothing of one exported column.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SmoothChoice {
    /// The default smoothing of the export.
    #[default]
    Default,
    /// Its own settings.
    Custom,
    /// Not smoothed.
    Off,
}

impl SmoothChoice {
    pub const ALL: [SmoothChoice; 3] = [SmoothChoice::Default, SmoothChoice::Custom, SmoothChoice::Off];

    pub fn label(self) -> &'static str {
        match self {
            SmoothChoice::Default => "Default",
            SmoothChoice::Custom => "Custom",
            SmoothChoice::Off => "None",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ColumnSmooth {
    pub column: String,
    pub choice: SmoothChoice,
    pub custom: SmoothConfig,
}

impl Default for ColumnSmooth {
    fn default() -> Self {
        ColumnSmooth {
            column: String::new(),
            choice: SmoothChoice::Default,
            custom: default_smoothing(),
        }
    }
}

/// Default smoothing of an export: light smoothing spline, values at the
/// measured points.
pub fn default_smoothing() -> SmoothConfig {
    SmoothConfig {
        enabled: true,
        method: SmoothMethod::Spline,
        smoothness: 0.3,
        interpolate: false,
        keep_original: false,
        ..Default::default()
    }
}

/// One level of the sorting of the files: a sub-folder or a part of the file
/// name per value of the column.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SplitLevel {
    pub column: String,
    pub folder: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CsvSep {
    #[default]
    Comma,
    Semicolon,
    Tab,
}

impl CsvSep {
    pub const ALL: [CsvSep; 3] = [CsvSep::Comma, CsvSep::Semicolon, CsvSep::Tab];

    pub fn char(self) -> char {
        match self {
            CsvSep::Comma => ',',
            CsvSep::Semicolon => ';',
            CsvSep::Tab => '\t',
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            CsvSep::Comma => "Comma ( , )",
            CsvSep::Semicolon => "Semicolon ( ; )",
            CsvSep::Tab => "Tab",
        }
    }
}

/// Format of the values of a Touchstone file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TsFormat {
    /// Magnitude in dB, angle in degrees.
    #[default]
    Db,
    /// Linear magnitude, angle in degrees.
    Ma,
    /// Real, imaginary parts.
    Ri,
}

impl TsFormat {
    pub const ALL: [TsFormat; 3] = [TsFormat::Db, TsFormat::Ma, TsFormat::Ri];

    pub fn code(self) -> &'static str {
        match self {
            TsFormat::Db => "DB",
            TsFormat::Ma => "MA",
            TsFormat::Ri => "RI",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            TsFormat::Db => "dB / angle (DB)",
            TsFormat::Ma => "Magnitude / angle (MA)",
            TsFormat::Ri => "Real / imaginary (RI)",
        }
    }
}

/// Names of the 8 S-parameter columns, in the Touchstone order.
pub const SPARAM_NAMES: [&str; 8] = [
    "S11 dB", "S11 deg", "S21 dB", "S21 deg", "S12 dB", "S12 deg", "S22 dB", "S22 deg",
];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DataExportConfig {
    /// Key of the data source.
    pub source: String,
    /// Export the data without the de-embedding.
    pub raw: bool,
    pub x: String,
    /// Exported value columns (CSV).
    pub columns: Vec<String>,
    /// X band (data units), `None` = unbounded.
    pub x_min: Option<f64>,
    pub x_max: Option<f64>,
    /// Other selection criteria (power range, SN…).
    pub filters: Vec<FilterConfig>,
    /// Columns defining a curve found automatically.
    pub auto_keys: bool,
    /// Columns defining a curve (with `auto_keys` off).
    pub curve_keys: Vec<String>,
    /// Sorting of the files: sub-folders / parts of the file names.
    pub split: Vec<SplitLevel>,
    /// Folder and file names contain the column names ("SN_1234").
    pub with_names: bool,
    pub smooth_enabled: bool,
    pub smooth: SmoothConfig,
    pub per_column: Vec<ColumnSmooth>,
    /// CSV: the unsmoothed values are written too.
    pub write_raw: bool,
    pub csv: bool,
    pub csv_sep: CsvSep,
    pub decimal_comma: bool,
    pub s2p: bool,
    pub ts_format: TsFormat,
    /// Unit of the X (frequency) column of the data.
    pub freq_unit: FreqUnit,
    /// Frequency unit of the .s2p files.
    pub ts_unit: FreqUnit,
    pub r_ref: f64,
    /// S11 dB, S11 deg, S21 dB… columns (empty = found from the names).
    pub sparams: [String; 8],
    pub folder: Option<PathBuf>,
    /// Beginning of the file names.
    pub prefix: String,
}

impl Default for DataExportConfig {
    fn default() -> Self {
        DataExportConfig {
            source: String::new(),
            raw: false,
            x: String::new(),
            columns: Vec::new(),
            x_min: None,
            x_max: None,
            filters: Vec::new(),
            auto_keys: true,
            curve_keys: Vec::new(),
            split: Vec::new(),
            with_names: true,
            smooth_enabled: false,
            smooth: default_smoothing(),
            per_column: Vec::new(),
            write_raw: false,
            csv: true,
            csv_sep: CsvSep::Comma,
            decimal_comma: false,
            s2p: false,
            ts_format: TsFormat::Db,
            freq_unit: FreqUnit::Auto,
            ts_unit: FreqUnit::GHz,
            r_ref: 50.0,
            sparams: Default::default(),
            folder: None,
            prefix: String::new(),
        }
    }
}

impl DataExportConfig {
    /// Smoothing applied to a column (`None` = not smoothed).
    pub fn smoothing_for(&self, column: &str) -> Option<&SmoothConfig> {
        if !self.smooth_enabled {
            return None;
        }
        match self.per_column.iter().find(|c| c.column == column) {
            Some(c) if c.choice == SmoothChoice::Off => None,
            Some(c) if c.choice == SmoothChoice::Custom => Some(&c.custom),
            _ => Some(&self.smooth),
        }
    }

    /// Smoothing settings of a column (created on first use).
    pub fn column_smooth_mut(&mut self, column: &str) -> &mut ColumnSmooth {
        let i = match self.per_column.iter().position(|c| c.column == column) {
            Some(i) => i,
            None => {
                self.per_column.push(ColumnSmooth {
                    column: column.to_owned(),
                    custom: self.smooth.clone(),
                    ..Default::default()
                });
                self.per_column.len() - 1
            }
        };
        &mut self.per_column[i]
    }

    pub fn choice_for(&self, column: &str) -> SmoothChoice {
        self.per_column
            .iter()
            .find(|c| c.column == column)
            .map_or(SmoothChoice::Default, |c| c.choice)
    }

    /// Identifies what the plan (curves, files) depends on.
    pub fn plan_key(&self, generation: u64) -> u64 {
        let mut c = self.clone();
        c.smooth = SmoothConfig::default();
        c.per_column.clear();
        c.smooth_enabled = false;
        c.write_raw = false;
        c.csv_sep = CsvSep::Comma;
        c.decimal_comma = false;
        c.ts_format = TsFormat::Db;
        c.r_ref = 0.0;
        c.folder = None;
        hash_one(&(serde_json::to_string(&c).unwrap_or_default(), generation))
    }

    /// Resolved frequency unit of the data.
    pub fn data_unit(&self, x: &Column) -> FreqUnit {
        if self.freq_unit == FreqUnit::Auto {
            FreqUnit::guess(x.max.abs().max(x.min.abs()))
        } else {
            self.freq_unit
        }
    }

    /// Beginning of the file names: the setting, else the source file name.
    pub fn file_prefix(&self, ds: &Dataset) -> String {
        file_part(if self.prefix.trim().is_empty() {
            Path::new(&ds.name)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("data")
        } else {
            self.prefix.trim()
        })
    }

    fn out_unit(&self) -> FreqUnit {
        if self.ts_unit == FreqUnit::Auto {
            FreqUnit::GHz
        } else {
            self.ts_unit
        }
    }
}

/// Phase column (unwrapped before smoothing): "S21deg", "S11 phase"…
pub fn is_phase(name: &str) -> bool {
    let n: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect();
    n.ends_with("deg")
        || n.ends_with("degree")
        || n.ends_with("degrees")
        || n.contains("phase")
        || n.ends_with("ang")
        || n.ends_with("angle")
}

// =============================================================================
// Plan: curves and files
// =============================================================================

pub struct Curve {
    /// Grouping keys of the curve columns (`Column::key`).
    pub keys: Vec<u64>,
    /// Rows sorted by X (whole X range, the band is applied when writing).
    pub rows: Vec<u32>,
    /// Number of points in the X band.
    pub in_band: usize,
}

pub struct Group {
    /// Relative path of the CSV file.
    pub csv: PathBuf,
    pub curves: std::ops::Range<usize>,
    pub points: usize,
}

pub struct Plan {
    pub key: u64,
    pub x: usize,
    pub columns: Vec<usize>,
    /// Columns defining a curve: the sorting columns first.
    pub key_cols: Vec<usize>,
    pub n_split: usize,
    /// Curve columns found automatically.
    pub auto_found: Vec<usize>,
    pub curves: Vec<Curve>,
    pub groups: Vec<Group>,
    /// Relative path of the .s2p file of every curve.
    pub s2p: Vec<PathBuf>,
    pub sparams: [Option<usize>; 8],
    /// Rows kept by the filters.
    pub n_rows: usize,
    /// Points in the X band.
    pub n_points: usize,
    /// Curves with repeated X values.
    pub duplicates: usize,
    pub band: (f64, f64),
    pub warnings: Vec<String>,
}

impl Plan {
    /// Index range of the points of a curve that are in the X band.
    pub fn band_range(&self, ds: &Dataset, rows: &[u32]) -> std::ops::Range<usize> {
        let xc = &ds.columns[self.x];
        let a = rows.partition_point(|&r| xc.value(r as usize) < self.band.0);
        let b = rows.partition_point(|&r| xc.value(r as usize) <= self.band.1);
        a..b.max(a)
    }

    /// "SN = 3008260, Set_TempC = -40" for a curve.
    pub fn curve_label(&self, ds: &Dataset, curve: usize) -> String {
        let c = &self.curves[curve];
        if self.key_cols.is_empty() {
            return "all the points".to_owned();
        }
        self.key_cols
            .iter()
            .zip(&c.keys)
            .map(|(&k, &v)| format!("{} = {}", ds.columns[k].name, ds.columns[k].key_label(v)))
            .collect::<Vec<_>>()
            .join(", ")
    }

    pub fn group_of(&self, curve: usize) -> usize {
        self.groups.partition_point(|g| g.curves.end <= curve)
    }
}

/// Folder / file name part (Windows-safe).
pub fn file_part(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        let c = if c.is_alphanumeric() || "-_.+=".contains(c) {
            c
        } else {
            '_'
        };
        if c == '_' && out.ends_with('_') {
            continue;
        }
        out.push(c);
    }
    let t = out.trim_matches('_').trim_end_matches('.').to_owned();
    let t: String = t.chars().take(80).collect();
    if t.is_empty() { "_".to_owned() } else { t }
}

/// Column name without its unit ("Set VDD Supply Voltage [V]" → "Set VDD Supply Voltage").
fn short_name(name: &str) -> &str {
    let cut = name.find(['[', '(']).unwrap_or(name.len());
    let t = name[..cut].trim();
    if t.is_empty() { name } else { t }
}

fn name_part(col: &Column, key: u64, with_names: bool) -> String {
    let v = col.key_label(key);
    if with_names {
        file_part(&format!("{}_{v}", short_name(&col.name)))
    } else {
        file_part(&v)
    }
}

/// Group number of every row for the given columns (rows with the same
/// values get the same number), and the number of groups.
fn group_ids(ds: &Dataset, rows: &[u32], cols: &[usize]) -> (Vec<u32>, usize) {
    let mut gid = vec![0u32; rows.len()];
    let mut n = usize::from(!rows.is_empty());
    for &c in cols {
        n = refine(ds, rows, &mut gid, c);
    }
    (gid, n)
}

/// Splits the groups `gid` by the values of a column; returns the number of groups.
fn refine(ds: &Dataset, rows: &[u32], gid: &mut [u32], c: usize) -> usize {
    let col = &ds.columns[c];
    let mut map: FxHashMap<(u32, u64), u32> = FxHashMap::default();
    for (i, &r) in rows.iter().enumerate() {
        let next = map.len() as u32;
        gid[i] = *map.entry((gid[i], col.key(r as usize))).or_insert(next);
    }
    map.len()
}

/// Number of points sharing their X value with another point of their group.
fn repeated_x(ds: &Dataset, rows: &[u32], gid: &[u32], x: usize) -> usize {
    let xc = &ds.columns[x];
    let mut seen: FxHashMap<(u32, u64), ()> = FxHashMap::default();
    seen.reserve(rows.len());
    for (i, &r) in rows.iter().enumerate() {
        seen.insert((gid[i], xc.key(r as usize)), ());
    }
    rows.len() - seen.len()
}

/// Columns defining the curves: the columns with few values that make every
/// X value unique inside a curve (fewest values first: SN, temperature,
/// supply…; measured values and time stamps are not candidates).
fn auto_curve_keys(ds: &Dataset, rows: &[u32], x: usize, values: &[usize], fixed: &[usize]) -> Vec<usize> {
    if rows.is_empty() {
        return Vec::new();
    }
    let (mut gid, _) = group_ids(ds, rows, fixed);
    let mut dup = repeated_x(ds, rows, &gid, x);
    if dup == 0 {
        return Vec::new();
    }
    // a curve has at least a few points on average: measured values (supply
    // currents, time stamps…) would cut the curves into crumbs
    const MIN_POINTS: usize = 5;
    let max_groups = rows.len() / MIN_POINTS;
    let mut cands: Vec<usize> = (0..ds.columns.len())
        .filter(|&c| c != x && !values.contains(&c) && !fixed.contains(&c))
        .filter(|&c| (2..=max_groups.max(2)).contains(&ds.columns[c].n_unique))
        .collect();
    cands.sort_by_key(|&c| (ds.columns[c].n_unique, !ds.columns[c].is_dict(), c));
    let mut found = Vec::new();
    for c in cands.into_iter().take(40) {
        let mut g = gid.clone();
        if refine(ds, rows, &mut g, c) > max_groups {
            continue;
        }
        let d = repeated_x(ds, rows, &g, x);
        if d < dup {
            found.push(c);
            gid = g;
            dup = d;
            if dup == 0 {
                break;
            }
        }
    }
    if dup > 0 {
        // no clean set of curves: keep only the obvious conditions (few
        // values); the page asks the user to complete the definition
        found.retain(|&c| ds.columns[c].n_unique <= 64);
    }
    found
}

fn cmp_keys(ds: &Dataset, cols: &[usize], a: &[u64], b: &[u64]) -> std::cmp::Ordering {
    for (k, &c) in cols.iter().enumerate() {
        let col = &ds.columns[c];
        let (ra, va) = col.key_order(a[k]);
        let (rb, vb) = col.key_order(b[k]);
        let o = ra.cmp(&rb).then(va.total_cmp(&vb));
        if o != std::cmp::Ordering::Equal {
            return o;
        }
    }
    std::cmp::Ordering::Equal
}

/// Makes the relative paths unique ("name_2.csv"…).
fn dedupe(paths: &mut [PathBuf]) {
    let mut seen: FxHashMap<String, usize> = FxHashMap::default();
    for p in paths.iter_mut() {
        let key = p.to_string_lossy().to_lowercase();
        let n = seen.entry(key).or_insert(0);
        *n += 1;
        if *n > 1 {
            let stem = p
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let ext = p
                .extension()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            p.set_file_name(format!("{stem}_{n}.{ext}"));
        }
    }
}

/// Curves and files of an export.
pub fn build_plan(ds: &Dataset, cfg: &DataExportConfig) -> Result<Plan, String> {
    let key = cfg.plan_key(ds.generation);
    let x = ds.column_index(&cfg.x).ok_or("Choose the X column (frequency…)")?;
    let xc = &ds.columns[x];
    if !xc.has_numbers() {
        return Err(format!("The X column '{}' contains no number", cfg.x));
    }
    let mut warnings = Vec::new();
    let mut columns = Vec::new();
    for name in &cfg.columns {
        match ds.column_index(name) {
            Some(i) if i != x && !columns.contains(&i) => columns.push(i),
            Some(_) => {}
            None => warnings.push(format!("Column '{name}' not found")),
        }
    }
    let mut rows = filter_rows(ds, &cfg.filters, &mut warnings);
    let before = rows.len();
    rows.retain(|&r| xc.value(r as usize).is_finite());
    if rows.len() < before {
        warnings.push(format!(
            "{} row(s) without a numeric X value ignored",
            before - rows.len()
        ));
    }
    let n_rows = rows.len();

    // ---- curve columns: sorting columns first ----------------------------
    let mut key_cols: Vec<usize> = Vec::new();
    let mut folder: Vec<bool> = Vec::new();
    for s in &cfg.split {
        match ds.column_index(&s.column) {
            Some(i) if i != x && !key_cols.contains(&i) => {
                key_cols.push(i);
                folder.push(s.folder);
            }
            Some(_) => {}
            None => warnings.push(format!("Sorting column '{}' not found", s.column)),
        }
    }
    let n_split = key_cols.len();
    let auto_found = if cfg.auto_keys {
        auto_curve_keys(ds, &rows, x, &columns, &key_cols)
    } else {
        for name in &cfg.curve_keys {
            match ds.column_index(name) {
                Some(i) if i != x && !key_cols.contains(&i) => key_cols.push(i),
                Some(_) => {}
                None => warnings.push(format!("Curve column '{name}' not found")),
            }
        }
        Vec::new()
    };
    key_cols.extend(&auto_found);

    // ---- curves ----------------------------------------------------------
    let (gid, n_curves) = group_ids(ds, &rows, &key_cols);
    let mut counts = vec![0usize; n_curves];
    for &g in &gid {
        counts[g as usize] += 1;
    }
    let mut curves: Vec<Curve> = counts
        .iter()
        .map(|&n| Curve {
            keys: Vec::new(),
            rows: Vec::with_capacity(n),
            in_band: 0,
        })
        .collect();
    for (i, &r) in rows.iter().enumerate() {
        let c = &mut curves[gid[i] as usize];
        if c.rows.is_empty() {
            c.keys = key_cols.iter().map(|&k| ds.columns[k].key(r as usize)).collect();
        }
        c.rows.push(r);
    }
    drop(rows);
    curves.sort_by(|a, b| cmp_keys(ds, &key_cols, &a.keys, &b.keys));
    let lo = cfg.x_min.unwrap_or(f64::NEG_INFINITY);
    let hi = cfg.x_max.unwrap_or(f64::INFINITY);
    let (lo, hi) = if lo <= hi { (lo, hi) } else { (hi, lo) };
    // sort every curve by X (stable: file order for equal X), in parallel
    let threads = std::thread::available_parallelism().map_or(4, |t| t.get()).clamp(1, 16);
    let chunk = curves.len().div_ceil(threads).max(1);
    let dup_count = AtomicUsize::new(0);
    std::thread::scope(|s| {
        for part in curves.chunks_mut(chunk) {
            let dup_count = &dup_count;
            s.spawn(move || {
                for c in part {
                    c.rows
                        .sort_by(|&a, &b| xc.value(a as usize).total_cmp(&xc.value(b as usize)));
                    let a = c.rows.partition_point(|&r| xc.value(r as usize) < lo);
                    let b = c.rows.partition_point(|&r| xc.value(r as usize) <= hi);
                    c.in_band = b.saturating_sub(a);
                    if c.rows[a..b.max(a)]
                        .windows(2)
                        .any(|w| xc.value(w[0] as usize) == xc.value(w[1] as usize))
                    {
                        dup_count.fetch_add(1, Ordering::Relaxed);
                    }
                }
            });
        }
    });
    curves.retain(|c| c.in_band > 0);
    let n_points = curves.iter().map(|c| c.in_band).sum();

    // ---- files -------------------------------------------------------------
    let prefix = cfg.file_prefix(ds);
    let dir_and_stem = |c: &Curve| -> (PathBuf, String) {
        let mut dir = PathBuf::new();
        let mut stem = prefix.clone();
        for k in 0..n_split {
            let part = name_part(&ds.columns[key_cols[k]], c.keys[k], cfg.with_names);
            if folder[k] {
                dir.push(part);
            } else {
                stem.push('_');
                stem.push_str(&part);
            }
        }
        (dir, stem)
    };
    let mut groups: Vec<Group> = Vec::new();
    let mut s2p: Vec<PathBuf> = Vec::with_capacity(curves.len());
    for (i, c) in curves.iter().enumerate() {
        let new_group = match groups.last() {
            Some(g) => curves[g.curves.start].keys[..n_split] != c.keys[..n_split],
            None => true,
        };
        let (dir, stem) = dir_and_stem(c);
        if new_group {
            groups.push(Group {
                csv: dir.join(format!("{stem}.csv")),
                curves: i..i + 1,
                points: 0,
            });
        }
        let g = groups.last_mut().expect("a group");
        g.curves.end = i + 1;
        g.points += c.in_band;
        let mut name = stem;
        for (&kc, &key) in key_cols.iter().zip(&c.keys).skip(n_split) {
            name.push('_');
            name.push_str(&name_part(&ds.columns[kc], key, cfg.with_names));
        }
        s2p.push(dir.join(format!("{name}.s2p")));
    }
    let mut csv_paths: Vec<PathBuf> = groups.iter().map(|g| g.csv.clone()).collect();
    dedupe(&mut csv_paths);
    for (g, p) in groups.iter_mut().zip(csv_paths) {
        g.csv = p;
    }
    dedupe(&mut s2p);

    // ---- S-parameter columns ---------------------------------------------
    let (db, deg) = find_sparam_columns(ds);
    let sparams: [Option<usize>; 8] = std::array::from_fn(|k| {
        let name = &cfg.sparams[k];
        if name.is_empty() {
            if k % 2 == 0 { db[k / 2] } else { deg[k / 2] }
        } else {
            ds.column_index(name)
        }
    });
    Ok(Plan {
        key,
        x,
        columns,
        key_cols,
        n_split,
        auto_found,
        curves,
        groups,
        s2p,
        sparams,
        n_rows,
        n_points,
        duplicates: dup_count.into_inner(),
        band: (lo, hi),
        warnings,
    })
}

// =============================================================================
// Smoothing
// =============================================================================

fn wrap_deg(d: f64) -> f64 {
    let r = (d + 180.0).rem_euclid(360.0) - 180.0;
    if r == -180.0 { 180.0 } else { r }
}

/// Removes the ±360° jumps of a phase (NaN values skipped).
pub fn unwrap_deg(v: &mut [f64]) {
    let mut offset = 0.0;
    let mut prev: Option<f64> = None;
    for y in v.iter_mut().filter(|y| y.is_finite()) {
        if let Some(p) = prev {
            let d = *y + offset - p;
            offset -= 360.0 * (d / 360.0).round();
        }
        *y += offset;
        prev = Some(*y);
    }
}

/// Smoothed values of a column along a curve (`x` ascending). A phase is
/// unwrapped first (no jump at ±180°) and wrapped back.
pub fn smooth_values(sm: &SmoothConfig, x: &[f64], y: &[f64], phase: bool) -> Vec<f64> {
    if !phase {
        return sm.values_at(x, y);
    }
    let mut u = y.to_vec();
    unwrap_deg(&mut u);
    let mut s = sm.values_at(x, &u);
    for v in s.iter_mut().filter(|v| v.is_finite()) {
        *v = wrap_deg(*v);
    }
    s
}

/// Values of a curve ready to be written: the points in the X band and the
/// smoothed columns (aligned with `rows`).
struct CurveValues<'a> {
    rows: &'a [u32],
    /// (column, smoothed values or None = the data as is).
    cols: Vec<(usize, Option<Vec<f64>>)>,
}

impl CurveValues<'_> {
    #[inline]
    fn value(&self, ds: &Dataset, col: usize, k: usize) -> f64 {
        match self.cols.iter().find(|c| c.0 == col) {
            Some((_, Some(s))) => s[k],
            _ => ds.columns[col].value(self.rows[k] as usize),
        }
    }

    fn smoothed(&self, col: usize) -> Option<&[f64]> {
        self.cols.iter().find(|c| c.0 == col).and_then(|c| c.1.as_deref())
    }
}

fn curve_values<'a>(
    ds: &Dataset,
    plan: &Plan,
    cfg: &DataExportConfig,
    curve: &'a Curve,
    needed: &[usize],
) -> CurveValues<'a> {
    let range = plan.band_range(ds, &curve.rows);
    let xc = &ds.columns[plan.x];
    let mut x: Option<Vec<f64>> = None;
    let cols = needed
        .iter()
        .map(|&c| {
            let col = &ds.columns[c];
            let smoothed = cfg.smoothing_for(&col.name).filter(|_| col.has_numbers()).map(|sm| {
                let x = x.get_or_insert_with(|| curve.rows.iter().map(|&r| xc.value(r as usize)).collect());
                let y: Vec<f64> = curve.rows.iter().map(|&r| col.value(r as usize)).collect();
                // whole curve smoothed, then cut to the band: no edge effect at the band limits
                smooth_values(sm, x, &y, is_phase(&col.name))[range.clone()].to_vec()
            });
            (c, smoothed)
        })
        .collect();
    CurveValues {
        rows: &curve.rows[range],
        cols,
    }
}

/// Raw and exported points of one column of a curve (preview of the page).
pub struct Preview {
    pub raw: Vec<[f64; 2]>,
    pub exported: Vec<[f64; 2]>,
    pub smoothed: bool,
}

pub fn preview(ds: &Dataset, plan: &Plan, cfg: &DataExportConfig, curve: usize, col: usize) -> Option<Preview> {
    let c = plan.curves.get(curve)?;
    let xc = &ds.columns[plan.x];
    let yc = ds.columns.get(col)?;
    let x: Vec<f64> = c.rows.iter().map(|&r| xc.value(r as usize)).collect();
    let y: Vec<f64> = c.rows.iter().map(|&r| yc.value(r as usize)).collect();
    let raw: Vec<[f64; 2]> = x.iter().zip(&y).map(|(&a, &b)| [a, b]).collect();
    let sm = cfg.smoothing_for(&yc.name).filter(|_| yc.has_numbers());
    let exported = match sm {
        Some(sm) => {
            let s = smooth_values(sm, &x, &y, is_phase(&yc.name));
            x.iter().zip(s).map(|(&a, b)| [a, b]).collect()
        }
        None => raw.clone(),
    };
    Some(Preview {
        raw,
        exported,
        smoothed: sm.is_some(),
    })
}

// =============================================================================
// Writers
// =============================================================================

/// Writes a number: exactly (`digits` = 0, values of the file) or with
/// `digits` significant digits (computed values), with a decimal comma if asked.
fn push_num(out: &mut String, v: f64, digits: usize, comma: bool) {
    use std::fmt::Write as _;
    let start = out.len();
    let a = v.abs();
    if v == 0.0 {
        out.push('0');
    } else if digits == 0 {
        if (1e-5..1e15).contains(&a) {
            let _ = write!(out, "{v}");
        } else {
            let _ = write!(out, "{v:e}");
        }
    } else {
        let mag = a.log10().floor() as i32;
        if (-4..15).contains(&mag) {
            let decimals = (digits as i32 - 1 - mag).max(0) as usize;
            let _ = write!(out, "{v:.decimals$}");
            if out[start..].contains('.') {
                let end = out.trim_end_matches('0').trim_end_matches('.').len();
                out.truncate(end);
            }
        } else {
            let _ = write!(out, "{:.*e}", digits - 1, v);
        }
    }
    if comma && out[start..].contains('.') {
        let tail = out[start..].replace('.', ",");
        out.truncate(start);
        out.push_str(&tail);
    }
}

fn push_quoted(out: &mut String, s: &str, sep: char) {
    if s.contains(sep) || s.contains('"') || s.contains('\n') || s.contains('\r') {
        out.push('"');
        out.push_str(&s.replace('"', "\"\""));
        out.push('"');
    } else {
        out.push_str(s);
    }
}

fn quote(s: &str, sep: char) -> String {
    let mut out = String::new();
    push_quoted(&mut out, s, sep);
    out
}

/// A data cell: the number, or the text when the cell is not a number.
/// Values read from the file are written exactly; computed columns
/// (de-embedding, functions) with 12 significant digits.
fn push_cell(out: &mut String, col: &Column, r: usize, sep: char, comma: bool) {
    let v = col.value(r);
    if v.is_finite() {
        let exact = col.formula.is_none() && col.is_dict();
        push_num(out, v, if exact { 0 } else { 12 }, comma);
    } else if col.is_dict() {
        push_quoted(out, &col.text(r), sep);
    }
}

/// Computed value (smoothed): 10 significant digits, empty if not a number.
fn push_computed(out: &mut String, v: f64, comma: bool) {
    if v.is_finite() {
        push_num(out, v, 10, comma);
    }
}

fn csv_header(ds: &Dataset, plan: &Plan, cfg: &DataExportConfig) -> String {
    let sep = cfg.csv_sep.char();
    let mut h: Vec<String> = plan.key_cols.iter().map(|&k| quote(&ds.columns[k].name, sep)).collect();
    h.push(quote(&ds.columns[plan.x].name, sep));
    for &c in &plan.columns {
        let name = &ds.columns[c].name;
        h.push(quote(name, sep));
        if cfg.write_raw && cfg.smoothing_for(name).is_some() {
            h.push(quote(&format!("{name} (unsmoothed)"), sep));
        }
    }
    let mut s = h.join(&sep.to_string());
    s.push('\n');
    s
}

fn csv_rows(ds: &Dataset, plan: &Plan, cfg: &DataExportConfig, cv: &CurveValues, out: &mut String) {
    let sep = cfg.csv_sep.char();
    let comma = cfg.decimal_comma;
    let xc = &ds.columns[plan.x];
    let cols: Vec<(&Column, Option<&[f64]>)> = plan
        .columns
        .iter()
        .map(|&c| (&*ds.columns[c], cv.smoothed(c)))
        .collect();
    for (k, &r) in cv.rows.iter().enumerate() {
        let r = r as usize;
        for &kc in &plan.key_cols {
            push_cell(out, &ds.columns[kc], r, sep, comma);
            out.push(sep);
        }
        push_cell(out, xc, r, sep, comma);
        for (col, smoothed) in &cols {
            out.push(sep);
            match smoothed {
                Some(s) => {
                    push_computed(out, s[k], comma);
                    if cfg.write_raw {
                        out.push(sep);
                        push_cell(out, col, r, sep, comma);
                    }
                }
                None => push_cell(out, col, r, sep, comma),
            }
        }
        out.push('\n');
    }
}

/// Text of the .s2p file of a curve; `notes` receives what the user should know.
fn s2p_text(
    ds: &Dataset,
    plan: &Plan,
    cfg: &DataExportConfig,
    curve: usize,
    cv: &CurveValues,
    info: &[String],
    notes: &mut BTreeSet<String>,
) -> String {
    use std::fmt::Write as _;
    let xc = &ds.columns[plan.x];
    let f_mult = cfg.data_unit(xc).mult() / cfg.out_unit().mult();
    let unit = cfg.out_unit().label().to_uppercase();
    let mut t = String::new();
    let _ = writeln!(
        t,
        "! Touchstone file written by CurveTracer v{}",
        env!("CARGO_PKG_VERSION")
    );
    for line in info {
        let _ = writeln!(t, "! {line}");
    }
    let _ = writeln!(t, "! Curve: {}", plan.curve_label(ds, curve));
    let mut smooth = Vec::new();
    for (k, c) in plan.sparams.iter().enumerate() {
        if let Some(c) = *c {
            let name = &ds.columns[c].name;
            if let Some(sm) = cfg.smoothing_for(name) {
                smooth.push(format!("{} ({name}): {}", SPARAM_NAMES[k], sm.describe()));
            }
        }
    }
    if !smooth.is_empty() {
        let _ = writeln!(t, "! Smoothing: {}", smooth.join("; "));
    }
    for p in 0..4 {
        let name = &SPARAM_NAMES[2 * p][..3];
        match (plan.sparams[2 * p], plan.sparams[2 * p + 1]) {
            (None, _) => {
                let _ = writeln!(
                    t,
                    "! {name}: not in the data, written as 0 (no transmission / reflection)"
                );
                notes.insert(format!("{name} is not in the data: written as 0"));
            }
            (Some(_), None) => {
                let _ = writeln!(t, "! {name}: magnitude only, phase written as 0");
                notes.insert(format!("{name}: no phase column, phase written as 0"));
            }
            _ => {}
        }
    }
    let _ = writeln!(t, "# {unit} S {} R {}", cfg.ts_format.code(), format_sig(cfg.r_ref, 10));
    let (a, b) = match cfg.ts_format {
        TsFormat::Db => ("dB", "ang"),
        TsFormat::Ma => ("mag", "ang"),
        TsFormat::Ri => ("re", "im"),
    };
    let _ = writeln!(
        t,
        "! freq[{}] S11{a} S11{b} S21{a} S21{b} S12{a} S12{b} S22{a} S22{b}",
        cfg.out_unit().label()
    );
    // points (equal frequencies averaged, incomplete points skipped)
    let mut pts: Vec<(f64, [f64; 8], usize)> = Vec::with_capacity(cv.rows.len());
    let mut skipped = 0;
    for k in 0..cv.rows.len() {
        let f = xc.value(cv.rows[k] as usize);
        let mut v = [0.0f64; 8];
        let mut ok = f.is_finite();
        for (j, c) in plan.sparams.iter().enumerate() {
            v[j] = match *c {
                Some(c) => cv.value(ds, c, k),
                None if j % 2 == 0 => f64::NEG_INFINITY,
                None => 0.0,
            };
            if c.is_some() && !v[j].is_finite() {
                ok = false;
            }
        }
        if !ok {
            skipped += 1;
            continue;
        }
        match pts.last_mut() {
            Some((pf, acc, n)) if *pf == f => {
                for j in 0..8 {
                    acc[j] += v[j];
                }
                *n += 1;
            }
            _ => pts.push((f, v, 1)),
        }
    }
    if skipped > 0 {
        notes.insert("Points with a missing S-parameter value were skipped".to_owned());
    }
    if pts.iter().any(|p| p.2 > 1) {
        notes.insert(
            "Repeated frequencies inside a curve were averaged: add a column to the curve definition".to_owned(),
        );
    }
    for (f, acc, n) in pts {
        let v: [f64; 8] = std::array::from_fn(|j| acc[j] / n as f64);
        let _ = write!(t, "{}", format_sig(f * f_mult, 12));
        for p in 0..4 {
            let (db, deg) = (v[2 * p], v[2 * p + 1]);
            let mag = if db.is_finite() { 10f64.powf(db / 20.0) } else { 0.0 };
            let (u, w) = match cfg.ts_format {
                TsFormat::Db => (if db.is_finite() { db } else { -200.0 }, deg),
                TsFormat::Ma => (mag, deg),
                TsFormat::Ri => {
                    let a = deg.to_radians();
                    (mag * a.cos(), mag * a.sin())
                }
            };
            let _ = write!(t, " {} {}", format_sig(u, 9), format_sig(w, 9));
        }
        t.push('\n');
    }
    t
}

// =============================================================================
// Export
// =============================================================================

pub struct ExportSummary {
    pub csv_files: usize,
    pub s2p_files: usize,
    pub points: usize,
    pub ms: f64,
    pub notes: Vec<String>,
}

/// Progress of a running export (shared with the user interface).
#[derive(Default)]
pub struct Progress {
    pub done: AtomicUsize,
    pub total: AtomicUsize,
    pub cancel: AtomicBool,
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    std::fs::write(path, bytes).map_err(|e| format!("Cannot write {}: {e}", path.display()))
}

/// Writes the files of an export into `dir`. `info`: description of the
/// source (name, de-embedding) written in the file headers.
pub fn run(
    ds: &Dataset,
    cfg: &DataExportConfig,
    dir: &Path,
    info: &[String],
    progress: &Progress,
    plan: Option<std::sync::Arc<Plan>>,
) -> Result<ExportSummary, String> {
    let t0 = std::time::Instant::now();
    if !cfg.csv && !cfg.s2p {
        return Err("Choose a format: CSV and / or Touchstone (.s2p)".to_owned());
    }
    // the plan of the page, if it matches these settings
    let plan = match plan.filter(|p| p.key == cfg.plan_key(ds.generation)) {
        Some(p) => p,
        None => std::sync::Arc::new(build_plan(ds, cfg)?),
    };
    let plan = &*plan;
    if plan.curves.is_empty() {
        return Err("No point to export: check the selection (band, criteria)".to_owned());
    }
    if cfg.csv && plan.columns.is_empty() {
        return Err("Choose the columns to export".to_owned());
    }
    if cfg.s2p && plan.sparams.iter().step_by(2).all(|c| c.is_none()) {
        return Err("No S-parameter column found (S11db, S21db…): choose them in the Touchstone settings".to_owned());
    }
    progress.total.store(plan.curves.len(), Ordering::Relaxed);
    progress.done.store(0, Ordering::Relaxed);
    let mut dirs: BTreeSet<PathBuf> = BTreeSet::new();
    for g in &plan.groups {
        dirs.insert(dir.join(g.csv.parent().unwrap_or(Path::new(""))));
    }
    for d in &dirs {
        std::fs::create_dir_all(d).map_err(|e| format!("Cannot create the folder {}: {e}", d.display()))?;
    }
    let mut needed: Vec<usize> = if cfg.csv { plan.columns.clone() } else { Vec::new() };
    if cfg.s2p {
        for c in plan.sparams.iter().flatten() {
            if !needed.contains(c) {
                needed.push(*c);
            }
        }
    }
    let threads = std::thread::available_parallelism().map_or(4, |t| t.get()).clamp(1, 16);
    let batch = (threads * 8).max(32);
    let notes = std::sync::Mutex::new(BTreeSet::new());
    let header = csv_header(ds, plan, cfg);
    let mut csv_file: Option<(usize, std::io::BufWriter<std::fs::File>)> = None;
    let mut csv_files = 0;
    let mut start = 0;
    while start < plan.curves.len() {
        if progress.cancel.load(Ordering::Relaxed) {
            return Err("Export cancelled".to_owned());
        }
        let end = (start + batch).min(plan.curves.len());
        // curves of the batch in parallel: CSV text (kept in order) and .s2p files
        let next = AtomicUsize::new(start);
        let results: Vec<Vec<(usize, Result<String, String>)>> = std::thread::scope(|s| {
            let handles: Vec<_> = (0..threads.min(end - start))
                .map(|_| {
                    let (next, notes, plan, needed) = (&next, &notes, &plan, &needed);
                    s.spawn(move || {
                        let mut out = Vec::new();
                        loop {
                            let i = next.fetch_add(1, Ordering::Relaxed);
                            if i >= end {
                                break;
                            }
                            let cv = curve_values(ds, plan, cfg, &plan.curves[i], needed);
                            let mut res = Ok(String::new());
                            if cfg.s2p {
                                let mut n = BTreeSet::new();
                                let text = s2p_text(ds, plan, cfg, i, &cv, info, &mut n);
                                if let Err(e) = write_file(&dir.join(&plan.s2p[i]), text.as_bytes()) {
                                    res = Err(e);
                                }
                                if !n.is_empty() {
                                    notes.lock().unwrap_or_else(|e| e.into_inner()).extend(n);
                                }
                            }
                            if cfg.csv && res.is_ok() {
                                let mut text = String::new();
                                csv_rows(ds, plan, cfg, &cv, &mut text);
                                res = Ok(text);
                            }
                            progress.done.fetch_add(1, Ordering::Relaxed);
                            out.push((i, res));
                        }
                        out
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap_or_default()).collect()
        });
        let mut results: Vec<(usize, Result<String, String>)> = results.into_iter().flatten().collect();
        if results.len() < end - start {
            return Err("Unexpected error while exporting".to_owned());
        }
        results.sort_by_key(|r| r.0);
        for (i, res) in results {
            let text = res?;
            if !cfg.csv {
                continue;
            }
            let g = plan.group_of(i);
            if csv_file.as_ref().is_none_or(|f| f.0 != g) {
                if let Some((_, mut w)) = csv_file.take() {
                    w.flush().map_err(|e| e.to_string())?;
                }
                let path = dir.join(&plan.groups[g].csv);
                let f = std::fs::File::create(&path).map_err(|e| format!("Cannot create {}: {e}", path.display()))?;
                let mut w = std::io::BufWriter::with_capacity(1 << 20, f);
                w.write_all(header.as_bytes()).map_err(|e| e.to_string())?;
                csv_file = Some((g, w));
                csv_files += 1;
            }
            if let Some((_, w)) = csv_file.as_mut() {
                w.write_all(text.as_bytes())
                    .map_err(|e| format!("Cannot write {}: {e}", plan.groups[g].csv.display()))?;
            }
        }
        start = end;
    }
    if let Some((_, mut w)) = csv_file.take() {
        w.flush().map_err(|e| e.to_string())?;
    }
    // description of the export next to the files
    let readme = describe(ds, plan, cfg, info);
    write_file(
        &dir.join(format!("{}_export_info.txt", cfg.file_prefix(ds))),
        readme.as_bytes(),
    )?;
    Ok(ExportSummary {
        csv_files,
        s2p_files: if cfg.s2p { plan.curves.len() } else { 0 },
        points: plan.n_points,
        ms: t0.elapsed().as_secs_f64() * 1000.0,
        notes: notes
            .into_inner()
            .unwrap_or_else(|e| e.into_inner())
            .into_iter()
            .collect(),
    })
}

/// Settings of an export, in plain text (written next to the files).
pub fn describe(ds: &Dataset, plan: &Plan, cfg: &DataExportConfig, info: &[String]) -> String {
    use std::fmt::Write as _;
    let mut t = String::new();
    let _ = writeln!(t, "Data exported by CurveTracer v{}", env!("CARGO_PKG_VERSION"));
    for line in info {
        let _ = writeln!(t, "{line}");
    }
    let _ = writeln!(t, "X: {}", ds.columns[plan.x].name);
    let band = |v: Option<f64>| v.map_or("-".to_owned(), |v| format_sig(v, 10));
    let _ = writeln!(t, "X band: {} … {} (data units)", band(cfg.x_min), band(cfg.x_max));
    for f in cfg.filters.iter().filter(|f| f.enabled) {
        let _ = match f.mode {
            crate::plot::config::FilterMode::Values => {
                let shown: Vec<&str> = f.values.iter().take(20).map(String::as_str).collect();
                let more = if f.values.len() > 20 { " …" } else { "" };
                writeln!(t, "Criterion: {} in {{{}{more}}}", f.column, shown.join(", "))
            }
            crate::plot::config::FilterMode::Range => {
                writeln!(t, "Criterion: {} in [{} … {}]", f.column, band(f.min), band(f.max))
            }
        };
    }
    let names = |cols: &[usize]| -> String {
        cols.iter()
            .map(|&c| ds.columns[c].name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let _ = writeln!(t, "Curves defined by: {}", names(&plan.key_cols));
    let _ = writeln!(t, "Files sorted by: {}", names(&plan.key_cols[..plan.n_split]));
    let _ = writeln!(
        t,
        "{} curve(s), {} point(s), {} CSV file(s), {} .s2p file(s)",
        plan.curves.len(),
        plan.n_points,
        if cfg.csv { plan.groups.len() } else { 0 },
        if cfg.s2p { plan.curves.len() } else { 0 }
    );
    let mut cols = plan.columns.clone();
    for c in plan.sparams.iter().flatten() {
        if !cols.contains(c) {
            cols.push(*c);
        }
    }
    let _ = writeln!(t, "Smoothing:");
    for c in cols {
        let name = &ds.columns[c].name;
        let s = match cfg.smoothing_for(name) {
            Some(sm) => format!(
                "{}{}",
                sm.describe(),
                if is_phase(name) { " (phase unwrapped)" } else { "" }
            ),
            None => "none".to_owned(),
        };
        let _ = writeln!(t, "  {name}: {s}");
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::deembed::Touchstone;
    use crate::data::loader::{ImportOptions, parse_csv};
    use crate::plot::config::FilterMode;

    /// 3 SN × 2 temperatures × 21 frequencies (GHz values in Hz), noisy S21.
    fn sample() -> Dataset {
        let mut s = String::from("SN,Temp,DUT_Temp,Freq,S11db,S11deg,S21db,S21deg,S22db,S22deg,Pin\n");
        let mut k = 0u64;
        for sn in ["A1", "B2", "C3"] {
            for t in [25, -40] {
                for f in 0..21 {
                    k = k.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                    let noise = ((k >> 33) as f64 / (1u64 << 31) as f64 - 0.5) * 0.2;
                    let ghz = 30.0 + f as f64 * 0.5;
                    let s21 = 10.0 - 0.1 * f as f64 + noise;
                    let deg = wrap_deg(170.0 - 25.0 * f as f64);
                    let dut = t as f64 + 0.1 * (f % 2) as f64;
                    s.push_str(&format!(
                        "{sn},{t},{dut},{}e9,-15.5,{deg},{s21},{deg},-12.25,-30,{}\n",
                        ghz,
                        -10 - (f % 3)
                    ));
                }
            }
        }
        parse_csv(s.as_bytes(), &ImportOptions::default()).unwrap()
    }

    fn cfg() -> DataExportConfig {
        DataExportConfig {
            x: "Freq".into(),
            columns: vec!["S21db".into(), "S21deg".into()],
            ..Default::default()
        }
    }

    #[test]
    fn curves_are_found_automatically() {
        let ds = sample();
        let plan = build_plan(&ds, &cfg()).unwrap();
        let names: Vec<&str> = plan.key_cols.iter().map(|&c| ds.columns[c].name.as_str()).collect();
        assert_eq!(names, ["Temp", "SN"]);
        assert_eq!(plan.curves.len(), 6);
        assert!(plan.curves.iter().all(|c| c.rows.len() == 21));
        assert_eq!(plan.duplicates, 0);
        // a single CSV file
        assert_eq!(plan.groups.len(), 1);
    }

    #[test]
    fn many_short_curves_and_measured_columns() {
        // 300 states × 8 random frequencies, a measured supply column (noise)
        let mut s = String::from("State,Vmeas,Freq,S21db\n");
        let mut k = 7u64;
        for st in 0..300 {
            for f in 0..8 {
                k = k.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                let freq = 1 + (st * 3 + f * 7) % 60;
                let v = 1.2 + ((k >> 20) % 1000) as f64 * 1e-6;
                s.push_str(&format!("S{st},{v},{freq},{}\n", f as f64 * 0.1));
            }
        }
        let ds = parse_csv(s.as_bytes(), &ImportOptions::default()).unwrap();
        let c = DataExportConfig {
            x: "Freq".into(),
            columns: vec!["S21db".into()],
            ..Default::default()
        };
        let plan = build_plan(&ds, &c).unwrap();
        let names: Vec<&str> = plan.key_cols.iter().map(|&c| ds.columns[c].name.as_str()).collect();
        assert_eq!(names, ["State"]);
        assert_eq!(plan.curves.len(), 300);
        assert_eq!(plan.duplicates, 0);
    }

    #[test]
    fn folders_files_band_and_criteria() {
        let ds = sample();
        let mut c = cfg();
        c.prefix = "CGY".into();
        c.split = vec![
            SplitLevel {
                column: "SN".into(),
                folder: true,
            },
            SplitLevel {
                column: "Temp".into(),
                folder: false,
            },
        ];
        c.x_min = Some(32e9);
        c.x_max = Some(34e9);
        c.filters = vec![FilterConfig {
            column: "SN".into(),
            mode: FilterMode::Values,
            values: vec!["A1".into(), "C3".into()],
            ..Default::default()
        }];
        let plan = build_plan(&ds, &c).unwrap();
        assert_eq!(plan.groups.len(), 4);
        let p0 = plan.groups[0].csv.to_string_lossy().replace('\\', "/");
        assert_eq!(p0, "SN_A1/CGY_Temp_-40.csv");
        // 32, 32.5 … 34 GHz
        assert!(plan.curves.iter().all(|c| c.in_band == 5));
        assert_eq!(plan.n_points, 4 * 5);
        let s = plan.s2p[0].to_string_lossy().replace('\\', "/");
        assert_eq!(s, "SN_A1/CGY_Temp_-40.s2p");
    }

    #[test]
    fn touchstone_round_trip() {
        let ds = sample();
        let mut c = cfg();
        c.s2p = true;
        c.csv = true;
        c.split = vec![SplitLevel {
            column: "SN".into(),
            folder: false,
        }];
        let dir = std::env::temp_dir().join(format!("ct_export_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let summary = run(&ds, &c, &dir, &["Source: test".into()], &Progress::default(), None).unwrap();
        assert_eq!(summary.s2p_files, 6);
        assert_eq!(summary.csv_files, 3);
        assert!(summary.notes.iter().any(|n| n.starts_with("S12")));
        let plan = build_plan(&ds, &c).unwrap();
        let text = std::fs::read_to_string(dir.join(&plan.s2p[0])).unwrap();
        let ts = Touchstone::parse(&text, "x").unwrap();
        assert_eq!(ts.freq.len(), 21);
        assert!((ts.freq[0] - 30e9).abs() < 1.0);
        let r = plan.curves[0].rows[3] as usize;
        let s21 = ds.column("S21db").unwrap().value(r);
        assert!((ts.db[3][1] - s21).abs() < 1e-6, "{} {}", ts.db[3][1], s21);
        assert!((ts.db[3][0] + 15.5).abs() < 1e-9);
        // CSV: header + 2 curves of 21 points
        let csv = std::fs::read_to_string(dir.join(&plan.groups[0].csv)).unwrap();
        assert_eq!(csv.lines().count(), 1 + 42);
        assert!(csv.starts_with("SN,Temp,Freq,S21db,S21deg\n"), "{}", &csv[..60]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn phase_is_smoothed_without_jumps() {
        let ds = sample();
        let mut c = cfg();
        c.smooth_enabled = true;
        let plan = build_plan(&ds, &c).unwrap();
        let col = ds.column_index("S21deg").unwrap();
        let p = preview(&ds, &plan, &c, 0, col).unwrap();
        assert!(p.smoothed);
        // the true phase is linear: the smoothed one stays close to it
        for (e, r) in p.exported.iter().zip(&p.raw) {
            let d = wrap_deg(e[1] - r[1]).abs();
            assert!(d < 1.0, "{} vs {}", e[1], r[1]);
        }
        // per-column choice
        c.column_smooth_mut("S21deg").choice = SmoothChoice::Off;
        assert!(c.smoothing_for("S21deg").is_none());
        assert!(c.smoothing_for("S21db").is_some());
    }

    #[test]
    fn smoothing_reduces_noise_and_keeps_points() {
        let ds = sample();
        let mut c = cfg();
        c.smooth_enabled = true;
        c.smooth.smoothness = 0.6;
        let plan = build_plan(&ds, &c).unwrap();
        let col = ds.column_index("S21db").unwrap();
        let p = preview(&ds, &plan, &c, 0, col).unwrap();
        assert_eq!(p.exported.len(), p.raw.len());
        let err = |pts: &[[f64; 2]]| -> f64 {
            pts.iter()
                .enumerate()
                .map(|(f, p)| (p[1] - (10.0 - 0.1 * f as f64)).powi(2))
                .sum::<f64>()
        };
        assert!(err(&p.exported) < 0.5 * err(&p.raw));
    }

    #[test]
    fn csv_semicolon_and_decimal_comma() {
        let ds = sample();
        let mut c = cfg();
        c.csv_sep = CsvSep::Semicolon;
        c.decimal_comma = true;
        c.columns = vec!["S11db".into()];
        let plan = build_plan(&ds, &c).unwrap();
        let cv = curve_values(&ds, &plan, &c, &plan.curves[0], &plan.columns);
        let mut out = csv_header(&ds, &plan, &c);
        csv_rows(&ds, &plan, &c, &cv, &mut out);
        let line = out.lines().nth(1).unwrap();
        assert_eq!(line, "-40;A1;30000000000;-15,5");
    }

    /// CT_BENCH_EXPORT=file.csv cargo test --release bench_export -- --ignored --nocapture
    #[test]
    #[ignore]
    fn bench_export() {
        let Ok(path) = std::env::var("CT_BENCH_EXPORT") else {
            return;
        };
        let t = std::time::Instant::now();
        let ds = crate::data::loader::load_path(Path::new(&path), &ImportOptions::default()).unwrap();
        eprintln!("load {:.2} s, {} rows", t.elapsed().as_secs_f64(), ds.n_rows);
        let mut c = cfg();
        c.columns = [
            "S11db", "S11deg", "S21db", "S21deg", "S12db", "S12deg", "S22db", "S22deg",
        ]
        .map(String::from)
        .to_vec();
        c.split = vec![SplitLevel {
            column: "SN".into(),
            folder: true,
        }];
        let t = std::time::Instant::now();
        let plan = build_plan(&ds, &c).unwrap();
        let keys: Vec<&str> = plan.key_cols.iter().map(|&k| ds.columns[k].name.as_str()).collect();
        eprintln!(
            "plan {:.2} s: {} curves, {} groups, keys {keys:?}, dup {}",
            t.elapsed().as_secs_f64(),
            plan.curves.len(),
            plan.groups.len(),
            plan.duplicates
        );
        let dir = std::env::temp_dir().join("ct_bench_export");
        for (smooth, s2p) in [(false, false), (true, false), (true, true)] {
            let _ = std::fs::remove_dir_all(&dir);
            c.smooth_enabled = smooth;
            c.s2p = s2p;
            let p = std::sync::Arc::new(build_plan(&ds, &c).unwrap());
            let t = std::time::Instant::now();
            let s = run(&ds, &c, &dir, &[], &Progress::default(), Some(p)).unwrap();
            eprintln!(
                "export smooth={smooth} s2p={s2p}: {:.2} s ({} csv, {} s2p, {} points)",
                t.elapsed().as_secs_f64(),
                s.csv_files,
                s.s2p_files,
                s.points
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn number_formats() {
        let f = |v: f64, d: usize, c: bool| {
            let mut s = String::new();
            push_num(&mut s, v, d, c);
            s
        };
        assert_eq!(f(-15.5, 0, false), "-15.5");
        assert_eq!(f(35e9, 0, false), "35000000000");
        assert_eq!(f(117.43656552631204, 12, false), "117.436565526");
        assert_eq!(f(117.43656552631204, 10, true), "117,4365655");
        assert_eq!(f(2.5e-9, 10, false), "2.500000000e-9");
        assert_eq!(f(0.0, 10, false), "0");
        assert_eq!(f(-40.0, 12, false), "-40");
        assert_eq!(quote("a;b", ';'), "\"a;b\"");
    }

    #[test]
    fn unwrap_and_names() {
        let mut v = vec![170.0, -170.0, -150.0, 179.0];
        unwrap_deg(&mut v);
        assert_eq!(v, vec![170.0, 190.0, 210.0, 179.0]);
        assert!(is_phase("S21deg") && is_phase("S11 phase") && !is_phase("Set_TempC"));
        assert_eq!(file_part("a/b: c"), "a_b_c");
        assert_eq!(file_part("-40.0"), "-40.0");
        assert_eq!(short_name("Set VDD Supply Voltage [V]"), "Set VDD Supply Voltage");
        assert_eq!(short_name("[V]"), "[V]");
    }
}
