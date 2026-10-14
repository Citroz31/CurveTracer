//! Fast CSV / text loader.
//!
//! * the delimiter (`,` `;` tab `|`) and the header line are detected
//!   automatically, metadata lines above the header are kept as "preamble";
//! * the body is split in chunks parsed in parallel (one thread per core)
//!   when the file contains no quoted fields;
//! * every column is dictionary encoded while parsing: each cell costs a
//!   single hash lookup, numbers are only parsed once per unique value.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use super::dataset::{Column, Dataset, FileKind, LoadInfo, parse_number};
use super::hash::FxHashMap;

/// Import options; `None` means "detect automatically".
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ImportOptions {
    pub delimiter: Option<char>,
    /// 1-based line number of the header (0 = no header line).
    pub header_line: Option<usize>,
    pub decimal_comma: Option<bool>,
    pub sheet: Option<String>,
}

const SAMPLE_RECORDS: usize = 400;

/// Loads a CSV / text file or a spreadsheet.
pub fn load_path(path: &Path, opts: &ImportOptions) -> Result<Dataset, String> {
    let t0 = Instant::now();
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    let mut ds = match ext.as_str() {
        "xlsx" | "xlsm" | "xlsb" | "xls" | "ods" => super::excel::load_excel(path, opts)?,
        _ => {
            let bytes = std::fs::read(path).map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
            let mut ds = parse_csv(&bytes, opts)?;
            ds.info.file_bytes = bytes.len() as u64;
            ds
        }
    };
    ds.name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    ds.path = path.to_path_buf();
    ds.info.load_ms = t0.elapsed().as_secs_f64() * 1000.0;
    Ok(ds)
}

/// Decodes bytes as UTF-8, falling back to Latin-1 / Windows-1252.
pub fn decode_text(b: &[u8]) -> String {
    match std::str::from_utf8(b) {
        Ok(s) => s.to_owned(),
        Err(_) => b.iter().map(|&c| c as char).collect(),
    }
}

fn strip_bom(b: &[u8]) -> &[u8] {
    b.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(b)
}

/// Parses one record starting at `pos` and returns the position right after
/// the record terminator. Handles quoted fields (with `""` escapes and
/// embedded new lines).
fn parse_record<'a>(data: &'a [u8], mut pos: usize, delim: u8, out: &mut Vec<Cow<'a, [u8]>>) -> usize {
    let n = data.len();
    loop {
        if pos < n && data[pos] == b'"' {
            let start = pos + 1;
            let mut i = start;
            let mut seg_start = start;
            let mut owned: Option<Vec<u8>> = None;
            let field: Cow<'a, [u8]>;
            loop {
                match data[i.min(n)..].iter().position(|&b| b == b'"') {
                    None => {
                        field = match owned.take() {
                            Some(mut o) => {
                                o.extend_from_slice(&data[seg_start.min(n)..n]);
                                Cow::Owned(o)
                            }
                            None => Cow::Borrowed(&data[start.min(n)..n]),
                        };
                        i = n;
                        break;
                    }
                    Some(p) => {
                        let q = i + p;
                        if q + 1 < n && data[q + 1] == b'"' {
                            let o = owned.get_or_insert_with(Vec::new);
                            o.extend_from_slice(&data[seg_start..=q]);
                            i = q + 2;
                            seg_start = i;
                        } else {
                            field = match owned.take() {
                                Some(mut o) => {
                                    o.extend_from_slice(&data[seg_start..q]);
                                    Cow::Owned(o)
                                }
                                None => Cow::Borrowed(&data[start..q]),
                            };
                            i = q + 1;
                            break;
                        }
                    }
                }
            }
            while i < n && data[i] != delim && data[i] != b'\n' {
                i += 1;
            }
            out.push(field);
            pos = i;
        } else {
            let start = pos;
            let mut i = pos;
            while i < n && data[i] != delim && data[i] != b'\n' {
                i += 1;
            }
            let mut end = i;
            while end > start && (data[end - 1] == b'\r' || data[end - 1] == b' ') {
                end -= 1;
            }
            let mut s = start;
            while s < end && data[s] == b' ' {
                s += 1;
            }
            out.push(Cow::Borrowed(&data[s..end]));
            pos = i;
        }
        if pos >= n {
            return n;
        }
        if data[pos] == delim {
            pos += 1;
            if pos >= n {
                out.push(Cow::Borrowed(&[]));
                return n;
            }
            continue;
        }
        // new line
        return pos + 1;
    }
}

