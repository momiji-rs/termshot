#!/usr/bin/env python3
"""Deflate inputs for the current C vs Rust comparison (run.sh deflate).

Renders each workload with the current termshot and writes the bytes its
compressor was given: the PNG's IDAT stream, inflated, which is the filtered
scanlines (a 0 filter byte, then RGB, per row).

  deflate_inputs.py <termshot> <poc dir> <out dir>

<poc dir> holds <name>.pty and <name>.meta ("cols rows px") from the
tests::poc_workloads helper. The 2026-10-03 compression round's image cases
(docs/performance.md, #20) come from scripts/bench.py.
"""
import struct
import subprocess
import sys
import tempfile
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'scripts'))
import bench  # noqa: E402

# Cases of the #20 round that the POC workloads don't already cover:
# blank is nearly all Adler-32, color-grid has short matches, large is wide.
BENCH_CASES = ('blank', 'color-grid', 'large')


def idat(png):
    assert png[:8] == b'\x89PNG\r\n\x1a\n', 'not a PNG'
    pos, data = 8, b''
    while pos < len(png):
        (length,) = struct.unpack('>I', png[pos:pos + 4])
        kind = png[pos + 4:pos + 8]
        if kind == b'IDAT':
            data += png[pos + 8:pos + 8 + length]
        pos += 12 + length
    return zlib.decompress(data)


binary, poc_dir, out_dir = sys.argv[1], Path(sys.argv[2]), Path(sys.argv[3])
out_dir.mkdir(parents=True, exist_ok=True)
with tempfile.TemporaryDirectory() as tmp:
    tmp = Path(tmp)
    png = tmp / 'out.png'
    for meta in sorted(poc_dir.glob('*.meta')):
        cols, rows, px = meta.read_text().split()
        log = meta.with_suffix('.pty')
        command = [binary, '--raw', '--cursor', 'none', '--px', str(int(float(px))),
                   '--size', f'{cols}x{rows}', str(log), str(png)]
        subprocess.run(command, check=True)
        raw = idat(png.read_bytes())
        (out_dir / f'{meta.stem}.raw').write_bytes(raw)
    for case in bench.legacy_workloads(tmp):
        if case.name not in BENCH_CASES:
            continue
        subprocess.run(case.command(binary, png), check=True)
        raw = idat(png.read_bytes())
        (out_dir / f'6-{case.name}.raw').write_bytes(raw)
for raw in sorted(out_dir.glob('*.raw')):
    print(f'  {raw.stem}: {raw.stat().st_size} bytes')
