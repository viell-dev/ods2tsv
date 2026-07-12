//! Expands the compressed sheet model into TSV rows and writes them out.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::PathBuf;

use crate::content::{Cell, Sheet};

/// Writes sheets as `<sheet name>.tsv` files into one directory, sanitizing
/// the names for use as file names and keeping them unique.
pub struct TsvWriter {
    dir: PathBuf,
    /// Already-claimed file names, lowercased so that sheets whose sanitized
    /// names differ only by case still get distinct files on
    /// case-insensitive filesystems.
    claimed: HashSet<String>,
}

impl TsvWriter {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            claimed: HashSet::new(),
        }
    }

    /// Writes `sheet` into the directory and returns the path of the created
    /// file.
    pub fn write(&mut self, sheet: &Sheet) -> io::Result<PathBuf> {
        let file_name = format!("{}.tsv", self.claim_name(&sheet.name));
        let path = self.dir.join(file_name);
        let mut out = BufWriter::new(File::create(&path)?);
        write_sheet(sheet, &mut out)?;
        out.flush()?;
        Ok(path)
    }

    fn claim_name(&mut self, sheet_name: &str) -> String {
        let base = sanitize_sheet_name(sheet_name);
        let mut candidate = base.clone();
        let mut n = 1u32;
        while !self.claimed.insert(candidate.to_lowercase()) {
            n += 1;
            candidate = format!("{base} ({n})");
        }
        candidate
    }
}

/// Reduces a sheet name to a safe, portable file name: an allowlist of
/// alphanumerics plus ` . _ -`, no leading/trailing separators (which would
/// produce hidden or awkward names), bounded length, and a fallback for
/// names with nothing left after cleaning.
fn sanitize_sheet_name(name: &str) -> String {
    const MAX_CHARS: usize = 80;

    let cleaned: String = name
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, ' ' | '.' | '_' | '-'))
        .take(MAX_CHARS)
        .collect();
    let cleaned = cleaned.trim_matches([' ', '.', '-']);
    if cleaned.is_empty() {
        "Sheet".to_owned()
    } else {
        cleaned.to_owned()
    }
}

pub fn write_sheet<W: Write>(sheet: &Sheet, out: &mut W) -> io::Result<()> {
    // Values of merged areas, keyed by the absolute (row, column) positions
    // they cover, so covered cells repeat the source value and stay aligned.
    let mut merged: HashMap<(usize, usize), String> = HashMap::new();
    let mut current_row = 0usize;
    let mut is_empty_row = false;

    'rows: for row in &sheet.rows {
        for _ in 0..row.repeat {
            let mut cells_out: Vec<Cow<'_, str>> = Vec::new();
            let mut col = 0usize;

            for cell in &row.cells {
                match cell {
                    Cell::Value {
                        repeat,
                        rows_spanned,
                        cols_spanned,
                        text,
                    } => {
                        for _ in 0..*repeat {
                            if *rows_spanned > 1 || *cols_spanned > 1 {
                                for rs in 0..*rows_spanned {
                                    for cs in 0..*cols_spanned {
                                        if rs == 0 && cs == 0 {
                                            continue;
                                        }
                                        merged.insert((current_row + rs, col + cs), text.clone());
                                    }
                                }
                            }

                            cells_out.push(Cow::Borrowed(text.as_str()));
                            col += 1;

                            // Bail on excessive trailing empty columns.
                            if text.is_empty() && *repeat > 1024 {
                                break;
                            }
                        }
                    }
                    Cell::Covered { repeat } => {
                        for _ in 0..*repeat {
                            let value = merged.remove(&(current_row, col)).unwrap_or_default();
                            cells_out.push(Cow::Owned(value));
                            col += 1;
                        }
                    }
                }
            }

            // Merged values spilling past the row's last stored cell.
            while let Some(value) = merged.remove(&(current_row, col)) {
                cells_out.push(Cow::Owned(value));
                col += 1;
            }

            is_empty_row = cells_out.iter().all(|c| c.is_empty());
            if is_empty_row && row.repeat > 100 && current_row > 10 && merged.is_empty() {
                break;
            }

            while cells_out.last().is_some_and(|c| c.is_empty()) {
                cells_out.pop();
            }

            if !is_empty_row {
                write_tsv_row(out, &cells_out)?;
            }
            current_row += 1;
        }

        // Bail on excessive trailing empty rows.
        if is_empty_row && row.repeat > 100 && merged.is_empty() {
            break 'rows;
        }
    }

    Ok(())
}

/// Writes one row like Python's `csv.writer` with `delimiter='\t'`: fields
/// containing a tab, CR, LF, or quote are wrapped in double quotes with inner
/// quotes doubled, and rows end in `\r\n`.
fn write_tsv_row<W: Write>(out: &mut W, cells: &[Cow<'_, str>]) -> io::Result<()> {
    for (i, cell) in cells.iter().enumerate() {
        if i > 0 {
            out.write_all(b"\t")?;
        }
        if cell.contains(['\t', '\r', '\n', '"']) {
            out.write_all(b"\"")?;
            out.write_all(cell.replace('"', "\"\"").as_bytes())?;
            out.write_all(b"\"")?;
        } else {
            out.write_all(cell.as_bytes())?;
        }
    }
    out.write_all(b"\r\n")
}
