#!/bin/sh
# Build src/deflate.rs as a static library for the C harnesses
# (tests/deflate_lib.rs) and print the system libraries a C link needs for
# it, as rustc names them. Used by test.sh and tests/run.sh:
#
#   libs=$(tests/deflate_lib.sh out.a [rustc flags])
#   cc harness.c out.a $libs
#
# With SANITIZE=1 it keeps overflow checks and debug assertions, since the
# C sanitizers can't instrument the Rust (that needs nightly rustc).
set -eu
cd "$(dirname "$0")/.."
lib=$1
shift
checks=''
[ "${SANITIZE:-}" = 1 ] && checks='-C debug-assertions=on -C overflow-checks=on'
log="$lib.log"
# shellcheck disable=SC2086
if ! rustc --edition 2021 --crate-type staticlib -C opt-level=2 $checks "$@" tests/deflate_lib.rs \
    -o "$lib" --print native-static-libs 2>"$log"; then
    cat "$log" >&2
    exit 1
fi
# Warnings still show; the notes are the libraries.
grep -v -e '^note: ' -e '^$' "$log" >&2 || true
libs=$(sed -n 's/^note: native-static-libs: //p' "$log")
rm -f "$log"
# On macOS, libc and libm are libSystem, which cc always links; naming them
# again only makes ld warn about duplicates.
if [ "$(uname)" = Darwin ]; then
    libs=$(printf '%s\n' "$libs" | tr ' ' '\n' | { grep -v -e '^-lSystem$' -e '^-lc$' -e '^-lm$' || true; } | tr '\n' ' ')
fi
printf '%s\n' "$libs"
