//! Columnar, dictionary-encoded in-memory table.
//!
//! Every column is stored either
//! * dictionary encoded (`codes` + unique values), which makes filters,
//!   series grouping and "unique values" lists essentially free, or
//! * as plain `f64` values when the column is numeric with a very high
//!   cardinality (e.g. a frequency sweep or a measured quantity).

use std::borrow::Cow;
use std::cmp::Ordering;
use std::path::PathBuf;
use std::sync::Arc;

use super::hash::FxHashMap;

/// Above this number of unique values a purely numeric column is stored as
/// plain `f64` values instead of being dictionary encoded.
pub const DICT_MAX_NUMERIC: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColumnKind {
    /// Every non-empty value is a number.
    Numeric,
    /// No value is a number.
    Text,
    /// Mix of numbers and text (e.g. numbers and "N/A").
    Mixed,
    /// Only empty cells.
    Empty,
}

impl ColumnKind {
    pub fn short(&self) -> &'static str {
        match self {
            ColumnKind::Numeric => "#",
            ColumnKind::Text => "Aa",
            ColumnKind::Mixed => "#?",
            ColumnKind::Empty => "∅",
        }
    }
}

pub enum ColumnData {
    Dict {
        codes: Vec<u32>,
        /// Unique values, in order of first appearance.
        dict: Vec<String>,
        /// Numeric value of every dictionary entry (NaN if not a number).
        dict_num: Vec<f64>,
        /// Dictionary codes in "natural" order (numbers ascending, then text).
        sorted: Vec<u32>,
        /// Rank of every code inside `sorted`.
        rank: Vec<u32>,
    },
    Num {
        values: Vec<f64>,
    },
}

pub struct Column {
    pub name: String,
    pub data: ColumnData,
    pub kind: ColumnKind,
    pub n_unique: usize,
    pub min: f64,
    pub max: f64,
    /// Expression of a user function (computed column).
    pub formula: Option<String>,
}

impl Column {
    /// Builds a column from dictionary codes and the decoded unique values.
    pub fn from_dict(name: String, codes: Vec<u32>, dict: Vec<String>, decimal_comma: bool) -> Column {
        let dict_num: Vec<f64> = dict
            .iter()
            .map(|s| parse_number(s, decimal_comma).unwrap_or(f64::NAN))
            .collect();
        let mut n_num = 0usize;
        let mut n_text = 0usize;
        let mut min = f64::INFINITY;
        let mut max = f64::NEG_INFINITY;
        for (s, v) in dict.iter().zip(&dict_num) {
            if s.is_empty() {
                continue;
            }
            if v.is_nan() {
                n_text += 1;
            } else {
                n_num += 1;
                if v.is_finite() {
                    min = min.min(*v);
                    max = max.max(*v);
                }
            }
        }
        let kind = classify(n_num, n_text);
        if min > max {
            min = f64::NAN;
            max = f64::NAN;
        }
        let n_unique = dict.len();
        if kind == ColumnKind::Numeric && n_unique > DICT_MAX_NUMERIC {
            let values = codes.iter().map(|&c| dict_num[c as usize]).collect();
            return Column {
                name,
                data: ColumnData::Num { values },
                kind,
                n_unique,
                min,
                max,
                formula: None,
            };
        }
        let (sorted, rank) = natural_order(&dict, &dict_num);
        Column {
            name,
            data: ColumnData::Dict {
                codes,
                dict,
                dict_num,
                sorted,
                rank,
            },
            kind,
            n_unique,
            min,
            max,
            formula: None,
        }
    }

