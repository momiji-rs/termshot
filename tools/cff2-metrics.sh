#!/bin/sh
# Record the metrics HarfBuzz gives a variable font at an instance, for
# src/metrics_tests.rs (tools/cff2-metrics.rs says what and how). Needs
# libharfbuzz, found with pkg-config or in the usual library directories.
#
#   tools/cff2-metrics.sh [--variations=LIST] FONT OUT
#
# FONT and OUT are relative to the current directory. With --build-only
# the tool is built and its path printed.
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
dirs="$(pkg-config --variable=libdir harfbuzz 2>/dev/null || true) /opt/homebrew/lib /usr/local/lib
    /usr/lib/$(uname -m)-linux-gnu /usr/lib64 /usr/lib"
lib=
for dir in $dirs; do
    for name in libharfbuzz.dylib libharfbuzz.0.dylib libharfbuzz.so libharfbuzz.so.0; do
        if [ -z "$lib" ] && [ -f "$dir/$name" ]; then
            lib=$dir/$name
        fi
    done
done
[ -n "$lib" ] || { echo "no libharfbuzz found (looked in: $(echo $dirs))" >&2; exit 1; }
bin=$root/target/tools/cff2-metrics
mkdir -p "$root/target/tools"
rustc --edition 2021 -O "$root/tools/cff2-metrics.rs" -o "$bin" -C link-arg="$lib"
if [ "${1:-}" = --build-only ]; then
    echo "$bin"
    exit 0
fi
exec "$bin" "$@"
