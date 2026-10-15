//! Touchstone files (`.s1p`, `.s2p`, `.s3p` … `.snp`): reading as a data
//! source, detection of the S-parameter columns of a dataset and writing of
//! n-port files (Touchstone 1.x layout).

use std::path::Path;
use std::sync::Arc;

use super::dataset::{Column, Dataset, FileKind, LoadInfo, format_sig};
use super::deembed::C;

/// S-parameters of an n-port network.
pub struct Network {
    pub nports: usize,
    /// Frequencies (Hz), increasing.
    pub freq: Vec<f64>,
    /// `n × n` values per frequency, row-major: S(i+1)(j+1) = `s[f][i * n + j]`.
    pub s: Vec<Vec<C>>,
    /// Reference impedance (Ω).
    pub r_ref: f64,
    /// Format of the file (DB, MA, RI).
    pub format: String,
    /// Comment lines of the file (without the `!`).
    pub comments: Vec<String>,
}

/// Number of ports given by the extension: `.s4p` → 4.
pub fn ports_from_ext(path: &Path) -> Option<usize> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    let n = ext.strip_prefix('s')?.strip_suffix('p')?.parse::<usize>().ok()?;
    (1..=99).contains(&n).then_some(n)
}

pub fn is_touchstone(path: &Path) -> bool {
    ports_from_ext(path).is_some()
}

/// Parses a Touchstone file (version 1 or 2). `nports`: given by the
/// extension (a version 2 file gives it itself).
pub fn parse(text: &str, nports: Option<usize>) -> Result<Network, String> {
    let mut n = nports;
    let mut freq_mult = 1e9;
    let mut format = "MA".to_owned();
    let mut r_ref = 50.0;
    let mut option_seen = false;
    let mut order_12_21 = false;
    let mut matrix = "full".to_owned();
    let mut in_data = true;
    let mut comments = Vec::new();
    let mut numbers: Vec<f64> = Vec::new();
    for (ln, raw) in text.lines().enumerate() {
        let (data, comment) = match raw.split_once('!') {
            Some((d, c)) => (d, Some(c)),
            None => (raw, None),
        };
        if let Some(c) = comment
            && numbers.is_empty()
            && comments.len() < 40
            && !c.trim().is_empty()
        {
            comments.push(c.trim().to_owned());
        }
        let line = data.trim();
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
                    "R" => {
                        if let Some(r) = toks.get(k + 1).and_then(|t| t.parse::<f64>().ok()) {
                            r_ref = r;
                        }
                        k += 1;
                    }
                    _ => {}
                }
                k += 1;
            }
            continue;
        }
        if line.starts_with('[') {
            let low = line.to_ascii_lowercase();
            let value = |key: &str| low.trim_start_matches(key).trim().to_owned();
            if low.starts_with("[number of ports]") {
                n = value("[number of ports]").parse::<usize>().ok().or(n);
            } else if low.starts_with("[two-port data order]") {
                order_12_21 = low.contains("12_21");
            } else if low.starts_with("[matrix format]") {
                matrix = value("[matrix format]");
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
    let n = n.unwrap_or(2).max(1);
    // stored values: full matrix, or one triangle
    let stored: Vec<(usize, usize)> = match matrix.as_str() {
        "lower" => (0..n).flat_map(|i| (0..=i).map(move |j| (i, j))).collect(),
        "upper" => (0..n).flat_map(|i| (i..n).map(move |j| (i, j))).collect(),
        _ if n == 2 && !order_12_21 => vec![(0, 0), (1, 0), (0, 1), (1, 1)],
        _ => (0..n).flat_map(|i| (0..n).map(move |j| (i, j))).collect(),
    };
    let rec = 1 + 2 * stored.len();
    if numbers.len() < rec {
        return Err(format!("no {n}-port data found"));
    }
    let to_c = |a: f64, b: f64| match format.as_str() {
        "RI" => C::new(a, b),
        "DB" => C::from_db_deg(a, b),
        _ => {
            let r = b.to_radians();
            C::new(a * r.cos(), a * r.sin())
        }
    };
    let mut freq = Vec::new();
    let mut s = Vec::new();
    let mut i = 0;
    while i + rec <= numbers.len() {
        let f = numbers[i] * freq_mult;
        // noise parameters (two-port, version 1) follow with decreasing frequencies
        if freq.last().is_some_and(|&pf| f <= pf) {
            break;
        }
        let mut m = vec![C::new(0.0, 0.0); n * n];
        for (k, &(a, b)) in stored.iter().enumerate() {
            let v = to_c(numbers[i + 1 + 2 * k], numbers[i + 2 + 2 * k]);
            m[a * n + b] = v;
            m[b * n + a] = if a == b || matrix == "full" { m[b * n + a] } else { v };
        }
        freq.push(f);
        s.push(m);
        i += rec;
    }
    if freq.is_empty() {
        return Err(format!("no {n}-port data found"));
    }
    Ok(Network {
        nports: n,
        freq,
        s,
        r_ref,
        format,
        comments,
    })
}

/// Name of an S-parameter column: "S21db", "S21deg"; "S1_10db" above 9 ports.
pub fn column_name(i: usize, j: usize, deg: bool, nports: usize) -> String {
    let unit = if deg { "deg" } else { "db" };
    if nports > 9 {
        format!("S{i}_{j}{unit}")
    } else {
        format!("S{i}{j}{unit}")
    }
}

/// Dataset of a Touchstone file: Freq (Hz), then S11db, S11deg, S12db…
pub fn to_dataset(net: &Network) -> Dataset {
    let n = net.nports;
    let mut columns = vec![Arc::new(Column::from_values("Freq".to_owned(), net.freq.clone(), None))];
    for i in 0..n {
        for j in 0..n {
            let db: Vec<f64> = net
                .s
                .iter()
                .map(|m| m[i * n + j].abs().max(1e-20).log10() * 20.0)
                .collect();
            let deg: Vec<f64> = net.s.iter().map(|m| m[i * n + j].deg()).collect();
            columns.push(Arc::new(Column::from_values(
                column_name(i + 1, j + 1, false, n),
                db,
                None,
            )));
            columns.push(Arc::new(Column::from_values(
                column_name(i + 1, j + 1, true, n),
                deg,
                None,
            )));
        }
    }
    let mut preamble = vec![
        format!("Touchstone: {n} port(s), {} frequencies", net.freq.len()),
        format!(
            "Frequencies: {} … {} GHz",
            format_sig(net.freq[0] / 1e9, 6),
            format_sig(net.freq[net.freq.len() - 1] / 1e9, 6)
        ),
        format!("File format: {} · R = {} Ω", net.format, format_sig(net.r_ref, 6)),
    ];
    preamble.extend(net.comments.iter().cloned());
    Dataset {
        name: String::new(),
        path: Default::default(),
        columns,
        n_rows: net.freq.len(),
        preamble,
        info: LoadInfo {
            kind: FileKind::Touchstone(n),
            delimiter: None,
            header_line: None,
            ..Default::default()
        },
        generation: 0,
    }
}

/// Loads a `.snp` file as a dataset.
pub fn load(path: &Path, bytes: &[u8]) -> Result<Dataset, String> {
    let text = super::loader::decode_text(bytes);
    let net = parse(&text, ports_from_ext(path))?;
    Ok(to_dataset(&net))
}

// =============================================================================
// S-parameter columns of a dataset
// =============================================================================

/// Magnitude (dB) / phase (deg) columns of every S(i, j) found in a dataset.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SparamColumns {
    /// Highest port number found.
    pub nports: usize,
    /// (i, j, dB column, phase column), ports numbered from 1.
    pub found: Vec<(usize, usize, Option<usize>, Option<usize>)>,
}

