#!/bin/sh
# Same command on macOS and Linux. Links libc and libm only.
# CFLAGS and RUSTC_LINK_ARGS add flags (test.sh uses them for sanitizers).
# CC picks the C compiler and TARGET a rustc target triple, for release builds
# (.github/workflows/release.yml); both default to the host.
set -eu
cd "$(dirname "$0")"
cc=${CC:-cc}
target=''
[ -n "${TARGET:-}" ] && target="--target $TARGET"
# musl keeps its math in libc.a, which rustc links before our objects; name it
# again after them, since -lm there would find the host's glibc libm.
libm=-C\ link-arg=-lm
case ${TARGET:-} in *-musl) libm=-C\ link-arg=-lc ;; esac
# -ffp-contract=off keeps stb_truetype's float math unfused, so Apple clang on
# arm64 (which emits FMA by default) matches x86-64 Linux pixel for pixel.
# Never add -ffast-math: it changes about half of all renders.
$cc -c src/draw.c -o draw.o -O2 -ffp-contract=off -Wall -Wextra -Wno-unused-function -Wno-missing-field-initializers -I third_party/stb ${CFLAGS:-}
$cc -c src/deflate.c -o deflate.o -O2 -Wall -Wextra ${CFLAGS:-}
# shellcheck disable=SC2086
rustc --edition 2021 src/main.rs -o termshot -C opt-level=2 $target \
  -C link-arg="$PWD/draw.o" -C link-arg="$PWD/deflate.o" \
  $libm ${RUSTC_LINK_ARGS:-}
