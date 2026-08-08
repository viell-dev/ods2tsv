use std::{
    fs::File,
    io::{self, ErrorKind, Read, Seek},
    path::Path,
};

use derive_more::{Display, Error, From};

use crate::magic::{
    UTF8_BOM, UTF16_BE_BOM, UTF16_LE_BOM, UTF32_BE_BOM, UTF32_LE_BOM, ZIP_LOCAL_HEADER,
};

/// Sniff errors
#[derive(Debug, Error, From, Display)]
pub enum Error {
    Io(io::Error),

    #[display("UTF-32 unsupported")]
    #[from(ignore)]
    Utf32Unsupported,

    #[display("Unknown file format")]
    #[from(ignore)]
    UnknownFormat,
}

/// Sniff format guess
pub enum Format {
    /// Looks like a ZIP file
    Zip(File),
    /// Looks like an XML file
    Xml(File),
}

fn first_non_whitespace_u16(bytes: &[u8], decode: impl Fn([u8; 2]) -> u16) -> Option<u16> {
    bytes
        .chunks_exact(2)
        .map(|chunk| decode([chunk[0], chunk[1]]))
        .find(|unit| !matches!(unit, 0x20 | 0x09 | 0x0A | 0x0D))
}

/// Try to identify the file variant.
pub fn sniff(path: &Path) -> Result<Format, Error> {
    let mut file = File::open(path)?;

    if !file.metadata()?.is_file() {
        return Err(
            io::Error::new(io::ErrorKind::InvalidInput, "path is not a regular file").into(),
        );
    }

    let mut head = [0u8; 128]; // maybe change to 16 or so later.
    file.read_exact(&mut head).map_err(|err| match err.kind() {
        ErrorKind::UnexpectedEof => Error::UnknownFormat,
        _ => err.into(),
    })?;

    let is_flat = match &head {
        // ZIP local-file header
        bytes if bytes.starts_with(ZIP_LOCAL_HEADER) => {
            file.rewind()?;
            return Ok(Format::Zip(file));
        }
        // UTF-32 is not supported
        bytes if bytes.starts_with(UTF32_LE_BOM) || bytes.starts_with(UTF32_BE_BOM) => {
            return Err(Error::Utf32Unsupported);
        }
        // UTF-16 LE BOM
        bytes if let Some(rest) = bytes.strip_prefix(UTF16_LE_BOM) => {
            first_non_whitespace_u16(rest, u16::from_le_bytes) == Some(b'<' as u16)
        }
        // UTF-16 BE BOM
        bytes if let Some(rest) = bytes.strip_prefix(UTF16_BE_BOM) => {
            first_non_whitespace_u16(rest, u16::from_be_bytes) == Some(b'<' as u16)
        }
        // UTF-8 BOM
        bytes if let Some(rest) = bytes.strip_prefix(UTF8_BOM) => {
            rest.trim_ascii_start().starts_with(b"<")
        }
        // None of those
        rest => rest.trim_ascii_start().starts_with(b"<"),
    };

    if is_flat {
        file.rewind()?;
        Ok(Format::Xml(file))
    } else {
        Err(Error::UnknownFormat)
    }
}
