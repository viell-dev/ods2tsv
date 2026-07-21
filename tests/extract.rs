//! Fixture-based extraction tests. Fixtures are synthetic ODS files: a ZIP
//! with a handwritten `content.xml` (or a flat XML document).

use std::fmt::Write as _;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use ods_to_tsv::{Error, ValueMode, extract_ods_to_tsv};
use zip::CompressionMethod;
use zip::write::{SimpleFileOptions, ZipWriter};

const HEADER: &str = concat!(
    r#"<?xml version="1.0" encoding="UTF-8"?>"#,
    r#"<office:document-content"#,
    r#" xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0""#,
    r#" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0""#,
    r#" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0">"#,
    r#"<office:body><office:spreadsheet>"#,
);
const FOOTER: &str = "</office:spreadsheet></office:body></office:document-content>";

fn write_ods(path: &Path, entries: &[(&str, &str)]) {
    let raw: Vec<(&str, &[u8])> = entries
        .iter()
        .map(|(name, data)| (*name, data.as_bytes()))
        .collect();
    write_ods_raw(path, &raw);
}

fn write_ods_raw(path: &Path, entries: &[(&str, &[u8])]) {
    let mut zip = ZipWriter::new(fs::File::create(path).unwrap());
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for (name, data) in entries {
        zip.start_file(*name, options).unwrap();
        zip.write_all(data).unwrap();
    }
    zip.finish().unwrap();
}