    /// Builds a numeric column from raw values (used by user functions).
    pub fn from_values(name: String, values: Vec<f64>, formula: Option<String>) -> Column {
        let mut bits: Vec<u64> = values
            .iter()
            .filter(|v| !v.is_nan())
            .map(|v| norm_zero(*v).to_bits())
            .collect();
        bits.sort_unstable();
        bits.dedup();
        let mut min = f64::INFINITY;
        let mut max = f64::NEG_INFINITY;
        for v in values.iter().filter(|v| v.is_finite()) {
            min = min.min(*v);
            max = max.max(*v);
        }
        if min > max {
            min = f64::NAN;
            max = f64::NAN;
        }
        let has_nan = values.iter().any(|v| v.is_nan());
        let n_unique = bits.len() + usize::from(has_nan);
        let kind = if bits.is_empty() {
            ColumnKind::Empty
        } else {
            ColumnKind::Numeric
        };
        if n_unique <= DICT_MAX_NUMERIC {
            let mut map: FxHashMap<u64, u32> = FxHashMap::default();
            let mut dict = Vec::new();
            let mut dict_num = Vec::new();
            let codes = values
                .iter()
                .map(|v| {
                    let key = if v.is_nan() { u64::MAX } else { norm_zero(*v).to_bits() };
                    *map.entry(key).or_insert_with(|| {
                        dict.push(if v.is_nan() { String::new() } else { format_value(*v) });
                        dict_num.push(*v);
                        (dict.len() - 1) as u32
                    })
                })
                .collect();
            let (sorted, rank) = natural_order(&dict, &dict_num);
            return Column {
                name,
                data: ColumnData::Dict {
                    codes,
                    dict,
                    dict_num,
                    sorted,
                    rank,
                },
                kind,
                n_unique,
                min,
                max,
                formula,
            };
        }
        Column {
            name,
            data: ColumnData::Num { values },
            kind,
            n_unique,
            min,
            max,
            formula,
        }
    }

    pub fn len(&self) -> usize {
        match &self.data {
            ColumnData::Dict { codes, .. } => codes.len(),
            ColumnData::Num { values } => values.len(),
        }
    }

    /// Numeric value of a row (NaN if the cell is not a number).
    #[inline]
    pub fn value(&self, row: usize) -> f64 {
        match &self.data {
            ColumnData::Dict { codes, dict_num, .. } => dict_num[codes[row] as usize],
            ColumnData::Num { values } => values[row],
        }
    }

    /// Text of a cell, as it appeared in the file.
    pub fn text(&self, row: usize) -> Cow<'_, str> {
        match &self.data {
            ColumnData::Dict { codes, dict, .. } => Cow::Borrowed(dict[codes[row] as usize].as_str()),
            ColumnData::Num { values } => Cow::Owned(format_value(values[row])),
        }
    }

    pub fn is_dict(&self) -> bool {
        matches!(self.data, ColumnData::Dict { .. })
    }

    pub fn has_numbers(&self) -> bool {
        matches!(self.kind, ColumnKind::Numeric | ColumnKind::Mixed)
    }

    /// Unique values (dictionary), if the column is dictionary encoded.
    pub fn dict(&self) -> Option<&[String]> {
        match &self.data {
            ColumnData::Dict { dict, .. } => Some(dict),
            ColumnData::Num { .. } => None,
        }
    }

    /// Dictionary codes in natural order.
    pub fn sorted_codes(&self) -> Option<&[u32]> {
        match &self.data {
            ColumnData::Dict { sorted, .. } => Some(sorted),
            ColumnData::Num { .. } => None,
        }
    }

    #[inline]
    pub fn code(&self, row: usize) -> Option<u32> {
        match &self.data {
            ColumnData::Dict { codes, .. } => Some(codes[row]),
            ColumnData::Num { .. } => None,
        }
    }

    /// Grouping key of a row: the dictionary code, or the bits of the value.
    #[inline]
    pub fn key(&self, row: usize) -> u64 {
        match &self.data {
            ColumnData::Dict { codes, .. } => codes[row] as u64,
            ColumnData::Num { values } => norm_zero(values[row]).to_bits(),
        }
    }

    /// Human readable label of a grouping key.
    pub fn key_label(&self, key: u64) -> String {
        match &self.data {
            ColumnData::Dict { dict, .. } => {
                let s = &dict[key as usize];
                if s.is_empty() { "(empty)".to_owned() } else { s.clone() }
            }
            ColumnData::Num { .. } => format_value(f64::from_bits(key)),
        }
    }

    /// Sorting rank of a grouping key (natural order).
    pub fn key_order(&self, key: u64) -> (u32, f64) {
        match &self.data {
            ColumnData::Dict { rank, .. } => (rank[key as usize], 0.0),
            ColumnData::Num { .. } => (0, f64::from_bits(key)),
        }
    }

    /// Rank in natural order of a row's value (for categorical X axes).
    #[inline]
    pub fn rank(&self, row: usize) -> Option<u32> {
        match &self.data {
            ColumnData::Dict { codes, rank, .. } => Some(rank[codes[row] as usize]),
            ColumnData::Num { .. } => None,
        }
    }

    pub fn memory_bytes(&self) -> usize {
        match &self.data {
            ColumnData::Dict { codes, dict, .. } => {
                codes.len() * 4 + dict.iter().map(|s| s.len() + 24 + 8 + 8).sum::<usize>()
            }
            ColumnData::Num { values } => values.len() * 8,
        }
    }
}

