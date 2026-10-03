#!/bin/sh
# Regenerate tests/fixtures/cff2-outlines.txt: the outline HarfBuzz draws for
# each character of the CFF2 test font's default instance, which
# src/cff_tests.rs checks cff.rs against. stb_truetype can't read CFF2, so
# HarfBuzz is the reference here. Needs hb-info and hb-vector (harfbuzz-utils).
#
#   tools/cff2-outlines.sh
#
# Each line is a glyph id, then the POSIX cksum (CRC and length) of the
# outline as hb-vector writes its SVG path. The font is 1000 units per em, so
# at --font-size=1000 the path is in font units.
set -eu
cd "$(dirname "$0")/.."
font=third_party/noto-sans-cjk-vf/NotoSansCJKtc-VF-Subset.otf
out=tests/fixtures/cff2-outlines.txt
tab=$(printf '\t')
# Each tool's output is captured on its own, so set -e sees it fail; a
# pipeline would report only its last command. The file is replaced only
# once every outline is in.
version=$(hb-vector --version)
unicodes=$(hb-info --list-unicodes "$font")
map=$(printf '%s\n' "$unicodes" | sed -n "s/^U+\([0-9A-F]*\)${tab}gid\([0-9]*\)\$/\1 \2/p")
[ -n "$map" ] || { echo "hb-info listed no characters" >&2; exit 1; }
{
    printf '# %s, from tools/cff2-outlines.sh\n' "$(printf '%s\n' "$version" | head -n 1)"
    while read -r cp gid; do
        svg=$(hb-vector --font-size=1000 -u "$cp" "$font")
        path=$(printf '%s\n' "$svg" | sed -n 's/.*<path d="\([^"]*\)".*/\1/p')
        [ -n "$path" ] || continue # a space: no outline
        # shellcheck disable=SC2046
        set -- $(printf %s "$path" | cksum)
        echo "$gid $1 $2"
    done <<EOF
$map
EOF
} > "$out.tmp"
mv "$out.tmp" "$out"
echo "wrote $out"
