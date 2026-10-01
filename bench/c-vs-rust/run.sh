#!/bin/sh
# C vs Rust POC (docs/c-vs-rust.md): paint and compress the same cell grids
# with draw.c/deflate.c and with their Rust ports, check that the outputs are
# byte-identical, and time both in one process.
#
#   bench/c-vs-rust/run.sh [rounds]     default 61 rounds per workload
set -eu
cd "$(dirname "$0")/../.."
rounds=${1:-61}
# The C the Rust was ported from. Later draw.c changes don't affect this run.
rev=bd726a6
work=target/c-vs-rust
rm -rf "$work"
mkdir -p "$work/snapshot" "$work/cells"
git archive "$rev" src third_party/stb | tar -x -C "$work/snapshot"
snap="$work/snapshot"

# The C side, with build.sh's flags.
cc -c bench/c-vs-rust/poc.c -o "$work/poc.o" -O2 -ffp-contract=off -Wno-deprecated-declarations \
    -I "$snap/src" -I "$snap/third_party/stb"
cc -c "$snap/src/deflate.c" -o "$work/deflate.o" -O2

# Workloads, parsed by the current termshot parser.
./build.sh
rustc --edition 2021 --test src/main.rs -o "$work/unit" \
    -C link-arg="$PWD/draw.o" -C link-arg="$PWD/deflate.o" -C link-arg=-lm
TERMSHOT_POC_DIR="$work/cells" "$work/unit" --ignored --exact tests::poc_workloads -q > /dev/null

poc() {
    rustc --edition 2021 -C opt-level=2 "$@" bench/c-vs-rust/poc.rs -o "$work/poc" \
        -C link-arg="$PWD/$work/poc.o" -C link-arg="$PWD/$work/deflate.o" -C link-arg=-lm
    "$work/poc" "$work/cells" "$rounds"
}
echo "== safe Rust (opt-level=2, as build.sh), time per call, median of $rounds rounds"
poc
echo "== the same, with deflate's two hot spots unchecked (--cfg unchecked)"
poc --cfg unchecked
