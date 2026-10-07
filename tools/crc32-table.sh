#!/bin/sh
# Regenerate src/crc32_table.h, the CRC-32 tables src/png_crc.h reads.
# test.sh checks that the committed file is what this writes.
#
#   tools/crc32-table.sh
set -eu
cd "$(dirname "$0")/.."
mkdir -p target
rustc --edition 2021 -O tools/crc32-table.rs -o target/crc32-table
target/crc32-table > src/crc32_table.h
echo "wrote src/crc32_table.h"