#[inline]
fn is_blank(fields: &[Cow<'_, [u8]>]) -> bool {
    fields
        .iter()
        .all(|f| f.iter().all(|&b| b == b' ' || b == b'\t' || b == b'\r'))
}

/// Number of fields up to the last non-empty one.
fn effective_count(fields: &[String]) -> usize {
    fields.iter().rposition(|s| !s.trim().is_empty()).map_or(0, |p| p + 1)
}

/// One column being built: dictionary + codes.
pub(crate) struct ColBuilder<'a> {
    map: FxHashMap<Cow<'a, [u8]>, u32>,
    dict: Vec<Cow<'a, [u8]>>,
    codes: Vec<u32>,
}

impl<'a> ColBuilder<'a> {
    pub(crate) fn new(capacity: usize) -> Self {
        ColBuilder {
            map: FxHashMap::default(),
            dict: Vec::new(),
            codes: Vec::with_capacity(capacity),
        }
    }

    #[inline]
    #[allow(clippy::ptr_arg)] // the Cow is cloned (cheaply) when the value is new
    pub(crate) fn push(&mut self, field: &Cow<'a, [u8]>) {
        let key: &[u8] = field.as_ref();
        let code = match self.map.get(key) {
            Some(&c) => c,
            None => {
                let c = self.dict.len() as u32;
                self.dict.push(field.clone());
                self.map.insert(field.clone(), c);
                c
            }
        };
        self.codes.push(code);
    }

    #[inline]
    pub(crate) fn push_bytes(&mut self, field: &[u8]) {
        let code = match self.map.get(field) {
            Some(&c) => c,
            None => {
                let c = self.dict.len() as u32;
                self.dict.push(Cow::Owned(field.to_vec()));
                self.map.insert(Cow::Owned(field.to_vec()), c);
                c
            }
        };
        self.codes.push(code);
    }
}

/// Merges the per-chunk builders of one column into a final column.
pub(crate) fn merge_column(name: String, parts: Vec<ColBuilder<'_>>, decimal_comma: bool) -> Column {
    let total: usize = parts.iter().map(|p| p.codes.len()).sum();
    let mut codes = Vec::with_capacity(total);
    let dict: Vec<String> = if parts.len() == 1 {
        let p = parts.into_iter().next().unwrap_or_else(|| ColBuilder::new(0));
        codes = p.codes;
        p.dict.iter().map(|b| decode_text(b)).collect()
    } else {
        let mut map: FxHashMap<&[u8], u32> = FxHashMap::default();
        let mut gdict: Vec<&[u8]> = Vec::new();
        for p in &parts {
            let remap: Vec<u32> = p
                .dict
                .iter()
                .map(|k| {
                    let k: &[u8] = k.as_ref();
                    *map.entry(k).or_insert_with(|| {
                        gdict.push(k);
                        (gdict.len() - 1) as u32
                    })
                })
                .collect();
            codes.extend(p.codes.iter().map(|&c| remap[c as usize]));
        }
        gdict.iter().map(|b| decode_text(b)).collect()
    };
    Column::from_dict(name, codes, dict, decimal_comma)
}

fn thread_count() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(1, 16)
}

/// Splits a buffer in roughly equal parts, cutting right after a new line.
fn split_chunks(body: &[u8], n: usize) -> Vec<&[u8]> {
    let mut out = Vec::with_capacity(n);
    let mut start = 0;
    let target = body.len() / n.max(1);
    for i in 1..n {
        let cut = (i * target).max(start);
        if cut >= body.len() {
            break;
        }
        match body[cut..].iter().position(|&b| b == b'\n') {
            Some(p) => {
                let end = cut + p + 1;
                out.push(&body[start..end]);
                start = end;
            }
            None => break,
        }
    }
    out.push(&body[start..]);
    out
}

