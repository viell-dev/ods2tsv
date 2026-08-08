//! Common ODS stuff used by `flat` and `package` modules.

use derive_more::{Display, Error};
use quick_xml::events::Event;

use crate::magic::{REGULAR_MIMETYPE, TEMPLATE_MIMETYPE};
use crate::xml;

/// A missing mimetype is tolerated; only a clearly invalid value is rejected.
pub fn validate_mimetype<S: AsRef<str>>(mimetype: Option<S>) -> bool {
    mimetype.is_none_or(|s| matches!(s.as_ref(), REGULAR_MIMETYPE | TEMPLATE_MIMETYPE))
}

/// Returns `false` if the manifest contains an `encryption-data` element
/// (by local name, on a start or empty tag), `true` otherwise. A manifest
/// that stops parsing mid-way is treated as not encrypted; the actual
/// content streams surface real errors later.
pub fn ensure_not_encrypted(manifest: xml::Xml<'_>) -> bool {
    const ENCRYPTION_DATA: &[u8] = b"encryption-data";

    fn is_encryption_data(event: &Event<'_>) -> bool {
        matches!(event, Event::Start(e) | Event::Empty(e)
            if e.name().local_name().as_ref() == ENCRYPTION_DATA)
    }

    let (mut reader, first) = manifest;

    if let Some(event) = &first
        && is_encryption_data(event)
    {
        return false;
    }

    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Eof) | Err(_) => return true,
            Ok(event) if is_encryption_data(&event) => return false,
            Ok(_) => {}
        }
        buf.clear();
    }
}
