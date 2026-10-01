#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
./build.sh
scratch=$(mktemp -d)
trap 'rm -f "$scratch/codec" "$scratch/draw" "$scratch/draw.png"; rmdir "$scratch"' EXIT HUP INT TERM
sanitize=''
if [ "${SANITIZE:-0}" = 1 ]; then
    sanitize='-fsanitize=address,undefined -fno-omit-frame-pointer'
fi
cc tests/codec.c -I third_party/stb -O2 -Wno-deprecated-declarations $sanitize -o "$scratch/codec"
python3 tests/codec.py "$scratch/codec"
cc tests/draw.c -I third_party/stb -O2 -Wno-deprecated-declarations $sanitize -lm -o "$scratch/draw"
"$scratch/draw" third_party/jetbrains-mono/JetBrainsMono-Regular.ttf "$scratch/draw.png"
python3 tests/render.py
