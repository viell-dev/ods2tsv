//! Extracts each sheet of an OpenDocument Spreadsheet (`.ods`) into separate
//! tab-separated (`.tsv`) files.
//!
//! This is a behavior-parity port of the original Python implementation; the
//! Python version's semantics are the spec. Output rows match Python's
//! `csv.writer` with a tab delimiter: minimal quoting and `\r\n` row
//! terminators.

mod content;
mod export;

use std::fmt;
use std::fs;
use std::io::{self, Read, Seek, Write};
use std::path::{Path, PathBuf};

use zip::ZipArchive;
use zip::result::ZipError;

pub use content::{Cell, Row, Sheet};

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    Zip(ZipError),
    Xml(quick_xml::Error),
    Encrypted,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::Zip(e) => write!(f, "{e}"),
            Self::Xml(e) => write!(f, "invalid content.xml: {e}"),
            Self::Encrypted => write!(
                f,
                "this file is password-protected; save a copy without a password first"
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
            Self::Encrypted => None,
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

/// Extracts every sheet of the spreadsheet at `ods_path` into a new
/// `<basename> - <ISO-datetime>` directory created under `output_parent`,
/// one `.tsv` file per sheet. Returns the created directory.
///
/// Pass an empty path as `output_parent` to export into the current working
/// directory, like the original Python script.
pub fn extract_ods_to_tsv(ods_path: &Path, output_parent: &Path) -> Result<PathBuf, Error> {
    let basename = ods_path.file_stem().unwrap_or_default().to_string_lossy();
    let timestamp = jiff::Zoned::now().strftime("%Y-%m-%dT%H-%M-%S");
    let output_dir = output_parent.join(format!("{basename} - {timestamp}"));

    fs::create_dir_all(&output_dir)?;
    println!("Exporting to directory: {}", output_dir.display());

    let mut archive = ZipArchive::new(io::BufReader::new(fs::File::open(ods_path)?))?;
    ensure_not_encrypted(&mut archive)?;

    let xml = {
        let mut buf = String::new();
        archive.by_name("content.xml")?.read_to_string(&mut buf)?;
        buf
    };

    for sheet in content::parse(&xml)? {
        println!("  Extracting {}...", sheet.name);
        let tsv_path = output_dir.join(format!("{}.tsv", sanitize_sheet_name(&sheet.name)));
        let mut out = io::BufWriter::new(fs::File::create(&tsv_path)?);
        export::write_sheet(&sheet, &mut out)?;
        out.flush()?;
    }

    Ok(output_dir)
}

/// LibreOffice's "Save with Password" keeps the file a valid ZIP but encrypts
/// each stream individually, recording the crypto parameters in the manifest.
/// Detect that up front and refuse with a clear message instead of failing
/// with a cryptic parse error on the encrypted `content.xml`.
fn ensure_not_encrypted<R: Read + Seek>(archive: &mut ZipArchive<R>) -> Result<(), Error> {
    match archive.by_name("META-INF/manifest.xml") {
        Ok(mut manifest) => {
            let mut buf = Vec::new();
            manifest.read_to_end(&mut buf)?;
            if String::from_utf8_lossy(&buf).contains("encryption-data") {
                return Err(Error::Encrypted);
            }
            Ok(())
        }
        Err(ZipError::FileNotFound) => Ok(()),
        Err(e) => Err(e.into()),
    }
}

fn sanitize_sheet_name(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, ' ' | '.' | '_' | '-'))
        .collect::<String>()
        .trim()
        .to_owned()
}