fn parse_chunk<'a>(chunk: &'a [u8], delim: u8, n_cols: usize) -> Vec<ColBuilder<'a>> {
    let estimate = chunk.len() / (n_cols * 6 + 1) + 16;
    let mut cols: Vec<ColBuilder<'a>> = (0..n_cols).map(|_| ColBuilder::new(estimate)).collect();
    let empty: Cow<'a, [u8]> = Cow::Borrowed(&[]);
    let mut fields: Vec<Cow<'a, [u8]>> = Vec::with_capacity(n_cols + 4);
    let mut pos = 0;
    while pos < chunk.len() {
        fields.clear();
        pos = parse_record(chunk, pos, delim, &mut fields);
        if is_blank(&fields) {
            continue;
        }
        for (c, col) in cols.iter_mut().enumerate() {
            col.push(fields.get(c).unwrap_or(&empty));
        }
    }
    cols
}

/// Picks the delimiter that gives the most consistent number of fields on
/// the data lines of the sample.
fn detect_delimiter(lines: &[&[u8]]) -> u8 {
    let data_lines: Vec<&[u8]> = lines
        .iter()
        .copied()
        .filter(|l| l.iter().any(|&b| !b.is_ascii_whitespace()))
        .collect();
    let tail = &data_lines[data_lines.len().saturating_sub(60)..];
    let mut best = (b',', 0.0f64);
    for &cand in b",;\t|" {
        let counts: Vec<usize> = tail.iter().map(|l| count_outside_quotes(l, cand)).collect();
        let mut freq: BTreeMap<usize, usize> = BTreeMap::new();
        for &c in &counts {
            *freq.entry(c).or_default() += 1;
        }
        if let Some((&mode, &n)) = freq.iter().filter(|(c, _)| **c > 0).max_by_key(|(c, n)| (**n, **c)) {
            let score = n as f64 / counts.len().max(1) as f64 * 1000.0 + mode as f64;
            if score > best.1 {
                best = (cand, score);
            }
        }
    }
    best.0
}

fn count_outside_quotes(line: &[u8], delim: u8) -> usize {
    let mut inside = false;
    let mut n = 0;
    for &b in line {
        if b == b'"' {
            inside = !inside;
        } else if b == delim && !inside {
            n += 1;
        }
    }
    n
}

/// Result of the header detection.
pub(crate) struct HeaderGuess {
    /// Index (in the sample) of the header record, `None` if there is no header.
    pub header: Option<usize>,
    /// Index of the first data record.
    pub first_data: usize,
    pub n_cols: usize,
}

/// Header detection on decoded sample records (blank records included).
pub(crate) fn detect_header(samples: &[Vec<String>], forced: Option<usize>) -> HeaderGuess {
    let eff: Vec<usize> = samples.iter().map(|f| effective_count(f)).collect();
    let max_after = |from: usize| eff.iter().skip(from).copied().max().unwrap_or(0);
    if let Some(line) = forced {
        if line == 0 {
            let first = eff.iter().position(|&e| e > 0).unwrap_or(0);
            return HeaderGuess {
                header: None,
                first_data: first,
                n_cols: max_after(first),
            };
        }
        let h = (line - 1).min(samples.len().saturating_sub(1));
        return HeaderGuess {
            header: Some(h),
            first_data: h + 1,
            n_cols: eff[h].max(max_after(h + 1)),
        };
    }
    let mut freq: BTreeMap<usize, usize> = BTreeMap::new();
    for &e in &eff {
        if e >= 2 {
            *freq.entry(e).or_default() += 1;
        }
    }
    let mode = freq
        .iter()
        .max_by_key(|(c, n)| (**n, **c))
        .map(|(c, _)| *c)
        .unwrap_or_else(|| eff.iter().copied().max().unwrap_or(0));
    let mut header = None;
    for (i, &e) in eff.iter().enumerate() {
        if e == 0 || e < mode {
            continue;
        }
        // the next non-blank record must look like data as well
        let next = eff.iter().skip(i + 1).copied().find(|&e| e > 0);
        if let Some(next) = next
            && next * 2 < mode
        {
            continue;
        }
        header = Some(i);
        break;
    }
    let Some(h) = header else {
        let first = eff.iter().position(|&e| e > 0).unwrap_or(0);
        return HeaderGuess {
            header: None,
            first_data: first,
            n_cols: max_after(first),
        };
    };
    // A "header" made of numbers is in fact the first data line.
    let numeric = samples[h].iter().filter(|s| parse_number(s, true).is_some()).count();
    if numeric * 2 > eff[h] {
        return HeaderGuess {
            header: None,
            first_data: h,
            n_cols: max_after(h),
        };
    }
    HeaderGuess {
        header: Some(h),
        first_data: h + 1,
        n_cols: eff[h].max(max_after(h + 1)),
    }
}

