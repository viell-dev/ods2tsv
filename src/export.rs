//! Expands the compressed sheet model into TSV rows and writes them out.
//!
//! Expansion preserves the sheet's cell grid: interior empty rows become
//! empty records and interior empty cells become empty fields, so data keeps
//! its row and column positions. Only trailing emptiness — where spreadsheet
//! files routinely claim thousands of repeated empty rows and columns — is
//! dropped, without ever materializing it.
//!
//! The parser intentionally keeps ODF repeats and spans compressed. This module is the boundary
//! where those counts become a rectangular stream of records. It uses two kinds of deferred work:
//!
//! - empty rows/fields are counted until later data proves they are interior and must be emitted;
//! - active merged ranges are remembered until the last row they cover has passed.
//!
//! Together those rules preserve meaningful coordinates while keeping pathological spreadsheet
//! tails (often more than a million empty rows) cheap.

use std::collections::HashSet;
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::ops::Range;
use std::path::PathBuf;

use crate::content::{Cell, Row, Sheet};

/// Writes sheets as `<sheet name>.tsv` files into one directory, sanitizing
/// the names for use as file names and keeping them unique.
pub struct TsvWriter {
    /// Destination chosen by the caller. It must already exist; this type creates only sheet files.
    dir: PathBuf,
    /// Already-claimed file names, lowercased so that sheets whose sanitized
    /// names differ only by case still get distinct files on
    /// case-insensitive filesystems.
    claimed: HashSet<String>,
}

impl TsvWriter {
    /// Creates a writer for an existing export directory.
    ///
    /// Name claims start empty for every export run, so a writer should be reused for all sheets in
    /// one workbook.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            claimed: HashSet::new(),
        }
    }

    /// Writes `sheet` into the directory and returns the path of the created
    /// file.
    pub fn write(&mut self, sheet: &Sheet) -> io::Result<PathBuf> {
        // Claim before creating the file. Two sheet names can sanitize to the same portable name,
        // and File::create would otherwise truncate the first sheet silently.
        let file_name = format!("{}.tsv", self.claim_name(&sheet.name));
        let path = self.dir.join(file_name);
        // BufWriter converts the many small delimiter/field writes below into larger filesystem
        // writes. An explicit flush makes delayed write errors visible to this method's caller.
        let mut out = BufWriter::new(File::create(&path)?);
        write_sheet(sheet, &mut out)?;
        out.flush()?;
        Ok(path)
    }

    fn claim_name(&mut self, sheet_name: &str) -> String {
        let base = sanitize_sheet_name(sheet_name);
        let mut candidate = base.clone();
        let mut n = 1u32;
        // Store lowercase names because Windows and common macOS filesystems compare file names
        // case-insensitively. The returned candidate retains the sheet's original casing.
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
    // The limit is measured in Unicode scalar values, not UTF-8 bytes. Rust's `chars` makes it
    // impossible to cut in the middle of a multi-byte character.
    const MAX_CHARS: usize = 80;

    let cleaned: String = name
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, ' ' | '.' | '_' | '-'))
        .take(MAX_CHARS)
        .collect();
    // Leading/trailing underscores are allowed. Dots, spaces, and hyphens are removed at the edges
    // to avoid hidden Unix names and awkward names that Windows normalizes.
    let cleaned = cleaned.trim_matches([' ', '.', '-']);
    if cleaned.is_empty() {
        "Sheet".to_owned()
    } else {
        cleaned.to_owned()
    }
}

/// A merged area whose source value fills the cells it covers.
///
/// Both ranges use half-open Rust coordinates: a `rows` value of `2..4` covers physical rows 2 and
/// 3. `text` borrows the owning [`Sheet`], so recording a merge does not clone cell contents.
struct Merge<'a> {
    rows: Range<usize>,
    cols: Range<usize>,
    text: &'a str,
}

/// Writes one parsed sheet to any byte sink.
///
/// Accepting [`Write`] rather than a file makes the grid algorithm reusable and directly testable.
/// The function never seeks and does not need to hold the expanded sheet in memory.
pub fn write_sheet<W: Write>(sheet: &Sheet, out: &mut W) -> io::Result<()> {
    // `merges` contains every non-empty merged value that might still affect the current or a later
    // row. Expired entries are pruned at the beginning of each physical row.
    let mut merges: Vec<Merge<'_>> = Vec::new();
    let mut row_index = 0usize;
    // Empty rows are counted, not written, until a non-empty row follows;
    // trailing empty rows thus never reach the file.
    let mut pending_empty_rows = 0usize;

    for row in &sheet.rows {
        // This answers whether the row template itself can produce data. Covered cells do not count
        // because only a previously registered merge can give them a value.
        let template_blank = row.cells.iter().all(|cell| match cell {
            Cell::Value { text, .. } => text.is_empty(),
            Cell::Covered { .. } => true,
        });

        let mut remaining = row.repeat;
        while remaining > 0 {
            // `Range::end` is exclusive, so a merge ending at this row is no longer active.
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
                // Do not write yet: these rows may be the meaningless empty tail. If later data is
                // encountered, the count is flushed first and their positions are preserved.
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
            // Delay allocation until a later non-empty field proves this empty position is
            // interior.
            *pending_empty += 1;
        } else {
            // `resize` emits precisely the tabs needed to keep this value at its original column.
            cells.resize(cells.len() + *pending_empty, "");
            *pending_empty = 0;
            cells.push(value);
        }
    }

    let mut cells: Vec<&'a str> = Vec::new();
    let mut pending_empty = 0usize;
    // `col` is the physical zero-based sheet column, which can be larger than cells.len() while
    // emptiness is pending.
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
                    // A repeated value cell creates one merge per repeated source position. The ODF
                    // counts describe the size of each occurrence, not one large combined range.
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
                    // With no active merge, covered cells carry no exportable value. Skip a
                    // repeated run in constant time instead of calling merge_at for every position.
                    pending_empty += repeat;
                    col += repeat;
                    continue;
                }
                for _ in 0..*repeat {
                    // Malformed or unusual documents may contain an uncovered "covered" cell.
                    // merge_at deliberately turns that into an empty field instead of failing.
                    push(
                        &mut cells,
                        &mut pending_empty,
                        merge_at(merges, row_index, col),
                    );
                    col += 1;
                }
            }
        }
    }

    // Merged areas can cover more columns than the row stores cells for;
    // fill the remainder so the values keep their column positions.
    // Some producers omit covered-table-cell markers entirely. Find active ranges extending beyond
    // the stored row so a vertical merge can still put its value in an otherwise cell-less row.
    let mut spill: Vec<(Range<usize>, &str)> = merges
        .iter()
        .filter(|m| m.rows.contains(&row_index) && m.cols.end > col)
        .map(|m| (m.cols.clone(), m.text))
        .collect();
    // Sorting makes left-to-right output deterministic even though merges were discovered over
    // several previous rows.
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
    // ODF merge ranges should not overlap. If malformed input does overlap them, using the first
    // source encountered gives stable best-effort output.
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
            // RFC 4180 escapes a literal quote by doubling it inside a quoted field. CR and LF are
            // allowed inside those quotes and remain part of the cell rather than ending the row.
            out.write_all(b"\"")?;
            out.write_all(cell.replace('"', "\"\"").as_bytes())?;
            out.write_all(b"\"")?;
        } else {
            out.write_all(cell.as_bytes())?;
        }
    }
    out.write_all(b"\r\n")
}
