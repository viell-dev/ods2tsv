//! Exit-code contract of the binary: 0 success, 1 other, 2 usage (clap),
//! 3 not a usable spreadsheet, 4 corrupt/malformed, 5 encrypted.

use std::fs;
use std::io::Write as _;
use std::path::Path;
use std::process::Command;

use zip::CompressionMethod;
use zip::write::{SimpleFileOptions, ZipWriter};

fn write_ods(path: &Path, entries: &[(&str, &str)]) {
    let mut zip = ZipWriter::new(fs::File::create(path).unwrap());
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for (name, data) in entries {
        zip.start_file(*name, options).unwrap();
        zip.write_all(data.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
}

/// Runs the binary against `input` with a temp dir as CWD and returns the
/// exit code.
fn run(input: &Path) -> i32 {
    let cwd = tempfile::tempdir().unwrap();
    let status = Command::new(env!("CARGO_BIN_EXE_ods-to-tsv"))
        .arg(input)
        .current_dir(cwd.path())
        .output()
        .unwrap()
        .status;
    status.code().unwrap()
}

#[test]
fn success_exits_zero() {
    let tmp = tempfile::tempdir().unwrap();
    let ods = tmp.path().join("ok.ods");
    let content = concat!(
        r#"<?xml version="1.0" encoding="UTF-8"?>"#,
        r#"<office:document-content"#,
        r#" xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0""#,
        r#" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0""#,
        r#" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0">"#,
        r#"<office:body><office:spreadsheet>"#,
        r#"<table:table table:name="Sheet1"><table:table-row>"#,
        r#"<table:table-cell office:value-type="string"><text:p>x</text:p></table:table-cell>"#,
        r#"</table:table-row></table:table>"#,
        r#"</office:spreadsheet></office:body></office:document-content>"#,
    );
    write_ods(
        &ods,
        &[
            ("mimetype", "application/vnd.oasis.opendocument.spreadsheet"),
            ("content.xml", content),
        ],
    );
    assert_eq!(run(&ods), 0);
}

#[test]
fn missing_input_exits_one() {
    let tmp = tempfile::tempdir().unwrap();
    assert_eq!(run(&tmp.path().join("nonexistent.ods")), 1);
}

#[test]
fn usage_error_exits_two() {
    let status = Command::new(env!("CARGO_BIN_EXE_ods-to-tsv"))
        .output()
        .unwrap()
        .status;
    assert_eq!(status.code(), Some(2));
}

#[test]
fn unusable_input_exits_three() {
    let tmp = tempfile::tempdir().unwrap();
    let bogus = tmp.path().join("bogus.ods");
    fs::write(&bogus, [0x7F, b'E', b'L', b'F', 0, 1, 2, 3]).unwrap();
    assert_eq!(run(&bogus), 3);

    let text = tmp.path().join("text.ods");
    write_ods(
        &text,
        &[("mimetype", "text/plain"), ("content.xml", "<x/>")],
    );
    assert_eq!(run(&text), 3);
}

#[test]
fn corrupt_input_exits_four() {
    let tmp = tempfile::tempdir().unwrap();
    let broken = tmp.path().join("broken.ods");
    fs::write(&broken, b"PK\x03\x04 this is not a zip archive").unwrap();
    assert_eq!(run(&broken), 4);
}

#[test]
fn encrypted_input_exits_five() {
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
    assert_eq!(run(&ods), 5);
}
