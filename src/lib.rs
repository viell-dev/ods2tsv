//! Library entry points for turning an OpenDocument Spreadsheet into one TSV file per sheet.
//!
//! There are two supported input containers:
//!
//! - `.ods` is a ZIP package. Its spreadsheet XML lives in `content.xml`.
//! - `.fods` is the same kind of spreadsheet XML stored directly as one flat file.
//!
//! The public API deliberately exposes the three stages separately:
//!
//! 1. [`OdsFile::open`] identifies and validates the input before output is created.
//! 2. [`OdsFile::sheets`] returns an iterator that parses one [`Sheet`] at a time.
//! 3. [`TsvWriter`] expands each sheet's compact row/cell representation and writes it.
//!
//! [`extract_ods_to_tsv`] composes those stages for callers that want the normal end-to-end
//! behavior. Output rows use RFC 4180-style quoting with tabs instead of commas and `\r\n` row
//! terminators.

mod content;
mod export;

use std::fs::{self, File};
use std::io::{self, BufReader, ErrorKind, Read, Seek};
use std::path::{Path, PathBuf};

use derive_more::{Display, Error, From};
use quick_xml::NsReader;
use quick_xml::events::Event;
use zip::ZipArchive;
use zip::result::ZipError;

pub use content::{Cell, Row, Sheet, SheetReader, ValueMode};
pub use export::{TsvWriter, write_sheet};

// ODF packages and flat documents identify their document kind with one of these media types.
// Templates contain the same spreadsheet structures, so they are useful extraction inputs too.
const SPREADSHEET_MIMETYPE: &str = "application/vnd.oasis.opendocument.spreadsheet";
const TEMPLATE_MIMETYPE: &str = "application/vnd.oasis.opendocument.spreadsheet-template";

/// Returns whether an ODF media type describes spreadsheet-shaped content.
///
/// This stays crate-private because callers should use [`OdsFile::open`] instead of duplicating
/// only one part of input validation.
pub(crate) fn is_spreadsheet_mimetype(mime: &str) -> bool {
    mime == SPREADSHEET_MIMETYPE || mime == TEMPLATE_MIMETYPE
}

/// Every failure that can occur while opening, parsing, or extracting a spreadsheet.
///
/// Keeping validation failures distinct lets the CLI map them to stable exit codes. The `From`
/// derivations also let implementation code use `?` with I/O, ZIP, and XML operations.
#[derive(Debug, Display, Error, From)]
pub enum Error {
    /// Opening, seeking, reading, creating, or writing a filesystem object failed.
    #[from]
    Io(io::Error),
    /// The input looked like ZIP but its package structure could not be read.
    #[from]
    Zip(ZipError),
    /// Spreadsheet XML was malformed or could not be decoded.
    #[display("invalid spreadsheet XML: {_0}")]
    #[from]
    Xml(quick_xml::Error),
    /// The package manifest says that at least one stream is encrypted.
    #[display("this file is password-protected; save a copy without a password first")]
    Encrypted,
    /// The file is an OpenDocument, but its declared mimetype is not a
    /// spreadsheet.
    #[display("not an OpenDocument spreadsheet (mimetype {_0:?})")]
    NotSpreadsheet(#[error(not(source))] String),
    /// The ZIP package has no `content.xml` entry.
    #[display("not a valid OpenDocument package: content.xml is missing")]
    MissingContent,
    /// The file is neither a ZIP package nor an XML document.
    #[display("not an OpenDocument spreadsheet (neither a ZIP package nor an XML document)")]
    UnrecognizedFormat,
}

/// An opened and validated OpenDocument spreadsheet: either a regular zipped
/// package or a flat XML (`.fods`) document.
pub struct OdsFile {
    source: Source,
}

/// The validated input container retained by [`OdsFile`].
///
/// A package keeps its `ZipArchive` open so [`OdsFile::sheets`] can borrow `content.xml` directly.
/// A flat document keeps the original file and rewinds it before every parse.
enum Source {
    Package(ZipArchive<BufReader<File>>),
    FlatXml(File),
}

/// The concrete sheet iterator produced by [`OdsFile::sheets`].
///
/// The boxed reader hides whether bytes come from a ZIP member or a normal file. The lifetime ties
/// the iterator to the mutable [`OdsFile`] borrow because a ZIP member borrows its archive.
pub type Sheets<'a> = SheetReader<BufReader<Box<dyn Read + 'a>>>;

impl OdsFile {
    /// Opens `path` and validates it without producing any output: the file
    /// must be a ZIP package with a spreadsheet (or absent) mimetype, an
    /// unencrypted `content.xml`, or a flat XML document.
    pub fn open(path: &Path) -> Result<Self, Error> {
        // Extensions are intentionally ignored. Reading the leading bytes makes renamed files work
        // and prevents a misleading `.ods` suffix from bypassing validation.
        let mut file = File::open(path)?;
        let mut magic = [0u8; 4];
        let read = read_up_to(&mut file, &mut magic)?;
        // Sniffing consumed up to four bytes. Both ZIP and XML readers must start at byte zero.
        file.rewind()?;

        let source = match &magic[..read] {
            // Only the local-file-header signature can begin an ODS package;
            // the other "PK" record signatures cannot start a valid one.
            [b'P', b'K', 0x03, 0x04] => {
                // Validation borrows the archive temporarily; the finished archive is then retained
                // so content.xml can be streamed later without reopening or reparsing the package.
                let mut archive = ZipArchive::new(BufReader::new(file))?;
                validate_package(&mut archive)?;
                Source::Package(archive)
            }
            m if looks_like_xml(m) => {
                // Flat-file validation performs only the root/mimetype check. Rewind afterward
                // because the real sheet iterator needs the entire document, including namespace
                // declarations on the root.
                content::validate_flat_root(BufReader::new(&mut file))?;
                file.rewind()?;
                Source::FlatXml(file)
            }
            _ => return Err(Error::UnrecognizedFormat),
        };
        Ok(Self { source })
    }

