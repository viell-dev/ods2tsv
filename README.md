# ODS to TSV Extractor

A self-contained command-line tool that extracts individual sheets from an `.ods` (OpenDocument Spreadsheet) file into
separate `.tsv` (Tab-Separated Values) files.

This is a Rust port of the original Python implementation ([`ods_to_tsv.py`](https://github.com/viell-dev/ods_to_tsv.py))
with byte-for-byte output parity: the Python version's semantics are the spec. Unlike the original, it compiles to a
single static binary with no runtime dependencies — no Python, no `lxml`.

## Features

- **Individual Sheet Extraction**: Each sheet in the ODS file is saved as a separate `.tsv` file.
- **Merged Cell Support**: Handles row-spans and col-spans accurately, repeating the value across all spanned cells to
  maintain column alignment.
- **Text Fidelity**:
  - Preserves multiple paragraphs within a single cell.
  - Handles manual line breaks (`text:line-break`).
  - Preserves repeated spaces (`text:s`).
- **Timestamped Backups**: Automatically creates a directory named `<filename> - <ISO-datetime>` in the current working
  directory to store the exports, allowing for dated backups.
- **Efficiency**: Optimized to skip excessive trailing empty rows and columns often found in large spreadsheets.
- **Sanitization**: Automatically sanitizes sheet names for use as valid filenames.
- **Encryption Detection**: Password-protected files are detected up front and refused with a clear message instead of
  a cryptic parse error.

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

## Output Format

Rows are written the way Python's `csv.writer` writes them with a tab delimiter: fields containing tabs, newlines, or
double quotes are wrapped in double quotes (with inner quotes doubled), and rows end in `\r\n`.

## Testing

A fixture-based parity test suite builds synthetic ODS files in-memory and verifies extraction behavior, covering the
same cases as the original Python test suite plus entity handling and encrypted-file detection.

```bash
cargo test
```

## License

This project is licensed under the **GNU General Public License v3.0 (GPL-3.0-only)**. See the
[COPYING.md](COPYING.md) file for details.

## How it Works

The tool reads `content.xml` from the ODS ZIP package with a streaming XML parser, building a compact model of each
sheet with repeats and spans kept as counts. A second pass expands that model — tracking merged areas and repeated
rows/columns — to faithfully reconstruct the flat TSV format while ensuring that data usually hidden in "covered"
cells is preserved for better machine readability.
