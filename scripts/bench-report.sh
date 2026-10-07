#!/bin/sh
# Markdown tables from scripts/bench.sh results (docs/performance.md): builds
# scripts/bench-report.rs into target/scripts/ when it is missing or older
# than its source, then runs it with these arguments.
#
#   scripts/bench-report.sh A.json [B.json ...] --label branch
set -eu
root=$(cd "$(dirname "$0")/.." && pwd -P)
bin=$root/target/scripts/bench-report
if [ ! -x "$bin" ] || [ "$root/scripts/bench-report.rs" -nt "$bin" ] || [ "$root/scripts/bench_common.rs" -nt "$bin" ]; then
    mkdir -p "$root/target/scripts"
    rustc --edition 2021 -O --crate-name bench_report "$root/scripts/bench-report.rs" -o "$bin"
fi
exec "$bin" "$@"
