#!/bin/sh
# Same command on macOS and Linux. Links libc and libm only.
set -eu
cd "$(dirname "$0")"
cc -c src/draw.c -o draw.o -O2 -Wall -Wextra -Wno-unused-function -Wno-missing-field-initializers -I third_party/stb
rustc --edition 2021 src/main.rs -o termshot -C opt-level=2 \
  -C link-arg="$PWD/draw.o" \
  -C link-arg=-lm
