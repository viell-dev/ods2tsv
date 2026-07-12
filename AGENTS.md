# AGENTS.md

## Project Summary

This repository contains a Rust command-line extractor for OpenDocument Spreadsheet files. Given an `.ods` (zipped
package) or `.fods` (flat XML) file, it validates the input, streams the content XML with `quick-xml`, and writes one
`.tsv` file per sheet into a timestamped export directory created in the current working directory.

It started as a behavior-parity port of a Python implementation (`ods_to_tsv.py`, kept in the sibling `../repo/`
directory of this workspace) but has since diverged deliberately: the OpenDocument specification is the spec now, not
the Python version's output. Do not restore Python quirks (empty-row dropping, repeat bail-out heuristics, annotation
text leaking into cells) for parity's sake.

## Repository Layout

- `src/lib.rs` - public API (`OdsFile`, `extract_ods_to_tsv`, `create_export_dir`), error type, input validation
  (format sniffing, mimetype, encryption manifest).
- `src/content.rs` - streaming parse of the content XML into a compressed sheet model (repeats/spans kept as counts);
  ODF text semantics (whitespace collapsing, `text:s`/`text:tab`/`text:line-break`, skipped subtrees) and the
  `ValueMode` selection between formatted display text and raw `office:*-value` values.
- `src/export.rs` - expansion of the model into TSV rows (`write_sheet`) and file naming (`TsvWriter`).
- `src/main.rs` - CLI entry point (clap derive: one positional argument, `-r`/`--raw`, `--help`/`--version`); owns
  all terminal output.
- `tests/extract.rs` - fixture-based tests using synthetic in-memory ODS files.
- `tests/real_files.rs` - smoke test over `test-files/`; skipped when the directory is absent.
- `scripts/generate-test-files.sh` - populates the gitignored `test-files/` directory (needs LibreOffice and network).
- `README.md` - user-facing overview, build, usage.
- `COPYING.md` - GNU GPL v3 license text.
- `.github/workflows/test.yml` - CI: fmt check, clippy with warnings denied, tests.

## Key Behavioral Constraints

- Validation happens before any output: `OdsFile::open` must refuse encrypted, mistyped, and malformed inputs without
  an export directory ever being created.
- The output directory is created relative to the caller's current working directory, not next to the input file, and
  same-second runs get distinct directories.
- Each sheet becomes its own TSV file named from a sanitized version of the sheet name (`[alnum ._-]`, trimmed of
  leading/trailing separators, length-bounded, deduplicated case-insensitively, `Sheet` as fallback).
- The cell grid is preserved: interior empty rows/cells are written, trailing emptiness is dropped without being
  materialized. Merged areas are tracked as ranges and spill into covered positions, including rows storing no cells.
- Text that is not cell content (annotations, draw shapes, nested tables, tracked changes) must not leak into output.
- Character-data whitespace is collapsed per the ODF rules; `text:s`/`text:tab`/`text:line-break` are exempt.
- The library does not print; all terminal output belongs to `src/main.rs`.
- Output stability matters. Small changes to parsing logic can change TSV shape, whitespace, or line-break behavior.

## Dependencies

Kept deliberately lean (see `docs/rust.md` at the workspace root for the original dependency analysis):

- `zip` with default features off and only `deflate` enabled - do not re-enable default features.
- `quick-xml` with the `encoding` feature - namespace-aware streaming parser. Two sharp edges:
  - Since 0.38 entity references arrive as separate `Event::GeneralRef` events; text events do not contain them.
  - The stream is wrapped in `DecodingReader`, which transcodes everything to UTF-8 up front. Event payloads must be
    read as plain UTF-8 (see `content::utf8`); the events' own `decode()`/`xml10_content()` would double-decode
    non-UTF-8 documents because the parser re-detects the declared encoding.
- `jiff` - local-time timestamp for the export directory name.
- `clap` (derive) - CLI argument parsing; binary only, the library must not depend on it.
- `derive_more` (display, error, from) - derives the `Error` enum boilerplate.
- `tempfile` (dev) - temp dirs for the tests.

## Testing Expectations

`cargo test` runs the fixture suite. When changing extraction logic, verify at minimum: basic export, merged
row/col spans (including spill into cell-less rows), multiline text, whitespace collapsing, repeated spaces/tabs,
entity references, repeated rows/columns, grid alignment (interior vs. trailing emptiness), filename
sanitization/deduplication, flat XML input, UTF-16 input, and the refusals (encrypted, wrong mimetype, missing
content, unrecognized format).

For changes that could affect output shape, also populate `test-files/` via `scripts/generate-test-files.sh` and run
`cargo test --test real_files` (or the binary directly) against the real-world files.

## Editing Guidance

- `cargo fmt`, `cargo clippy --all-targets` (pedantic is enabled crate-wide; keep it warning-free), `cargo test`.
- Prefer `#[expect(lint, reason = "...")]` over `#[allow]`.
- Preserve CLI simplicity: one positional argument plus `-r`/`--raw` and `--help`/`--version`, nothing else.
- Be careful with whitespace semantics. TSV output is sensitive to paragraph joins, `text:s`, `text:tab`,
  `text:line-break`, and the collapsing rules in `PendingCell`.
