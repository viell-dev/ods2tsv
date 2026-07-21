//! Streaming parse of spreadsheet content XML into a compressed sheet model.
//!
//! The model mirrors the document structure: row/column repeats and spans are
//! kept as counts rather than expanded, exactly as the file states them.
//! Expansion happens in [`crate::export`].

use std::io::BufRead;

use quick_xml::encoding::{DecodingReader, EncodingError};
use quick_xml::escape::EscapeError;
use quick_xml::events::{BytesDecl, BytesStart, Event};
use quick_xml::name::{Namespace, ResolveResult};
use quick_xml::{NsReader, XmlVersion};

use crate::Error;

/// Reads event payload bytes as UTF-8.
///
/// The [`DecodingReader`] transcodes the whole stream to UTF-8 up front, so
/// this is the correct decoding regardless of the source encoding. The
/// events' own `decode()`/`xml10_content()` must not be used: the parser
/// re-detects the source encoding from the XML declaration and would decode
/// the already-transcoded bytes a second time.
fn utf8(bytes: &[u8]) -> Result<&str, quick_xml::Error> {
    std::str::from_utf8(bytes).map_err(|e| quick_xml::Error::Encoding(EncodingError::Utf8(e)))
}

/// Applies the `encoding` pseudo-attribute of the XML declaration to the
/// stream decoder.
///
/// The [`DecodingReader`] detects UTF-8 and UTF-16 on its own but assumes
/// UTF-8 for every other, necessarily ASCII-compatible, stream; a declared
/// legacy encoding such as ISO-8859-1 has to be applied manually. Switching
/// is only possible while the reader's detection prefix is still buffered,
/// so a declaration ending past it is refused instead of mis-decoded.
fn apply_declared_encoding<R: BufRead>(
    reader: &mut NsReader<DecodingReader<R>>,
    decl: &BytesDecl<'_>,
) -> Result<(), Error> {
    // The `DecodingReader` buffers 64 source bytes (its private `PREFIX_CAP`)
    // for detection; up to 3 of them can be a UTF-8 BOM, which the buffer
    // position does not count.
    const DETECTION_WINDOW: u64 = 61;

    // An unknown encoding label stays on UTF-8: genuinely mismatched bytes
    // still fail decoding later instead of turning into garbage.
    let Some(declared) = decl.encoder() else {
        return Ok(());
    };
    let current = reader.get_ref().encoding();
    if declared == current
        // A UTF-16 stream was recognized from its BOM or byte pattern and is
        // already being transcoded; the declaration must not replace the
        // correctly-oriented decoder ("UTF-16" resolves to little endian).
        // A UTF-16 declaration on an ASCII-compatible stream is equally
        // impossible and ignored.
        || !current.is_ascii_compatible()
        || !declared.is_ascii_compatible()
    {
        return Ok(());
    }
    if reader.buffer_position() > DETECTION_WINDOW {
        // `set_encoding` panics once the prefix is drained, and everything
        // after the declaration was already decoded as UTF-8.
        return Err(quick_xml::Error::Encoding(EncodingError::Other(declared)).into());
    }
    reader.get_mut().set_encoding(declared);
    Ok(())
}

const NS_TABLE: Namespace<'_> = Namespace(b"urn:oasis:names:tc:opendocument:xmlns:table:1.0");
const NS_TEXT: Namespace<'_> = Namespace(b"urn:oasis:names:tc:opendocument:xmlns:text:1.0");
const NS_OFFICE: Namespace<'_> = Namespace(b"urn:oasis:names:tc:opendocument:xmlns:office:1.0");
const NS_DRAW: Namespace<'_> = Namespace(b"urn:oasis:names:tc:opendocument:xmlns:drawing:1.0");
const NS_DR3D: Namespace<'_> = Namespace(b"urn:oasis:names:tc:opendocument:xmlns:dr3d:1.0");

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

/// Which of a cell's two representations becomes its extracted text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ValueMode {
    /// The formatted display text, as the spreadsheet application shows it
    /// (localized dates and numbers, currency symbols). Falls back to the
    /// stored value for cells without display text.
    #[default]
    Formatted,
    /// The stored values from the `office:*-value` attributes (ISO dates,
    /// unlocalized numbers). Falls back to the display text for cells
    /// without a stored value, such as plain string cells.
    Raw,
}

struct PendingCell {
    covered: bool,
    repeat: usize,
    rows_spanned: usize,
    cols_spanned: usize,
    /// Value from the `office:*-value` attributes: the primary text in
    /// [`ValueMode::Raw`], the fallback for cells without paragraphs in
    /// [`ValueMode::Formatted`] (spreadsheet applications always write the
    /// formatted text as paragraphs; minimal producers may not).
    attr_value: Option<String>,
    paragraphs: Vec<String>,
    /// Whitespace-collapsing state of the paragraph being accumulated: the
    /// last content ended in whitespace (so further character-data
    /// whitespace is ignored).
    last_was_ws: bool,
    /// A collapsed space is owed but not yet written; it materializes before
    /// the next content and evaporates at the paragraph end.
    pending_space: bool,
}

