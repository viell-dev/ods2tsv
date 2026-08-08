// todo: common xml parser stuff

use std::{
    fs::File,
    io::{BufReader, Read},
};

use quick_xml::{Reader, encoding::DecodingReader, events::Event};
use zip::read::ZipFile;

#[expect(
    clippy::large_enum_variant,
    reason = "one short-lived instance per document; boxing buys nothing"
)]
pub enum XmlSource<'a> {
    Package(ZipFile<'a, BufReader<File>>),
    Flat(File),
}

impl From<File> for XmlSource<'_> {
    fn from(file: File) -> Self {
        Self::Flat(file)
    }
}

impl<'a> From<ZipFile<'a, BufReader<File>>> for XmlSource<'a> {
    fn from(file: ZipFile<'a, BufReader<File>>) -> Self {
        Self::Package(file)
    }
}

impl Read for XmlSource<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Package(reader) => reader.read(buf),
            Self::Flat(reader) => reader.read(buf),
        }
    }
}

pub type XmlReader<'a> = Reader<DecodingReader<BufReader<XmlSource<'a>>>>;
pub type Xml<'a> = (XmlReader<'a>, Option<Event<'static>>);

/// Opens an XML stream, honors its `encoding` declaration, and returns the
/// reader positioned after the prolog. If the document had no declaration,
/// the already-consumed first event is handed back so nothing is lost.
pub fn open_xml<'a>(source: impl Into<XmlSource<'a>>) -> quick_xml::Result<Xml<'a>> {
    let mut reader = Reader::from_reader(DecodingReader::new(BufReader::new(source.into())));

    let mut buf = Vec::new();
    match reader.read_event_into(&mut buf)? {
        Event::Decl(decl) => {
            let decoding = reader.get_mut();
            // BOM/byte-pattern detection already ran; it only ever yields
            // UTF-8 or UTF-16, and UTF-16 evidence outranks the encoding
            // attribute. Honor the attribute only when detection kept the
            // ASCII-compatible default, the label is known (`encoder()` is
            // `None` for missing or unresolvable labels. Fall back to UTF-8
            // rather than erroring), and the target is itself ASCII-compatible
            // (a declared UTF-16 in a byte stream that parsed as ASCII is
            // wrong, and `set_encoding` requires ASCII compatibility).
            if decoding.encoding().is_ascii_compatible()
                && let Some(encoding) = decl.encoder()
                && encoding.is_ascii_compatible()
            {
                decoding.set_encoding(encoding);
            }
            Ok((reader, None))
        }
        Event::Eof => Ok((reader, None)),
        other => Ok((reader, Some(other.into_owned()))),
    }
}
