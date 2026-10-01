#!/usr/bin/env python3
"""Independently decode the codec harness, including CRCs and every PNG filter."""
import struct
import subprocess
import sys
import zlib
from render import decode_png

count = 0
with subprocess.Popen([sys.argv[1]], stdout=subprocess.PIPE) as process:
    with process.stdout as stream:
        while kind := stream.read(1):
            raw_size, packed_size = struct.unpack('=II', stream.read(8))
            raw = stream.read(raw_size)
            packed = stream.read(packed_size)
            assert len(raw) == raw_size and len(packed) == packed_size
            decoded = zlib.decompress(packed) if kind == b'Z' else decode_png(packed)[3]
            assert decoded == raw, (kind, count, raw_size)
            count += 1
    assert process.wait() == 0
assert count == 4976, count
print(f'{count} codec round trips passed')
