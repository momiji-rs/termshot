#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
./build.sh
scratch=$(mktemp -d)
trap 'rm -f "$scratch/codec" "$scratch/codec-custom" "$scratch/deflate.a" "$scratch/termshot-faults" "$scratch/fault.png" "$scratch/fault.txt" "$scratch/fault.err" "$scratch/stamps-alloc" "$scratch/draw" "$scratch/draw.png" "$scratch/unit" "$scratch/profile" "$scratch/image"; rmdir "$scratch"' EXIT HUP INT TERM
sanitize=''
if [ "${SANITIZE:-0}" = 1 ]; then
    sanitize='-fsanitize=address,undefined -fno-omit-frame-pointer'
fi
# src/deflate.rs for the C harnesses; with SANITIZE=1, with overflow checks.
deflate_libs=$(tests/deflate_lib.sh "$scratch/deflate.a")
# shellcheck disable=SC2086
cc tests/image.c "$scratch/deflate.a" -I third_party/stb -O2 -Wno-unused-function $sanitize $deflate_libs -o "$scratch/image"
"$scratch/image"
# shellcheck disable=SC2086
cc tests/codec.c -I third_party/stb -O2 -Wno-deprecated-declarations $sanitize -o "$scratch/codec"
"$scratch/codec"
# shellcheck disable=SC2086
cc tests/codec.c "$scratch/deflate.a" -DTEST_CUSTOM_DEFLATE -I third_party/stb -O2 -Wno-deprecated-declarations $sanitize $deflate_libs -o "$scratch/codec-custom"
"$scratch/codec-custom"
# shellcheck disable=SC2086
cc tests/stamps_alloc.c "$scratch/deflate.a" -I third_party/stb -O2 -ffp-contract=off -Wno-deprecated-declarations $sanitize -lm $deflate_libs -o "$scratch/stamps-alloc"
"$scratch/stamps-alloc"
# shellcheck disable=SC2086
cc tests/draw.c "$scratch/deflate.a" -I third_party/stb -O2 -ffp-contract=off -Wno-deprecated-declarations $sanitize -lm $deflate_libs -o "$scratch/draw"
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

rustc --edition 2021 --test src/main.rs -o "$scratch/unit" \
    -L native="$PWD" -l static=termshot_c
rustc --edition 2021 tests/profile.rs -o "$scratch/profile"
"$scratch/profile" "$scratch/unit"
