//! Turns a dataset + a plot configuration into traces:
//! filters → series grouping → (optional) aggregation → line segments.

use std::cmp::Ordering;
use std::time::Instant;

use super::config::{AggFunc, FilterMode, PlotConfig};
use crate::data::dataset::{Column, ColumnKind, Dataset};
use crate::data::hash::FxHashMap;

/// (y column, remaining series values, indices of the member traces)
type AggGroup = (String, Vec<(String, String)>, Vec<usize>);

pub struct Trace {
    /// Stable identifier (used for per-trace style overrides).
    pub key: String,
    /// Default legend label.
    pub label: String,
    pub y_col: String,
    pub series: Vec<(String, String)>,
    /// Line segments. When `sorted`, each segment has ascending X.
    pub segments: Vec<Vec<[f64; 2]>>,
    pub sorted: bool,
    pub n_points: usize,
    pub aggregate: bool,
}

#[derive(Default)]
pub struct PlotData {
    pub traces: Vec<Trace>,
    /// Rows of the dataset kept by the filters.
    pub rows: Vec<u32>,
    /// Category names when the X column is textual.
    pub x_categories: Option<Vec<String>>,
    pub x_label: String,
    pub y_label: String,
    /// xmin, xmax, ymin, ymax of all the points.
    pub bounds: Option<[f64; 4]>,
    pub warnings: Vec<String>,
    pub error: Option<String>,
    pub total_points: usize,
    pub compute_ms: f64,
    /// Whether original traces are shown next to aggregated ones.
    pub has_aggregates_and_originals: bool,
    /// Generation of the dataset the rows refer to.
    pub generation: u64,
}

impl PlotData {
    pub fn error(msg: impl Into<String>) -> Self {
        PlotData {
            error: Some(msg.into()),
            ..Default::default()
        }
    }
}

