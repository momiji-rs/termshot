#!/bin/sh
# C vs Rust (docs/c-vs-rust.md). Three comparisons, each against a snapshot of
# the C taken with git archive, so later changes to src/ don't affect them:
#
#   bench/c-vs-rust/run.sh [rounds]           2026-10-01 POC: paint and compress
#                                             cell grids with draw.c/deflate.c as
#                                             of $rev and with their Rust ports
#   bench/c-vs-rust/run.sh deflate [rounds]   2026-10-03: deflate.c as of
#                                             $deflate_rev against its Rust port
#                                             (deflate.rs): default, portable-Adler
#                                             and zeroed-table C, safe and
#                                             unchecked Rust, and the shipped
#                                             src/deflate.rs (#12 step 1)
#   bench/c-vs-rust/run.sh geometry [rounds]  2026-10-04: draw.c's box drawing,
#                                             blocks and stroke cache as of
#                                             $geometry_rev against the shipped
#                                             src/geometry.rs (#12 step 2a):
#                                             the same pixels at every cell
#                                             size, then the time of each
#   bench/c-vs-rust/run.sh images [rounds]    2026-10-04: draw.c's image
#                                             layers and backdrop as of
#                                             $images_rev against the shipped
#                                             src/composite.rs (#12 step 2b):
#                                             random scenes, then every image
#                                             and cursor fixture through the
#                                             CLI built at $images_rev and
#                                             now, then the time of each
#
# src/deflate.c is gone from the tree since #12 step 1, draw.c's geometry
# since step 2a and its image layers since step 2b; the comparisons still
# take them from the old revisions. All
# check that the outputs are byte-identical before timing both sides in
# one process. Default 61 rounds. CC picks the C compiler of the deflate
# comparison (default cc); RUSTFLAGS adds rustc flags to its Rust side.
set -eu
cd "$(dirname "$0")/../.."
mode=poc
if [ "${1:-}" = deflate ] || [ "${1:-}" = geometry ] || [ "${1:-}" = images ]; then
    mode=$1
    shift
fi
rounds=${1:-61}
# The C each comparison was ported from. Full SHAs, so CI can fetch just
# these commits into its shallow clone (ci.yml reads them from these lines).
rev=bd726a6b53957c389722f018dbabb6b093ac90bb
deflate_rev=a8a95e0bd7b8ba998688554e09680b34a40a8d4e
geometry_rev=24d71feb5e80a0df5c079a64b1e31b8b2bc7f46b
images_rev=431ed233a1d24cd8e7e4b471343e5dac70fc3462

# Workloads, parsed by the current termshot parser. build.sh leaves the
# current C in libtermshot_c.a; link it the way test.sh does.
workloads() {
    ./build.sh
    rustc --edition 2021 --test src/main.rs -o "$1/unit" \
        -L native="$PWD" -l static=termshot_c
    TERMSHOT_POC_DIR="$2" "$1/unit" --ignored --exact tests::poc_workloads -q > /dev/null
}

if [ "$mode" = geometry ]; then
    cc=${CC:-cc}
    work=target/c-vs-rust-geometry
    rm -rf "$work"
    mkdir -p "$work/snapshot"
    git archive "$geometry_rev" src/draw.c src/png_crc.h src/crc32_table.h | tar -x -C "$work/snapshot"
    # The Rust as the C harnesses link it, and the C with build.sh's flags.
    rust_libs=$(tests/rust_lib.sh "$work/rust.a")
    # shellcheck disable=SC2086
    $cc bench/c-vs-rust/geometry.c "$work/rust.a" -o "$work/geometry" -O2 -ffp-contract=off -w \
        -I "$work/snapshot/src" -I third_party/stb -lm $rust_libs
    echo "== draw.c's geometry at $geometry_rev vs src/geometry.rs"
    echo "C: $($cc --version | head -n 1)"
    echo "Rust: $(rustc --version), -C opt-level=2"
    echo "host: $(uname -sm), load: $(uptime | sed 's/.*load average[s]*: //')"
    "$work/geometry" third_party/jetbrains-mono/JetBrainsMono-Regular.ttf "$rounds"
    echo "load after: $(uptime | sed 's/.*load average[s]*: //')"
    exit 0
fi

