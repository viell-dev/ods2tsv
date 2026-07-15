# ODS to TSV Extractor

A self-contained command-line tool that extracts individual sheets from an `.ods` (OpenDocument Spreadsheet) file into
separate `.tsv` (Tab-Separated Values) files.

It started as a port of the original Python implementation
([`ods_to_tsv.py`](https://github.com/viell-dev/ods_to_tsv.py)) and has since diverged: it targets the OpenDocument
specification directly rather than mirroring the Python version's output. It compiles to a single static binary with no
runtime dependencies — no Python, no `lxml`.

## Features

- **Individual Sheet Extraction**: Each sheet in the ODS file is saved as a separate `.tsv` file. File names are
  sanitized, deduplicated, and never empty.
- **Any standards-compliant ODS**: Zipped packages (`.ods`) and flat XML documents (`.fods`) in any XML encoding,
  produced by any ODF version. Inputs are validated up front — wrong mimetypes, missing content, and non-ODF files are
  refused before any output directory is created.
- **Merged Cell Support**: Handles row-spans and col-spans accurately, repeating the value across all spanned cells to
  maintain column alignment — even across rows that store no cells of their own.
- **Grid Fidelity**: Interior empty rows and cells are preserved so values keep their row and column positions; only
  trailing emptiness is dropped (without ever materializing the up-to-2²⁰ repeated empty rows sheets like to claim).
- **Text Fidelity**:
  - Preserves multiple paragraphs within a single cell.
  - Handles manual line breaks (`text:line-break`), tabs (`text:tab`), repeated spaces (`text:s`), and CDATA.
  - Applies the ODF whitespace-collapsing rules to character data.
  - Excludes text that isn't cell content: cell comments (annotations), embedded shapes, change-tracking history.
  - Falls back to the `office:*-value` attributes for cells without display text.
- **Raw or Formatted Values**: `-r`/`--raw` extracts the stored values (ISO dates, unlocalized numbers) instead of the
  formatted display text.
- **Timestamped Exports**: Automatically creates a directory named `<filename> - <ISO-datetime>` in the current working
  directory to store the exports, allowing for dated backups. Same-second runs get distinct directories.
- **Encryption Detection**: Password-protected files are detected up front and refused with a clear message instead of
  a cryptic parse error.
- **Streaming**: `content.xml` is parsed as a stream, sheet by sheet, and never loaded into memory whole.

## Building

Requires a Rust toolchain (the pinned version is in [`rust-toolchain.toml`](rust-toolchain.toml)).

```bash
cargo build --release
```

The binary ends up at `target/release/ods-to-tsv`.

## Usage

Run the tool from the command line, providing the path to your `.ods` file:

```bash
ods-to-tsv "Path/To/Your/File.ods"
```

A new directory is created in your current location containing the extracted `.tsv` files.

By default cells are extracted as their formatted display text, the way the spreadsheet application shows them
(localized dates and numbers, currency symbols). Pass `-r`/`--raw` to extract the underlying stored values instead —
ISO dates, unlocalized numbers — which is usually what you want when the output is consumed by other programs:

```bash
ods-to-tsv --raw "Path/To/Your/File.ods"
```

Cells without a stored value (plain text cells) fall back to their display text, and vice versa.

### Exit Codes

The exit code distinguishes the failure categories, so scripts can branch without parsing stderr:

| Code | Meaning                                                                               |
| ---- | ------------------------------------------------------------------------------------- |
| 0    | Success.                                                                              |
| 1    | Other error (I/O: unreadable input, failure while writing output).                    |
| 2    | Command-line usage error.                                                             |
| 3    | Not a usable spreadsheet (not an ODF file, wrong mimetype, or missing `content.xml`). |
| 4    | Corrupt or malformed input (broken ZIP package, invalid spreadsheet XML).             |
| 5    | Password-protected input.                                                             |

## Output Format

Rows are written in RFC 4180 style with a tab delimiter: fields containing tabs, newlines, or double quotes are wrapped
in double quotes (with inner quotes doubled), and rows end in `\r\n`. Fully empty rows between data become empty
records to keep row positions stable.

## Testing

A fixture-based test suite builds synthetic ODS files in-memory and verifies extraction behavior: merges, repeats,
whitespace semantics, entity and character references, UTF-16 input, flat XML documents, sheet-name handling, and the
various refusals.

```bash
cargo test
```

There is also a smoke test over real spreadsheet files in the gitignored `test-files/` directory. Populate it with
[`scripts/generate-test-files.sh`](scripts/generate-test-files.sh), which builds feature-rich fixtures (via LibreOffice
where needed) and downloads real-world samples from the LibreOffice test suite. The smoke test is skipped when the
directory is absent.

## License

This project is licensed under the **GNU General Public License v3.0 (GPL-3.0-only)**. See the
[COPYING.md](COPYING.md) file for details.

## How it Works

The input is validated first: the file is sniffed (ZIP package vs. flat XML), the package mimetype and encryption
manifest are checked, and only then is the export directory created. The spreadsheet content is then parsed with a
streaming XML parser into a compact per-sheet model with repeats and spans kept as counts. A second pass expands that
model — tracking merged areas as ranges and counting rather than materializing trailing emptiness — to reconstruct the
flat TSV grid while ensuring that data usually hidden in "covered" cells is preserved for better machine readability.
