//! Extracts each sheet of an OpenDocument Spreadsheet (`.ods`) into separate
//! tab-separated (`.tsv`) files.
//!
//! Output rows follow RFC 4180 conventions with a tab delimiter: minimal
//! quoting and `\r\n` row terminators.
//!
//! The building blocks are exposed individually — [`OdsFile::open`] validates
//! the input up front (before any output is created), [`OdsFile::sheets`]
//! streams the parsed sheets, and [`TsvWriter`] writes them — while
//! [`extract_ods_to_tsv`] composes them for the common case.

mod content;
mod export;

use std::fmt;
use std::fs::{self, File};
use std::io::{self, BufReader, ErrorKind, Read, Seek};
use std::path::{Path, PathBuf};

use quick_xml::NsReader;
use quick_xml::events::Event;
use zip::ZipArchive;
use zip::result::ZipError;

pub use content::{Cell, Row, Sheet, SheetReader};
pub use export::{TsvWriter, write_sheet};

const SPREADSHEET_MIMETYPE: &str = "application/vnd.oasis.opendocument.spreadsheet";
const TEMPLATE_MIMETYPE: &str = "application/vnd.oasis.opendocument.spreadsheet-template";

pub(crate) fn is_spreadsheet_mimetype(mime: &str) -> bool {
    mime == SPREADSHEET_MIMETYPE || mime == TEMPLATE_MIMETYPE
}

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    Zip(ZipError),
    Xml(quick_xml::Error),
    Encrypted,
    /// The file is an OpenDocument, but its declared mimetype is not a
    /// spreadsheet.
    NotSpreadsheet(String),
    /// The ZIP package has no `content.xml` entry.
    MissingContent,
    /// The file is neither a ZIP package nor an XML document.
    UnrecognizedFormat,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::Zip(e) => write!(f, "{e}"),
            Self::Xml(e) => write!(f, "invalid spreadsheet XML: {e}"),
            Self::Encrypted => write!(
                f,
                "this file is password-protected; save a copy without a password first"
            ),
            Self::NotSpreadsheet(mime) => {
                write!(f, "not an OpenDocument spreadsheet (mimetype {mime:?})")
            }
            Self::MissingContent => {
                write!(
                    f,
                    "not a valid OpenDocument package: content.xml is missing"
                )
            }
            Self::UnrecognizedFormat => write!(
                f,
                "not an OpenDocument spreadsheet (neither a ZIP package nor an XML document)"
            ),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Zip(e) => Some(e),
            Self::Xml(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<ZipError> for Error {
    fn from(e: ZipError) -> Self {
        Self::Zip(e)
    }
}

impl From<quick_xml::Error> for Error {
    fn from(e: quick_xml::Error) -> Self {
        Self::Xml(e)
    }
}

/// An opened and validated OpenDocument spreadsheet: either a regular zipped
/// package or a flat XML (`.fods`) document.
pub struct OdsFile {
    source: Source,
}

enum Source {
    Package(ZipArchive<BufReader<File>>),
    FlatXml(File),
}

/// The sheet stream produced by [`OdsFile::sheets`].
pub type Sheets<'a> = SheetReader<BufReader<Box<dyn Read + 'a>>>;

impl OdsFile {
    /// Opens `path` and validates it without producing any output: the file
    /// must be a ZIP package with a spreadsheet (or absent) mimetype, an
    /// unencrypted `content.xml`, or a flat XML document.
    pub fn open(path: &Path) -> Result<Self, Error> {
        let mut file = File::open(path)?;
        let mut magic = [0u8; 4];
        let read = read_up_to(&mut file, &mut magic)?;
        file.rewind()?;

        let source = match &magic[..read] {
            m if m.starts_with(b"PK") => {
                let mut archive = ZipArchive::new(BufReader::new(file))?;
                validate_package(&mut archive)?;
                Source::Package(archive)
            }
            m if looks_like_xml(m) => {
                content::validate_flat_root(BufReader::new(&mut file))?;
                file.rewind()?;
                Source::FlatXml(file)
            }
            _ => return Err(Error::UnrecognizedFormat),
        };
        Ok(Self { source })
    }

    /// Streams the sheets of the spreadsheet, parsing as it goes.
    pub fn sheets(&mut self) -> Result<Sheets<'_>, Error> {
        let raw: Box<dyn Read + '_> = match &mut self.source {
            Source::Package(archive) => Box::new(archive.by_name("content.xml")?),
            Source::FlatXml(file) => {
                file.rewind()?;
                Box::new(file)
            }
        };
        Ok(SheetReader::new(BufReader::new(raw)))
    }
}

/// Creates a fresh `<file stem> - <local timestamp>` directory under
/// `parent` (the current working directory when `parent` is empty) and
/// returns its path. When a directory from a same-second run already exists,
/// a ` (2)`, ` (3)`, … suffix keeps runs from writing into each other.
pub fn create_export_dir(ods_path: &Path, parent: &Path) -> io::Result<PathBuf> {
    let stem = ods_path.file_stem().unwrap_or_default().to_string_lossy();
    let stem = if stem.is_empty() {
        "export".into()
    } else {
        stem
    };
    let timestamp = jiff::Zoned::now().strftime("%Y-%m-%dT%H-%M-%S");
    let base = format!("{stem} - {timestamp}");

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
pub fn extract_ods_to_tsv(ods_path: &Path, output_parent: &Path) -> Result<PathBuf, Error> {
    let mut ods = OdsFile::open(ods_path)?;
    let output_dir = create_export_dir(ods_path, output_parent)?;
    let mut writer = TsvWriter::new(&output_dir);
    for sheet in ods.sheets()? {
        writer.write(&sheet?)?;
    }
    Ok(output_dir)
}

fn read_up_to(file: &mut File, buf: &mut [u8]) -> io::Result<usize> {
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
