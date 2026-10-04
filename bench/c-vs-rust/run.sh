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
#   bench/c-vs-rust/run.sh glyphs [rounds]    2026-10-04: draw.c's glyph
#                                             painting as of $glyphs_rev
#                                             against the shipped
#                                             src/glyphs.rs (#12 step 2c):
#                                             every fixture and generated
#                                             logs (bench/c-vs-rust/glyphs.rs)
#                                             through the CLI built at
#                                             $glyphs_rev and now, with each
#                                             font kind at several sizes,
#                                             then the glyph stages' time
#
# src/deflate.c is gone from the tree since #12 step 1, draw.c's geometry
# since step 2a, its image layers since step 2b and its glyphs since step
# 2c; the comparisons still take them from the old revisions. All
# check that the outputs are byte-identical before timing both sides in
# one process. Default 61 rounds. CC picks the C compiler of the deflate
# comparison (default cc); RUSTFLAGS adds rustc flags to its Rust side.
set -eu
cd "$(dirname "$0")/../.."
mode=poc
if [ "${1:-}" = deflate ] || [ "${1:-}" = geometry ] || [ "${1:-}" = images ] || [ "${1:-}" = glyphs ]; then
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
glyphs_rev=c0b7b02f41da34f687b86b911f743841422db1d7

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