/// Builds unique, non-empty column names.
pub(crate) fn make_names(header: Option<&Vec<String>>, n_cols: usize) -> Vec<String> {
    let mut names = Vec::with_capacity(n_cols);
    let mut seen: FxHashMap<String, usize> = FxHashMap::default();
    for i in 0..n_cols {
        let raw = header
            .and_then(|h| h.get(i))
            .map(|s| s.trim().to_owned())
            .unwrap_or_default();
        let base = if raw.is_empty() {
            format!("Column {}", i + 1)
        } else {
            raw
        };
        let count = seen.entry(base.clone()).or_insert(0);
        *count += 1;
        let name = if *count == 1 { base } else { format!("{base} ({count})") };
        names.push(name);
    }
    names
}

pub(crate) fn preamble_lines(samples: &[Vec<String>], upto: usize) -> Vec<String> {
    samples
        .iter()
        .take(upto)
        .filter_map(|f| {
            let n = effective_count(f);
            if n == 0 {
                return None;
            }
            Some(f[..n].iter().map(|s| s.trim()).collect::<Vec<_>>().join("  "))
        })
        .collect()
}

/// Automatic decimal comma detection on the sample data records.
pub(crate) fn detect_decimal_comma(samples: &[Vec<String>], from: usize) -> bool {
    let mut comma = 0;
    let mut dot = 0;
    for f in samples.iter().skip(from) {
        for s in f {
            let t = s.trim();
            if t.parse::<f64>().is_ok() {
                if t.contains('.') {
                    dot += 1;
                }
            } else if parse_number(t, true).is_some() {
                comma += 1;
            }
        }
    }
    comma > dot
}