impl PendingCell {
    fn start_paragraph(&mut self) {
        self.paragraphs.push(String::new());
        // Whitespace at the start of a paragraph is ignored.
        self.last_was_ws = true;
        self.pending_space = false;
    }

    /// Appends character data (text, CDATA, resolved references), applying
    /// the ODF whitespace rules: runs of space/tab/CR/LF collapse to a
    /// single space, whitespace following whitespace is ignored even across
    /// child elements, and whitespace at paragraph edges disappears.
    fn push_characters(&mut self, text: &str) {
        let Some(para) = self.paragraphs.last_mut() else {
            return;
        };
        for ch in text.chars() {
            if matches!(ch, ' ' | '\t' | '\r' | '\n') {
                if !self.last_was_ws {
                    self.pending_space = true;
                    self.last_was_ws = true;
                }
            } else {
                if self.pending_space {
                    para.push(' ');
                    self.pending_space = false;
                }
                para.push(ch);
                self.last_was_ws = false;
            }
        }
    }

    /// Appends whitespace that an element (`text:s`, `text:tab`,
    /// `text:line-break`) encodes explicitly; unlike character data it is
    /// exempt from collapsing.
    fn push_explicit(&mut self, ch: char, count: usize) {
        let Some(para) = self.paragraphs.last_mut() else {
            return;
        };
        if self.pending_space {
            para.push(' ');
            self.pending_space = false;
        }
        para.extend(std::iter::repeat_n(ch, count));
        self.last_was_ws = true;
    }