    /// Streams the sheets of the spreadsheet, parsing as it goes. `mode`
    /// selects between each cell's formatted display text and its raw
    /// stored value.
    pub fn sheets(&mut self, mode: ValueMode) -> Result<Sheets<'_>, Error> {
        // Erasing the concrete reader type here is a small, deliberate use of dynamic dispatch. It
        // keeps callers and SheetReader identical for package and flat inputs; XML parsing dominates
        // the negligible virtual-call cost.
        let raw: Box<dyn Read + '_> = match &mut self.source {
            Source::Package(archive) => Box::new(archive.by_name("content.xml")?),
            Source::FlatXml(file) => {
                file.rewind()?;
                Box::new(file)
            }
        };
        Ok(SheetReader::new(BufReader::new(raw), mode))
    }
}

/// Creates a fresh `<file stem> - <local timestamp>` directory under
/// `parent` (the current working directory when `parent` is empty) and
/// returns its path. When a directory from a same-second run already exists,
/// a ` (2)`, ` (3)`, … suffix keeps runs from writing into each other.
pub fn create_export_dir(ods_path: &Path, parent: &Path) -> io::Result<PathBuf> {
    // file_stem removes only the final extension. Lossy conversion guarantees that even a Unix path
    // containing non-UTF-8 bytes can still contribute a readable directory name.
    let stem = ods_path.file_stem().unwrap_or_default().to_string_lossy();
    let stem = if stem.is_empty() {
        "export".into()
    } else {
        stem
    };
    // Colons are avoided because they are forbidden in Windows file names. Local time is more useful
    // to a person browsing exports than UTC would be.
    let timestamp = jiff::Zoned::now().strftime("%Y-%m-%dT%H-%M-%S");
    let base = format!("{stem} - {timestamp}");

    // create_dir is the atomic collision check. A separate exists() check would have a race in which
    // two processes could both choose the same name.
    for attempt in 1..=100u32 {
        let name = if attempt == 1 {
            base.clone()
        } else {
            format!("{base} ({attempt})")
        };
        let dir = parent.join(name);
        match fs::create_dir(&dir) {
            Ok(()) => return Ok(dir),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(
        ErrorKind::AlreadyExists,
        format!("could not create a fresh export directory named {base:?}"),
    ))
}

/// Extracts every sheet of the spreadsheet at `ods_path` into a new export
/// directory created under `output_parent` (see [`create_export_dir`]), one
/// `.tsv` file per sheet. Returns the created directory.
pub fn extract_ods_to_tsv(
    ods_path: &Path,
    output_parent: &Path,
    mode: ValueMode,
) -> Result<PathBuf, Error> {
    // Keep this order: OdsFile::open performs all validation available before output. Failed
    // validation must not leave an empty export directory behind.
    let mut ods = OdsFile::open(ods_path)?;
    let output_dir = create_export_dir(ods_path, output_parent)?;
    let mut writer = TsvWriter::new(&output_dir);
    // SheetReader reports parse errors lazily, so either obtaining the iterator or advancing it may
    // fail.
    for sheet in ods.sheets(mode)? {
        writer.write(&sheet?)?;
    }
    Ok(output_dir)
}

fn read_up_to(file: &mut File, buf: &mut [u8]) -> io::Result<usize> {
    // Read::read is allowed to return fewer bytes than requested without reaching EOF. Looping
    // distinguishes a short read from a genuinely short file, which matters when recognizing
    // four-byte ZIP magic.
    let mut filled = 0;
    while filled < buf.len() {
        match file.read(&mut buf[filled..])? {
            0 => break,
            n => filled += n,
        }
    }
    Ok(filled)
}

fn looks_like_xml(magic: &[u8]) -> bool {
    // This is intentionally permissive. Its job is format routing, not XML validation; quick-xml
    // produces the useful error later. Leading whitespace covers legal XML that begins before its
    // declaration/root with spacing.
    magic.starts_with(&[0xEF, 0xBB, 0xBF]) // UTF-8 BOM
        || magic.starts_with(&[0xFF, 0xFE]) // UTF-16 LE BOM
        || magic.starts_with(&[0xFE, 0xFF]) // UTF-16 BE BOM
        || matches!(magic.first(), Some(b'<'))
        || matches!(magic, [b'<', 0, ..] | [0, b'<', ..]) // BOM-less UTF-16
        || matches!(magic.first(), Some(c) if c.is_ascii_whitespace())
}

fn validate_package<R: Read + Seek>(archive: &mut ZipArchive<R>) -> Result<(), Error> {
    // The mimetype entry is required of producers but tolerated as absent
    // here; when present it must declare a spreadsheet.
    if let Some(mimetype) = read_package_entry(archive, "mimetype")? {
        let mimetype = String::from_utf8_lossy(&mimetype).trim().to_owned();
        if !is_spreadsheet_mimetype(&mimetype) {
            return Err(Error::NotSpreadsheet(mimetype));
        }
    }

    if archive.index_for_name("content.xml").is_none() {
        return Err(Error::MissingContent);
    }

    // "Save with Password" keeps the file a valid ZIP but encrypts each
    // stream individually, recording the crypto parameters in the manifest.
    // Detect that up front and refuse with a clear message instead of failing
    // with a cryptic parse error on the encrypted content.xml.
    if let Some(manifest) = read_package_entry(archive, "META-INF/manifest.xml")?
        && manifest_declares_encryption(&manifest)
    {
        return Err(Error::Encrypted);
    }
    Ok(())
}

fn read_package_entry<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    name: &str,
) -> Result<Option<Vec<u8>>, Error> {
    // Small metadata entries are read eagerly because validation needs their complete contents.
    // content.xml is never routed through this helper; it remains streamed by OdsFile::sheets.
    match archive.by_name(name) {
        Ok(mut entry) => {
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf)?;
            Ok(Some(buf))
        }
        Err(ZipError::FileNotFound) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn manifest_declares_encryption(manifest: &[u8]) -> bool {
    // Namespace prefixes are arbitrary in XML, so compare the resolved local element name instead of
    // searching the raw bytes for the conventional `manifest:encryption-data` spelling.
    let mut reader = NsReader::from_reader(manifest);
    loop {
        // A malformed manifest is not itself fatal; the extraction then
        // stands or falls with content.xml.
        match reader.read_resolved_event() {
            Ok((_, Event::Start(e) | Event::Empty(e)))
                if e.local_name().as_ref() == b"encryption-data" =>
            {
                return true;
            }
            Ok((_, Event::Eof)) | Err(_) => return false,
            _ => {}
        }
    }
}
