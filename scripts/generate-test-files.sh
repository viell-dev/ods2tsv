#!/usr/bin/env bash
# Populates the gitignored test-files/ directory with ODS files for manual and
# smoke testing:
#
#   1. Writes a feature-rich spreadsheet body, packaged both as a flat XML
#      document (rich.fods) and as a spec-correct zipped package (rich.ods,
#      stored mimetype first, manifest included).
#   2. Round-trips rich.ods through LibreOffice (rich-roundtrip.ods) to get a
#      file genuinely written by LibreOffice, and converts a plain CSV
#      (simple.ods) for a minimal real-world file.
#   3. Downloads a handful of real-world fixtures from the LibreOffice test
#      suite (skipped when offline; already-downloaded files are kept).
#
# Requires: python3, libreoffice (soffice), curl.
set -euo pipefail

cd "$(dirname "$0")/.."
mkdir -p test-files

body=$(cat <<'EOF'
  <office:body>
    <office:spreadsheet>
      <table:table table:name="Data">
        <table:table-row>
          <table:table-cell office:value-type="string"><text:p>Name</text:p></table:table-cell>
          <table:table-cell office:value-type="string"><text:p>Amount</text:p></table:table-cell>
          <table:table-cell office:value-type="string"><text:p>Date</text:p></table:table-cell>
          <table:table-cell office:value-type="string"><text:p>Notes</text:p></table:table-cell>
        </table:table-row>
        <table:table-row>
          <table:table-cell office:value-type="string">
            <text:p>Ampersand &amp; angles &lt;here&gt;</text:p>
            <office:annotation><text:p>A cell comment that must not leak into output.</text:p></office:annotation>
          </table:table-cell>
          <table:table-cell office:value-type="currency" office:value="9.99" office:currency="EUR"><text:p>9,99 €</text:p></table:table-cell>
          <table:table-cell office:value-type="date" office:date-value="2024-03-01"><text:p>2024-03-01</text:p></table:table-cell>
          <table:table-cell office:value-type="string"><text:p>Multi<text:line-break/>line and gap<text:s text:c="4"/>then tab<text:tab/>end</text:p></table:table-cell>
        </table:table-row>
        <table:table-row>
          <table:table-cell table:number-rows-spanned="2" table:number-columns-spanned="2" office:value-type="string"><text:p>Merged 2x2</text:p></table:table-cell>
          <table:covered-table-cell/>
          <table:table-cell table:number-columns-repeated="2" office:value-type="float" office:value="7"><text:p>7</text:p></table:table-cell>
        </table:table-row>
        <table:table-row>
          <table:covered-table-cell table:number-columns-repeated="2"/>
          <table:table-cell office:value-type="boolean" office:boolean-value="true"><text:p>TRUE</text:p></table:table-cell>
        </table:table-row>
        <table:table-row><table:table-cell/></table:table-row>
        <table:table-row>
          <table:table-cell office:value-type="string"><text:p>after an empty row</text:p></table:table-cell>
        </table:table-row>
        <table:table-row table:number-rows-repeated="500">
          <table:table-cell table:number-columns-repeated="16384"/>
        </table:table-row>
      </table:table>
      <table:table table:name="Ünïcode ✓ / slash">
        <table:table-row>
          <table:table-cell office:value-type="string"><text:p>§çÜ ✓ 漢字</text:p></table:table-cell>
        </table:table-row>
      </table:table>
    </office:spreadsheet>
  </office:body>
EOF
)

xmlns='xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
    xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0"
    xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"
    office:version="1.3"'

cat > test-files/rich.fods <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<office:document
    $xmlns
    office:mimetype="application/vnd.oasis.opendocument.spreadsheet">
$body
</office:document>
EOF
echo "wrote test-files/rich.fods"

cat > test-files/content.xml.tmp <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<office:document-content
    $xmlns>
$body
</office:document-content>
EOF

python3 - <<'EOF'
import zipfile

MIME = "application/vnd.oasis.opendocument.spreadsheet"
MANIFEST = f"""<?xml version="1.0" encoding="UTF-8"?>
<manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" manifest:version="1.3">
 <manifest:file-entry manifest:full-path="/" manifest:version="1.3" manifest:media-type="{MIME}"/>
 <manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"/>
</manifest:manifest>
"""

with open("test-files/content.xml.tmp", "rb") as f:
    content = f.read()

with zipfile.ZipFile("test-files/rich.ods", "w") as z:
    # Per OpenDocument Part 3, the mimetype must be first and uncompressed.
    z.writestr("mimetype", MIME, compress_type=zipfile.ZIP_STORED)
    z.writestr("content.xml", content, compress_type=zipfile.ZIP_DEFLATED)
    z.writestr("META-INF/manifest.xml", MANIFEST, compress_type=zipfile.ZIP_DEFLATED)
EOF
rm test-files/content.xml.tmp
echo "wrote test-files/rich.ods"

# A throwaway profile keeps soffice from fighting a running desktop instance.
profile="$(mktemp -d)"
roundtrip="$(mktemp -d)"
trap 'rm -rf "$profile" "$roundtrip"' EXIT
# LibreOffice 26.2 refuses same-format conversions, so round-trip via xlsx to
# get a file genuinely written by LibreOffice.
soffice --headless "-env:UserInstallation=file://$profile" \
    --convert-to xlsx --outdir "$roundtrip" test-files/rich.ods > /dev/null
soffice --headless "-env:UserInstallation=file://$profile" \
    --convert-to ods --outdir "$roundtrip" "$roundtrip/rich.xlsx" > /dev/null
mv "$roundtrip/rich.ods" test-files/rich-roundtrip.ods
echo "wrote test-files/rich-roundtrip.ods (via LibreOffice)"

printf 'id,name,note\n1,alpha,"has ""quotes"""\n2,beta,tab\tinside\n' > "$roundtrip/simple.csv"
soffice --headless "-env:UserInstallation=file://$profile" \
    --convert-to ods --outdir test-files "$roundtrip/simple.csv" > /dev/null
echo "wrote test-files/simple.ods (via LibreOffice)"

# Small real-world fixtures from LibreOffice's Calc import/export test suite.
base="https://raw.githubusercontent.com/LibreOffice/core/master/sc/qa/unit/data/ods"
fixtures=(
    tdf75702_textLineBreak.ods   # cell text with line breaks
    merged.ods                   # merged cell areas
    repeatedColumns.ods          # table:number-columns-repeated
    miscemptyrepeatedrowheights.ods # repeated (empty) rows
    sheet_name_with_dots.ods     # awkward sheet names
    formula-quote-in-sheet-name.ods # quotes in sheet names, formulas
    CommentTextHAlign.ods        # cell comments (annotations)
    passwordOld.ods              # password-protected, must be refused
)
for f in "${fixtures[@]}"; do
    if [[ -s "test-files/$f" ]]; then
        echo "have  test-files/$f"
    elif curl -fsSL --max-time 60 -o "test-files/$f" "$base/$f"; then
        echo "fetched test-files/$f"
    else
        echo "SKIP  $f (download failed)" >&2
        rm -f "test-files/$f"
    fi
done