    fn finish(self, mode: ValueMode) -> Cell {
        if self.covered {
            Cell::Covered {
                repeat: self.repeat,
            }
        } else {
            let text = match mode {
                ValueMode::Formatted if !self.paragraphs.is_empty() => self.paragraphs.join("\n"),
                ValueMode::Formatted => self.attr_value.unwrap_or_default(),
                ValueMode::Raw => self
                    .attr_value
                    .unwrap_or_else(|| self.paragraphs.join("\n")),
            };
            Cell::Value {
                repeat: self.repeat,
                rows_spanned: self.rows_spanned,
                cols_spanned: self.cols_spanned,
                text,
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
    mode: ValueMode,
    done: bool,
}

impl<R: BufRead> SheetReader<R> {
    pub fn new(source: R, mode: ValueMode) -> Self {
        let mut reader = NsReader::from_reader(DecodingReader::new(source));
        reader.config_mut().expand_empty_elements = true;
        Self {
            reader,
            buf: Vec::new(),
            mode,
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
        // Depth inside a subtree whose content is not cell text (cell
        // comments, embedded shapes, nested tables, change history).
        let mut skip_depth = 0usize;

        loop {
            self.buf.clear();
            match self.reader.read_resolved_event_into(&mut self.buf)? {
                (_, Event::Eof) => {
                    self.done = true;
                    // Best effort for truncated input: an unclosed sheet is
                    // still returned rather than dropped.
                    return Ok(sheet);
                }
                (_, Event::Start(_)) if skip_depth > 0 => skip_depth += 1,
                (_, Event::End(_)) if skip_depth > 0 => skip_depth -= 1,
                (_, _) if skip_depth > 0 => {}
                (ResolveResult::Bound(ns), Event::Start(e))
                    if skips_subtree(ns, &e, sheet.is_some()) =>
                {
                    skip_depth = 1;
                }
                (ResolveResult::Bound(ns), Event::Start(e)) if ns == NS_TABLE => {
                    match e.local_name().as_ref() {
                        b"table" => {
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
                                repeat: attr_count(
                                    &mut self.reader,
                                    &e,
                                    b"number-columns-repeated",
                                )?,
                                rows_spanned: attr_count(
                                    &mut self.reader,
                                    &e,
                                    b"number-rows-spanned",
                                )?,
                                cols_spanned: attr_count(
                                    &mut self.reader,
                                    &e,
                                    b"number-columns-spanned",
                                )?,
                                attr_value: attr_cell_value(&mut self.reader, &e)?,
                                paragraphs: Vec::new(),
                                last_was_ws: true,
                                pending_space: false,
                            });
                        }
                        b"covered-table-cell" if row.is_some() && cell.is_none() => {
                            cell = Some(PendingCell {
                                covered: true,
                                repeat: attr_count(
                                    &mut self.reader,
                                    &e,
                                    b"number-columns-repeated",
                                )?,
                                rows_spanned: 1,
                                cols_spanned: 1,
                                attr_value: None,
                                paragraphs: Vec::new(),
                                last_was_ws: true,
                                pending_space: false,
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
                                    c.start_paragraph();
                                }
                                para_depth += 1;
                            }
                            b"s" if para_depth > 0 => {
                                // `text:c` counts the spaces; 0 is valid.
                                let count = attr_string(&mut self.reader, &e, NS_TEXT, b"c")?
                                    .and_then(|v| v.parse().ok())
                                    .unwrap_or(1);
                                c.push_explicit(' ', count);
                            }
                            b"tab" if para_depth > 0 => c.push_explicit('\t', 1),
                            b"line-break" if para_depth > 0 => c.push_explicit('\n', 1),
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
                                r.cells.push(c.finish(self.mode));
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
                    {
                        c.push_characters(utf8(&t)?);
                    }
                }
                (_, Event::CData(t)) => {
                    if para_depth > 0
                        && let Some(c) = cell.as_mut().filter(|c| !c.covered)
                    {
                        c.push_characters(utf8(&t)?);
                    }
                }
                // Entity and character references arrive as separate events.
                (_, Event::GeneralRef(r)) => {
                    if para_depth > 0
                        && let Some(c) = cell.as_mut().filter(|c| !c.covered)
                    {
                        let resolved = resolve_reference(utf8(&r)?)?;
                        c.push_characters(resolved.encode_utf8(&mut [0u8; 4]));
                    }
                }
                (_, Event::Decl(e)) => apply_declared_encoding(&mut self.reader, &e)?,
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

/// Checks the root element of a flat XML document without parsing further:
/// when it declares an `office:mimetype`, it must be a spreadsheet. Lets
/// [`crate::OdsFile::open`] refuse text documents and the like before any
/// output is created.
pub(crate) fn validate_flat_root<R: BufRead>(source: R) -> Result<(), Error> {
    let mut reader = NsReader::from_reader(DecodingReader::new(source));
    reader.config_mut().expand_empty_elements = true;
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader.read_resolved_event_into(&mut buf)? {
            (ResolveResult::Bound(ns), Event::Start(e)) if ns == NS_OFFICE => {
                if e.local_name().as_ref() == b"document"
                    && let Some(mime) = attr_string(&mut reader, &e, NS_OFFICE, b"mimetype")?
                    && !crate::is_spreadsheet_mimetype(&mime)
                {
                    return Err(Error::NotSpreadsheet(mime));
                }
                return Ok(());
            }
            // Any other root element is left to the sheet parser.
            (_, Event::Start(_) | Event::Eof) => return Ok(()),
            // The declared encoding must be honored here too: reaching the
            // root element can require decoding past pure-ASCII content.
            (_, Event::Decl(e)) => apply_declared_encoding(&mut reader, &e)?,
            _ => {}
        }
    }
}

/// Resolves a general entity or character reference (the text between `&`
/// and `;`). Only the five predefined XML entities are known; anything else
/// would need a DTD, which OpenDocument files don't use.
fn resolve_reference(name: &str) -> Result<char, Error> {
    let char_ref = |digits: &str, radix| {
        u32::from_str_radix(digits, radix)
            .ok()
            .and_then(char::from_u32)
    };
    let resolved = match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        _ => name.strip_prefix('#').and_then(|num| {
            num.strip_prefix(['x', 'X'])
                .map_or_else(|| char_ref(num, 10), |hex| char_ref(hex, 16))
        }),
    };
    resolved.ok_or_else(|| {
        quick_xml::Error::Escape(EscapeError::UnrecognizedEntity(0..0, name.to_owned())).into()
    })
}

/// Whether this element opens a subtree whose paragraphs are not cell text:
/// a table nested inside a cell, cell comments, embedded shapes, or the
/// change-tracking history (which stores previous cell content).
fn skips_subtree(ns: Namespace<'_>, e: &BytesStart<'_>, in_sheet: bool) -> bool {
    if ns == NS_TABLE {
        match e.local_name().as_ref() {
            b"table" => in_sheet,
            b"tracked-changes" => true,
            _ => false,
        }
    } else if ns == NS_OFFICE {
        e.local_name().as_ref() == b"annotation"
    } else {
        ns == NS_DRAW || ns == NS_DR3D
    }
}

/// Reads a repeat/span count attribute, defaulting to 1 when absent or not a
/// positive integer (the spec requires positive values).
fn attr_count<R>(
    reader: &mut NsReader<R>,
    e: &BytesStart<'_>,
    local: &[u8],
) -> Result<usize, quick_xml::Error> {
    let value = attr_string(reader, e, NS_TABLE, local)?;
    Ok(value
        .and_then(|v| v.parse().ok())
        .filter(|&n| n >= 1)
        .unwrap_or(1))
}

/// Picks the display text for a cell without paragraphs from the value
/// attributes. `office:string-value` wins, then the others in a fixed order;
/// non-string values are used verbatim as the file states them.
fn attr_cell_value<R>(
    reader: &mut NsReader<R>,
    e: &BytesStart<'_>,
) -> Result<Option<String>, quick_xml::Error> {
    for local in [
        b"string-value".as_slice(),
        b"value",
        b"date-value",
        b"time-value",
        b"boolean-value",
    ] {
        if let Some(value) = attr_string(reader, e, NS_OFFICE, local)? {
            return Ok(Some(value));
        }
    }
    Ok(None)
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
