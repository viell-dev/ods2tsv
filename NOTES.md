# NOTES

Design sketch for the content.xml parsing loop (`for_each_sheet`), written against quick-xml 0.41 and
the current module layout. Not wired in yet; semantics mirror the Python original (`repo/ods_to_tsv.py`),
which is the behavior spec. Quirks preserved on purpose are listed at the end.

## Shape

Recursive descent, one function per nesting level, each owning its loop until its end tag:

- `for_each_sheet` — scans for `table:table` starts, hands each finished `Sheet` to the callback.
- `read_sheet` — rows + merge map + trailing-padding heuristics, until `</table:table>`.
- `read_row` — collects `RawCell`s (value, repeat, spans, covered flag), until `</table:table-row>`.
- `read_cell_text` — paragraph/`text:s`/`line-break` accumulation, until `</table:table-cell>`.
- `expand_row` — the Python core: expand column repeats, resolve merges, trim trailing empties.

Error policy: the callback writes a TSV per sheet as it completes; any `Err` (parse or I/O)
propagates straight out and `handle` deletes the output directory. `--no-cleanup` /
`--ignore-errors` flags maybe later, not now.

Wiring: `handle` calls `open_xml`; the tag-along first event from the `Xml` tuple can simply be
dropped here — content.xml's first real event is the `office:document-content` root, which no loop
below cares about.

## Sketch

```rust
use std::{collections::HashMap, io::BufRead};

use quick_xml::{
    Reader, XmlVersion,
    events::{BytesStart, Event},
};

pub struct Sheet {
    pub name: String,
    pub rows: Vec<Vec<String>>,
}

/// One `table:table-cell` / `covered-table-cell` as written, before
/// repeat-expansion and merge resolution.
struct RawCell {
    value: String,
    repeat: usize,
    rows_spanned: usize,
    cols_spanned: usize,
    covered: bool,
}

// Python parity: huge repeats of empty cells/rows are trailing padding.
const TRAILING_EMPTY_COLS: usize = 1024;
const TRAILING_EMPTY_ROWS: usize = 100;

pub fn for_each_sheet<R: BufRead>(
    mut reader: Reader<R>,
    mut f: impl FnMut(Sheet) -> Result<(), Error>,
) -> Result<(), Error> {
    // Turns `<table:covered-table-cell/>` into Start+End pairs, so every
    // loop below only handles Start/End/Text.
    reader.config_mut().expand_empty_elements = true;

    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader.read_event_into(&mut buf)? {
            Event::Start(e) if e.local_name().as_ref() == b"table" => {
                let name = attr(&e, b"name")?.unwrap_or_default();
                f(read_sheet(&mut reader, name)?)?;
            }
            Event::Eof => return Ok(()),
            _ => {}
        }
    }
}

fn read_sheet<R: BufRead>(reader: &mut Reader<R>, name: String) -> Result<Sheet, Error> {
    let mut rows = Vec::new();
    let mut merged: HashMap<(usize, usize), String> = HashMap::new();
    let mut row_index = 0;
    let mut done = false; // trailing-padding heuristic hit; drain to </table>

    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader.read_event_into(&mut buf)? {
            Event::Start(e) if !done && e.local_name().as_ref() == b"table-row" => {
                let repeat = attr_usize(&e, b"number-rows-repeated", 1)?;
                let cells = read_row(reader)?;
                for _ in 0..repeat {
                    let (row, is_empty) = expand_row(&cells, row_index, &mut merged);
                    if is_empty && repeat > TRAILING_EMPTY_ROWS && row_index > 10 && merged.is_empty() {
                        done = true;
                        break;
                    }
                    if !row.is_empty() {
                        rows.push(row);
                    }
                    row_index += 1;
                }
            }
            // columns, shapes, named ranges, or rows after `done`
            Event::Start(e) => reader.read_to_end_into(e.name(), &mut Vec::new()).map(drop)?,
            Event::End(e) if e.local_name().as_ref() == b"table" => break,
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(Sheet { name, rows })
}

fn read_row<R: BufRead>(reader: &mut Reader<R>) -> Result<Vec<RawCell>, Error> {
    let mut cells = Vec::new();
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader.read_event_into(&mut buf)? {
            Event::Start(e) => {
                let covered = match e.local_name().as_ref() {
                    b"table-cell" => false,
                    b"covered-table-cell" => true,
                    _ => {
                        reader.read_to_end_into(e.name(), &mut Vec::new())?;
                        continue;
                    }
                };
                let repeat = attr_usize(&e, b"number-columns-repeated", 1)?;
                let rows_spanned = attr_usize(&e, b"number-rows-spanned", 1)?;
                let cols_spanned = attr_usize(&e, b"number-columns-spanned", 1)?;
                let value = if covered {
                    // its own content is ignored; the merge map supplies it
                    reader.read_to_end_into(e.name(), &mut Vec::new())?;
                    String::new()
                } else {
                    read_cell_text(reader)?
                };
                cells.push(RawCell { value, repeat, rows_spanned, cols_spanned, covered });
            }
            Event::End(e) if e.local_name().as_ref() == b"table-row" => return Ok(cells),
            Event::Eof => return Ok(cells),
            _ => {}
        }
    }
}

/// Text content of one `table:table-cell`, consumed through its end tag.
fn read_cell_text<R: BufRead>(reader: &mut Reader<R>) -> Result<String, Error> {
    let mut value = String::new();
    let mut paragraphs = 0usize;
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader.read_event_into(&mut buf)? {
            Event::Start(e) => match e.local_name().as_ref() {
                b"p" => {
                    if paragraphs > 0 {
                        value.push('\n');
                    }
                    paragraphs += 1;
                }
                b"s" => {
                    let count = attr_usize(&e, b"c", 1)?;
                    value.extend(std::iter::repeat_n(' ', count));
                }
                b"line-break" => value.push('\n'),
                _ => {} // spans etc.: descend, nested text still counts
            },
            Event::Text(t) => value.push_str(&t.xml_content(XmlVersion::V1_0)?),
            Event::CData(t) => value.push_str(&t.decode()?),
            Event::End(e) if e.local_name().as_ref() == b"table-cell" => return Ok(value),
            Event::Eof => return Ok(value),
            _ => {}
        }
    }
}

/// The Python core: expand column repeats, resolve merges, trim trailing
/// empties. Returns the finished row and whether it was entirely empty.
fn expand_row(
    cells: &[RawCell],
    row_index: usize,
    merged: &mut HashMap<(usize, usize), String>,
) -> (Vec<String>, bool) {
    let mut out: Vec<String> = Vec::new();
    let mut col = 0;

    for cell in cells {
        for _ in 0..cell.repeat {
            if cell.covered {
                out.push(merged.remove(&(row_index, col)).unwrap_or_default());
            } else {
                if cell.rows_spanned > 1 || cell.cols_spanned > 1 {
                    for rs in 0..cell.rows_spanned {
                        for cs in 0..cell.cols_spanned {
                            if rs != 0 || cs != 0 {
                                merged.insert((row_index + rs, col + cs), cell.value.clone());
                            }
                        }
                    }
                }
                out.push(cell.value.clone());
            }
            col += 1;
            // Python parity: an empty cell repeated >1024 times is trailing
            // padding; emit it once and truncate the rest of the *repeat*
            // (following cells in the row are still processed).
            if !cell.covered && cell.value.is_empty() && cell.repeat > TRAILING_EMPTY_COLS {
                break;
            }
        }
    }

    // merges hanging past the last explicit cell of the row
    while let Some(value) = merged.remove(&(row_index, col)) {
        out.push(value);
        col += 1;
    }

    let is_empty = out.iter().all(String::is_empty);
    while out.last().is_some_and(String::is_empty) {
        out.pop();
    }
    (out, is_empty)
}

fn attr(e: &BytesStart<'_>, name: &[u8]) -> Result<Option<String>, Error> {
    for attr in e.attributes() {
        let attr = attr?;
        if attr.key.local_name().as_ref() == name {
            return Ok(Some(attr.unescape_value()?.into_owned()));
        }
    }
    Ok(None)
}

fn attr_usize(e: &BytesStart<'_>, name: &[u8], default: usize) -> Result<usize, Error> {
    Ok(attr(e, name)?
        .and_then(|v| v.parse().ok()) // lenient: garbage attr → default
        .unwrap_or(default))
}
```