enum Pred<'a> {
    Codes(&'a Column, Vec<bool>),
    Bits(&'a Column, Vec<u64>),
    Range(&'a Column, f64, f64),
}

impl Pred<'_> {
    #[inline]
    fn pass(&self, row: usize) -> bool {
        match self {
            Pred::Codes(col, allowed) => col.code(row).is_some_and(|c| allowed[c as usize]),
            Pred::Bits(col, bits) => bits.binary_search(&col.key(row)).is_ok(),
            Pred::Range(col, lo, hi) => {
                let v = col.value(row);
                v >= *lo && v <= *hi
            }
        }
    }
}

/// Parallel filter of `0..n`.
fn par_filter(n: usize, pass: impl Fn(usize) -> bool + Sync) -> Vec<u32> {
    let threads = std::thread::available_parallelism()
        .map(|t| t.get())
        .unwrap_or(4)
        .clamp(1, 16);
    if n < 200_000 || threads == 1 {
        return (0..n).filter(|&r| pass(r)).map(|r| r as u32).collect();
    }
    let chunk = n.div_ceil(threads);
    let pass = &pass;
    let parts: Vec<Vec<u32>> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..threads)
            .map(|t| {
                s.spawn(move || {
                    let start = t * chunk;
                    let end = ((t + 1) * chunk).min(n);
                    (start..end)
                        .filter(|&r| pass(r))
                        .map(|r| r as u32)
                        .collect::<Vec<u32>>()
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap_or_default()).collect()
    });
    parts.concat()
}

fn series_text(pairs: &[(String, String)]) -> String {
    pairs
        .iter()
        .map(|(c, v)| format!("{c}={v}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Splits points (file order) into segments with monotonic X, each returned
/// in ascending X order.
fn make_segments(pts: Vec<[f64; 2]>, sort_x: bool, break_on_reversal: bool) -> (Vec<Vec<[f64; 2]>>, bool) {
    if pts.is_empty() {
        return (Vec::new(), true);
    }
    if sort_x {
        let mut pts = pts;
        pts.sort_by(|a, b| a[0].partial_cmp(&b[0]).unwrap_or(Ordering::Equal));
        return (vec![pts], true);
    }
    let n = pts.len();
    let mut segs: Vec<Vec<[f64; 2]>> = Vec::new();
    let mut cur: Vec<[f64; 2]> = Vec::with_capacity(n.min(4096));
    let mut dir = 0i8;
    for p in pts {
        if let Some(last) = cur.last() {
            let d = if p[0] > last[0] {
                1
            } else if p[0] < last[0] {
                -1
            } else {
                0
            };
            if d != 0 && dir != 0 && d != dir {
                if !break_on_reversal {
                    // keep the drawing order: one unsorted segment
                    cur.push(p);
                    dir = 2;
                    continue;
                }
                let mut done = std::mem::take(&mut cur);
                if dir < 0 {
                    done.reverse();
                }
                segs.push(done);
                dir = 0;
            } else if dir == 0 && d != 0 {
                dir = d;
            }
        }
        cur.push(p);
    }
    if dir == 2 {
        // not monotonic and no break wanted: drawn as is
        segs.push(cur);
        return (segs, false);
    }
    if dir < 0 {
        cur.reverse();
    }
    segs.push(cur);
    // Badly ordered (scattered) data: sort it.
    if segs.len() > 16 && segs.len() * 8 > n {
        let mut all: Vec<[f64; 2]> = segs.concat();
        all.sort_by(|a, b| a[0].partial_cmp(&b[0]).unwrap_or(Ordering::Equal));
        return (vec![all], true);
    }
    (segs, true)
}

/// Aggregates points sharing the same X (relative tolerance 1e-9 of the span).
fn aggregate(points: &mut [[f64; 2]], func: AggFunc, span: (f64, f64)) -> Vec<[f64; 2]> {
    if points.is_empty() {
        return Vec::new();
    }
    let (lo, hi) = span;
    let width = if hi > lo { hi - lo } else { 1.0 };
    let q = |x: f64| ((x - lo) / width * 1e9).round() as i64;
    points.sort_by(|a, b| {
        q(a[0])
            .cmp(&q(b[0]))
            .then(a[1].partial_cmp(&b[1]).unwrap_or(Ordering::Equal))
    });
    let mut out = Vec::new();
    let mut i = 0;
    while i < points.len() {
        let key = q(points[i][0]);
        let mut j = i + 1;
        while j < points.len() && q(points[j][0]) == key {
            j += 1;
        }
        let run = &points[i..j];
        let n = run.len() as f64;
        let x = run.iter().map(|p| p[0]).sum::<f64>() / n;
        let sum: f64 = run.iter().map(|p| p[1]).sum();
        let y = match func {
            AggFunc::Mean => sum / n,
            AggFunc::Sum => sum,
            AggFunc::Min => run[0][1],
            AggFunc::Max => run[run.len() - 1][1],
            AggFunc::Range => run[run.len() - 1][1] - run[0][1],
            AggFunc::Median => {
                let m = run.len() / 2;
                if run.len() % 2 == 1 {
                    run[m][1]
                } else {
                    0.5 * (run[m - 1][1] + run[m][1])
                }
            }
            AggFunc::Rms => (run.iter().map(|p| p[1] * p[1]).sum::<f64>() / n).sqrt(),
            AggFunc::Std => {
                let mean = sum / n;
                if run.len() < 2 {
                    0.0
                } else {
                    (run.iter().map(|p| (p[1] - mean).powi(2)).sum::<f64>() / (n - 1.0)).sqrt()
                }
            }
            AggFunc::Count => n,
        };
        out.push([x, y]);
        i = j;
    }
    out
}

pub fn compute(ds: &Dataset, cfg: &PlotConfig) -> PlotData {
    let t0 = Instant::now();
    if cfg.x.is_empty() || cfg.ys.is_empty() {
        return PlotData::error("Select an X column and at least one Y column (Data → Select Data).");
    }
    let Some(xc) = ds.column(&cfg.x) else {
        return PlotData::error(format!("Column '{}' not found in {}", cfg.x, ds.name));
    };
    let mut warnings = Vec::new();
    let mut ycols: Vec<&Column> = Vec::new();
    for y in &cfg.ys {
        match ds.column(y) {
            Some(c) => {
                if !c.has_numbers() {
                    warnings.push(format!("Column '{y}' contains no numeric value"));
                }
                ycols.push(c);
            }
            None => warnings.push(format!("Column '{y}' not found")),
        }
    }
    if ycols.is_empty() {
        return PlotData {
            warnings,
            ..PlotData::error("None of the Y columns exists in this data source")
        };
    }

    // ---- filters ----------------------------------------------------------
    let mut preds: Vec<Pred<'_>> = Vec::new();
    for f in cfg.filters.iter().filter(|f| f.enabled) {
        let Some(col) = ds.column(&f.column) else {
            warnings.push(format!("Filter column '{}' not found", f.column));
            continue;
        };
        match f.mode {
            FilterMode::Values => {
                if let Some(dict) = col.dict() {
                    // one hash lookup per selected value (thousands of states)
                    let index: FxHashMap<&str, usize> = dict.iter().enumerate().map(|(i, d)| (d.as_str(), i)).collect();
                    let mut allowed = vec![false; dict.len()];
                    for v in &f.values {
                        if let Some(&c) = index.get(v.as_str()) {
                            allowed[c] = true;
                        }
                    }
                    if allowed.iter().all(|&a| a) {
                        continue; // every value selected: no filtering
                    }
                    preds.push(Pred::Codes(col, allowed));
                } else {
                    let mut bits: Vec<u64> = f
                        .values
                        .iter()
                        .filter_map(|v| v.trim().parse::<f64>().ok())
                        .map(|v| if v == 0.0 { 0.0f64 } else { v }.to_bits())
                        .collect();
                    bits.sort_unstable();
                    preds.push(Pred::Bits(col, bits));
                }
            }
            FilterMode::Range => {
                let lo = f.min.unwrap_or(f64::NEG_INFINITY);
                let hi = f.max.unwrap_or(f64::INFINITY);
                preds.push(Pred::Range(col, lo, hi));
            }
        }
    }
    let rows: Vec<u32> = if preds.is_empty() {
        (0..ds.n_rows as u32).collect()
    } else {
        par_filter(ds.n_rows, |r| preds.iter().all(|p| p.pass(r)))
    };

    // ---- series grouping ---------------------------------------------------
    let mut scols: Vec<&Column> = Vec::new();
    for s in &cfg.series {
        match ds.column(s) {
            Some(c) => scols.push(c),
            None => warnings.push(format!("Series column '{s}' not found")),
        }
    }
    let mut gid: Vec<u32> = vec![0; rows.len()];
    let mut keys: Vec<Vec<u64>> = vec![Vec::new()];
    for col in &scols {
        let mut map: FxHashMap<(u32, u64), u32> = FxHashMap::default();
        let mut new_keys: Vec<Vec<u64>> = Vec::new();
        for (i, &r) in rows.iter().enumerate() {
            let k = col.key(r as usize);
            let prev = gid[i];
            let id = *map.entry((prev, k)).or_insert_with(|| {
                let mut kk = keys[prev as usize].clone();
                kk.push(k);
                new_keys.push(kk);
                (new_keys.len() - 1) as u32
            });
            gid[i] = id;
        }
        keys = if new_keys.is_empty() {
            vec![Vec::new()]
        } else {
            new_keys
        };
    }
    let n_groups = keys.len();
    let mut order: Vec<usize> = (0..n_groups).collect();
    order.sort_by(|&a, &b| {
        for (j, col) in scols.iter().enumerate() {
            let (ra, va) = col.key_order(keys[a][j]);
            let (rb, vb) = col.key_order(keys[b][j]);
            let o = ra.cmp(&rb).then(va.partial_cmp(&vb).unwrap_or(Ordering::Equal));
            if o != Ordering::Equal {
                return o;
            }
        }
        Ordering::Equal
    });
    let mut group_rows: Vec<Vec<u32>> = vec![Vec::new(); n_groups];
    for (i, &r) in rows.iter().enumerate() {
        group_rows[gid[i] as usize].push(r);
    }
    let group_pairs: Vec<Vec<(String, String)>> = keys
        .iter()
        .map(|k| {
            scols
                .iter()
                .zip(k)
                .map(|(c, &key)| (c.name.clone(), c.key_label(key)))
                .collect()
        })
        .collect();

    // ---- X values -----------------------------------------------------------
    let x_cat = xc.kind == ColumnKind::Text && xc.is_dict();
    let xval = |r: usize| -> f64 {
        if x_cat {
            xc.rank(r).map_or(f64::NAN, |k| k as f64)
        } else {
            cfg.x_scale.apply(xc.value(r))
        }
    };
    if !x_cat && !xc.has_numbers() {
        warnings.push(format!("X column '{}' contains no numeric value", cfg.x));
    }

    // ---- traces -------------------------------------------------------------
    let multi_y = ycols.len() > 1;
    let mut traces: Vec<Trace> = Vec::new();
    for yc in &ycols {
        for &g in &order {
            if group_rows[g].is_empty() {
                continue;
            }
            let pts: Vec<[f64; 2]> = group_rows[g]
                .iter()
                .filter_map(|&r| {
                    let r = r as usize;
                    let x = xval(r);
                    let y = cfg.y_scale.apply(yc.value(r));
                    (x.is_finite() && y.is_finite()).then_some([x, y])
                })
                .collect();
            if pts.is_empty() {
                continue;
            }
            let pairs = group_pairs[g].clone();
            let st = series_text(&pairs);
            let label = match (multi_y, st.is_empty()) {
                (_, true) => yc.name.clone(),
                (false, false) => st.clone(),
                (true, false) => format!("{} | {st}", yc.name),
            };
            let n_points = pts.len();
            let (segments, sorted) = make_segments(pts, cfg.options.sort_x, cfg.options.break_on_reversal);
            traces.push(Trace {
                key: format!("{}|{st}", yc.name),
                label,
                y_col: yc.name.clone(),
                series: pairs,
                segments,
                sorted,
                n_points,
                aggregate: false,
            });
        }
    }

    // ---- processing ---------------------------------------------------------
    let mut has_both = false;
    if cfg.process.enabled && !traces.is_empty() {
        let func = cfg.process.func;
        let mut lo = f64::INFINITY;
        let mut hi = f64::NEG_INFINITY;
        for t in &traces {
            for s in &t.segments {
                for p in s {
                    lo = lo.min(p[0]);
                    hi = hi.max(p[0]);
                }
            }
        }
        let collapse: Vec<&String> = cfg.process.collapse.iter().filter(|c| cfg.series.contains(c)).collect();
        if collapse.is_empty() {
            for t in &mut traces {
                let mut all: Vec<[f64; 2]> = t.segments.concat();
                let agg = aggregate(&mut all, func, (lo, hi));
                t.n_points = agg.len();
                t.segments = vec![agg];
                t.sorted = true;
                t.aggregate = true;
            }
        } else {
            // group traces by (y column, remaining series values)
            // (y column, remaining series values, member traces)
            let mut groups: Vec<AggGroup> = Vec::new();
            let mut index: std::collections::HashMap<(String, Vec<(String, String)>), usize> =
                std::collections::HashMap::new();
            for (i, t) in traces.iter().enumerate() {
                let remaining: Vec<(String, String)> = t
                    .series
                    .iter()
                    .filter(|(c, _)| !collapse.contains(&c))
                    .cloned()
                    .collect();
                match index.get(&(t.y_col.clone(), remaining.clone())) {
                    Some(&g) => groups[g].2.push(i),
                    None => {
                        index.insert((t.y_col.clone(), remaining.clone()), groups.len());
                        groups.push((t.y_col.clone(), remaining, vec![i]));
                    }
                }
            }
            let collapsed_names = collapse.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ");
            let mut agg_traces = Vec::new();
            for (y, remaining, members) in groups {
                let mut all: Vec<[f64; 2]> = members.iter().flat_map(|&i| traces[i].segments.concat()).collect();
                let agg = aggregate(&mut all, func, (lo, hi));
                let st = series_text(&remaining);
                let fname = func.short();
                let mut label = if st.is_empty() {
                    format!("{fname} over {collapsed_names}")
                } else {
                    format!("{st} ({fname})")
                };
                if multi_y {
                    label = format!("{y} | {label}");
                }
                agg_traces.push(Trace {
                    key: format!("{fname}:{y}|{st}"),
                    label,
                    y_col: y,
                    series: remaining,
                    n_points: agg.len(),
                    segments: vec![agg],
                    sorted: true,
                    aggregate: true,
                });
            }
            if cfg.process.keep_original {
                has_both = true;
                traces.extend(agg_traces);
            } else {
                traces = agg_traces;
            }
        }
    }

    // ---- smoothing ------------------------------------------------------------
    let sm = &cfg.process.smooth;
    if sm.enabled && !traces.is_empty() {
        let desc = sm.describe();
        let smoothed: Vec<Trace> = traces
            .iter()
            .filter(|t| !(has_both && !t.aggregate))
            .map(|t| {
                let segments: Vec<Vec<[f64; 2]>> = t.segments.iter().map(|s| sm.apply(s)).collect();
                let sorted = segments.iter().all(|s| s.windows(2).all(|w| w[0][0] <= w[1][0]));
                Trace {
                    key: format!("smooth:{}", t.key),
                    label: if sm.keep_original {
                        format!("{} ({desc})", t.label)
                    } else {
                        t.label.clone()
                    },
                    y_col: t.y_col.clone(),
                    series: t.series.clone(),
                    n_points: segments.iter().map(Vec::len).sum(),
                    segments,
                    sorted,
                    aggregate: true,
                }
            })
            .collect();
        if sm.keep_original {
            // the raw curves stay, faded; the smoothed ones are drawn on top
            for t in &mut traces {
                t.aggregate = false;
            }
            has_both = true;
            traces.extend(smoothed);
        } else {
            traces = smoothed;
        }
    }

    // ---- bounds -----------------------------------------------------------
    let mut b = [f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY];
    let mut total = 0;
    for t in &traces {
        total += t.n_points;
        for s in &t.segments {
            for p in s {
                b[0] = b[0].min(p[0]);
                b[1] = b[1].max(p[0]);
                b[2] = b[2].min(p[1]);
                b[3] = b[3].max(p[1]);
            }
        }
    }
    let bounds = (b[0] <= b[1] && b[2] <= b[3]).then_some(b);
    if traces.is_empty() {
        warnings.push(if rows.is_empty() {
            "No row matches the filters".to_owned()
        } else {
            "No numeric point to plot".to_owned()
        });
    }

    let x_categories = x_cat.then(|| {
        let dict = xc.dict().unwrap_or(&[]);
        xc.sorted_codes()
            .unwrap_or(&[])
            .iter()
            .map(|&c| dict[c as usize].clone())
            .collect()
    });
    let x_label = if x_cat {
        cfg.x.clone()
    } else {
        cfg.x_scale.apply_to_label(&cfg.x)
    };
    let y_label = ycols
        .iter()
        .map(|c| cfg.y_scale.apply_to_label(&c.name))
        .collect::<Vec<_>>()
        .join(", ");

    PlotData {
        traces,
        rows,
        x_categories,
        x_label,
        y_label,
        bounds,
        warnings,
        error: None,
        total_points: total,
        compute_ms: t0.elapsed().as_secs_f64() * 1000.0,
        has_aggregates_and_originals: has_both,
        generation: ds.generation,
    }
}

/// Trace data in "long" CSV format: trace, x, y.
pub fn traces_to_csv(data: &PlotData, x_name: &str, y_name: &str) -> String {
    let mut out = String::with_capacity(data.total_points * 24 + 64);
    let q = |s: &str| {
        if s.contains(',') || s.contains('"') {
            format!("\"{}\"", s.replace('"', "\"\""))
        } else {
            s.to_owned()
        }
    };
    out.push_str(&format!("Trace,{},{}\n", q(x_name), q(y_name)));
    for t in &data.traces {
        let label = q(&t.label);
        for s in &t.segments {
            for p in s {
                out.push_str(&format!("{label},{},{}\n", p[0], p[1]));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::loader::{ImportOptions, parse_csv};
    use crate::plot::config::{FilterConfig, Modifier, ProcessConfig, Scale};

    fn sample() -> Dataset {
        let mut s = String::from("SN,Temp,Freq,S21\n");
        for sn in ["A", "B"] {
            for t in [25, -40] {
                for f in 0..5 {
                    let y = f as f64 + if sn == "A" { 0.0 } else { 1.0 } + if t == 25 { 0.0 } else { 10.0 };
                    s.push_str(&format!("{sn},{t},{}e9,{y}\n", 35 + f));
                }
            }
        }
        parse_csv(s.as_bytes(), &ImportOptions::default()).unwrap()
    }

    fn cfg() -> PlotConfig {
        PlotConfig {
            x: "Freq".into(),
            x_scale: Scale {
                modifier: Modifier::Giga,
                custom: 1.0,
            },
            ys: vec!["S21".into()],
            ..Default::default()
        }
    }

    #[test]
    fn series_and_filters() {
        let ds = sample();
        let mut c = cfg();
        c.series = vec!["SN".into()];
        c.filters = vec![FilterConfig {
            column: "Temp".into(),
            values: vec!["25".into()],
            ..Default::default()
        }];
        let d = compute(&ds, &c);
        assert!(d.error.is_none(), "{:?}", d.error);
        assert_eq!(d.traces.len(), 2);
        assert_eq!(d.traces[0].label, "SN=A");
        assert_eq!(d.traces[0].segments.len(), 1);
        assert_eq!(d.traces[0].segments[0][0], [35.0, 0.0]);
        assert_eq!(d.traces[1].segments[0][4], [39.0, 5.0]);
        assert_eq!(d.x_label, "Freq (GHz)");
        assert_eq!(d.rows.len(), 10);
    }

    #[test]
    fn reversal_split() {
        let ds = sample();
        let mut c = cfg();
        c.series = vec!["SN".into()];
        let d = compute(&ds, &c);
        // two temperatures per SN → two sweeps → two segments
        assert_eq!(d.traces[0].segments.len(), 2);
        c.options.sort_x = true;
        let d = compute(&ds, &c);
        assert_eq!(d.traces[0].segments.len(), 1);
    }

    #[test]
    fn smoothing_and_all_values_filter() {
        let ds = sample();
        let mut c = cfg();
        c.series = vec!["SN".into()];
        c.options.sort_x = true;
        c.filters = vec![FilterConfig {
            column: "Temp".into(),
            values: vec!["25".into(), "-40".into()],
            ..Default::default()
        }];
        let plain = compute(&ds, &c);
        c.process.smooth.enabled = true;
        c.process.smooth.method = crate::plot::smooth::SmoothMethod::MovingAverage;
        c.process.smooth.window = 3;
        let d = compute(&ds, &c);
        // raw curves kept (faded) + smoothed curves
        assert_eq!(d.traces.len(), 2 * plain.traces.len());
        assert!(d.has_aggregates_and_originals);
        let sm = d.traces.iter().find(|t| t.key.starts_with("smooth:")).unwrap();
        assert!(sm.label.contains("avg 3 pts"));
        // few points: drawn as a dense smooth curve
        assert!(sm.n_points >= plain.traces[0].n_points);
        c.process.smooth.keep_original = false;
        let d = compute(&ds, &c);
        assert_eq!(d.traces.len(), plain.traces.len());
        // every value of the filter selected = same rows as no filter
        c.filters.clear();
        assert_eq!(compute(&ds, &c).rows.len(), d.rows.len());
    }

    #[test]
    fn aggregation() {
        let ds = sample();
        let mut c = cfg();
        c.series = vec!["SN".into(), "Temp".into()];
        c.process = ProcessConfig {
            enabled: true,
            func: AggFunc::Mean,
            collapse: vec!["SN".into()],
            keep_original: false,
            ..Default::default()
        };
        let d = compute(&ds, &c);
        assert_eq!(d.traces.len(), 2);
        assert_eq!(d.traces[0].label, "Temp=-40 (mean)");
        assert_eq!(d.traces[0].segments[0][0], [35.0, 10.5]);
        c.process.func = AggFunc::Max;
        c.process.keep_original = true;
        let d = compute(&ds, &c);
        assert_eq!(d.traces.len(), 6);
        assert_eq!(d.traces[5].segments[0][0], [35.0, 1.0]);
    }

    #[test]
    fn categorical_x() {
        let ds = sample();
        let mut c = cfg();
        c.x = "SN".into();
        let d = compute(&ds, &c);
        assert_eq!(d.x_categories.as_deref(), Some(&["A".to_owned(), "B".to_owned()][..]));
    }

    #[test]
    fn segments() {
        let pts = vec![[0.0, 0.0], [1.0, 1.0], [2.0, 2.0], [0.0, 5.0], [1.0, 6.0]];
        let (s, sorted) = make_segments(pts.clone(), false, true);
        assert!(sorted);
        assert_eq!(s.len(), 2);
        let (s, sorted) = make_segments(pts, false, false);
        assert!(!sorted);
        assert_eq!(s.len(), 1);
        let (s, _) = make_segments(vec![[3.0, 0.0], [2.0, 1.0], [1.0, 2.0]], false, true);
        assert_eq!(s[0][0], [1.0, 2.0]);
    }
}