fn fake_ods(path: &Path, sheets: &[(&str, &[&[&str]])]) {
    let mut body = String::new();
    for (name, rows) in sheets {
        write!(body, r#"<table:table table:name="{name}">"#).unwrap();
        for row in *rows {
            body.push_str("<table:table-row>");
            for cell in *row {
                write!(
                    body,
                    r#"<table:table-cell office:value-type="string"><text:p>{cell}</text:p></table:table-cell>"#
                )
                .unwrap();
            }
            body.push_str("</table:table-row>");
        }
        body.push_str("</table:table>");
    }
    let content = format!("{HEADER}{body}{FOOTER}");
    write_ods(
        path,
        &[
            ("mimetype", "application/vnd.oasis.opendocument.spreadsheet"),
            ("content.xml", &content),
        ],
    );
}

/// Runs the extractor in a fresh temp dir and returns the export directory.
fn extract(ods: &Path) -> PathBuf {
    let parent = ods.parent().unwrap();
    let out_dir = extract_ods_to_tsv(ods, parent, ValueMode::Formatted).unwrap();
    assert!(out_dir.is_dir());
    out_dir
}

fn read(dir: &Path, file: &str) -> String {
    fs::read_to_string(dir.join(file)).unwrap()
}

#[test]
fn basic_extraction() {
    let tmp = tempfile::tempdir().unwrap();
    let ods = tmp.path().join("basic.ods");
    fake_ods(
        &ods,
        &[
            ("Sheet1", &[&["A1", "B1"], &["A2", "B2"]]),
            ("Sheet2", &[&["only"]]),
        ],
    );

    let out = extract(&ods);
    let dir_name = out.file_name().unwrap().to_str().unwrap();
    assert!(
        dir_name.starts_with("basic - ") && dir_name.len() == "basic - ".len() + 19,
        "unexpected export dir name: {dir_name}"
    );
    assert_eq!(read(&out, "Sheet1.tsv"), "A1\tB1\r\nA2\tB2\r\n");
    assert_eq!(read(&out, "Sheet2.tsv"), "only\r\n");
}

#[test]
fn merged_cells() {
    let tmp = tempfile::tempdir().unwrap();
    let ods = tmp.path().join("merged.ods");
    let content = format!(
        concat!(
            "{header}",
            r#"<table:table table:name="Spans">"#,
            "<table:table-row>",
            r#"<table:table-cell table:number-rows-spanned="2" office:value-type="string"><text:p>MergedRow</text:p></table:table-cell>"#,
            r#"<table:table-cell office:value-type="string"><text:p>B1</text:p></table:table-cell>"#,
            "</table:table-row>",
            "<table:table-row>",
            "<table:covered-table-cell/>",
            r#"<table:table-cell office:value-type="string"><text:p>B2</text:p></table:table-cell>"#,
            "</table:table-row>",
            "<table:table-row>",
            r#"<table:table-cell table:number-columns-spanned="2" office:value-type="string"><text:p>MergedCol</text:p></table:table-cell>"#,
            "<table:covered-table-cell/>",
            r#"<table:table-cell office:value-type="string"><text:p>C3</text:p></table:table-cell>"#,
            "</table:table-row>",
            "</table:table>",
            "{footer}",
        ),
        header = HEADER,
        footer = FOOTER,
    );
    write_ods(&ods, &[("content.xml", &content)]);

    let out = extract(&ods);
    assert_eq!(
        read(&out, "Spans.tsv"),
        "MergedRow\tB1\r\nMergedRow\tB2\r\nMergedCol\tMergedCol\tC3\r\n"
    );
}

#[test]
fn multiline_and_spaces() {
    let tmp = tempfile::tempdir().unwrap();
    let ods = tmp.path().join("text.ods");
    let content = format!(
        concat!(
            "{header}",
            r#"<table:table table:name="Text">"#,
            "<table:table-row>",
            r#"<table:table-cell office:value-type="string">"#,
            "<text:p>Line 1</text:p><text:p>Line 2</text:p>",
            "</table:table-cell>",
            r#"<table:table-cell office:value-type="string">"#,
            r#"<text:p>Space<text:s text:c="3"/>test</text:p>"#,
            "</table:table-cell>",
            r#"<table:table-cell office:value-type="string">"#,
            "<text:p>Break<text:line-break/>here</text:p>",
            "</table:table-cell>",
            "</table:table-row>",
            "</table:table>",
            "{footer}",
        ),
        header = HEADER,
        footer = FOOTER,
    );
    write_ods(&ods, &[("content.xml", &content)]);

    let out = extract(&ods);
    // Multiline cells are quoted, matching Python's csv QUOTE_MINIMAL.
    assert_eq!(
        read(&out, "Text.tsv"),
        "\"Line 1\nLine 2\"\tSpace   test\t\"Break\nhere\"\r\n"
    );
}

#[test]
fn entity_and_character_references() {
    let tmp = tempfile::tempdir().unwrap();
    let ods = tmp.path().join("entities.ods");
    fake_ods(
        &ods,
        &[(
            "Refs",
            &[&[
                "a&amp;b",
                "&lt;tag&gt;",
                "&quot;q&quot;&apos;a&apos;",
                "&#65;&#x42;",
            ]],
        )],
    );

    let out = extract(&ods);
    assert_eq!(
        read(&out, "Refs.tsv"),
        "a&b\t<tag>\t\"\"\"q\"\"'a'\"\tAB\r\n"
    );
}

#[test]
fn repeated_rows_and_columns() {
    let tmp = tempfile::tempdir().unwrap();
    let ods = tmp.path().join("repeats.ods");
    let content = format!(
        concat!(
            "{header}",
            r#"<table:table table:name="Repeats">"#,
            "<table:table-row>",
            r#"<table:table-cell table:number-columns-repeated="3" office:value-type="string"><text:p>X</text:p></table:table-cell>"#,
            "</table:table-row>",
            r#"<table:table-row table:number-rows-repeated="2">"#,
            r#"<table:table-cell office:value-type="string"><text:p>Y</text:p></table:table-cell>"#,
            "</table:table-row>",
            "</table:table>",
            "{footer}",
        ),
        header = HEADER,
        footer = FOOTER,
    );
    write_ods(&ods, &[("content.xml", &content)]);

    let out = extract(&ods);
    assert_eq!(read(&out, "Repeats.tsv"), "X\tX\tX\r\nY\r\nY\r\n");
}

#[test]
fn whitespace_collapsing() {
    let tmp = tempfile::tempdir().unwrap();
    let ods = tmp.path().join("ws.ods");
    let content = format!(
        concat!(
            "{header}",
            r#"<table:table table:name="Ws">"#,
            "<table:table-row>",
            "<table:table-cell><text:p>  Leading   and\n\tinner  </text:p></table:table-cell>",
            "<table:table-cell><text:p>a <text:s/> b</text:p></table:table-cell>",
            "</table:table-row>",
            "</table:table>",
            "{footer}",
        ),
        header = HEADER,
        footer = FOOTER,
    );
    write_ods(&ods, &[("content.xml", &content)]);

    let out = extract(&ods);
    // Character-data whitespace collapses per ODF; the explicit text:s space
    // survives next to the collapsed one.
    assert_eq!(read(&out, "Ws.tsv"), "Leading and inner\ta  b\r\n");
}

#[test]
fn tab_element_and_cdata() {
    let tmp = tempfile::tempdir().unwrap();
    let ods = tmp.path().join("tab.ods");
    let content = format!(
        concat!(
            "{header}",
            r#"<table:table table:name="Tab">"#,
            "<table:table-row>",
            "<table:table-cell><text:p>A<text:tab/>B<![CDATA[ <raw> & data ]]>C</text:p></table:table-cell>",
            "</table:table-row>",
            "</table:table>",
            "{footer}",
        ),
        header = HEADER,
        footer = FOOTER,
    );
    write_ods(&ods, &[("content.xml", &content)]);

    let out = extract(&ods);
    assert_eq!(read(&out, "Tab.tsv"), "\"A\tB <raw> & data C\"\r\n");
}

#[test]
fn annotations_and_nested_tables_are_not_cell_text() {
    let tmp = tempfile::tempdir().unwrap();
    let ods = tmp.path().join("noise.ods");
    let content = format!(
        concat!(
            "{header}",
            r#"<table:table table:name="Noise">"#,
            "<table:table-row>",
            "<table:table-cell>",
            "<office:annotation><text:p>a comment</text:p></office:annotation>",
            "<text:p>visible</text:p>",
            r#"<table:table table:name="inner">"#,
            "<table:table-row><table:table-cell><text:p>nested</text:p></table:table-cell></table:table-row>",
            "</table:table>",
            "</table:table-cell>",
            "</table:table-row>",
            "</table:table>",
            "{footer}",
        ),
        header = HEADER,
        footer = FOOTER,
    );
    write_ods(&ods, &[("content.xml", &content)]);

    let out = extract(&ods);
    assert_eq!(read(&out, "Noise.tsv"), "visible\r\n");
    // The nested table must not surface as a sheet of its own.
    assert!(!out.join("inner.tsv").exists());
}

#[test]
fn value_attributes_used_without_paragraphs() {
    let tmp = tempfile::tempdir().unwrap();
    let ods = tmp.path().join("values.ods");
    let content = format!(
        concat!(
            "{header}",
            r#"<table:table table:name="Values">"#,
            "<table:table-row>",
            r#"<table:table-cell office:value-type="float" office:value="42.5"/>"#,
            r#"<table:table-cell office:value-type="date" office:date-value="2024-03-01"/>"#,
            r#"<table:table-cell office:value-type="boolean" office:boolean-value="true"/>"#,
            r#"<table:table-cell office:value-type="string" office:string-value="from attr"/>"#,
            r#"<table:table-cell office:value-type="float" office:value="1"><text:p>display text wins</text:p></table:table-cell>"#,
            "</table:table-row>",
            "</table:table>",
            "{footer}",
        ),
        header = HEADER,
        footer = FOOTER,
    );
    write_ods(&ods, &[("content.xml", &content)]);

    let out = extract(&ods);
    assert_eq!(
        read(&out, "Values.tsv"),
        "42.5\t2024-03-01\ttrue\tfrom attr\tdisplay text wins\r\n"
    );
}

#[test]
fn empty_rows_and_cells_keep_alignment() {
    let tmp = tempfile::tempdir().unwrap();
    let ods = tmp.path().join("align.ods");
    let content = format!(
        concat!(
            "{header}",
            r#"<table:table table:name="Align">"#,
            // Leading empty row, then interior empty cells and rows, then a
            // sheet-tail of repeated emptiness that must all be dropped.
            "<table:table-row><table:table-cell/></table:table-row>",
            "<table:table-row>",
            "<table:table-cell><text:p>a</text:p></table:table-cell>",
            "<table:table-cell/>",
            "<table:table-cell><text:p>b</text:p></table:table-cell>",
            r#"<table:table-cell table:number-columns-repeated="16384"/>"#,
            "</table:table-row>",
            r#"<table:table-row table:number-rows-repeated="3"><table:table-cell/></table:table-row>"#,
            "<table:table-row><table:table-cell><text:p>c</text:p></table:table-cell></table:table-row>",
            r#"<table:table-row table:number-rows-repeated="1048570">"#,
            r#"<table:table-cell table:number-columns-repeated="16384"/>"#,
            "</table:table-row>",
            "</table:table>",
            "{footer}",
        ),
        header = HEADER,
        footer = FOOTER,
    );
    write_ods(&ods, &[("content.xml", &content)]);

    let out = extract(&ods);
    assert_eq!(read(&out, "Align.tsv"), "\r\na\t\tb\r\n\r\n\r\n\r\nc\r\n");
}

#[test]
fn merges_spill_into_rows_without_stored_cells() {
    let tmp = tempfile::tempdir().unwrap();
    let ods = tmp.path().join("spill.ods");
    let content = format!(
        concat!(
            "{header}",
            r#"<table:table table:name="Spill">"#,
            "<table:table-row>",
            r#"<table:table-cell/>"#,
            r#"<table:table-cell table:number-rows-spanned="3" office:value-type="string"><text:p>tall</text:p></table:table-cell>"#,
            "</table:table-row>",
            // Rows under the merge storing no cells at all: the merge value
            // must still land in its column.
            "<table:table-row><table:table-cell/></table:table-row>",
            "<table:table-row><table:table-cell/></table:table-row>",
            "</table:table>",
            "{footer}",
        ),
        header = HEADER,
        footer = FOOTER,
    );
    write_ods(&ods, &[("content.xml", &content)]);

    let out = extract(&ods);
    assert_eq!(read(&out, "Spill.tsv"), "\ttall\r\n\ttall\r\n\ttall\r\n");
}

#[test]
fn raw_mode_prefers_stored_values() {
    let tmp = tempfile::tempdir().unwrap();
    let ods = tmp.path().join("modes.ods");
    let content = format!(
        concat!(
            "{header}",
            r#"<table:table table:name="Modes">"#,
            "<table:table-row>",
            r#"<table:table-cell office:value-type="date" office:date-value="2024-03-01"><text:p>01/03/2024</text:p></table:table-cell>"#,
            r#"<table:table-cell office:value-type="currency" office:value="1234.5" office:currency="EUR"><text:p>1.234,50 €</text:p></table:table-cell>"#,
            r#"<table:table-cell office:value-type="boolean" office:boolean-value="true"><text:p>SANT</text:p></table:table-cell>"#,
            r#"<table:table-cell office:value-type="string"><text:p>plain text</text:p></table:table-cell>"#,
            "</table:table-row>",
            "</table:table>",
            "{footer}",
        ),
        header = HEADER,
        footer = FOOTER,
    );
    write_ods(&ods, &[("content.xml", &content)]);

    let formatted = extract_ods_to_tsv(&ods, tmp.path(), ValueMode::Formatted).unwrap();
    assert_eq!(
        read(&formatted, "Modes.tsv"),
        "01/03/2024\t1.234,50 €\tSANT\tplain text\r\n"
    );

    // Raw mode takes the stored values; string cells without a stored value
    // fall back to their display text.
    let raw = extract_ods_to_tsv(&ods, tmp.path(), ValueMode::Raw).unwrap();
    assert_eq!(
        read(&raw, "Modes.tsv"),
        "2024-03-01\t1234.5\ttrue\tplain text\r\n"
    );
}

#[test]
fn sheet_name_sanitization() {
    let tmp = tempfile::tempdir().unwrap();
    let ods = tmp.path().join("dirty.ods");
    fake_ods(&ods, &[(r"Sheet/With\Bad:Chars*", &[&["data"]])]);

    let out = extract(&ods);
    assert_eq!(read(&out, "SheetWithBadChars.tsv"), "data\r\n");
}

#[test]
fn duplicate_and_unusable_sheet_names_get_distinct_files() {
    let tmp = tempfile::tempdir().unwrap();
    let ods = tmp.path().join("dupes.ods");
    fake_ods(
        &ods,
        &[
            ("Dup/", &[&["first"]]),
            (r"Dup\", &[&["second"]]),
            ("dup", &[&["third"]]),
            ("///", &[&["nameless"]]),
        ],
    );

    let out = extract(&ods);
    assert_eq!(read(&out, "Dup.tsv"), "first\r\n");
    assert_eq!(read(&out, "Dup (2).tsv"), "second\r\n");
    // Deduplication is case-insensitive for case-insensitive filesystems.
    assert_eq!(read(&out, "dup (3).tsv"), "third\r\n");
    assert_eq!(read(&out, "Sheet.tsv"), "nameless\r\n");
}

#[test]
fn flat_xml_document_is_extracted() {
    let tmp = tempfile::tempdir().unwrap();
    let fods = tmp.path().join("flat.fods");
    let content = concat!(
        r#"<?xml version="1.0" encoding="UTF-8"?>"#,
        r#"<office:document"#,
        r#" xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0""#,
        r#" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0""#,
        r#" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0""#,
        r#" office:mimetype="application/vnd.oasis.opendocument.spreadsheet">"#,
        r#"<office:body><office:spreadsheet>"#,
        r#"<table:table table:name="Flat">"#,
        "<table:table-row><table:table-cell><text:p>flat data</text:p></table:table-cell></table:table-row>",
        "</table:table>",
        "</office:spreadsheet></office:body></office:document>",
    );
    fs::write(&fods, content).unwrap();

    let out = extract(&fods);
    assert_eq!(read(&out, "Flat.tsv"), "flat data\r\n");
}

#[test]
fn utf16_content_is_decoded() {
    let tmp = tempfile::tempdir().unwrap();
    let ods = tmp.path().join("utf16.ods");
    let content = format!(
        concat!(
            "{header}",
            r#"<table:table table:name="Wide">"#,
            "<table:table-row><table:table-cell><text:p>åäö ✓</text:p></table:table-cell></table:table-row>",
            "</table:table>",
            "{footer}",
        ),
        header = HEADER.replace(r#"encoding="UTF-8""#, r#"encoding="UTF-16""#),
        footer = FOOTER,
    );
    let mut bytes = vec![0xFF, 0xFE]; // UTF-16 LE BOM
    bytes.extend(content.encode_utf16().flat_map(u16::to_le_bytes));
    write_ods_raw(&ods, &[("content.xml", &bytes)]);

    let out = extract(&ods);
    assert_eq!(read(&out, "Wide.tsv"), "åäö ✓\r\n");
}

#[test]
fn utf16_be_content_is_decoded() {
    // The declaration says "UTF-16", which resolves to little endian; it must
    // not replace the big-endian decoder chosen from the BOM.
    let tmp = tempfile::tempdir().unwrap();
    let ods = tmp.path().join("utf16be.ods");
    let content = format!(
        concat!(
            "{header}",
            r#"<table:table table:name="Wide">"#,
            "<table:table-row><table:table-cell><text:p>åäö ✓</text:p></table:table-cell></table:table-row>",
            "</table:table>",
            "{footer}",
        ),
        header = HEADER.replace(r#"encoding="UTF-8""#, r#"encoding="UTF-16""#),
        footer = FOOTER,
    );
    let mut bytes = vec![0xFE, 0xFF]; // UTF-16 BE BOM
    bytes.extend(content.encode_utf16().flat_map(u16::to_be_bytes));
    write_ods_raw(&ods, &[("content.xml", &bytes)]);

    let out = extract(&ods);
    assert_eq!(read(&out, "Wide.tsv"), "åäö ✓\r\n");
}

/// Encodes a string as ISO-8859-1, one byte per char.
fn latin1(s: &str) -> Vec<u8> {
    s.chars()
        .map(|c| u8::try_from(u32::from(c)).expect("latin-1 range"))
        .collect()
}

#[test]
fn declared_legacy_encoding_is_honored() {
    let tmp = tempfile::tempdir().unwrap();
    let ods = tmp.path().join("latin1.ods");
    let content = format!(
        concat!(
            "{header}",
            r#"<table:table table:name="Legacy">"#,
            "<table:table-row><table:table-cell><text:p>café über</text:p></table:table-cell></table:table-row>",
            "</table:table>",
            "{footer}",
        ),
        header = HEADER.replace(r#"encoding="UTF-8""#, r#"encoding="ISO-8859-1""#),
        footer = FOOTER,
    );
    write_ods_raw(
        &ods,
        &[
            (
                "mimetype",
                b"application/vnd.oasis.opendocument.spreadsheet",
            ),
            ("content.xml", &latin1(&content)),
        ],
    );

    let out = extract(&ods);
    assert_eq!(read(&out, "Legacy.tsv"), "café über\r\n");
}

#[test]
fn flat_xml_with_legacy_encoding_is_extracted() {
    let tmp = tempfile::tempdir().unwrap();
    let fods = tmp.path().join("latin1.fods");
    let content = concat!(
        r#"<?xml version="1.0" encoding="ISO-8859-1"?>"#,
        r#"<office:document"#,
        r#" xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0""#,
        r#" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0""#,
        r#" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0""#,
        r#" office:mimetype="application/vnd.oasis.opendocument.spreadsheet">"#,
        r#"<office:body><office:spreadsheet>"#,
        r#"<table:table table:name="Flat">"#,
        "<table:table-row><table:table-cell><text:p>café über</text:p></table:table-cell></table:table-row>",
        "</table:table>",
        "</office:spreadsheet></office:body></office:document>",
    );
    fs::write(&fods, latin1(content)).unwrap();

    let out = extract(&fods);
    assert_eq!(read(&out, "Flat.tsv"), "café über\r\n");
}

#[test]
fn declaration_past_detection_window_is_refused() {
    // A legacy encoding can only be applied while the decoder's 64-byte
    // detection prefix is buffered; a declaration padded past it must be
    // refused (not panic, not mis-decode).
    let tmp = tempfile::tempdir().unwrap();
    let fods = tmp.path().join("padded.fods");
    let content = format!(
        concat!(
            r#"<?xml version="1.0"{pad}encoding="ISO-8859-1"?>"#,
            r#"<office:document"#,
            r#" xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0""#,
            r#" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0""#,
            r#" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0">"#,
            r#"<office:body><office:spreadsheet>"#,
            r#"<table:table table:name="Padded"><table:table-row>"#,
            "<table:table-cell><text:p>café</text:p></table:table-cell>",
            "</table:table-row></table:table>",
            "</office:spreadsheet></office:body></office:document>",
        ),
        pad = " ".repeat(50),
    );
    fs::write(&fods, latin1(&content)).unwrap();

    let err = extract_ods_to_tsv(&fods, tmp.path(), ValueMode::Formatted).unwrap_err();
    assert!(matches!(err, Error::Xml(_)));
}

#[test]
fn same_second_exports_get_distinct_dirs() {
    let tmp = tempfile::tempdir().unwrap();
    let ods = tmp.path().join("twice.ods");
    fake_ods(&ods, &[("Sheet1", &[&["x"]])]);

    let first = extract_ods_to_tsv(&ods, tmp.path(), ValueMode::Formatted).unwrap();
    let second = extract_ods_to_tsv(&ods, tmp.path(), ValueMode::Formatted).unwrap();
    assert_ne!(first, second);
    assert!(first.is_dir() && second.is_dir());
}

#[test]
fn wrong_package_mimetype_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let ods = tmp.path().join("actually-text.ods");
    write_ods(
        &ods,
        &[
            ("mimetype", "application/vnd.oasis.opendocument.text"),
            ("content.xml", "<x/>"),
        ],
    );

    let err = extract_ods_to_tsv(&ods, tmp.path(), ValueMode::Formatted).unwrap_err();
    assert!(
        matches!(err, Error::NotSpreadsheet(m) if m == "application/vnd.oasis.opendocument.text")
    );
}

#[test]
fn flat_document_with_wrong_mimetype_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let fodt = tmp.path().join("letter.fods");
    let content = concat!(
        r#"<?xml version="1.0" encoding="UTF-8"?>"#,
        r#"<office:document"#,
        r#" xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0""#,
        r#" office:mimetype="application/vnd.oasis.opendocument.text">"#,
        "<office:body/></office:document>",
    );
    fs::write(&fodt, content).unwrap();

    let err = extract_ods_to_tsv(&fodt, tmp.path(), ValueMode::Formatted).unwrap_err();
    assert!(matches!(err, Error::NotSpreadsheet(_)));
    // Validation failed, so no export directory may exist.
    assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 1);
}

#[test]
fn package_without_content_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let ods = tmp.path().join("hollow.ods");
    write_ods(
        &ods,
        &[("mimetype", "application/vnd.oasis.opendocument.spreadsheet")],
    );

    let err = extract_ods_to_tsv(&ods, tmp.path(), ValueMode::Formatted).unwrap_err();
    assert!(matches!(err, Error::MissingContent));
}

#[test]
fn unrecognized_file_is_refused_before_output_exists() {
    let tmp = tempfile::tempdir().unwrap();
    let bogus = tmp.path().join("bogus.ods");
    fs::write(&bogus, [0x7F, b'E', b'L', b'F', 0, 1, 2, 3]).unwrap();

    let err = extract_ods_to_tsv(&bogus, tmp.path(), ValueMode::Formatted).unwrap_err();
    assert!(matches!(err, Error::UnrecognizedFormat));
    // Validation failed, so no export directory may exist.
    assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 1);
}

