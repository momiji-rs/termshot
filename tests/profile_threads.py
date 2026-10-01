#!/usr/bin/env python3
"""Check profile records while the Rust test renders eight frames concurrently."""
import json
import math
import os
from pathlib import Path
import subprocess
import sys

root = Path(__file__).resolve().parents[1]
(root / 'target/test').mkdir(parents=True, exist_ok=True)
result = subprocess.run(
    [str(Path(sys.argv[1]).resolve()), '--exact', 'draw_tests::draw_png_is_reentrant', '--nocapture'],
    cwd=root, env={**os.environ, 'TERMSHOT_PROFILE': '1'},
    capture_output=True, text=True, check=True,
)
assert '1 passed' in result.stdout, result.stdout
records = [json.loads(line.split(' ', 1)[1]) for line in result.stderr.splitlines()
           if line.startswith('termshot-profile ')]
assert len(records) == 8, result.stderr
for record in records:
    assert all(math.isfinite(value) and value >= 0 for value in record.values()), record
    assert record['png_bytes'] == (root / 'target/test/thread-0.png').stat().st_size
    assert record['png_filter_ms'] + record['png_deflate_ms'] + record['png_pack_ms'] <= record['png_encode_ms'] + 0.00001
    for key in ('glyph_rasterizations', 'glyph_cache_hits', 'pixel_bytes'):
        assert record[key] == records[0][key], (key, records)
print('8 concurrent renders produced consistent pixels and independent profile records')
