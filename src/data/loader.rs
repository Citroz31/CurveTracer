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

/// Above this many distinct values in a chunk, a column made of numbers
/// switches from the dictionary to plain numeric values.
const NUMERIC_SWITCH: usize = 4096;

/// Above this many distinct texts, a numeric column with texts is kept as a
/// (mixed) dictionary column instead of treating the texts as missing values.
const MAX_TEXTS_IN_NUMERIC: usize = 16;

/// Exact powers of ten (as f64) for the fast decimal path.
const POW10: [f64; 23] = [
    1e0, 1e1, 1e2, 1e3, 1e4, 1e5, 1e6, 1e7, 1e8, 1e9, 1e10, 1e11, 1e12, 1e13, 1e14, 1e15, 1e16, 1e17, 1e18, 1e19, 1e20,
    1e21, 1e22,
];

/// Fast path for the usual decimal numbers ("-3.5945252", "35000000000.0",
/// "1.5e-3"): integer mantissa and one exact multiplication / division by a
/// power of ten, which is correctly rounded when the mantissa is below 2^53
/// and the exponent within ±22 (Clinger). `None`: use the full parser.
#[inline]
fn fast_decimal(t: &[u8]) -> Option<f64> {
    let n = t.len();
    let mut i = 0;
    let neg = match t.first()? {
        b'-' => {
            i = 1;
            true
        }
        b'+' => {
            i = 1;
            false
        }
        _ => false,
    };
    let mut mant: u64 = 0;
    let mut digits = 0u32; // significant digits
    let mut exp: i32 = 0;
    let mut any = false;
    while i < n && t[i].is_ascii_digit() {
        let d = (t[i] - b'0') as u64;
        if mant != 0 || d != 0 {
            digits += 1;
        }
        mant = mant.wrapping_mul(10).wrapping_add(d);
        any = true;
        i += 1;
    }
    if i < n && t[i] == b'.' {
        i += 1;
        while i < n && t[i].is_ascii_digit() {
            let d = (t[i] - b'0') as u64;
            if mant != 0 || d != 0 {
                digits += 1;
            }
            mant = mant.wrapping_mul(10).wrapping_add(d);
            exp -= 1;
            any = true;
            i += 1;
        }
    }
    if !any || digits > 18 {
        return None;
    }
    if i < n && (t[i] == b'e' || t[i] == b'E') {
        i += 1;
        let eneg = match t.get(i) {
            Some(b'-') => {
                i += 1;
                true
            }
            Some(b'+') => {
                i += 1;
                false
            }
            _ => false,
        };
        let start = i;
        let mut e: i32 = 0;
        while i < n && t[i].is_ascii_digit() && i - start < 5 {
            e = e * 10 + (t[i] - b'0') as i32;
            i += 1;
        }
        if i == start {
            return None;
        }
        exp += if eneg { -e } else { e };
    }
    if i != n {
        return None;
    }
    if mant > (1u64 << 53) || !(-22..=22).contains(&exp) {
        return None;
    }
    let m = mant as f64;
    let v = if exp >= 0 {
        m * POW10[exp as usize]
    } else {
        m / POW10[(-exp) as usize]
    };
    Some(if neg { -v } else { v })
}

/// Number from raw bytes (same rules as [`parse_number`]).
#[inline]
pub(crate) fn parse_number_bytes(b: &[u8], decimal_comma: bool) -> Option<f64> {
    let t = b.trim_ascii();
    let first = *t.first()?;
    if !(first.is_ascii_digit() || first == b'-' || first == b'+' || first == b'.' || first == b',') {
        return None;
    }
    if let Some(v) = fast_decimal(t) {
        return Some(v);
    }
    let s = std::str::from_utf8(t).ok()?;
    if let Ok(v) = s.parse::<f64>() {
        return Some(v);
    }
    if decimal_comma {
        return parse_number(s, true);
    }
    None
}

