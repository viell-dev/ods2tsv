//! Expands the compressed sheet model into TSV rows.
//!
//! The expansion logic is a line-for-line port of the Python implementation,
//! including its bail-outs for excessive trailing empty space and its exact
//! `csv.writer` output format (tab delimiter, minimal quoting, `\r\n`).

use std::borrow::Cow;
use std::collections::HashMap;
use std::io::{self, Write};

use crate::content::{Cell, Sheet};

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
