//! Streaming parse of spreadsheet content XML into a compressed sheet model.
//!
//! The model mirrors the document structure: row/column repeats and spans are
//! kept as counts rather than expanded, exactly as the file states them.
//! Expansion happens in [`crate::export`].

use std::io::BufRead;

use quick_xml::encoding::DecodingReader;
use quick_xml::escape::EscapeError;
use quick_xml::events::{BytesStart, Event};
use quick_xml::name::{Namespace, ResolveResult};
use quick_xml::{NsReader, XmlVersion};

use crate::Error;

const NS_TABLE: Namespace<'_> = Namespace(b"urn:oasis:names:tc:opendocument:xmlns:table:1.0");
const NS_TEXT: Namespace<'_> = Namespace(b"urn:oasis:names:tc:opendocument:xmlns:text:1.0");
const NS_OFFICE: Namespace<'_> = Namespace(b"urn:oasis:names:tc:opendocument:xmlns:office:1.0");

pub struct Sheet {
    pub name: String,
    pub rows: Vec<Row>,
}

pub struct Row {
    pub repeat: usize,
    pub cells: Vec<Cell>,
}

pub enum Cell {
    Value {
        repeat: usize,
        rows_spanned: usize,
        cols_spanned: usize,
        text: String,
    },
    /// A cell hidden under a merged area. Any content it carries is ignored;
    /// the export step fills it from the merged source value.
    Covered { repeat: usize },
}

struct PendingCell {
    covered: bool,
    repeat: usize,
    rows_spanned: usize,
    cols_spanned: usize,
    paragraphs: Vec<String>,
}

impl PendingCell {
    fn finish(self) -> Cell {
        if self.covered {
            Cell::Covered {
                repeat: self.repeat,
            }
        } else {
            Cell::Value {
                repeat: self.repeat,
                rows_spanned: self.rows_spanned,
                cols_spanned: self.cols_spanned,
                text: self.paragraphs.join("\n"),
            }
        }
    }
}

/// Streaming reader that yields one [`Sheet`] per `table:table` element as it
/// is parsed, without materializing the whole document.
///
/// The input may be a `content.xml` stream from a package or a whole flat XML
/// document; any encoding the XML declaration announces is handled. When the
/// root element carries an `office:mimetype` (flat documents do), it must be
/// the spreadsheet mimetype.
pub struct SheetReader<R: BufRead> {
    reader: NsReader<DecodingReader<R>>,
    buf: Vec<u8>,
    done: bool,
}

impl<R: BufRead> SheetReader<R> {
    pub fn new(source: R) -> Self {
        let mut reader = NsReader::from_reader(DecodingReader::new(source));
        reader.config_mut().expand_empty_elements = true;
        Self {
            reader,
            buf: Vec::new(),
            done: false,
        }
    }

