#!/bin/sh
# Record the outline HarfBuzz draws for each character of a CFF2 font, which
# src/cff_tests.rs checks cff.rs against. stb_truetype can't read CFF2, so
# HarfBuzz is the reference here. Needs hb-info and hb-vector (harfbuzz-utils).
#
#   tools/cff2-outlines.sh [--variations=LIST] [FONT OUT]
#
# With no FONT, regenerates tests/fixtures/cff2-outlines.txt from the CFF2
# test font. --variations takes hb-vector's list (wght=700,wdth=100) and
# draws that instance instead of the default one; it is recorded in a
# "# variations:" line.
#
# Each line is a glyph id, then the POSIX cksum (CRC and length) of the
# outline as hb-vector writes its SVG path. The font is 1000 units per em, so
# at --font-size=1000 the path is in font units. Points of an instance fall
# between font units, so they are printed to 9 places and cut toward zero,
# as termshot hands them to the rasterizer.
set -eu
cd "$(dirname "$0")/.."
variations=''
case ${1:-} in
--variations=*) variations=${1#--variations=}; shift ;;
esac
font=${1:-third_party/noto-sans-cjk-vf/NotoSansCJKtc-VF-Subset.otf}
out=${2:-tests/fixtures/cff2-outlines.txt}
tab=$(printf '\t')
# Each tool's output is captured on its own, so set -e sees it fail; a
# pipeline would report only its last command. The file is replaced only
# once every outline is in.
version=$(hb-vector --version)
unicodes=$(hb-info --list-unicodes "$font")
# A font with glyph names lists characters by name, so --list-glyphs gives
# their ids; one without lists them as gidN.
glyphs=$(hb-info --list-glyphs "$font")
map=$(printf '%s\n@@\n%s\n' "$glyphs" "$unicodes" | awk -F "$tab" '
    $0 == "@@" { chars = 1; next }
    !chars { if ($1 ~ /^[0-9]+$/) id[$2] = $1; next }
    $1 ~ /^U\+[0-9A-F]+$/ {
        gid = $2 ~ /^gid[0-9]+$/ ? substr($2, 4) : id[$2]
        if (gid != "") print substr($1, 3), gid
    }')
[ -n "$map" ] || { echo "hb-info listed no characters" >&2; exit 1; }
{
    printf '# %s, from tools/cff2-outlines.sh\n' "$(printf '%s\n' "$version" | head -n 1)"
    [ -z "$variations" ] || printf '# variations: %s\n' "$variations"
    # Drawn by glyph id, not by character: shaping a character could pick
    # another glyph, as GSUB's FeatureVariations do at some instances, or
    # move a combining mark off its outline's origin.
    while read -r _ gid; do
        svg=$(hb-vector --font-size=1000 --precision=9 --variations="$variations" --glyphs "$font" "gid$gid")
        path=$(printf '%s\n' "$svg" | sed -n 's/.*<path d="\([^"]*\)".*/\1/p' | awk '{
            out = ""; s = $0
            while (match(s, /-?[0-9]+(\.[0-9]+)?/)) {
                v = int(substr(s, RSTART, RLENGTH) + 0)
                out = out substr(s, 1, RSTART - 1) sprintf("%d", v == 0 ? 0 : v)
                s = substr(s, RSTART + RLENGTH)
            }
            print out s }')
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