/// One column being built. It starts as a dictionary (one hash lookup per
/// cell, ideal for repeated values: SN, temperature, frequency…) and switches
/// to plain numbers when it holds many distinct numbers (measurements), which
/// avoids huge hash maps and the conversion of every value to text.
pub(crate) struct ColBuilder<'a> {
    map: FxHashMap<Cow<'a, [u8]>, u32>,
    dict: Vec<Cow<'a, [u8]>>,
    codes: Vec<u32>,
    /// Numeric mode: the values.
    values: Option<Vec<f64>>,
    /// Numeric mode: cells that are not numbers (row in the chunk, text).
    texts: Vec<(u32, Cow<'a, [u8]>)>,
    /// The column has too much text to switch to numbers.
    no_switch: bool,
    decimal_comma: bool,
}

impl<'a> ColBuilder<'a> {
    pub(crate) fn new(capacity: usize, decimal_comma: bool) -> Self {
        ColBuilder {
            map: FxHashMap::default(),
            dict: Vec::new(),
            codes: Vec::with_capacity(capacity),
            values: None,
            texts: Vec::new(),
            no_switch: false,
            decimal_comma,
        }
    }

    #[inline]
    fn push_number(&mut self, field: &[u8], owned: impl FnOnce() -> Cow<'a, [u8]>) -> bool {
        let Some(values) = &mut self.values else {
            return false;
        };
        match parse_number_bytes(field, self.decimal_comma) {
            Some(v) => values.push(v),
            None => {
                if !field.trim_ascii().is_empty() {
                    self.texts.push((values.len() as u32, owned()));
                }
                values.push(f64::NAN);
            }
        }
        true
    }

    #[inline]
    #[allow(clippy::ptr_arg)] // the Cow is cloned (cheaply) when the value is new
    pub(crate) fn push(&mut self, field: &Cow<'a, [u8]>) {
        let key: &[u8] = field.as_ref();
        if self.push_number(key, || field.clone()) {
            return;
        }
        // same value as the previous row (very common): no hashing
        if let Some(&last) = self.codes.last()
            && self.dict[last as usize].as_ref() == key
        {
            self.codes.push(last);
            return;
        }
        let code = match self.map.get(key) {
            Some(&c) => c,
            None => {
                let c = self.dict.len() as u32;
                self.dict.push(field.clone());
                self.map.insert(field.clone(), c);
                self.maybe_switch();
                c
            }
        };
        self.push_code(code);
    }

    #[inline]
    pub(crate) fn push_bytes(&mut self, field: &[u8]) {
        if self.push_number(field, || Cow::Owned(field.to_vec())) {
            return;
        }
        let code = match self.map.get(field) {
            Some(&c) => c,
            None => {
                let c = self.dict.len() as u32;
                self.dict.push(Cow::Owned(field.to_vec()));
                self.map.insert(Cow::Owned(field.to_vec()), c);
                self.maybe_switch();
                c
            }
        };
        self.push_code(code);
    }

    #[inline]
    fn push_code(&mut self, code: u32) {
        match &mut self.values {
            // switched while adding this (new) value
            Some(values) => {
                let field = self.dict[code as usize].clone();
                values.push(f64::NAN);
                let row = values.len() - 1;
                if let Some(v) = parse_number_bytes(&field, self.decimal_comma) {
                    values[row] = v;
                } else if !field.trim_ascii().is_empty() {
                    self.texts.push((row as u32, field));
                }
                self.dict.clear();
            }
            None => self.codes.push(code),
        }
    }

    /// Switches to numeric mode when the dictionary gets big and is (mostly) numbers.
    fn maybe_switch(&mut self) {
        if self.no_switch || self.dict.len() <= NUMERIC_SWITCH {
            return;
        }
        let nums: Vec<f64> = self
            .dict
            .iter()
            .map(|d| parse_number_bytes(d, self.decimal_comma).unwrap_or(f64::NAN))
            .collect();
        let n_text = self
            .dict
            .iter()
            .zip(&nums)
            .filter(|(d, v)| v.is_nan() && !d.trim_ascii().is_empty())
            .count();
        if n_text > MAX_TEXTS_IN_NUMERIC {
            self.no_switch = true;
            return;
        }
        let mut values = Vec::with_capacity(self.codes.capacity().max(self.codes.len() + 1));
        for (row, &c) in self.codes.iter().enumerate() {
            let v = nums[c as usize];
            if v.is_nan() && !self.dict[c as usize].trim_ascii().is_empty() {
                self.texts.push((row as u32, self.dict[c as usize].clone()));
            }
            values.push(v);
        }
        self.values = Some(values);
        self.codes = Vec::new();
        self.map = FxHashMap::default();
        // the dictionary is cleared once the current value is stored (push_code)
    }
}

