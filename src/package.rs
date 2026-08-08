//! Package ODS file processing

use std::{
    fs::File,
    io::{self, BufReader, Read, Seek},
};

use derive_more::{Display, Error, From};
use zip::{ZipArchive, result::ZipError};

use crate::{
    helpers,
    magic::{MANIFEST_XML_PATH, MIMETYPE_PATH, REGULAR_MIMETYPE, TEMPLATE_MIMETYPE},
    ods, xml,
};

#[derive(Debug, Error, From, Display)]
pub enum Error {
    Io(io::Error),
    Zip(ZipError),
    Xml(quick_xml::Error),

    #[from(ignore)]
    InvalidMimetype,

    #[from(ignore)]
    Encrypted,
}

#[derive(Debug)]
pub struct OdsPackage {
    zip: ZipArchive<BufReader<File>>,
}

impl OdsPackage {
    pub fn open(file: File) -> Result<Self, Error> {
        Ok(Self {
            zip: ZipArchive::new(BufReader::new(file))?,
        })
    }

    pub fn validate(&mut self) -> Result<(), Error> {
        // if mimetype exists and is invalid
        let mimetype = self.try_read_file_to_string(MIMETYPE_PATH)?;
        if !ods::validate_mimetype(mimetype) {
            return Err(Error::InvalidMimetype);
        }

        // a missing manifest cannot declare encrypted files
        if let Some(manifest) = self.try_read_xml(MANIFEST_XML_PATH)?
            && !ods::ensure_not_encrypted(manifest)
        {
            return Err(Error::Encrypted);
        }

        Ok(())
    }

    fn try_read_file_to_string(&mut self, name: &str) -> Result<Option<String>, Error> {
        let mut file = match self.zip.by_name(name) {
            Ok(file) => file,
            Err(ZipError::FileNotFound) => return Ok(None),
            Err(e) => return Err(e.into()),
        };

        let mut buf = String::new();
        file.read_to_string(&mut buf)?;

        helpers::trim_in_place(&mut buf);

        Ok(Some(buf))
    }

    fn try_read_xml(&mut self, name: &str) -> Result<Option<xml::Xml<'_>>, Error> {
        let file = match self.zip.by_name(name) {
            Ok(file) => file,
            Err(ZipError::FileNotFound) => return Ok(None),
            Err(e) => return Err(e.into()),
        };

        Ok(Some(xml::open_xml(file)?))
    }
}