impl SparamColumns {
    pub fn get(&self, i: usize, j: usize) -> (Option<usize>, Option<usize>) {
        self.found
            .iter()
            .find(|f| f.0 == i && f.1 == j)
            .map_or((None, None), |f| (f.2, f.3))
    }
}

/// "S21db" → (2, 1, false), "S21 [deg]" → (2, 1, true), "S1_10dB" → (1, 10, false).
pub fn parse_column_name(name: &str) -> Option<(usize, usize, bool)> {
    let rest = name.trim().strip_prefix(['S', 's'])?;
    let d1: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if d1.is_empty() {
        return None;
    }
    let mut rest = &rest[d1.len()..];
    let (i, j) = if let Some(r) = rest.strip_prefix(['_', ',']) {
        let d2: String = r.chars().take_while(|c| c.is_ascii_digit()).collect();
        if d2.is_empty() {
            return None;
        }
        rest = &r[d2.len()..];
        (d1.parse().ok()?, d2.parse().ok()?)
    } else if d1.len() == 2 {
        let b = d1.as_bytes();
        ((b[0] - b'0') as usize, (b[1] - b'0') as usize)
    } else {
        return None;
    };
    if i == 0 || j == 0 {
        return None;
    }
    let unit: String = rest
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect();
    match unit.as_str() {
        "db" | "mag" | "magdb" | "dbmag" => Some((i, j, false)),
        "deg" | "ang" | "angle" | "phase" | "phasedeg" | "phdeg" | "angdeg" => Some((i, j, true)),
        _ => None,
    }
}

/// The S-parameter columns of a dataset and the number of ports.
pub fn detect_sparams(ds: &Dataset) -> SparamColumns {
    let mut out = SparamColumns::default();
    for (k, c) in ds.columns.iter().enumerate() {
        let Some((i, j, deg)) = parse_column_name(&c.name) else {
            continue;
        };
        if !c.has_numbers() {
            continue;
        }
        let pos = match out.found.iter().position(|f| f.0 == i && f.1 == j) {
            Some(p) => p,
            None => {
                out.found.push((i, j, None, None));
                out.found.len() - 1
            }
        };
        let f = &mut out.found[pos];
        let slot = if deg { &mut f.3 } else { &mut f.2 };
        if slot.is_none() {
            *slot = Some(k);
        }
        out.nports = out.nports.max(i).max(j);
    }
    out.found.sort_by_key(|f| (f.0, f.1));
    out
}