fn classify(n_num: usize, n_text: usize) -> ColumnKind {
    match (n_num, n_text) {
        (0, 0) => ColumnKind::Empty,
        (_, 0) => ColumnKind::Numeric,
        (0, _) => ColumnKind::Text,
        _ => ColumnKind::Mixed,
    }
}

#[inline]
fn norm_zero(v: f64) -> f64 {
    if v == 0.0 { 0.0 } else { v }
}

/// Natural ordering of dictionary entries: numbers ascending, then text
/// (case-insensitive), empty values last.
fn natural_order(dict: &[String], dict_num: &[f64]) -> (Vec<u32>, Vec<u32>) {
    let mut sorted: Vec<u32> = (0..dict.len() as u32).collect();
    sorted.sort_by(|&a, &b| {
        compare_entries(
            &dict[a as usize],
            dict_num[a as usize],
            &dict[b as usize],
            dict_num[b as usize],
        )
    });
    let mut rank = vec![0u32; dict.len()];
    for (r, &c) in sorted.iter().enumerate() {
        rank[c as usize] = r as u32;
    }
    (sorted, rank)
}

pub fn compare_entries(sa: &str, na: f64, sb: &str, nb: f64) -> Ordering {
    let ea = sa.is_empty();
    let eb = sb.is_empty();
    if ea || eb {
        return ea.cmp(&eb);
    }
    match (na.is_nan(), nb.is_nan()) {
        (false, false) => na.partial_cmp(&nb).unwrap_or(Ordering::Equal).then_with(|| sa.cmp(sb)),
        (false, true) => Ordering::Less,
        (true, false) => Ordering::Greater,
        (true, true) => natural_text_cmp(sa, sb),
    }
}

/// Case-insensitive comparison where digit runs are compared numerically
/// ("State 2" < "State 10").
pub fn natural_text_cmp(a: &str, b: &str) -> Ordering {
    let mut ia = a.chars().peekable();
    let mut ib = b.chars().peekable();
    loop {
        match (ia.peek().copied(), ib.peek().copied()) {
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(ca), Some(cb)) => {
                if ca.is_ascii_digit() && cb.is_ascii_digit() {
                    let mut na = String::new();
                    while let Some(c) = ia.peek().copied().filter(|c| c.is_ascii_digit()) {
                        na.push(c);
                        ia.next();
                    }
                    let mut nb = String::new();
                    while let Some(c) = ib.peek().copied().filter(|c| c.is_ascii_digit()) {
                        nb.push(c);
                        ib.next();
                    }
                    let ta = na.trim_start_matches('0');
                    let tb = nb.trim_start_matches('0');
                    let ord = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                    if ord != Ordering::Equal {
                        return ord;
                    }
                } else {
                    let la = ca.to_lowercase().next().unwrap_or(ca);
                    let lb = cb.to_lowercase().next().unwrap_or(cb);
                    if la != lb {
                        return la.cmp(&lb);
                    }
                    ia.next();
                    ib.next();
                }
            }
        }
    }
}

/// Parses a number, returns `None` for anything that is not a number.
pub fn parse_number(s: &str, decimal_comma: bool) -> Option<f64> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    let first = t.as_bytes()[0];
    // Fast reject of obvious text (keeps "inf"/"nan" out as well).
    if !(first.is_ascii_digit() || first == b'-' || first == b'+' || first == b'.' || first == b',') {
        return None;
    }
    if let Ok(v) = t.parse::<f64>() {
        return Some(v);
    }
    if decimal_comma
        && t.contains(',')
        && !t.contains('.')
        && let Ok(v) = t.replacen(',', ".", 1).parse::<f64>()
    {
        return Some(v);
    }
    None
}

/// Compact textual representation of a value.
pub fn format_value(v: f64) -> String {
    if v.is_nan() {
        return String::new();
    }
    let a = v.abs();
    if a != 0.0 && !(1e-5..1e15).contains(&a) {
        format!("{v:e}")
    } else {
        format!("{v}")
    }
}

