#!/bin/sh
# Build the #25 CFF POC driver into target/cff-poc (docs/cff-rust-vs-c.md).
# Flags as build.sh: C at -O2 -ffp-contract=off, Rust at opt-level=2.
set -eu
cd "$(dirname "$0")/../.."
work=$PWD/target/cff-poc
mkdir -p "$work"
cc -c bench/cff-poc/cff.c -o "$work/cff.o" -O2 -ffp-contract=off -Wall -Wextra
cc -c bench/cff-poc/stb_ref.c -o "$work/stb_ref.o" -O2 -ffp-contract=off -Wno-unused-function -I third_party/stb
rustc --edition 2021 -C opt-level=2 bench/cff-poc/poc.rs -o "$work/poc" \
    -C link-arg="$work/cff.o" -C link-arg="$work/stb_ref.o" -C link-arg=-lm

# The fuzzer: all C under ASan + UBSan, cff.rs with overflow checks.
san="-fsanitize=address,undefined -fno-sanitize-recover=undefined -O1 -g"
# shellcheck disable=SC2086
cc $san -c bench/cff-poc/cff.c -o "$work/cff-san.o"
# shellcheck disable=SC2086
cc $san -c bench/cff-poc/stb_ref.c -o "$work/stb_ref-san.o" -Wno-unused-function -I third_party/stb
# shellcheck disable=SC2086
cc $san -c bench/cff-poc/fuzz.c -o "$work/fuzz.o" -I bench/cff-poc
rustc --edition 2021 --crate-type staticlib -C opt-level=1 -C overflow-checks=on -C debug-assertions=on \
    bench/cff-poc/fuzz_rs.rs -o "$work/libfuzz_rs.a"
# shellcheck disable=SC2086
cc $san "$work/fuzz.o" "$work/cff-san.o" "$work/stb_ref-san.o" "$work/libfuzz_rs.a" -o "$work/fuzz" -lm -lpthread -ldl