/// Parses CSV / delimited text bytes.
pub fn parse_csv(bytes: &[u8], opts: &ImportOptions) -> Result<Dataset, String> {
    let bytes = strip_bom(bytes);
    if bytes.iter().all(|b| b.is_ascii_whitespace()) {
        return Err("The file is empty".to_owned());
    }

    // --- sample: raw lines for the delimiter, then records -----------------
    let mut raw_lines: Vec<&[u8]> = Vec::new();
    {
        let mut start = 0;
        while start < bytes.len() && raw_lines.len() < SAMPLE_RECORDS {
            let end = bytes[start..]
                .iter()
                .position(|&b| b == b'\n')
                .map_or(bytes.len(), |p| start + p);
            raw_lines.push(&bytes[start..end]);
            start = end + 1;
        }
    }
    let delim = match opts.delimiter {
        Some(c) if c.is_ascii() => c as u8,
        _ => detect_delimiter(&raw_lines),
    };

    let wanted = SAMPLE_RECORDS.max(opts.header_line.unwrap_or(0) + 50);
    let mut samples: Vec<Vec<String>> = Vec::new();
    let mut offsets: Vec<usize> = Vec::new(); // byte offset after each record
    {
        let mut pos = 0;
        let mut fields = Vec::new();
        while pos < bytes.len() && samples.len() < wanted {
            fields.clear();
            pos = parse_record(bytes, pos, delim, &mut fields);
            samples.push(fields.iter().map(|f| decode_text(f)).collect());
            offsets.push(pos);
        }
    }
    let guess = detect_header(&samples, opts.header_line);
    let names = make_names(guess.header.map(|h| &samples[h]), guess.n_cols);
    if names.is_empty() {
        return Err("No column found in the file".to_owned());
    }
    let decimal_comma = opts
        .decimal_comma
        .unwrap_or_else(|| delim != b',' && detect_decimal_comma(&samples, guess.first_data));
    let preamble = preamble_lines(&samples, guess.header.unwrap_or(guess.first_data));
    let body_start = if guess.first_data == 0 {
        0
    } else {
        offsets[guess.first_data - 1]
    };
    let body = &bytes[body_start.min(bytes.len())..];

    // --- body: parallel chunks when there are no quotes -------------------
    let n_cols = names.len();
    let threads = thread_count();
    let has_quotes = body.contains(&b'"');
    let chunks = if !has_quotes && body.len() > (1 << 20) && threads > 1 {
        split_chunks(body, threads)
    } else {
        vec![body]
    };
    let parsed: Vec<Vec<ColBuilder<'_>>> = if chunks.len() == 1 {
        vec![parse_chunk(chunks[0], delim, n_cols)]
    } else {
        std::thread::scope(|s| {
            let handles: Vec<_> = chunks
                .iter()
                .map(|&c| s.spawn(move || parse_chunk(c, delim, n_cols)))
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().unwrap_or_else(|_| Vec::new()))
                .collect()
        })
    };
    if parsed.iter().any(|p| p.len() != n_cols) {
        return Err("Internal error while parsing the file".to_owned());
    }

    // transpose: per column, the list of chunk builders
    let mut per_col: Vec<Vec<ColBuilder<'_>>> = (0..n_cols).map(|_| Vec::new()).collect();
    for chunk_cols in parsed {
        for (c, b) in chunk_cols.into_iter().enumerate() {
            per_col[c].push(b);
        }
    }
    let columns = merge_all(names, per_col, decimal_comma, threads);
    let n_rows = columns.first().map_or(0, |c| c.len());

    Ok(Dataset {
        name: String::new(),
        path: Default::default(),
        columns: columns.into_iter().map(Arc::new).collect(),
        n_rows,
        preamble,
        info: LoadInfo {
            kind: FileKind::Csv,
            delimiter: Some(delim as char),
            header_line: guess.header.map(|h| h + 1),
            decimal_comma,
            ..Default::default()
        },
        generation: 0,
    })
}

