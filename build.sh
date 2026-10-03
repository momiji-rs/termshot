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
# -ffp-contract=off keeps stb_truetype's float math unfused, so Apple clang on
# arm64 (which emits FMA by default) matches x86-64 Linux pixel for pixel.
# Never add -ffast-math: it changes about half of all renders.
$cc -c src/draw.c -o draw.o -O2 -ffp-contract=off -Wall -Wextra -Wno-unused-function -Wno-missing-field-initializers -I third_party/stb ${CFLAGS:-}
$cc -c src/deflate.c -o deflate.o -O2 -Wall -Wextra ${CFLAGS:-}
$cc -c src/image.c -o image.o -O2 -Wall -Wextra -Wno-unused-function -I third_party/stb ${CFLAGS:-}
# The C goes in as a static library (-l static=termshot_c, here and in test.sh)
# rather than as bare objects, so rustc puts it before libc and libm. Objects
# passed with -C link-arg land after them, which breaks static musl (math in
# libc.a) and glibc on aarch64 (__stack_chk_guard in ld.so, dropped by
# --as-needed). libm itself comes with std on Linux and libSystem on macOS.
rm -f libtermshot_c.a
ar rcs libtermshot_c.a draw.o deflate.o image.o
# shellcheck disable=SC2086
rustc --edition 2021 src/main.rs -o termshot -C opt-level=2 $target \
  -L native="$PWD" -l static=termshot_c ${RUSTC_LINK_ARGS:-}