## Python quirks preserved deliberately

Decide later if any should diverge:

1. **Empty rows vanish entirely** (`if row_cells or not is_empty_row`) — no blank TSV line, so later
   rows shift up relative to the spreadsheet's row numbers.
2. **Annotation text leaks into cells**: the Python takes _all_ descendant `text:p`, including
   `office:annotation` contents; the streaming version descends the same way. Arguably a bug in the
   spec — a one-line `read_to_end_into` skip on `annotation` would fix it if ever wanted.
3. **The >1024 empty-repeat break** only truncates that one repeat run, not the rest of the row —
   subsequent cells still land, at compressed column indices (which can slightly desync the merge
   map in exotic layouts). Same behavior as the Python.
4. **Merged values are copied** into every covered position, and merges are consumed on use
   (`remove`), including the after-loop sweep for merges extending past the row's last explicit
   cell.

## Rust-side notes

- Matching is by `local_name` (prefix-agnostic, like the old `lib.old.rs`), not true namespace
  resolution — `NsReader` would be the upgrade if hostile prefixes ever matter.
- `expand_empty_elements` is flipped on at entry so self-closed cells/rows don't need a parallel
  `Event::Empty` arm anywhere.
- Memory stays one sheet's rows at a time.
- `Error` is a placeholder for whatever unified error lands (needs `From<quick_xml::Error>` and the
  attribute error type).
- API details verified against quick-xml 0.41: `BytesText::xml_content(XmlVersion)` unescapes,
  `BytesCData::decode()` doesn't (correct — CDATA is literal), `Attribute::unescape_value()`,
  `Reader::config_mut().expand_empty_elements`.
