//! Expands the compressed sheet model into TSV rows and writes them out.
//!
//! Expansion preserves the sheet's cell grid: interior empty rows become
//! empty records and interior empty cells become empty fields, so data keeps
//! its row and column positions. Only trailing emptiness — where spreadsheet
//! files routinely claim thousands of repeated empty rows and columns — is
//! dropped, without ever materializing it.

use std::collections::HashSet;
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::ops::Range;
use std::path::PathBuf;

use crate::content::{Cell, Row, Sheet};

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

/// A merged area whose source value fills the cells it covers.
struct Merge<'a> {
    rows: Range<usize>,
    cols: Range<usize>,
    text: &'a str,
}

pub fn write_sheet<W: Write>(sheet: &Sheet, out: &mut W) -> io::Result<()> {
    let mut merges: Vec<Merge<'_>> = Vec::new();
    let mut row_index = 0usize;
    // Empty rows are counted, not written, until a non-empty row follows;
    // trailing empty rows thus never reach the file.
    let mut pending_empty_rows = 0usize;

    for row in &sheet.rows {
        let template_blank = row.cells.iter().all(|cell| match cell {
            Cell::Value { text, .. } => text.is_empty(),
            Cell::Covered { .. } => true,
        });

        let mut remaining = row.repeat;
        while remaining > 0 {
            merges.retain(|m| m.rows.end > row_index);
            if template_blank && merges.is_empty() {
                // Nothing can put content into these rows anymore; a single
                // repeated empty row often claims the rest of the sheet
                // (up to 2^20 rows), so skip them in bulk.
                pending_empty_rows += remaining;
                row_index += remaining;
                break;
            }

            let cells = expand_row(row, row_index, &mut merges);
            if cells.is_empty() {
                pending_empty_rows += 1;
            } else {
                for _ in 0..pending_empty_rows {
                    out.write_all(b"\r\n")?;
                }
                pending_empty_rows = 0;
                write_tsv_row(out, &cells)?;
            }
            row_index += 1;
            remaining -= 1;
        }
    }

    Ok(())
}

/// Expands one stored row into its field values, filling covered positions
/// from the merged areas and trimming trailing empty fields.
fn expand_row<'a>(row: &'a Row, row_index: usize, merges: &mut Vec<Merge<'a>>) -> Vec<&'a str> {
    // Empty fields are counted, not pushed, until a non-empty field needs
    // them as padding; trailing empty fields thus never materialize.
    fn push<'a>(cells: &mut Vec<&'a str>, pending_empty: &mut usize, value: &'a str) {
        if value.is_empty() {
            *pending_empty += 1;
        } else {
            cells.resize(cells.len() + *pending_empty, "");
            *pending_empty = 0;
            cells.push(value);
        }
    }

    let mut cells: Vec<&'a str> = Vec::new();
    let mut pending_empty = 0usize;
    let mut col = 0usize;

    for cell in &row.cells {
        match cell {
            Cell::Value {
                repeat,
                rows_spanned,
                cols_spanned,
                text,
            } => {
                if text.is_empty() {
                    // Covered positions of an empty merge would be empty
                    // anyway, so no merge needs recording either.
                    pending_empty += repeat;
                    col += repeat;
                    continue;
                }
                for _ in 0..*repeat {
                    if *rows_spanned > 1 || *cols_spanned > 1 {
                        merges.push(Merge {
                            rows: row_index..row_index + rows_spanned,
                            cols: col..col + cols_spanned,
                            text,
                        });
                    }
                    push(&mut cells, &mut pending_empty, text);
                    col += 1;
                }
            }
            Cell::Covered { repeat } => {
                if merges.is_empty() {
                    pending_empty += repeat;
                    col += repeat;
                    continue;
                }
                for _ in 0..*repeat {
                    push(&mut cells, &mut pending_empty, merge_at(merges, row_index, col));
                    col += 1;
                }
            }
        }
    }

    // Merged areas can cover more columns than the row stores cells for;
    // fill the remainder so the values keep their column positions.
    let mut spill: Vec<(Range<usize>, &str)> = merges
        .iter()
        .filter(|m| m.rows.contains(&row_index) && m.cols.end > col)
        .map(|m| (m.cols.clone(), m.text))
        .collect();
    spill.sort_by_key(|(cols, _)| cols.start);
    for (cols, text) in spill {
        if cols.start > col {
            pending_empty += cols.start - col;
            col = cols.start;
        }
        while col < cols.end {
            push(&mut cells, &mut pending_empty, text);
            col += 1;
        }
    }

    cells
}

fn merge_at<'a>(merges: &[Merge<'a>], row: usize, col: usize) -> &'a str {
    merges
        .iter()
        .find(|m| m.rows.contains(&row) && m.cols.contains(&col))
        .map_or("", |m| m.text)
}

/// Writes one row in RFC 4180 style with a tab delimiter: fields containing
/// a tab, CR, LF, or quote are wrapped in double quotes with inner quotes
/// doubled, and rows end in `\r\n`.
fn write_tsv_row<W: Write>(out: &mut W, cells: &[&str]) -> io::Result<()> {
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
