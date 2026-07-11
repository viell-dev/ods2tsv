# AGENTS.md

## Project Summary

This repository contains a Rust command-line extractor for OpenDocument Spreadsheet files. Given an `.ods` file, it
opens the archive, parses `content.xml` with `quick-xml`, and writes one `.tsv` file per sheet into a timestamped
export directory created in the current working directory.

It is a behavior-parity port of the original Python implementation (`ods_to_tsv.py`, kept in the sibling `../repo/`
directory of this workspace). The Python version's semantics are the spec: output must stay byte-for-byte identical,
including Python `csv.writer` quoting (minimal quoting, `\r\n` row terminators).

## Repository Layout

- `src/lib.rs` - public API (`extract_ods_to_tsv`), error type, encryption detection, sheet-name sanitization.
- `src/content.rs` - streaming parse of `content.xml` into a compressed sheet model (repeats/spans kept as counts).
- `src/export.rs` - expansion of the model into TSV rows; a line-for-line port of the Python loop.
- `src/main.rs` - CLI entry point (single positional argument).
- `tests/parity.rs` - fixture-based parity tests using synthetic in-memory ODS files.
- `README.md` - user-facing overview, build, usage.
- `COPYING.md` - GNU GPL v3 license text.
- `.github/workflows/test.yml` - CI: fmt check, clippy with warnings denied, tests.

## Key Behavioral Constraints

- The output directory is created relative to the caller's current working directory, not next to the input file.
- Each sheet becomes its own TSV file named from a sanitized version of the sheet name (`[alnum ._-]`, trimmed).
- Merged areas are flattened by repeating the source value into the covered cells.
- Repeat compression is expanded, but with the Python version's bail-outs: an empty cell repeated more than 1024
  times is emitted once, and an empty row repeated more than 100 times past row 10 stops the sheet.
- Trailing empty cells are trimmed per row; fully empty rows are not written at all.
- Password-protected (encrypted) files are detected via `manifest:encryption-data` in `META-INF/manifest.xml` and
  refused with a friendly error (`Error::Encrypted`). No decryption support.
- Output stability matters. Small changes to parsing logic can change TSV shape, whitespace, or line-break behavior.

## Dependencies

Kept deliberately lean (see `docs/rust.md` at the workspace root for the dependency analysis):

- `zip` with default features off and only `deflate` enabled - do not re-enable default features.
- `quick-xml` - namespace-aware streaming parser. Note: since 0.38 entity references arrive as separate
  `Event::GeneralRef` events; text events do not contain them.
- `jiff` - local-time timestamp for the export directory name.
- `tempfile` (dev) - temp dirs for the parity tests.

## Testing Expectations

`cargo test` runs the parity suite. When changing extraction logic, verify at minimum: basic export, merged row/col
spans, multiline text, repeated spaces, entity references, repeated rows/columns, filename sanitization, and
encrypted-file refusal.

For changes that could affect output shape, also run a differential test against the Python original: generate a
fixture ODS, run both `../repo/ods_to_tsv.py` and this binary on it from separate directories, and `diff -r` the
export directories.

## Editing Guidance

- `cargo fmt`, `cargo clippy --all-targets` (pedantic is enabled crate-wide; keep it warning-free), `cargo test`.
- Prefer `#[expect(lint, reason = "...")]` over `#[allow]`.
- Preserve CLI simplicity: one positional argument, nothing else.
- Be careful with whitespace semantics. TSV output is sensitive to paragraph joins, `text:s`, and `text:line-break`.
