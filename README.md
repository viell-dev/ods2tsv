# ODS to TSV

`ods-to-tsv` is a small Rust command-line program and library that turns every sheet in an
OpenDocument spreadsheet into a separate tab-separated file.

The project is intentionally narrow: it reads spreadsheet values and grid structure, but it does
not attempt to reproduce formulas, styles, charts, or other presentation features. Its interesting
work lies in preserving cell coordinates while safely handling the compact and sometimes enormous
XML representation used by `.ods` files.

## Try it

The repository pins its Rust version in `rust-toolchain.toml`. Build and run the program with:

```bash
cargo build --release
./target/release/ods-to-tsv "path/to/workbook.ods"
```

The output goes into a new directory under the current working directory:

```text
workbook - 2026-08-09T14-30-00/
├── January.tsv
├── February.tsv
└── Summary.tsv
```

Running twice within the same second creates `... (2)`, `... (3)`, and so on instead of reusing an
existing directory.

By default, cells contain the formatted text shown by a spreadsheet application. Use `--raw` (or
`-r`) for underlying machine-readable values:

```bash
./target/release/ods-to-tsv --raw "path/to/workbook.ods"
```

For example, a localized date displayed as `09/08/2026` may become `2026-08-09`, and `1 234,50 kr`
may become `1234.5`. Plain text cells have no separate raw value, so they keep their display text.

## Input and output contract

The extractor accepts two ODF representations:

- `.ods`: a ZIP package whose spreadsheet data is stored in `content.xml`.
- `.fods`: a flat XML document containing the spreadsheet directly.

Actual bytes determine the format; the file extension does not. Spreadsheet templates using the
ODF spreadsheet-template media type are accepted as well.

Each sheet becomes `<sanitized sheet name>.tsv`. The writer:

- retains Unicode letters and digits plus spaces, dots, underscores, and hyphens;
- removes unsafe punctuation and leading/trailing spaces, dots, or hyphens;
- uses `Sheet` if nothing remains;
- limits the base name to 80 Unicode characters; and
- adds ` (2)`, ` (3)`, and so on for case-insensitive collisions.

TSV records use tabs between fields and `\r\n` at the end of rows. A field containing a tab,
line break, carriage return, or double quote is wrapped in double quotes; a literal quote inside it
is doubled. This is the usual RFC 4180 escaping rule with a tab delimiter.

## What “grid fidelity” means

ODF can compress rows and columns using repeat counts. It can also represent merged cells as one
source cell followed by covered positions. The extractor reconstructs physical coordinates so that
downstream programs receive stable row and column positions:

- repeated rows and cells are expanded;
- values from merged cells fill every covered coordinate;
- leading and interior empty rows/cells are preserved when later data makes them meaningful;
- trailing empty rows/cells are omitted; and
- a huge repeated empty tail is counted and discarded rather than allocated.

The last point is important. Spreadsheet applications commonly serialize a row template repeated up
to the sheet limit (more than a million rows). Building that whole grid in memory would make a tiny
visible spreadsheet unnecessarily expensive.

## Text and value semantics

Formatted text follows OpenDocument rules rather than HTML-like intuition:

- ordinary spaces, tabs, and line breaks in XML character data collapse to one space;
- whitespace at paragraph boundaries disappears;
- `text:s`, `text:tab`, and `text:line-break` encode explicit whitespace and therefore survive;
- multiple `text:p` or `text:h` paragraphs join with newline characters;
- predefined XML entities, numeric character references, and CDATA are decoded; and
- paragraphs inside annotations, drawings, nested tables, and change history are ignored.

Cells may contain both displayed paragraphs and typed `office:*-value` attributes. Formatted mode
prefers paragraphs and falls back to attributes. Raw mode does the reverse.

## Validation and failures

Input validation happens before the export directory is created. For a package, the library checks
the ODF media type when present, requires `content.xml`, and inspects the manifest for encrypted
streams. For a flat document, it validates a root spreadsheet media type when declared.

