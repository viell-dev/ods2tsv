use std::{
    fmt, io,
    path::{Path, PathBuf},
    process::ExitCode,
};

use derive_more::From;
use zip::result::ZipError;

use crate::{package::OdsPackage, sniff::sniff};

mod dir;
mod flat;
mod helpers;
mod magic;
mod ods;
mod package;
mod sniff;
mod xml;

#[derive(Debug)]
pub enum CliError {
    Generic(String),
    Unusable(String),
    Parse(String),
    Encrypted(String),
}

impl CliError {
    const fn exit_code(&self) -> u8 {
        match self {
            Self::Generic(_) => 1,
            Self::Unusable(_) => 3,
            Self::Parse(_) => 4,
            Self::Encrypted(_) => 5,
        }
    }

    const fn message(&self) -> &String {
        match self {
            Self::Generic(message)
            | Self::Unusable(message)
            | Self::Parse(message)
            | Self::Encrypted(message) => message,
        }
    }
}

impl From<Error> for CliError {
    fn from(_value: Error) -> Self {
        todo!()
    }
}

impl From<CliError> for ExitCode {
    fn from(value: CliError) -> Self {
        Self::from(value.exit_code())
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message())
    }
}

// wrapper for handle used by main.rs
pub fn run(path: &Path, raw: bool) -> Result<String, CliError> {
    handle(path, raw, None)
        .map(|s| s.display().to_string())
        .map_err(Into::into)
}

#[derive(Debug, From)]
pub enum Error {
    Io(io::Error),
    Zip(ZipError),
    Xml(quick_xml::Error),
    InvalidMimetype,
    Encrypted,
    // todo: flatten other custom errors here
}

impl From<package::Error> for Error {
    fn from(value: package::Error) -> Self {
        match value {
            package::Error::Io(e) => Error::Io(e),
            package::Error::Zip(e) => Error::Zip(e),
            package::Error::Xml(e) => Error::Xml(e),
            package::Error::InvalidMimetype => Error::InvalidMimetype,
            package::Error::Encrypted => Error::Encrypted,
        }
    }
}

// todo2: move the contents of this to another file? (keep it as a lite wrapper?)
// todo: output_parent is test-only
pub fn handle(path: &Path, raw: bool, output_parent: Option<&Path>) -> Result<PathBuf, Error> {
    // thought process:
    // 1. does it look like it might be a valid file?
    // 2a. Try to open the ZIP archive if the file looks like a ZIP.
    // 2b. Try to parse the XML if the file looks like XML.
    // 3. Create the output directory
    // 4. Export sheets to TSV, lazy if possible.
    // 5. Return the output path, if #4 fails clean up the output dir

    let format = sniff(&path).expect("ignore error, fix later");

    // during this step, verify it looks like valid ODF variant, expose lazy sheet iterator?
    let _sheets = match format {
        sniff::Format::Zip(file) => {
            // todo: zip path
            let mut ods = OdsPackage::open(file)?;
            ods.validate()?;
            todo!()
        }
        sniff::Format::Xml(_file) => {
            // todo: xml path
            todo!()
        }
    };

    let output_path = dir::create(path, output_parent)?;

    // todo: iterate sheets, collect in-memory representation, export to TSV, capture failure

    // todo: clean up output dir on failures after it was made.

    Ok(output_path)
}