    #[expect(
        clippy::too_many_lines,
        reason = "single event-loop state machine; splitting the match arms apart would obscure the flow"
    )]
    fn next_sheet(&mut self) -> Result<Option<Sheet>, Error> {
        let mut sheet: Option<Sheet> = None;
        let mut row: Option<Row> = None;
        let mut cell: Option<PendingCell> = None;
        // Depth of nested `text:p`/`text:h` elements inside the current cell.
        // Text is only collected from paragraph content.
        let mut para_depth = 0usize;

        loop {
            self.buf.clear();
            match self.reader.read_resolved_event_into(&mut self.buf)? {
                (ResolveResult::Bound(ns), Event::Start(e)) if ns == NS_TABLE => {
                    match e.local_name().as_ref() {
                        b"table" if sheet.is_none() => {
                            sheet = Some(Sheet {
                                name: attr_string(&mut self.reader, &e, NS_TABLE, b"name")?
                                    .unwrap_or_default(),
                                rows: Vec::new(),
                            });
                        }
                        b"table-row" if sheet.is_some() && row.is_none() => {
                            row = Some(Row {
                                repeat: attr_count(&mut self.reader, &e, b"number-rows-repeated")?,
                                cells: Vec::new(),
                            });
                        }
                        b"table-cell" if row.is_some() && cell.is_none() => {
                            cell = Some(PendingCell {
                                covered: false,
                                repeat: attr_count(&mut self.reader, &e, b"number-columns-repeated")?,
                                rows_spanned: attr_count(&mut self.reader, &e, b"number-rows-spanned")?,
                                cols_spanned: attr_count(&mut self.reader, &e, b"number-columns-spanned")?,
                                paragraphs: Vec::new(),
                            });
                        }
                        b"covered-table-cell" if row.is_some() && cell.is_none() => {
                            cell = Some(PendingCell {
                                covered: true,
                                repeat: attr_count(&mut self.reader, &e, b"number-columns-repeated")?,
                                rows_spanned: 1,
                                cols_spanned: 1,
                                paragraphs: Vec::new(),
                            });
                        }
                        _ => {}
                    }
                }
                (ResolveResult::Bound(ns), Event::Start(e)) if ns == NS_OFFICE => {
                    // Flat XML documents carry the mimetype on the root
                    // element; refuse text documents and the like early.
                    if e.local_name().as_ref() == b"document"
                        && let Some(mime) =
                            attr_string(&mut self.reader, &e, NS_OFFICE, b"mimetype")?
                        && !crate::is_spreadsheet_mimetype(&mime)
                    {
                        return Err(Error::NotSpreadsheet(mime));
                    }
                }
                (ResolveResult::Bound(ns), Event::Start(e)) if ns == NS_TEXT => {
                    if let Some(c) = cell.as_mut().filter(|c| !c.covered) {
                        match e.local_name().as_ref() {
                            b"p" | b"h" => {
                                if para_depth == 0 {
                                    c.paragraphs.push(String::new());
                                }
                                para_depth += 1;
                            }
                            b"s" if para_depth > 0 => {
                                let count = attr_count(&mut self.reader, &e, b"c")?;
                                if let Some(para) = c.paragraphs.last_mut() {
                                    para.push_str(&" ".repeat(count));
                                }
                            }
                            b"line-break" if para_depth > 0 => {
                                if let Some(para) = c.paragraphs.last_mut() {
                                    para.push('\n');
                                }
                            }
                            _ => {}
                        }
                    }
                }
                (ResolveResult::Bound(ns), Event::End(e)) if ns == NS_TABLE => {
                    match e.local_name().as_ref() {
                        b"table" => {
                            if let Some(s) = sheet.take() {
                                return Ok(Some(s));
                            }
                        }
                        b"table-row" => {
                            if let (Some(s), Some(r)) = (sheet.as_mut(), row.take()) {
                                s.rows.push(r);
                            }
                        }
                        b"table-cell" | b"covered-table-cell" => {
                            if let (Some(r), Some(c)) = (row.as_mut(), cell.take()) {
                                r.cells.push(c.finish());
                            }
                            para_depth = 0;
                        }
                        _ => {}
                    }
                }
                (ResolveResult::Bound(ns), Event::End(e))
                    if ns == NS_TEXT && matches!(e.local_name().as_ref(), b"p" | b"h") =>
                {
                    if cell.as_ref().is_some_and(|c| !c.covered) {
                        para_depth = para_depth.saturating_sub(1);
                    }
                }
                (_, Event::Text(t)) => {
                    if para_depth > 0
                        && let Some(c) = cell.as_mut().filter(|c| !c.covered)
                        && let Some(para) = c.paragraphs.last_mut()
                    {
                        para.push_str(&t.xml10_content().map_err(quick_xml::Error::Encoding)?);
                    }
                }
                // Entity and character references arrive as separate events.
                (_, Event::GeneralRef(r)) => {
                    if para_depth > 0
                        && let Some(c) = cell.as_mut().filter(|c| !c.covered)
                        && let Some(para) = c.paragraphs.last_mut()
                    {
                        if let Some(ch) = r.resolve_char_ref()? {
                            para.push(ch);
                        } else {
                            let name = r.decode().map_err(quick_xml::Error::Encoding)?;
                            match name.as_ref() {
                                "amp" => para.push('&'),
                                "lt" => para.push('<'),
                                "gt" => para.push('>'),
                                "quot" => para.push('"'),
                                "apos" => para.push('\''),
                                _ => {
                                    return Err(quick_xml::Error::Escape(
                                        EscapeError::UnrecognizedEntity(0..0, name.into_owned()),
                                    )
                                    .into());
                                }
                            }
                        }
                    }
                }
                (_, Event::Eof) => {
                    self.done = true;
                    // Best effort for truncated input: an unclosed sheet is
                    // still returned rather than dropped.
                    return Ok(sheet);
                }
                _ => {}
            }
        }
    }
}

impl<R: BufRead> Iterator for SheetReader<R> {
    type Item = Result<Sheet, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        match self.next_sheet() {
            Ok(sheet) => sheet.map(Ok),
            Err(e) => {
                self.done = true;
                Some(Err(e))
            }
        }
    }
}

/// Reads a numeric `table:*` attribute (or `text:c` for spaces), defaulting
/// to 1 when the attribute is absent or not a sensible count.
fn attr_count<R>(
    reader: &mut NsReader<R>,
    e: &BytesStart<'_>,
    local: &[u8],
) -> Result<usize, quick_xml::Error> {
    // `text:c` is the only non-table attribute we read; accept either
    // namespace since local names never collide between the two.
    let value =
        attr_string(reader, e, NS_TABLE, local)?.or(attr_string(reader, e, NS_TEXT, local)?);
    Ok(value.and_then(|v| v.parse().ok()).unwrap_or(1))
}

fn attr_string<R>(
    reader: &mut NsReader<R>,
    e: &BytesStart<'_>,
    ns: Namespace<'_>,
    local: &[u8],
) -> Result<Option<String>, quick_xml::Error> {
    for attr in e.attributes() {
        let attr = attr?;
        let matched = {
            let (resolved, name) = reader.resolver_mut().resolve_attribute(attr.key);
            name.as_ref() == local && matches!(resolved, ResolveResult::Bound(b) if b == ns)
        };
        if matched {
            return Ok(Some(
                attr.normalized_value(XmlVersion::Implicit1_0)?.into_owned(),
            ));
        }
    }
    Ok(None)
}
