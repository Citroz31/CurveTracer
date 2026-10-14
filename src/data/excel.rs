//! Spreadsheet loader (xlsx, xlsm, xlsb, xls, ods) based on `calamine`.

use std::path::Path;
use std::sync::Arc;

use calamine::{Data, Reader, open_workbook_auto};

use super::dataset::{Dataset, FileKind, LoadInfo};
use super::loader::{
    ColBuilder, ImportOptions, detect_decimal_comma, detect_header, make_names, merge_all, preamble_lines,
};

const SAMPLE_ROWS: usize = 400;

pub fn load_excel(path: &Path, opts: &ImportOptions) -> Result<Dataset, String> {
    let mut wb = open_workbook_auto(path).map_err(|e| format!("Cannot open workbook: {e}"))?;
    let sheets = wb.sheet_names();
    let sheet = opts
        .sheet
        .clone()
        .filter(|s| sheets.contains(s))
        .or_else(|| sheets.first().cloned())
        .ok_or_else(|| "The workbook has no sheet".to_owned())?;
    let range = wb
        .worksheet_range(&sheet)
        .map_err(|e| format!("Cannot read sheet '{sheet}': {e}"))?;
    // Physical row of the first row of the range (0-based).
    let first_row = range.start().map_or(0, |(r, _)| r as usize);
    let first_col = range.start().map_or(0, |(_, c)| c as usize);

    // Header detection works on a sample of decoded rows. Rows above the
    // used range are blank, add them so that line numbers stay physical.
    let mut samples: Vec<Vec<String>> = vec![Vec::new(); first_row];
    for row in range.rows().take(SAMPLE_ROWS) {
        let mut fields: Vec<String> = vec![String::new(); first_col];
        fields.extend(row.iter().map(cell_text));
        samples.push(fields);
    }
    let guess = detect_header(&samples, opts.header_line);
    let names = make_names(guess.header.map(|h| &samples[h]), guess.n_cols);
    if names.is_empty() {
        return Err(format!("Sheet '{sheet}' is empty"));
    }
    let decimal_comma = opts
        .decimal_comma
        .unwrap_or_else(|| detect_decimal_comma(&samples, guess.first_data));
    let preamble = preamble_lines(&samples, guess.header.unwrap_or(guess.first_data));

    let n_cols = names.len();
    let skip = guess.first_data.saturating_sub(first_row);
    let mut cols: Vec<ColBuilder<'static>> = (0..n_cols)
        .map(|_| ColBuilder::new(range.height(), decimal_comma))
        .collect();
    let mut buf = String::new();
    for row in range.rows().skip(skip) {
        let blank = row.iter().all(|c| matches!(c, Data::Empty));
        if blank {
            continue;
        }
        for (c, col) in cols.iter_mut().enumerate() {
            buf.clear();
            if c >= first_col
                && let Some(cell) = row.get(c - first_col)
            {
                write_cell(&mut buf, cell);
            }
            col.push_bytes(buf.trim().as_bytes());
        }
    }
    let per_col: Vec<Vec<ColBuilder<'static>>> = cols.into_iter().map(|c| vec![c]).collect();
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(1, 16);
    let columns = merge_all(names, per_col, decimal_comma, threads);
    let n_rows = columns.first().map_or(0, |c| c.len());
    let file_bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    Ok(Dataset {
        name: String::new(),
        path: path.to_path_buf(),
        columns: columns.into_iter().map(Arc::new).collect(),
        n_rows,
        preamble,
        info: LoadInfo {
            kind: FileKind::Excel,
            delimiter: None,
            header_line: guess.header.map(|h| h + 1),
            decimal_comma,
            sheet: Some(sheet),
            sheets,
            file_bytes,
            load_ms: 0.0,
        },
        generation: 0,
    })
}

fn cell_text(cell: &Data) -> String {
    let mut s = String::new();
    write_cell(&mut s, cell);
    s
}

fn write_cell(out: &mut String, cell: &Data) {
    use std::fmt::Write as _;
    match cell {
        Data::Empty => {}
        Data::Int(i) => {
            let _ = write!(out, "{i}");
        }
        Data::Float(f) => {
            out.push_str(&super::dataset::format_value(*f));
        }
        Data::String(s) | Data::DateTimeIso(s) | Data::DurationIso(s) => out.push_str(s),
        Data::Bool(b) => {
            let _ = write!(out, "{b}");
        }
        Data::DateTime(dt) => {
            if dt.is_duration() {
                let _ = write!(out, "{}", dt.as_f64());
            } else {
                out.push_str(&excel_serial_to_string(dt.as_f64()));
            }
        }
        Data::Error(e) => {
            let _ = write!(out, "{e}");
        }
    }
}

/// Converts an Excel serial date (days since 1899-12-30) to
/// `YYYY-MM-DD HH:MM:SS`.
pub fn excel_serial_to_string(serial: f64) -> String {
    if !serial.is_finite() {
        return String::new();
    }
    let days = serial.floor() as i64;
    let secs = ((serial - serial.floor()) * 86_400.0).round() as i64;
    let (days, secs) = if secs >= 86_400 {
        (days + 1, secs - 86_400)
    } else {
        (days, secs)
    };
    // 1899-12-30 is 25569 days before 1970-01-01
    let (y, m, d) = civil_from_days(days - 25_569);
    let (hh, mm, ss) = (secs / 3600, (secs / 60) % 60, secs % 60);
    if secs == 0 {
        format!("{y:04}-{m:02}-{d:02}")
    } else {
        format!("{y:04}-{m:02}-{d:02} {hh:02}:{mm:02}:{ss:02}")
    }
}

/// Days since 1970-01-01 to (year, month, day) – H. Hinnant's algorithm.
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serial_dates() {
        assert_eq!(excel_serial_to_string(45_939.0), "2025-10-09");
        assert_eq!(excel_serial_to_string(45_939.5), "2025-10-09 12:00:00");
        assert_eq!(civil_from_days(0), (1970, 1, 1));
    }

    /// Run with CT_TEST_XLSX=/path/to/file.xlsx (header expected on line 5).
    #[test]
    fn load_workbook_from_env() {
        let Ok(p) = std::env::var("CT_TEST_XLSX") else {
            return;
        };
        let ds = load_excel(Path::new(&p), &ImportOptions::default()).unwrap();
        assert_eq!(ds.info.header_line, Some(5));
        assert_eq!(ds.columns[3].name, "SN");
        assert!(ds.n_rows > 100);
        assert_eq!(ds.preamble.len(), 3);
        let f = ds.column("Freq").unwrap();
        assert_eq!(f.value(0), 35e9);
        assert_eq!(ds.column("SN").unwrap().text(0), "3008260");
        let other = load_excel(
            Path::new(&p),
            &ImportOptions {
                sheet: Some("Other".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(other.columns[1].text(0), "2026-10-09 14:21:42");
    }
}