// =============================================================================
// Writing
// =============================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Db,
    Ma,
    Ri,
}

/// One frequency of an n-port file (Touchstone 1.x layout): `vals` = (dB,
/// deg) row-major; two-port order S11 S21 S12 S22; matrix rows on their own
/// lines (4 pairs at most per line) above 2 ports.
pub fn write_point(out: &mut String, f: f64, vals: &[(f64, f64)], n: usize, format: Format) {
    use std::fmt::Write as _;
    let pair = |out: &mut String, (db, deg): (f64, f64)| {
        let mag = if db.is_finite() { 10f64.powf(db / 20.0) } else { 0.0 };
        let (a, b) = match format {
            Format::Db => (if db.is_finite() { db } else { -200.0 }, deg),
            Format::Ma => (mag, deg),
            Format::Ri => {
                let r = deg.to_radians();
                (mag * r.cos(), mag * r.sin())
            }
        };
        let _ = write!(out, " {} {}", format_sig(a, 9), format_sig(b, 9));
    };
    out.push_str(&format_sig(f, 12));
    if n <= 2 {
        let order: &[usize] = if n == 1 { &[0] } else { &[0, 2, 1, 3] };
        for &k in order {
            pair(out, vals[k]);
        }
        out.push('\n');
        return;
    }
    for i in 0..n {
        for j in 0..n {
            if j > 0 && j % 4 == 0 {
                out.push_str("\n ");
            }
            pair(out, vals[i * n + j]);
        }
        out.push('\n');
        if i + 1 < n {
            out.push(' ');
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn names() {
        assert_eq!(parse_column_name("S21db"), Some((2, 1, false)));
        assert_eq!(parse_column_name("S21 [deg]"), Some((2, 1, true)));
        assert_eq!(parse_column_name("s1_10dB"), Some((1, 10, false)));
        assert_eq!(parse_column_name("S21db raw"), None);
        assert_eq!(parse_column_name("SN"), None);
        assert_eq!(parse_column_name("S211db"), None);
        assert_eq!(ports_from_ext(Path::new("a.S4P")), Some(4));
        assert_eq!(ports_from_ext(Path::new("a.csv")), None);
    }

    #[test]
    fn three_port_round_trip() {
        // 3-port, 2 frequencies, written then read back
        let n = 3;
        let mut text = String::from("! test\n# GHz S DB R 50\n");
        let mut vals = Vec::new();
        for f in [1.0, 2.0] {
            let v: Vec<(f64, f64)> = (0..9).map(|k| (-(k as f64) - f, 10.0 * k as f64 + f)).collect();
            write_point(&mut text, f, &v, n, Format::Db);
            vals.push(v);
        }
        let net = parse(&text, Some(3)).unwrap();
        assert_eq!(net.nports, 3);
        assert_eq!(net.freq, vec![1e9, 2e9]);
        assert_eq!(net.comments, vec!["test".to_owned()]);
        for (m, v) in net.s.iter().zip(&vals) {
            for k in 0..9 {
                assert!(close(m[k].db(), v[k].0), "{k}");
                assert!(close(m[k].deg(), v[k].1), "{k}");
            }
        }
        let ds = to_dataset(&net);
        assert_eq!(ds.columns.len(), 1 + 18);
        assert_eq!(ds.columns[3].name, "S12db");
        let sp = detect_sparams(&ds);
        assert_eq!(sp.nports, 3);
        assert_eq!(sp.get(2, 1), (Some(7), Some(8)));
    }

    #[test]
    fn two_port_order_and_noise() {
        // version 1 two-port: S11 S21 S12 S22, then noise data (decreasing frequency)
        let text = "# MHz S MA R 50\n100 0.5 0 2 90 0.1 0 0.4 0\n200 0.5 0 2 80 0.1 0 0.4 0\n50 1 2 3 4\n";
        let net = parse(text, Some(2)).unwrap();
        assert_eq!(net.freq, vec![1e8, 2e8]);
        assert!(close(net.s[0][2].abs(), 2.0)); // S21 = row 2, column 1
        assert!(close(net.s[0][1].abs(), 0.1)); // S12
        // version 2, 12_21 order
        let text = "[Version] 2.0\n# GHz S RI R 50\n[Number of Ports] 2\n[Two-Port Data Order] 12_21\n[Network Data]\n1 0 0 0.1 0 2 0 0 0\n[End]\n";
        let net = parse(text, None).unwrap();
        assert!(close(net.s[0][1].re, 0.1));
        assert!(close(net.s[0][2].re, 2.0));
    }
}
