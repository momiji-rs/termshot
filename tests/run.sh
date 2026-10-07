#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
./build.sh
scratch=$(mktemp -d)
trap 'rm -f "$scratch/glyphs.pty" "$scratch/codec" "$scratch/codec-custom" "$scratch/rust.a" "$scratch/render.a" "$scratch/termshot-faults" "$scratch/fault.png" "$scratch/fault.txt" "$scratch/fault.err" "$scratch/strokes.pty" "$scratch/strokes.png" "$scratch/draw" "$scratch/draw.png" "$scratch/unit" "$scratch/profile" "$scratch/image" "$scratch/libtermshot-faults.rlib" "$scratch/library-faults" "$scratch/png_read.o" "$scratch/libpng_read.a"; rmdir "$scratch"' EXIT HUP INT TERM
sanitize=''
if [ "${SANITIZE:-0}" = 1 ]; then
    sanitize='-fsanitize=address,undefined -fno-omit-frame-pointer'
fi
# The Rust the C harnesses call, and the render with it for tests/draw.c;
# with SANITIZE=1, with overflow checks.
rust_libs=$(tests/rust_lib.sh "$scratch/rust.a")
render_libs=$(tests/rust_lib.sh "$scratch/render.a" --cfg termshot_render)
# shellcheck disable=SC2086
cc tests/image.c "$scratch/rust.a" -I third_party/stb -O2 -Wno-unused-function $sanitize $rust_libs -o "$scratch/image"
"$scratch/image"
# shellcheck disable=SC2086
cc tests/codec.c -I third_party/stb -O2 -Wno-deprecated-declarations $sanitize -o "$scratch/codec"
"$scratch/codec"
# shellcheck disable=SC2086
cc tests/codec.c "$scratch/rust.a" -DTEST_CUSTOM_DEFLATE -I third_party/stb -O2 -Wno-deprecated-declarations $sanitize $rust_libs -o "$scratch/codec-custom"
"$scratch/codec-custom"
# shellcheck disable=SC2086
cc tests/draw.c "$scratch/render.a" -I third_party/stb -O2 -ffp-contract=off -Wno-deprecated-declarations $sanitize -lm $render_libs -o "$scratch/draw"
"$scratch/draw" third_party/jetbrains-mono/JetBrainsMono-Regular.ttf "$scratch/draw.png"

# Fail each compressor allocation in turn, in the CLI: the unit tests check
# that each returns NULL and frees the rest; here the run must exit 2, say
# so, and leave no output behind. A build with the fault hook: the library
# built with it, and the CLI linking it, as build.sh links termshot.
rustc --edition 2021 --crate-type rlib --crate-name termshot src/lib.rs -o "$scratch/libtermshot-faults.rlib" \
    -C opt-level=2 --cfg termshot_alloc_faults -L native="$PWD" -l static=termshot_c
rustc --edition 2021 src/main.rs -o "$scratch/termshot-faults" -C opt-level=2 \
    --extern termshot="$scratch/libtermshot-faults.rlib"
n=1
while :; do
    rm -f "$scratch/fault.png" "$scratch/fault.txt"
    set +e
    TERMSHOT_DEFLATE_FAIL_AT=$n "$scratch/termshot-faults" --text "$scratch/fault.txt" \
        examples/reply-sent.pty "$scratch/fault.png" 2>"$scratch/fault.err"
    code=$?
    set -e
    [ "$code" -eq 0 ] && break
    if [ "$code" -ne 2 ] || [ -e "$scratch/fault.png" ] || [ -e "$scratch/fault.txt" ] ||
        ! grep -q "out of memory encoding" "$scratch/fault.err"; then
        echo "FAIL compressor allocation $n: exit $code, want 2 with no output left" >&2
        cat "$scratch/fault.err" >&2
        exit 1
    fi
    n=$((n + 1))
done
# The hash table, its counts, the output buffer and at least one growth.
if [ "$n" -le 4 ]; then
    echo "FAIL only $((n - 1)) compressor allocations failed" >&2
    exit 1
fi
echo "ok, each of $((n - 1)) compressor allocation failures exits 2 and leaves no output"