/// Merges all columns, in parallel.
pub(crate) fn merge_all(
    names: Vec<String>,
    per_col: Vec<Vec<ColBuilder<'_>>>,
    decimal_comma: bool,
    threads: usize,
) -> Vec<Column> {
    let n = names.len();
    let mut work: Vec<Vec<(usize, String, Vec<ColBuilder<'_>>)>> = (0..threads.max(1)).map(|_| Vec::new()).collect();
    for (i, (name, parts)) in names.into_iter().zip(per_col).enumerate() {
        work[i % threads.max(1)].push((i, name, parts));
    }
    let mut done: Vec<(usize, Column)> = std::thread::scope(|s| {
        let handles: Vec<_> = work
            .into_iter()
            .map(|items| {
                s.spawn(move || {
                    items
                        .into_iter()
                        .map(|(i, name, parts)| (i, merge_column(name, parts, decimal_comma)))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
    });
    done.sort_by_key(|(i, _)| *i);
    debug_assert_eq!(done.len(), n);
    done.into_iter().map(|(_, c)| c).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "Test Request ID,1234,,,\n\
Operator,John,,,\n\
\n\
,,,,\n\
Data Timestamp,RunID,SN,Set_TempC,Freq,S21db,Comment\n\
2026-10-09 14-21-42,-1,07102601,-40.0,35000000000.0,16.39,RIBBON\n\
2026-10-09 14-21-42,-1,07102601,-40.0,35100000000.0,16.49,RIBBON\n\
2026-10-09 14-21-43,-1,07102602,25,35000000000.0,15.5,\"A, quoted\"\"x\"\"\"\n\
2026-10-09 14-21-43,-1,07102602,25,35100000000.0,N/A,RIBBON\n";

    #[test]
    fn parse_sample() {
        let ds = parse_csv(SAMPLE.as_bytes(), &ImportOptions::default()).unwrap();
        assert_eq!(ds.info.delimiter, Some(','));
        assert_eq!(ds.info.header_line, Some(5));
        assert_eq!(ds.n_rows, 4);
        assert_eq!(ds.columns.len(), 7);
        assert_eq!(
            ds.preamble,
            vec!["Test Request ID  1234".to_owned(), "Operator  John".to_owned()]
        );
        let sn = ds.column("SN").unwrap();
        assert_eq!(sn.text(0), "07102601");
        assert_eq!(sn.n_unique, 2);
        let f = ds.column("Freq").unwrap();
        assert_eq!(f.value(1), 35.1e9);
        let s21 = ds.column("S21db").unwrap();
        assert!(s21.value(3).is_nan());
        assert_eq!(s21.value(0), 16.39);
        let c = ds.column("Comment").unwrap();
        assert_eq!(c.text(2), "A, quoted\"x\"");
        assert_eq!(c.n_unique, 2);
    }

    #[test]
    fn semicolon_decimal_comma() {
        let text = "a;b;c\n1,5;2;x\n2,5;3;y\n";
        let ds = parse_csv(text.as_bytes(), &ImportOptions::default()).unwrap();
        assert_eq!(ds.info.delimiter, Some(';'));
        assert!(ds.info.decimal_comma);
        assert_eq!(ds.column("a").unwrap().value(1), 2.5);
    }

    #[test]
    fn no_header() {
        let text = "1,2,3\n4,5,6\n";
        let ds = parse_csv(text.as_bytes(), &ImportOptions::default()).unwrap();
        assert_eq!(ds.info.header_line, None);
        assert_eq!(ds.n_rows, 2);
        assert_eq!(ds.columns[0].name, "Column 1");
    }

    #[test]
    fn forced_header() {
        let opts = ImportOptions {
            header_line: Some(2),
            ..Default::default()
        };
        let text = "junk\nx,y\n1,2\n";
        let ds = parse_csv(text.as_bytes(), &opts).unwrap();
        assert_eq!(ds.columns[1].name, "y");
        assert_eq!(ds.n_rows, 1);
    }

    #[test]
    fn parallel_matches_sequential() {
        let mut text = String::from("Freq,SN,Val\n");
        for i in 0..200_000 {
            text.push_str(&format!("{},{},{}\n", (i % 1000) as f64 * 1e7, i % 7, i as f64 * 0.5));
        }
        let ds = parse_csv(text.as_bytes(), &ImportOptions::default()).unwrap();
        assert_eq!(ds.n_rows, 200_000);
        let v = ds.column("Val").unwrap();
        assert!(!v.is_dict());
        for i in [0usize, 1, 99_999, 150_001, 199_999] {
            assert_eq!(v.value(i), i as f64 * 0.5);
            assert_eq!(ds.column("SN").unwrap().value(i), (i % 7) as f64);
            assert_eq!(ds.column("Freq").unwrap().value(i), (i % 1000) as f64 * 1e7);
        }
        assert_eq!(ds.column("SN").unwrap().n_unique, 7);
    }

    #[test]
    fn split_is_complete() {
        let body = b"a,b\nc,d\ne,f\ng,h\n";
        let chunks = split_chunks(body, 3);
        let joined: Vec<u8> = chunks.concat();
        assert_eq!(joined, body.to_vec());
        assert!(chunks.iter().all(|c| c.is_empty() || c.ends_with(b"\n")));
    }
}
