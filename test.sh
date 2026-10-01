#!/bin/sh
# Build, then run the parser unit tests, the CLI exit-code checks and the
# pixel goldens. Renders land in target/test/ and are kept for inspection.
#
#   ./test.sh                    run everything
#   ./test.sh --update-goldens   rewrite tests/goldens.txt from this build
#   SANITIZE=1 ./test.sh         build draw.c with ASan + UBSan (macOS clang)
set -eu
cd "$(dirname "$0")"
mode=check
[ "${1:-}" = --update-goldens ] && mode=update
out=target/test
mkdir -p "$out"
font=third_party/jetbrains-mono/JetBrainsMono-Regular.ttf

if [ "${SANITIZE:-}" = 1 ]; then
    [ "$(uname)" = Darwin ] || { echo "SANITIZE=1 is wired up for macOS clang only" >&2; exit 2; }
    # rustc links with -nodefaultlibs, so the sanitizer runtime is named explicitly.
    rt="$(cc -print-resource-dir)/lib/darwin"
    export CFLAGS="-O1 -g -fno-omit-frame-pointer -fsanitize=address,undefined -fno-sanitize-recover=undefined"
    export RUSTC_LINK_ARGS="-C link-arg=-fsanitize=address,undefined -C link-arg=$rt/libclang_rt.asan_osx_dynamic.dylib -C link-arg=-Wl,-rpath,$rt"
    export ASAN_OPTIONS=detect_leaks=0:abort_on_error=1
fi

./build.sh

echo "== unit tests"
# shellcheck disable=SC2086
rustc --edition 2021 --test src/main.rs -o "$out/unit" \
    -C link-arg="$PWD/draw.o" -C link-arg=-lm ${RUSTC_LINK_ARGS:-}
"$out/unit" -q

echo "== cli"
fail=0
expect() {
    want=$1
    shift
    set +e
    ./termshot "$@" >/dev/null 2>&1
    got=$?
    set -e
    if [ "$got" -ne "$want" ]; then
        echo "FAIL exit $got, want $want: termshot $*"
        fail=1
    fi
}
expect 2
expect 0 --help
expect 1 "$out/missing.pty" "$out/x.png" "$font"
expect 1 examples/reply-sent.pty "$out/x.png" "$out/missing.ttf"
expect 2 examples/reply-sent.pty "$out/x.png" "$font" 0
expect 2 examples/reply-sent.pty "$out/x.png" "$font" 48 0
expect 2 examples/reply-sent.pty "$out/x.png" "$font" 255 500 73
[ "$fail" -eq 0 ] && echo "ok"

echo "== goldens"
# px 46 is sensitive to FMA contraction (macOS vs Linux, #3); px 48 is the README size.
goldens=tests/goldens.txt
fresh="$out/goldens.txt"
echo "# sha256 of decoded RGBA pixels, size, log, px. Rewrite with ./test.sh --update-goldens" > "$fresh"
for log in reply-sent draft-ready; do
    for px in 46 48; do
        png="$out/$log-$px.png"
        ./termshot "examples/$log.pty" "$png" "$font" "$px" 2>/dev/null
        echo "$(python3 tests/pixel_hash.py "$png") $log $px" >> "$fresh"
    done
done
if [ "$mode" = update ]; then
    cp "$fresh" "$goldens"
    echo "updated $goldens"
elif diff -u "$goldens" "$fresh"; then
    echo "ok"
else
    echo "FAIL pixels changed; renders are in $out/"
    fail=1
fi
exit "$fail"