# Fail each of the render's own allocations in turn (src/api_render.rs: the
# copies of the cells, the marks and the image views it prepares; then
# src/render.rs: the canvas, then the PNG's buffer, which stb_image_write
# asks for through termshot_png_alloc and the render returns): the run must
# exit 2, say which ran out, and leave no output. The fault build says which allocation failed; when none does, all
# have been.
n=1
while :; do
    rm -f "$scratch/fault.png" "$scratch/fault.txt"
    set +e
    TERMSHOT_RENDER_FAIL_AT=$n "$scratch/termshot-faults" --text "$scratch/fault.txt" \
        examples/reply-sent.pty "$scratch/fault.png" 2>"$scratch/fault.err"
    code=$?
    set -e
    if ! grep -q "render allocation $n " "$scratch/fault.err"; then
        [ "$code" -eq 0 ] && [ -e "$scratch/fault.png" ] && break
        echo "FAIL with no render allocation $n failing: exit $code" >&2
        cat "$scratch/fault.err" >&2
        exit 1
    fi
    case $n in
        1) want="out of memory for the cells of a 100x30 grid" ;;
        2) want="out of memory for the combining marks of a 100x30 grid" ;;
        3) want="out of memory for the image views of a 100x30 grid" ;;
        4) want="out of memory for a 2200x1440 image" ;;
        *) want="out of memory encoding a 2200x1440 PNG" ;;
    esac
    if [ "$code" -ne 2 ] || [ -e "$scratch/fault.png" ] || [ -e "$scratch/fault.txt" ] ||
        ! grep -q "$want" "$scratch/fault.err"; then
        echo "FAIL render allocation $n: exit $code, want 2 with no output left" >&2
        cat "$scratch/fault.err" >&2
        exit 1
    fi
    n=$((n + 1))
done
if [ "$n" -ne 6 ]; then
    echo "FAIL $((n - 1)) render allocations failed, want the three copies, the canvas and the PNG's buffer" >&2
    exit 1
fi
echo "ok, each of $((n - 1)) render allocation failures exits 2 and leaves no output"

# Fail each of the parser's allocations that grow with the grid in turn
# (src/screen.rs: both screens' cells and rows, the tab stops, the marks,
# the marks and the cells in screen order): the run must exit 2, say so,
# and leave no output.
printf 'e\314\201q\314\202\r\n%.0s' $(seq 40) > "$scratch/glyphs.pty"
n=1
while :; do
    rm -f "$scratch/fault.png" "$scratch/fault.txt"
    set +e
    TERMSHOT_PARSE_FAIL_AT=$n "$scratch/termshot-faults" --text "$scratch/fault.txt" \
        "$scratch/glyphs.pty" "$scratch/fault.png" 2>"$scratch/fault.err"
    code=$?
    set -e
    if ! grep -q "parse allocation $n " "$scratch/fault.err"; then
        [ "$code" -eq 0 ] && [ -e "$scratch/fault.png" ] && break
        echo "FAIL with no parse allocation $n failing: exit $code" >&2
        cat "$scratch/fault.err" >&2
        exit 1
    fi
    if [ "$code" -ne 2 ] || [ -e "$scratch/fault.png" ] || [ -e "$scratch/fault.txt" ] ||
        ! grep -q "out of memory replaying the log on a 100x30 grid" "$scratch/fault.err"; then
        echo "FAIL parse allocation $n: exit $code, want 2 with no output left" >&2
        cat "$scratch/fault.err" >&2
        exit 1
    fi
    n=$((n + 1))
done
# Five for the screens, the marks, the marks and the cells in screen order.
if [ "$n" -ne 9 ]; then
    echo "FAIL $((n - 1)) parse allocations failed, want 8" >&2
    exit 1
fi
echo "ok, each of $((n - 1)) parse allocation failures exits 2 and leaves no output"

# The same faults through the library, as an embedder links it: the rlib
# built with the fault hook (as the CLI above links it), and
# tests/library.rs, which fails each of a parse's and a render's
# allocations in turn (the render's, the compressor's and the glyphs') and
# checks that each returns Error::OutOfMemory.
cc -c tests/png_read.c -o "$scratch/png_read.o" -O2 -I third_party/stb
ar rcs "$scratch/libpng_read.a" "$scratch/png_read.o"
rustc --edition 2021 tests/library.rs -o "$scratch/library-faults" --extern termshot="$scratch/libtermshot-faults.rlib" \
    -L native="$scratch" -l static=png_read
"$scratch/library-faults" --faults 2>"$scratch/fault.err" || { cat "$scratch/fault.err" >&2; exit 1; }

