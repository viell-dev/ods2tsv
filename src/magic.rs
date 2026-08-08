// ODS:
pub const REGULAR_MIMETYPE: &str = "application/vnd.oasis.opendocument.spreadsheet";
pub const TEMPLATE_MIMETYPE: &str = "application/vnd.oasis.opendocument.spreadsheet-template";
pub const MANIFEST_XML_PATH: &str = "META-INF/manifest.xml";
pub const MIMETYPE_PATH: &str = "mimetype";
pub const CONTENT_XML_PATH: &str = "content.xml";
// ODS XML Namespaces:
pub const NS_TABLE: &str = "urn:oasis:names:tc:opendocument:xmlns:table:1.0";
pub const NS_TEXT: &str = "urn:oasis:names:tc:opendocument:xmlns:text:1.0";
pub const NS_OFFICE: &str = "urn:oasis:names:tc:opendocument:xmlns:office:1.0";
pub const NS_DRAWING: &str = "urn:oasis:names:tc:opendocument:xmlns:drawing:1.0";
pub const NS_DR3D: &str = "urn:oasis:names:tc:opendocument:xmlns:dr3d:1.0";
// Other:
pub const ZIP_LOCAL_HEADER: &[u8] = b"PK\x03\x04";
pub const UTF32_LE_BOM: &[u8] = b"\xFF\xFE\x00\x00";
pub const UTF32_BE_BOM: &[u8] = b"\x00\x00\xFE\xFF";
pub const UTF16_LE_BOM: &[u8] = b"\xFF\xFE";
pub const UTF16_BE_BOM: &[u8] = b"\xFE\xFF";
pub const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";
