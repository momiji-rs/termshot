#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
./build.sh
scratch=$(mktemp -d)
trap 'rm -f "$scratch/codec" "$scratch/codec-custom" "$scratch/deflate-alloc" "$scratch/draw" "$scratch/draw.png" "$scratch/unit"; rmdir "$scratch"' EXIT HUP INT TERM
sanitize=''
if [ "${SANITIZE:-0}" = 1 ]; then
    sanitize='-fsanitize=address,undefined -fno-omit-frame-pointer'
fi
cc tests/codec.c -I third_party/stb -O2 -Wno-deprecated-declarations $sanitize -o "$scratch/codec"
python3 tests/codec.py "$scratch/codec"
cc tests/codec.c src/deflate.c -DTEST_CUSTOM_DEFLATE -I third_party/stb -O2 -Wno-deprecated-declarations $sanitize -o "$scratch/codec-custom"
python3 tests/codec.py "$scratch/codec-custom"
cc tests/deflate_alloc.c -O2 $sanitize -o "$scratch/deflate-alloc"
"$scratch/deflate-alloc"
cc tests/draw.c src/deflate.c -I third_party/stb -O2 -ffp-contract=off -Wno-deprecated-declarations $sanitize -lm -o "$scratch/draw"
"$scratch/draw" third_party/jetbrains-mono/JetBrainsMono-Regular.ttf "$scratch/draw.png"
python3 tests/render.py
rustc --edition 2021 --test src/main.rs -o "$scratch/unit" \
    -C link-arg="$PWD/draw.o" -C link-arg="$PWD/deflate.o" -C link-arg=-lm
python3 tests/profile_threads.py "$scratch/unit"