/// Merges the per-chunk builders of one column into a final column.
pub(crate) fn merge_column(name: String, parts: Vec<ColBuilder<'_>>, decimal_comma: bool) -> Column {
    if parts.iter().any(|p| p.values.is_some()) {
        return merge_numeric(name, parts, decimal_comma);
    }
    let total: usize = parts.iter().map(|p| p.codes.len()).sum();
    let mut codes = Vec::with_capacity(total);
    let dict: Vec<String> = if parts.len() == 1 {
        let p = parts
            .into_iter()
            .next()
            .unwrap_or_else(|| ColBuilder::new(0, decimal_comma));
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

/// Merge when at least one chunk switched to numbers.
fn merge_numeric(name: String, parts: Vec<ColBuilder<'_>>, decimal_comma: bool) -> Column {
    let total: usize = parts
        .iter()
        .map(|p| p.values.as_ref().map_or(p.codes.len(), |v| v.len()))
        .sum();
    let mut values: Vec<f64> = Vec::with_capacity(total);
    let mut texts: Vec<(usize, &[u8])> = Vec::new();
    for p in &parts {
        let offset = values.len();
        match &p.values {
            Some(v) => {
                values.extend_from_slice(v);
                texts.extend(p.texts.iter().map(|(r, t)| (offset + *r as usize, t.as_ref())));
            }
            None => {
                let nums: Vec<f64> = p
                    .dict
                    .iter()
                    .map(|d| parse_number_bytes(d, decimal_comma).unwrap_or(f64::NAN))
                    .collect();
                for (row, &c) in p.codes.iter().enumerate() {
                    let v = nums[c as usize];
                    let d: &[u8] = p.dict[c as usize].as_ref();
                    if v.is_nan() && !d.trim_ascii().is_empty() {
                        texts.push((offset + row, d));
                    }
                    values.push(v);
                }
            }
        }
    }
    let mut distinct: Vec<&[u8]> = texts.iter().map(|(_, t)| *t).collect();
    distinct.sort_unstable();
    distinct.dedup();
    if distinct.len() <= MAX_TEXTS_IN_NUMERIC {
        // a few markers ("N/A", "FAIL"…) in a measurement column: missing values
        return Column::from_values(name, values, None);
    }
    // really mixed: dictionary of the numbers (as text) and the texts
    let mut is_text = vec![false; values.len()];
    for (r, _) in &texts {
        is_text[*r] = true;
    }
    let mut text_at: FxHashMap<usize, &[u8]> = FxHashMap::default();
    text_at.extend(texts.iter().copied());
    let mut map: FxHashMap<Vec<u8>, u32> = FxHashMap::default();
    let mut dict: Vec<String> = Vec::new();
    let mut codes = Vec::with_capacity(values.len());
    for (row, v) in values.iter().enumerate() {
        let key: Vec<u8> = if is_text[row] {
            text_at[&row].to_vec()
        } else {
            super::dataset::format_value(*v).into_bytes()
        };
        let next = dict.len() as u32;
        let code = *map.entry(key).or_insert_with_key(|k| {
            dict.push(decode_text(k));
            next
        });
        codes.push(code);
    }
    Column::from_dict(name, codes, dict, decimal_comma)
}

fn thread_count() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(1, 16)
}

/// Splits a buffer in roughly equal parts, cutting right after a new line
/// that is not inside a quoted field (the quotes before the cut are counted).
fn split_chunks(body: &[u8], n: usize) -> Vec<&[u8]> {
    let quotes = |b: &[u8]| memchr::memchr_iter(b'"', b).count();
    let has_quotes = memchr::memchr(b'"', body).is_some();
    let mut out = Vec::with_capacity(n);
    let mut start = 0;
    // quotes counted up to `counted`
    let mut counted = 0;
    let mut parity = 0;
    let target = body.len() / n.max(1);
    for i in 1..n {
        let mut cut = (i * target).max(start);
        if has_quotes {
            parity = (parity + quotes(&body[counted..cut.min(body.len())])) % 2;
            counted = cut.min(body.len());
        }
        let end = loop {
            if cut >= body.len() {
                break None;
            }
            let Some(p) = memchr::memchr(b'\n', &body[cut..]) else {
                break None;
            };
            let nl = cut + p;
            if has_quotes {
                parity = (parity + quotes(&body[counted..nl])) % 2;
                counted = nl;
            }
            if parity == 0 {
                break Some(nl + 1);
            }
            cut = nl + 1; // new line inside a quoted field: try the next one
        };
        let Some(end) = end else {
            break;
        };
        out.push(&body[start..end]);
        start = end;
    }
    out.push(&body[start..]);
    out
}

/// Trims the spaces (and the `\r` of Windows line ends) around a field.
#[inline]
fn trim_field(f: &[u8]) -> &[u8] {
    let mut s = 0;
    let mut e = f.len();
    while e > s && (f[e - 1] == b'\r' || f[e - 1] == b' ') {
        e -= 1;
    }
    while s < e && f[s] == b' ' {
        s += 1;
    }
    &f[s..e]
}

/// Fast path for text without quotes: lines and fields are found with
/// vectorized byte searches, without building a list of fields per line.
fn parse_chunk_unquoted<'a>(chunk: &'a [u8], delim: u8, cols: &mut [ColBuilder<'a>]) {
    let n_cols = cols.len();
    let empty: Cow<'a, [u8]> = Cow::Borrowed(&[]);
    let mut start = 0;
    while start < chunk.len() {
        let end = memchr::memchr(b'\n', &chunk[start..]).map_or(chunk.len(), |p| start + p);
        let line = &chunk[start..end];
        start = end + 1;
        if line.iter().all(|&b| matches!(b, b' ' | b'\t' | b'\r') || b == delim) {
            continue; // blank line (only empty fields)
        }
        let mut c = 0;
        let mut f0 = 0;
        for p in memchr::memchr_iter(delim, line) {
            if c < n_cols {
                cols[c].push(&Cow::Borrowed(trim_field(&line[f0..p])));
            }
            c += 1;
            f0 = p + 1;
        }
        if c < n_cols {
            cols[c].push(&Cow::Borrowed(trim_field(&line[f0..])));
            c += 1;
        }
        while c < n_cols {
            cols[c].push(&empty);
            c += 1;
        }
    }
}

fn parse_chunk<'a>(chunk: &'a [u8], delim: u8, n_cols: usize, decimal_comma: bool) -> Vec<ColBuilder<'a>> {
    let estimate = chunk.len() / (n_cols * 6 + 1) + 16;
    let mut cols: Vec<ColBuilder<'a>> = (0..n_cols).map(|_| ColBuilder::new(estimate, decimal_comma)).collect();
    if memchr::memchr(b'"', chunk).is_none() {
        parse_chunk_unquoted(chunk, delim, &mut cols);
    } else {
        parse_chunk_records(chunk, delim, &mut cols);
    }
    cols
}