if [ "$mode" = images ]; then
    cc=${CC:-cc}
    work=target/c-vs-rust-images
    rm -rf "$work"
    mkdir -p "$work/snapshot" "$work/out"
    # What the old CLI builds from, and the old draw.c for the harness.
    git archive "$images_rev" build.sh src third_party | tar -x -C "$work/snapshot"
    rust_libs=$(tests/rust_lib.sh "$work/rust.a")
    # shellcheck disable=SC2086
    $cc bench/c-vs-rust/images.c "$work/rust.a" -o "$work/images" -O2 -ffp-contract=off -w \
        -I "$work/snapshot/src" -I "$work/snapshot/third_party/stb" -lm $rust_libs
    echo "== draw.c's image layers at $images_rev vs src/composite.rs"
    echo "C: $($cc --version | head -n 1)"
    echo "Rust: $(rustc --version), -C opt-level=2"
    echo "host: $(uname -sm), load: $(uptime | sed 's/.*load average[s]*: //')"
    "$work/images" third_party/jetbrains-mono/JetBrainsMono-Regular.ttf "$rounds"
    if [ -z "${TIME_ONLY:-}" ]; then
        # The fixtures through both CLIs, whose PNGs must be the same bytes:
        # the C and the compressor are the same but for the image layers.
        (cd "$work/snapshot" && ./build.sh)
        ./build.sh
        n=0
        for log in tests/fixtures/kitty-*.pty tests/fixtures/sixel-*.pty tests/fixtures/cursor-*.pty; do
            # Small and large cells, a fractional size, both cursor marks,
            # and rasters past 16 MiB, whose backdrop goes by rows.
            for args in '--px 1' '--px 9' '--px 24 --cursor-shape bar' '--px 47.5 --cursor-shape underline' \
                '--px 48 --size 200x60' '--px 128 --size 120x40 --cursor-shape bar'; do
                # shellcheck disable=SC2086
                "$work/snapshot/termshot" $args "$log" "$work/out/c.png"
                # shellcheck disable=SC2086
                ./termshot $args "$log" "$work/out/rust.png"
                if ! cmp -s "$work/out/c.png" "$work/out/rust.png"; then
                    echo "FAIL $log $args: the PNGs differ" >&2
                    exit 1
                fi
                n=$((n + 1))
            done
        done
        rm -f "$work/out/c.png" "$work/out/rust.png"
        echo "ok, $n fixture renders byte-identical to the CLI at $images_rev"
    fi
    echo "load after: $(uptime | sed 's/.*load average[s]*: //')"
    exit 0
fi

if [ "$mode" = deflate ]; then
    cc=${CC:-cc}
    work=target/c-vs-rust-deflate
    rm -rf "$work"
    mkdir -p "$work/snapshot" "$work/logs" "$work/inputs"
    git archive "$deflate_rev" src/deflate.c src/deflate_profile.h | tar -x -C "$work/snapshot"
    snap="$work/snapshot"
    # build.sh's flags. Three builds of the same snapshot under different
    # names: the default, the plain Adler-32 loop clang otherwise skips, and
    # the default with its hash table zeroed as the safe Rust's is.
    for build in cdef cport czero; do
        extra=''
        [ "$build" = cport ] && extra=-DTERMSHOT_PORTABLE_ADLER
        [ "$build" = czero ] && extra=-DSHIM_ZEROED_TABLE
        # shellcheck disable=SC2086
        $cc -c bench/c-vs-rust/deflate_shim.c -o "$work/$build.o" -O2 $extra -I "$snap/src" \
            -Dtermshot_zlib_compress=${build}_zlib_compress \
            -Dtermshot_deflate_profile=${build}_deflate_profile \
            -DSHIM_ADLER32=${build}_adler32
    done
    ar rcs "$work/libdeflate_c.a" "$work/cdef.o" "$work/cport.o" "$work/czero.o"
    workloads "$work" "$work/logs"
    # shellcheck disable=SC2086
    rustc --edition 2021 -C opt-level=2 ${RUSTFLAGS:-} bench/c-vs-rust/deflate.rs -o "$work/deflate" \
        -L native="$PWD/$work" -l static=deflate_c
    # The deflate inputs: each workload rendered by the current CLI, its
    # PNG's IDAT inflated. The bench binary does this, so nothing needs Python.
    "$work/deflate" --inputs ./termshot "$work/logs" "$work/inputs"
    echo "== deflate.c at $deflate_rev vs deflate.rs"
    echo "C: $($cc --version | head -n 1)"
    echo "Rust: $(rustc --version), -C opt-level=2 ${RUSTFLAGS:-}"
    echo "host: $(uname -sm), load: $(uptime | sed 's/.*load average[s]*: //')"
    "$work/deflate" "$work/inputs" "$rounds"
    echo "load after: $(uptime | sed 's/.*load average[s]*: //')"
    exit 0
fi

work=target/c-vs-rust
rm -rf "$work"
mkdir -p "$work/snapshot" "$work/cells"
git archive "$rev" src third_party/stb | tar -x -C "$work/snapshot"
snap="$work/snapshot"

# The C side, with build.sh's flags.
cc -c bench/c-vs-rust/poc.c -o "$work/poc.o" -O2 -ffp-contract=off -Wno-deprecated-declarations \
    -I "$snap/src" -I "$snap/third_party/stb"
cc -c "$snap/src/deflate.c" -o "$work/deflate.o" -O2
# A static library, as in build.sh: rustc puts it before libc and libm, where
# objects passed with -C link-arg would land after them (glibc on aarch64
# needs __stack_chk_guard from ld.so, which --as-needed has dropped by then).
ar rcs "$work/libpoc_c.a" "$work/poc.o" "$work/deflate.o"

workloads "$work" "$work/cells"

poc() {
    rustc --edition 2021 -C opt-level=2 "$@" bench/c-vs-rust/poc.rs -o "$work/poc" \
        -L native="$PWD/$work" -l static=poc_c
    "$work/poc" "$work/cells" "$rounds"
}
echo "== safe Rust (opt-level=2, as build.sh), time per call, median of $rounds rounds"
poc
echo "== the same, with deflate's two hot spots unchecked (--cfg unchecked)"
poc --cfg unchecked
