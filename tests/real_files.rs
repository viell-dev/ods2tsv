//! Smoke tests over real spreadsheet files in the gitignored `test-files/`
//! directory (populated by `scripts/generate-test-files.sh`). Skipped when
//! the directory is absent, so CI runs are unaffected.

use std::fs;
use std::path::Path;

use ods_to_tsv::{Error, ValueMode, extract_ods_to_tsv};

#[test]
fn extracts_every_test_file() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("test-files");
    let Ok(entries) = fs::read_dir(&dir) else {
        eprintln!("skipping: {} does not exist", dir.display());
        return;
    };

    let tmp = tempfile::tempdir().unwrap();
    let mut seen = 0;
    for entry in entries {
        let path = entry.unwrap().path();
        let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if !matches!(extension, "ods" | "fods") {
            continue;
        }
        seen += 1;
        let name = path.file_name().unwrap().to_string_lossy().into_owned();

        let result = extract_ods_to_tsv(&path, tmp.path(), ValueMode::Formatted);
        if name.starts_with("password") {
            assert!(
                matches!(result, Err(Error::Encrypted)),
                "{name}: expected the encrypted-file refusal, got {result:?}"
            );
            continue;
        }

        let out = result.unwrap_or_else(|e| panic!("{name}: extraction failed: {e}"));
        let sheets = fs::read_dir(&out).unwrap().count();
        assert!(sheets > 0, "{name}: extracted no sheets");
    }

    if seen == 0 {
        eprintln!("skipping: no spreadsheet files in {}", dir.display());
    }
}
