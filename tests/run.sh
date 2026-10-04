#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
./build.sh
scratch=$(mktemp -d)
trap 'rm -f "$scratch/codec" "$scratch/codec-custom" "$scratch/rust.a" "$scratch/termshot-faults" "$scratch/fault.png" "$scratch/fault.txt" "$scratch/fault.err" "$scratch/strokes.pty" "$scratch/strokes.png" "$scratch/draw" "$scratch/draw.png" "$scratch/unit" "$scratch/profile" "$scratch/image"; rmdir "$scratch"' EXIT HUP INT TERM
sanitize=''
if [ "${SANITIZE:-0}" = 1 ]; then
    sanitize='-fsanitize=address,undefined -fno-omit-frame-pointer'
fi
# The Rust draw.c calls, for the C harnesses; with SANITIZE=1, with overflow
# checks.
rust_libs=$(tests/rust_lib.sh "$scratch/rust.a")
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
cc tests/draw.c "$scratch/rust.a" -I third_party/stb -O2 -ffp-contract=off -Wno-deprecated-declarations $sanitize -lm $rust_libs -o "$scratch/draw"
"$scratch/draw" third_party/jetbrains-mono/JetBrainsMono-Regular.ttf "$scratch/draw.png"

# Fail each compressor allocation in turn, in the CLI: the unit tests check
# that each returns NULL and frees the rest; here the run must exit 2, say
# so, and leave no output behind. A build with the fault hook, linked as
# build.sh links termshot.
rustc --edition 2021 src/main.rs -o "$scratch/termshot-faults" -C opt-level=2 --cfg termshot_alloc_faults \
    -L native="$PWD" -l static=termshot_c
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

# The image layers and the backdrop (src/composite.rs) allocate nothing; the
# raster they paint is draw.c's one allocation for them. When it fails the
# run exits 2, says so, and leaves no output. Linux only, where ulimit -v
# caps the address space: 300x80 cells at --px 100 are a 321 MB raster.
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

rustc --edition 2021 --test src/main.rs -o "$scratch/unit" \
    -L native="$PWD" -l static=termshot_c
rustc --edition 2021 tests/profile.rs -o "$scratch/profile"
"$scratch/profile" "$scratch/unit"