/// General path: records with quoted fields (delimiters / new lines inside quotes).
fn parse_chunk_records<'a>(chunk: &'a [u8], delim: u8, cols: &mut [ColBuilder<'a>]) {
    let n_cols = cols.len();
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
    let chunks = if body.len() > (1 << 20) && threads > 1 {
        split_chunks(body, threads)
    } else {
        vec![body]
    };
    let parsed: Vec<Vec<ColBuilder<'_>>> = if chunks.len() == 1 {
        vec![parse_chunk(chunks[0], delim, n_cols, decimal_comma)]
    } else {
        std::thread::scope(|s| {
            let handles: Vec<_> = chunks
                .iter()
                .map(|&c| s.spawn(move || parse_chunk(c, delim, n_cols, decimal_comma)))
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

/// Merges all columns, in parallel (each thread takes the next column to do,
/// the expensive ones first).
pub(crate) fn merge_all(
    names: Vec<String>,
    per_col: Vec<Vec<ColBuilder<'_>>>,
    decimal_comma: bool,
    threads: usize,
) -> Vec<Column> {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let n = names.len();
    type Job<'a> = (usize, String, Vec<ColBuilder<'a>>);
    let mut jobs: Vec<Job<'_>> = names
        .into_iter()
        .zip(per_col)
        .enumerate()
        .map(|(i, (name, parts))| (i, name, parts))
        .collect();
    // numeric columns and big dictionaries take the longest
    let cost = |parts: &[ColBuilder<'_>]| -> usize {
        parts
            .iter()
            .map(|p| p.values.as_ref().map_or(p.dict.len(), |v| v.len()))
            .sum()
    };
    jobs.sort_by_key(|(_, _, parts)| std::cmp::Reverse(cost(parts)));
    let queue: Vec<Mutex<Option<Job<'_>>>> = jobs.into_iter().map(|j| Mutex::new(Some(j))).collect();
    let next = AtomicUsize::new(0);
    let mut done: Vec<(usize, Column)> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..threads.clamp(1, n.max(1)))
            .map(|_| {
                s.spawn(|| {
                    let mut out = Vec::new();
                    loop {
                        let k = next.fetch_add(1, Ordering::Relaxed);
                        let Some(slot) = queue.get(k) else {
                            break;
                        };
                        let job = slot.lock().ok().and_then(|mut g| g.take());
                        if let Some((i, name, parts)) = job {
                            out.push((i, merge_column(name, parts, decimal_comma)));
                        }
                    }
                    out
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
    use crate::data::ColumnKind;

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
    fn fast_decimal_is_exact() {
        // same result as the standard parser, bit for bit
        let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        let mut cases: Vec<String> = [
            "0",
            "-0",
            "0.0",
            "1",
            "-1.5",
            "+2.25",
            ".5",
            "5.",
            "35000000000.0",
            "1e9",
            "1.5e-3",
            "-3.5945252",
            "123456789012345678",
            "1234567890123456789",
            "0.000000000000000000001",
            "9007199254740993",
            "1e22",
            "1e23",
            "4.35E+02",
            "1e",
            "1.2.3",
            "-",
            ".",
            "12a",
            "1e-400",
            "0.1",
            "0.3",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        for _ in 0..200_000 {
            let r = next();
            let int = (r % 1_000_000_000) as i64 - 500_000_000;
            let dec = (r >> 32) % 10;
            let s = match r % 4 {
                0 => format!("{}", int as f64 / 10f64.powi(dec as i32)),
                1 => format!("{:.*}", dec as usize, f64::from_bits(next() >> 2) % 1e6),
                2 => format!("{}e{}", int, (r >> 40) % 40),
                _ => format!("{:e}", (int as f64) * 1.234567e-5),
            };
            cases.push(s);
        }
        for s in &cases {
            let fast = fast_decimal(s.as_bytes());
            let std = s.parse::<f64>().ok();
            if let Some(f) = fast {
                assert_eq!(Some(f.to_bits()), std.map(f64::to_bits), "{s}");
            }
            assert_eq!(parse_number_bytes(s.as_bytes(), false), parse_number(s, false), "{s}");
        }
    }

    /// Texts of every cell of the columns built from `chunk`.
    fn cells(cols: Vec<ColBuilder<'_>>) -> Vec<Vec<String>> {
        cols.into_iter()
            .enumerate()
            .map(|(i, b)| {
                let c = merge_column(format!("c{i}"), vec![b], false);
                (0..c.len()).map(|r| c.text(r).into_owned()).collect()
            })
            .collect()
    }

    #[test]
    fn fast_path_matches_records() {
        for delim in *b",;\t" {
            let d = delim as char;
            let text = format!("1{d} a {d}x\r\n\n2{d}{d}y\n{d}{d}\n 3 {d}b\n4{d}c{d}z{d}extra\n   \n5{d}d{d}w");
            let bytes = text.as_bytes();
            let mut fast: Vec<ColBuilder<'_>> = (0..3).map(|_| ColBuilder::new(8, false)).collect();
            parse_chunk_unquoted(bytes, delim, &mut fast);
            let mut slow: Vec<ColBuilder<'_>> = (0..3).map(|_| ColBuilder::new(8, false)).collect();
            parse_chunk_records(bytes, delim, &mut slow);
            let (fast, slow) = (cells(fast), cells(slow));
            assert_eq!(fast, slow, "delimiter {d:?}");
            assert_eq!(fast[0], ["1", "2", "3", "4", "5"]);
            assert_eq!(fast[1], ["a", "", "b", "c", "d"]);
            assert_eq!(fast[2], ["x", "y", "", "z", "w"]);
        }
    }

    #[test]
    fn many_distinct_numbers() {
        // > 1 MB: parsed in parallel chunks; the columns switch to numbers
        let mut text = String::from("Meas,Id,Cond,Mixed\n");
        let n = 60_000;
        for i in 0..n {
            let meas = if i % 1000 == 7 {
                "N/A".to_owned()
            } else {
                format!("{:.6}", i as f64 * 0.001 + 0.5)
            };
            let mixed = if i % 100 == 3 {
                format!("err{i}")
            } else {
                format!("{}", i as f64 * 0.25)
            };
            text.push_str(&format!("{meas},id{i},{},{mixed}\n", i % 10));
        }
        let ds = parse_csv(text.as_bytes(), &ImportOptions::default()).unwrap();
        assert_eq!(ds.n_rows, n);
        // measurements with a few "N/A": numeric column, N/A = missing
        let m = ds.column("Meas").unwrap();
        assert_eq!(m.kind, ColumnKind::Numeric);
        assert!(!m.is_dict());
        for i in [0usize, 6, 8, 29_999, 59_999] {
            assert!((m.value(i) - (i as f64 * 0.001 + 0.5)).abs() < 1e-9, "row {i}");
        }
        assert!(m.value(7).is_nan() && m.value(31_007).is_nan());
        // texts stay texts
        let id = ds.column("Id").unwrap();
        assert_eq!(id.kind, ColumnKind::Text);
        assert_eq!(id.n_unique, n);
        assert_eq!(id.text(45_678), "id45678");
        // repeated values stay a dictionary
        let c = ds.column("Cond").unwrap();
        assert!(c.is_dict());
        assert_eq!(c.n_unique, 10);
        assert_eq!(c.value(12_345), 5.0);
        // many different texts among numbers: kept as a mixed column
        let x = ds.column("Mixed").unwrap();
        assert_eq!(x.kind, ColumnKind::Mixed);
        assert_eq!(x.text(503), "err503");
        assert_eq!(x.value(502), 125.5);
    }

    #[test]
    fn quoted_parallel_matches_sequential() {
        // > 1 MB, quoted fields with delimiters and new lines inside
        let mut text = String::from("\"Id\",\"Note\",\"Val\"\n");
        let n = 40_000;
        for i in 0..n {
            let note = if i % 7 == 0 {
                format!("line {i}\nnext, \"\"quoted\"\"")
            } else {
                format!("n{i}")
            };
            text.push_str(&format!("\"{i}\",\"{note}\",\"{}\"\n", i as f64 * 0.5));
        }
        let chunks = split_chunks(text.as_bytes(), 4);
        assert!(chunks.len() > 1);
        let ds = parse_csv(text.as_bytes(), &ImportOptions::default()).unwrap();
        assert_eq!(ds.n_rows, n);
        let note = ds.column("Note").unwrap();
        for i in [0usize, 1, 7, 13_999, 14_000, 39_998, 39_999] {
            assert_eq!(ds.column("Id").unwrap().value(i), i as f64, "row {i}");
            assert_eq!(ds.column("Val").unwrap().value(i), i as f64 * 0.5, "row {i}");
            let expected = if i % 7 == 0 {
                format!("line {i}\nnext, \"quoted\"")
            } else {
                format!("n{i}")
            };
            assert_eq!(note.text(i), expected, "row {i}");
        }
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

#[cfg(test)]
mod bench {
    /// `CT_BENCH=file.csv cargo test --release bench_load -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn bench_load() {
        let Ok(path) = std::env::var("CT_BENCH") else {
            return;
        };
        for _ in 0..3 {
            let t = std::time::Instant::now();
            let ds = super::load_path(std::path::Path::new(&path), &Default::default()).unwrap();
            eprintln!(
                "{} rows × {} cols in {:.0} ms ({:.0} MB in memory)",
                ds.n_rows,
                ds.columns.len(),
                t.elapsed().as_secs_f64() * 1000.0,
                ds.memory_bytes() as f64 / 1e6
            );
        }
    }
}