/// Formats a value with a number of significant digits (for read-outs).
pub fn format_sig(v: f64, digits: usize) -> String {
    if !v.is_finite() {
        return format!("{v}");
    }
    if v == 0.0 {
        return "0".to_owned();
    }
    let mag = v.abs().log10().floor() as i32;
    if !(-4..15).contains(&mag) {
        return format!("{:.*e}", digits.saturating_sub(1), v);
    }
    let decimals = (digits as i32 - 1 - mag).max(0) as usize;
    let s = format!("{v:.decimals$}");
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        s
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileKind {
    Csv,
    Excel,
}

#[derive(Clone, Debug)]
pub struct LoadInfo {
    pub kind: FileKind,
    pub delimiter: Option<char>,
    /// 1-based line (or spreadsheet row) of the header, `None` if no header.
    pub header_line: Option<usize>,
    pub decimal_comma: bool,
    pub sheet: Option<String>,
    pub sheets: Vec<String>,
    pub file_bytes: u64,
    pub load_ms: f64,
}

impl Default for LoadInfo {
    fn default() -> Self {
        LoadInfo {
            kind: FileKind::Csv,
            delimiter: Some(','),
            header_line: Some(1),
            decimal_comma: false,
            sheet: None,
            sheets: Vec::new(),
            file_bytes: 0,
            load_ms: 0.0,
        }
    }
}

pub struct Dataset {
    pub name: String,
    pub path: PathBuf,
    pub columns: Vec<Arc<Column>>,
    pub n_rows: usize,
    /// Metadata lines found before the header.
    pub preamble: Vec<String>,
    pub info: LoadInfo,
    /// Incremented every time the content changes (user functions).
    pub generation: u64,
}

impl Clone for Dataset {
    fn clone(&self) -> Self {
        Dataset {
            name: self.name.clone(),
            path: self.path.clone(),
            columns: self.columns.clone(),
            n_rows: self.n_rows,
            preamble: self.preamble.clone(),
            info: self.info.clone(),
            generation: self.generation,
        }
    }
}

impl Dataset {
    pub fn column_index(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|c| c.name == name)
    }

    pub fn column(&self, name: &str) -> Option<&Column> {
        self.columns.iter().find(|c| c.name == name).map(|c| c.as_ref())
    }

    pub fn memory_bytes(&self) -> usize {
        self.columns.iter().map(|c| c.memory_bytes()).sum()
    }

    /// Removes every computed column (user function).
    pub fn strip_formulas(&mut self) {
        self.columns.retain(|c| c.formula.is_none());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers() {
        assert_eq!(parse_number("1.5", false), Some(1.5));
        assert_eq!(parse_number(" -40 ", false), Some(-40.0));
        assert_eq!(parse_number("35000000000.0", false), Some(35e9));
        assert_eq!(parse_number("1e-3", false), Some(1e-3));
        assert_eq!(parse_number("N/A", false), None);
        assert_eq!(parse_number("State 1", false), None);
        assert_eq!(parse_number("1,5", false), None);
        assert_eq!(parse_number("1,5", true), Some(1.5));
        assert_eq!(parse_number("", false), None);
        assert_eq!(parse_number("nan", false), None);
    }

    #[test]
    fn natural() {
        assert_eq!(natural_text_cmp("State 2", "State 10"), Ordering::Less);
        assert_eq!(natural_text_cmp("abc", "ABD"), Ordering::Less);
    }

    #[test]
    fn dict_column() {
        let dict = vec!["10".to_owned(), "2".to_owned(), "N/A".to_owned(), "".to_owned()];
        let col = Column::from_dict("c".into(), vec![0, 1, 2, 3, 0], dict, false);
        assert_eq!(col.kind, ColumnKind::Mixed);
        assert_eq!(col.value(0), 10.0);
        assert!(col.value(2).is_nan());
        assert_eq!(col.sorted_codes().unwrap(), &[1, 0, 2, 3]);
        assert_eq!(col.min, 2.0);
        assert_eq!(col.max, 10.0);
    }

    #[test]
    fn values_column() {
        let col = Column::from_values("f".into(), vec![1.0, 2.0, 1.0, f64::NAN], None);
        assert!(col.is_dict());
        assert_eq!(col.n_unique, 3);
        assert_eq!(col.text(1), "2");
        let big: Vec<f64> = (0..10_000).map(|i| i as f64).collect();
        let col = Column::from_values("g".into(), big, None);
        assert!(!col.is_dict());
        assert_eq!(col.value(9999), 9999.0);
    }

    #[test]
    fn sig() {
        assert_eq!(format_sig(28.123456, 5), "28.123");
        assert_eq!(format_sig(35.0, 5), "35");
        assert_eq!(format_sig(0.00012345, 3), "0.000123");
    }
}