if [ "$mode" = glyphs ]; then
    cc=${CC:-cc}
    work=target/c-vs-rust-glyphs
    rm -rf "$work"
    mkdir -p "$work/snapshot" "$work/out"
    # The CLI as of $glyphs_rev, whose draw.c painted the glyphs, and now.
    # Both use the same parser, geometry, images and compressor, so their
    # PNGs differ only if the glyph painting does.
    git archive "$glyphs_rev" build.sh src third_party | tar -x -C "$work/snapshot"
    (cd "$work/snapshot" && ./build.sh)
    ./build.sh
    old="$work/snapshot/termshot"
    rustc --edition 2021 -C opt-level=2 bench/c-vs-rust/glyphs.rs -o "$work/glyphs"
    jb=third_party/jetbrains-mono/JetBrainsMono-Regular.ttf
    cff=third_party/noto-sans-cjk/NotoSansCJKtc-Subset.otf
    vf=third_party/noto-sans-cjk-vf/NotoSansCJKtc-VF-Subset.otf
    marks=third_party/noto-sans-marks/NotoSans-Marks-Subset.ttf
    "$work/glyphs" logs "$work/logs" "$jb" "$cff" "$vf" "$marks"
    # tests/glyphs.c writes the built-in font with no outline for 'A' (an
    # empty glyph, as color bitmap fonts have), running its checks against
    # the Rust on the way.
    rust_libs=$(tests/rust_lib.sh "$work/rust.a")
    $cc -c tests/png_read.c -o "$work/png_read.o" -O2 -I third_party/stb
    # shellcheck disable=SC2086
    $cc tests/glyphs.c "$work/png_read.o" "$work/rust.a" -o "$work/glyph-checks" -O2 -Wno-deprecated-declarations \
        -I src -I third_party/stb -lm $rust_libs
    hollow="$work/hollow-A.ttf"
    "$work/glyph-checks" "$jb" "$work/out/glyphs.png" "$hollow" "$work/out/fb-reference.png"
    echo "== draw.c's glyph painting at $glyphs_rev vs src/glyphs.rs, through the CLI"
    echo "C: $($cc --version | head -n 1)"
    echo "Rust: $(rustc --version), -C opt-level=2"
    echo "host: $(uname -sm), load: $(uptime | sed 's/.*load average[s]*: //')"
    if [ -z "${TIME_ONLY:-}" ]; then
        # Each log through both CLIs, whose PNGs, exit codes and stderr (the
        # empty-glyph warning included) must be the same bytes. The fonts:
        # the built-in TrueType alone and as a file, with CFF, CFF2 and the
        # marks font as fallback and as primary, the hollow font, which
        # sends 'A' to the fallback, CFF2 instances, and the system Noto CJK
        # when there is one (starship; CJK_FONT names another copy).
        set -- "" "--font $jb --fallback-font $cff" "--font $cff --fallback-font $jb" "--font $vf" \
            "--fallback-font $vf" "--fallback-font $marks" "--font $marks --fallback-font $cff" \
            "--fallback-font $jb" "--font $hollow --fallback-font $jb" "--font $hollow --fallback-font $vf" \
            "--font $vf#wght=900 --fallback-font $marks" "--font $jb --fallback-font $vf#wght=350.5"
        system_cjk=${CJK_FONT:-/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc}
        [ -e "$system_cjk" ] && set -- "$@" "--fallback-font $system_cjk#3"
        n=0
        compare() {
            log=$1
            shift
            rm -f "$work/out/c.png" "$work/out/rust.png"
            set +e
            "$old" "$@" "$log" "$work/out/c.png" 2>"$work/out/c.err"
            c=$?
            ./termshot "$@" "$log" "$work/out/rust.png" 2>"$work/out/rust.err"
            r=$?
            set -e
            if [ "$c" -ne "$r" ] || ! cmp -s "$work/out/c.err" "$work/out/rust.err" ||
                { [ -e "$work/out/c.png" ] && ! cmp -s "$work/out/c.png" "$work/out/rust.png"; } ||
                { [ ! -e "$work/out/c.png" ] && [ -e "$work/out/rust.png" ]; }; then
                echo "FAIL $log $*: exit $c vs $r, or the PNGs or stderr differ" >&2
                diff "$work/out/c.err" "$work/out/rust.err" >&2 || true
                exit 1
            fi
            n=$((n + 1))
        }
        for log in examples/*.pty tests/fixtures/*.pty tests/vt/real/*.log tests/perf/*.pty "$work"/logs/*.pty; do
            # Every font at a common size and at 46, where FMA would show.
            for fonts in "$@"; do
                for px in 24 46; do
                    # shellcheck disable=SC2086
                    compare "$log" $fonts --px "$px"
                done
            done
            # Small, fractional and large cells, with TrueType, CFF and marks.
            for fonts in "" "--font $cff --fallback-font $jb" "--fallback-font $marks"; do
                for px in '9' '47.5' '128 --size 40x12'; do
                    # shellcheck disable=SC2086
                    compare "$log" $fonts --px $px
                done
            done
        done
        rm -f "$work/out/c.png" "$work/out/rust.png"
        echo "ok, $n renders byte-identical to the CLI at $glyphs_rev, with the same exit code and stderr"
    fi
    # The glyph stages' time in each CLI (TERMSHOT_PROFILE): bench.py's text
    # cases, glyph_ms finding and rasterizing, blend_ms blending, and
    # foreground_ms all of the text.
    echo "== glyph stages per render, median of $rounds alternating rounds (ms, Rust/C)"
    t() {
        "$work/glyphs" time "$rounds" "$old" ./termshot "$@"
    }
    t examples/reply-sent.pty --px 48
    t examples/reply-sent.pty --px 128
    t tests/perf/glyph-overflow.pty --px 24
    t tests/perf/cjk-dense.pty --px 24 --fallback-font "$cff"
    t tests/perf/cjk-dense.pty --px 24 --font "$cff"
    t tests/perf/mixed-script.pty --px 24 --fallback-font "$cff"
    t tests/fixtures/italic.pty --px 48
    t "$work/logs/marks.pty" --px 48 --fallback-font "$marks"
    t "$work/logs/cover-jetbrainsmono-regular-00-italic.pty" --px 48
    t "$work/logs/cover-notosanscjktc-subset-00-bold.pty" --px 48 --font "$cff"
    t "$work/logs/cover-notosanscjktc-vf-subset-00-upright.pty" --px 48 --font "$vf"
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
