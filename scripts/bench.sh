#!/bin/sh
# The interleaved CLI benchmark (docs/performance.md): builds scripts/bench.rs
# into target/scripts/ when it is missing or older than its source, then runs
# it with these arguments. --help lists them.
#
#   scripts/bench.sh --binary main=/tmp/main --binary branch=./termshot \
#       --reference main --output /tmp/result.json
set -eu
root=$(cd "$(dirname "$0")/.." && pwd -P)
bin=$root/target/scripts/bench
if [ ! -x "$bin" ] || [ "$root/scripts/bench.rs" -nt "$bin" ] || [ "$root/scripts/bench_common.rs" -nt "$bin" ]; then
    mkdir -p "$root/target/scripts"
    rustc --edition 2021 -O --crate-name bench "$root/scripts/bench.rs" -o "$bin"
fi
exec "$bin" "$@"
