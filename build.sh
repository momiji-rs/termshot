#!/bin/sh
# Same command on macOS and Linux. Links libc and libm only.
set -eu
cd "$(dirname "$0")"
# -ffp-contract=off keeps stb_truetype's float math unfused, so Apple clang on
# arm64 (which emits FMA by default) matches x86-64 Linux pixel for pixel.
# Never add -ffast-math: it changes about half of all renders.
cc -c src/draw.c -o draw.o -O2 -ffp-contract=off -Wall -Wextra -Wno-unused-function -Wno-missing-field-initializers -I third_party/stb
rustc --edition 2021 src/main.rs -o termshot -C opt-level=2 \
  -C link-arg="$PWD/draw.o" \
  -C link-arg=-lm