# Fail each allocation of the box-drawing caches in turn (the arcs' offsets
# and the strokes reused): the strokes are stamped afresh, so the run
# succeeds with the same PNG. The fault build says which allocation failed;
# when none does, all have been.
printf '\033[1m\342\225\255\342\225\256\033[m\342\225\260\342\225\257\342\225\261\342\225\262\342\225\263\r\n%.0s' $(seq 6) > "$scratch/strokes.pty"
./termshot --size 7x6 "$scratch/strokes.pty" "$scratch/strokes.png"
n=1
while :; do
    rm -f "$scratch/fault.png"
    set +e
    TERMSHOT_GEOMETRY_FAIL_AT=$n "$scratch/termshot-faults" --size 7x6 "$scratch/strokes.pty" "$scratch/fault.png" \
        2>"$scratch/fault.err"
    code=$?
    set -e
    if [ "$code" -ne 0 ] || ! cmp -s "$scratch/fault.png" "$scratch/strokes.png"; then
        echo "FAIL geometry allocation $n: exit $code, or another PNG" >&2
        cat "$scratch/fault.err" >&2
        exit 1
    fi
    grep -q "geometry allocation $n " "$scratch/fault.err" || break
    n=$((n + 1))
done
# The geometry, the arcs' offsets, and the cache's points, ids, sequences,
# slots, mask and runs.
if [ "$n" -le 8 ]; then
    echo "FAIL only $((n - 1)) geometry allocations failed" >&2
    exit 1
fi
echo "ok, each of $((n - 1)) geometry allocation failures draws the same PNG"

# Fail each glyph allocation in turn (src/glyphs.rs: the cache's slots, a
# CFF outline's scratch, each glyph's bitmap), with a CFF font, a TrueType
# fallback, italic and a mark: the run must exit 2, say so, and leave no
# output. The fault build says which allocation failed; when none does,
# all have been.
printf 'ab \033[3mcd\033[0m \344\270\255q\314\201 \033[1m\342\224\200x\033[0m\r\n' > "$scratch/glyphs.pty"
n=1
while :; do
    rm -f "$scratch/fault.png" "$scratch/fault.txt"
    set +e
    TERMSHOT_GLYPH_FAIL_AT=$n "$scratch/termshot-faults" --size 20x2 --text "$scratch/fault.txt" \
        --font third_party/noto-sans-cjk/NotoSansCJKtc-Subset.otf \
        --fallback-font third_party/jetbrains-mono/JetBrainsMono-Regular.ttf \
        "$scratch/glyphs.pty" "$scratch/fault.png" 2>"$scratch/fault.err"
    code=$?
    set -e
    if ! grep -q "glyph allocation $n " "$scratch/fault.err"; then
        [ "$code" -eq 0 ] && [ -e "$scratch/fault.png" ] && break
        echo "FAIL with no glyph allocation $n failing: exit $code" >&2
        cat "$scratch/fault.err" >&2
        exit 1
    fi
    if [ "$code" -ne 2 ] || [ -e "$scratch/fault.png" ] || [ -e "$scratch/fault.txt" ] ||
        ! grep -q "glyph allocation failed" "$scratch/fault.err"; then
        echo "FAIL glyph allocation $n: exit $code, want 2 with no output left" >&2
        cat "$scratch/fault.err" >&2
        exit 1
    fi
    n=$((n + 1))
done
# The cache, the scratch, and the bitmaps of a, b, c, d, 中, q, the mark and x.
if [ "$n" -le 9 ]; then
    echo "FAIL only $((n - 1)) glyph allocations failed" >&2
    exit 1
fi
echo "ok, each of $((n - 1)) glyph allocation failures exits 2 and leaves no output"

# The image layers and the backdrop (src/composite.rs) allocate nothing; the
# raster they paint is the render's (src/render.rs). When the system can't
# give it, not just the fault build, the run exits 2, says so, and leaves no
# output. Linux only, where ulimit -v caps the address space: 300x80 cells at
# --px 100 are a 321 MB raster.
if [ "$(uname)" = Linux ]; then
    rm -f "$scratch/fault.png" "$scratch/fault.txt"
    set +e
    (ulimit -v 262144 && exec ./termshot --text "$scratch/fault.txt" --px 100 --size 300x80 \
        tests/fixtures/kitty-png-alpha-z.pty "$scratch/fault.png") 2>"$scratch/fault.err"
    code=$?
    set -e
    if [ "$code" -ne 2 ] || [ -e "$scratch/fault.png" ] || [ -e "$scratch/fault.txt" ] ||
        ! grep -q "out of memory for a 13500x7920 image" "$scratch/fault.err"; then
        echo "FAIL raster allocation: exit $code, want 2 with no output left" >&2
        cat "$scratch/fault.err" >&2
        exit 1
    fi
    echo "ok, a raster allocation failure exits 2 and leaves no output"
fi

rustc --edition 2021 --test src/lib.rs -o "$scratch/unit" \
    -L native="$PWD" -l static=termshot_c
rustc --edition 2021 tests/profile.rs -o "$scratch/profile"
"$scratch/profile" "$scratch/unit"