The command writes an explanation to standard error and returns a category-specific exit code:

| Code | Meaning |
| ---: | --- |
| 0 | Extraction succeeded. |
| 1 | A general I/O failure occurred while reading or writing. |
| 2 | Command-line arguments were invalid; this code is produced by clap. |
| 3 | The input is not a usable ODF spreadsheet. |
| 4 | ZIP data or spreadsheet XML is corrupt, malformed, or undecodable. |
| 5 | The package is password-protected. |

The project detects encryption but does not decrypt files. Save an unencrypted copy in a spreadsheet
application before extraction.

## How the code is organized

The end-to-end path is deliberately split into four small responsibilities:

```text
src/main.rs
  parse CLI, print progress, map errors to exit codes
      │
      ▼
src/lib.rs
  sniff and validate input, open content stream, compose extraction
      │
      ▼
src/content.rs
  decode XML, stream events, select values, build one compact Sheet at a time
      │
      ▼
src/export.rs
  expand repeats/merges, trim only trailing emptiness, quote TSV, claim file names
```

The principal public types and functions are:

- `OdsFile::open`: open and validate without creating output;
- `OdsFile::sheets`: lazily yield parsed `Sheet` values;
- `ValueMode`: choose formatted or raw cell values;
- `write_sheet`: write one parsed sheet to any `std::io::Write` sink;
- `TsvWriter`: assign safe unique file names and write sheets into a directory; and
- `extract_ods_to_tsv`: compose the normal complete workflow.

Module-level documentation and inline comments explain the state machines and invariants beside the
code that implements them.

## A request’s journey

For a packaged `.ods`, one successful CLI invocation follows this sequence:

1. Clap parses the path and `--raw` flag.
2. `OdsFile::open` reads four leading bytes and recognizes ZIP local-file-header magic.
3. Package validation checks the optional `mimetype`, required `content.xml`, and optional
   encryption manifest.
4. Only after validation, `create_export_dir` atomically claims a timestamped directory.
5. `SheetReader` streams `content.xml`, returning when each outer `table:table` closes.
6. `TsvWriter` sanitizes and claims the sheet file name.
7. `write_sheet` expands repeated templates and active merge ranges directly into TSV records.
8. The parsed sheet is dropped, and parsing resumes with the next sheet.

Flat `.fods` input follows the same route after step 2, except the root media type is checked and
the original file is rewound and streamed directly.

## Development and verification

Run the deterministic checks with:

```bash
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

The integration suite constructs minimal spreadsheets and covers validation, two input containers,
multiple encodings, XML references, whitespace, formatted/raw selection, repeats, merges, alignment,
file naming, and CLI exit codes.

An optional real-file smoke suite reads `test-files/`. Generate that gitignored directory with:

```bash
scripts/generate-test-files.sh
cargo test --locked --test real_files
```

The generator requires `python3`, LibreOffice (`soffice`), `curl`, and network access. It creates
handwritten rich fixtures, round-trips files through LibreOffice, and downloads selected LibreOffice
regression documents. The normal test suite does not require those tools or the network.

## Deliberate boundaries

This is a text extractor, not a spreadsheet calculation engine. In particular:

- formulas are not evaluated; the stored/displayed cell result is extracted;
- formatting and style definitions are not reproduced;
- annotations, drawings, nested tables, and tracked changes are excluded from cell text;
- encrypted packages are refused rather than decrypted; and
- malformed but parseable structures may receive stable best-effort output rather than full ODF
  conformance diagnostics.

The implementation targets the OpenDocument model directly. It began as a port of
[`ods_to_tsv.py`](https://github.com/viell-dev/ods_to_tsv.py), but compatibility with historical
Python quirks is not a design goal.

## License

`ods-to-tsv` is licensed under GPL-3.0-only. See [COPYING.md](COPYING.md).
