#!/bin/sh
# C vs Rust (docs/c-vs-rust.md). Two comparisons, each against a snapshot of
# the C taken with git archive, so later changes to src/ don't affect them:
#
#   bench/c-vs-rust/run.sh [rounds]           2026-10-01 POC: paint and compress
#                                             cell grids with draw.c/deflate.c as
#                                             of $rev and with their Rust ports
#   bench/c-vs-rust/run.sh deflate [rounds]   2026-10-03: deflate.c as of
#                                             $deflate_rev against its Rust port
#                                             (deflate.rs), default and portable
#                                             Adler-32 C, safe and unchecked Rust
#
# Both check that the outputs are byte-identical before timing both sides in
# one process. Default 61 rounds. CC picks the C compiler of the deflate
# comparison (default cc); RUSTFLAGS adds rustc flags to its Rust side.
set -eu
cd "$(dirname "$0")/../.."
mode=poc
if [ "${1:-}" = deflate ]; then
    mode=deflate
    shift
fi
rounds=${1:-61}
# The C each comparison was ported from. Full SHAs, so CI can fetch just
# these commits into its shallow clone (ci.yml reads them from these lines).
rev=bd726a6b53957c389722f018dbabb6b093ac90bb
deflate_rev=a8a95e0bd7b8ba998688554e09680b34a40a8d4e

# Workloads, parsed by the current termshot parser. build.sh leaves the
# current C in libtermshot_c.a; link it the way test.sh does.
workloads() {
    ./build.sh
    rustc --edition 2021 --test src/main.rs -o "$1/unit" \
        -L native="$PWD" -l static=termshot_c
    TERMSHOT_POC_DIR="$2" "$1/unit" --ignored --exact tests::poc_workloads -q > /dev/null
}

if [ "$mode" = deflate ]; then
    cc=${CC:-cc}
    work=target/c-vs-rust-deflate
    rm -rf "$work"
    mkdir -p "$work/snapshot" "$work/logs" "$work/inputs"
    git archive "$deflate_rev" src/deflate.c src/deflate_profile.h | tar -x -C "$work/snapshot"
    snap="$work/snapshot"
    # build.sh's flags. Two builds of the same snapshot under different
    # names: the default, and the plain Adler-32 loop clang otherwise skips.
    for build in cdef cport; do
        extra=''
        [ "$build" = cport ] && extra=-DTERMSHOT_PORTABLE_ADLER
        # shellcheck disable=SC2086
        $cc -c bench/c-vs-rust/deflate_shim.c -o "$work/$build.o" -O2 $extra -I "$snap/src" \
            -Dtermshot_zlib_compress=${build}_zlib_compress \
            -Dtermshot_deflate_profile=${build}_deflate_profile \
            -DSHIM_ADLER32=${build}_adler32
    done
    ar rcs "$work/libdeflate_c.a" "$work/cdef.o" "$work/cport.o"
    workloads "$work" "$work/logs"
    python3 bench/c-vs-rust/deflate_inputs.py ./termshot "$work/logs" "$work/inputs" > /dev/null
    # shellcheck disable=SC2086
    rustc --edition 2021 -C opt-level=2 ${RUSTFLAGS:-} bench/c-vs-rust/deflate.rs -o "$work/deflate" \
        -L native="$PWD/$work" -l static=deflate_c
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
