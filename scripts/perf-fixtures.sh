#!/bin/sh
# Regenerate the committed benchmark logs in tests/perf/, or with --check
# fail if one differs (scripts/perf-fixtures.rs).
#
#   scripts/perf-fixtures.sh [--check]
set -eu
cd "$(dirname "$0")/.."
mkdir -p target/scripts
rustc --edition 2021 -O scripts/perf-fixtures.rs -o target/scripts/perf-fixtures
exec target/scripts/perf-fixtures "$@"