#[test]
fn encrypted_file_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let ods = tmp.path().join("locked.ods");
    let manifest = concat!(
        r#"<?xml version="1.0" encoding="UTF-8"?>"#,
        r#"<manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0">"#,
        r#"<manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml" manifest:size="1234">"#,
        r#"<manifest:encryption-data manifest:checksum-type="SHA1/1K" manifest:checksum="AAAA">"#,
        r#"<manifest:algorithm manifest:algorithm-name="http://www.w3.org/2001/04/xmlenc#aes256-cbc" manifest:initialisation-vector="BBBB"/>"#,
        r#"<manifest:key-derivation manifest:key-derivation-name="PBKDF2" manifest:salt="CCCC" manifest:iteration-count="1024"/>"#,
        r#"</manifest:encryption-data>"#,
        r#"</manifest:file-entry>"#,
        r#"</manifest:manifest>"#,
    );
    write_ods(
        &ods,
        &[
            ("mimetype", "application/vnd.oasis.opendocument.spreadsheet"),
            ("META-INF/manifest.xml", manifest),
            ("content.xml", "\u{1}\u{2}not really xml\u{3}"),
        ],
    );

    let err = extract_ods_to_tsv(&ods, tmp.path(), ValueMode::Formatted).unwrap_err();
    assert!(matches!(err, Error::Encrypted));
    assert_eq!(
        err.to_string(),
        "this file is password-protected; save a copy without a password first"
    );
}
