#!/bin/sh
# Regenerate src/unicode_tables.rs (character widths and canonical
# compositions) from the Unicode Character Database.
#
#   tools/unicode-tables.sh [version]     default 17.0.0
set -eu
cd "$(dirname "$0")/.."
version=${1:-17.0.0}
ucd=target/ucd-$version
mkdir -p "$ucd"
for file in UnicodeData.txt EastAsianWidth.txt emoji/emoji-data.txt DerivedNormalizationProps.txt; do
    [ -f "$ucd/$(basename "$file")" ] || curl -fsS -o "$ucd/$(basename "$file")" \
        "https://www.unicode.org/Public/$version/ucd/$file"
done
rustc --edition 2021 -O tools/unicode-tables.rs -o target/unicode-tables
target/unicode-tables "$ucd" "$version" > src/unicode_tables.rs
echo "wrote src/unicode_tables.rs from Unicode $version"
