#!/bin/sh
# #25 CFF POC (docs/cff-rust-vs-c.md): check cff.rs and cff.c against stock
# stb_truetype on every glyph of the system's CFF fonts, time all three, and
# fuzz them. Needs the Arch packages gsfonts, noto-fonts-cjk and harfbuzz-utils
# (for hb-subset); fonts that are missing are skipped.
#
#   bench/cff-poc/run.sh [mutants]     default 20000 mutants per fuzzed font
set -eu
cd "$(dirname "$0")/../.."
mutants=${1:-20000}
work=target/cff-poc
cjk=/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc
latin=/usr/share/fonts/gsfonts/NimbusSans-Regular.otf
sh bench/cff-poc/build.sh

# A small CID-keyed font for the fuzzer: Noto Sans CJK TC cut to 22 CJK
# characters and ASCII, 176 glyphs. --unicodes+= adds to --text; a plain
# --unicodes= would replace it and leave only the ASCII.
hb-subset "$cjk" --face-index=3 --text="骨直角永東京台灣測試字型漢字中文繁體簡體龍鬱鑿齉" \
    --unicodes+=20-7e -o "$work/cjk-subset.otf"
mkdir -p "$work/craft"
python3 bench/cff-poc/craft.py "$work/craft"

echo "== every glyph, cff.rs and cff.c against stb"
"$work/poc" check /usr/share/fonts/gsfonts/*.otf
"$work/poc" check /usr/share/fonts/noto-cjk/*.ttc

echo "== time, in one process, alternating"
"$work/poc" time 21 "$cjk" 3 U+4E00 1500
"$work/poc" time 61 "$latin" 0 U+0020 95

echo "== hand-made hostile fonts"
"$work/fuzz" one "$work"/craft/*.otf

echo "== mutation fuzz, ASan + UBSan, 3 s per run"
"$work/fuzz" "$work/cjk-subset.otf" "$mutants" 2
"$work/fuzz" "$latin" "$mutants" 3
