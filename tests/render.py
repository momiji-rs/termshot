#!/usr/bin/env python3
"""Check decoded pixels against pre-optimization goldens; Python stdlib only."""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import random
import struct
import subprocess
import tempfile
import zlib

ROOT = Path(__file__).resolve().parents[1]


def decode_png(data):
    assert data[:8] == b'\x89PNG\r\n\x1a\n'
    pos, compressed = 8, bytearray()
    while pos < len(data):
        size = struct.unpack_from('>I', data, pos)[0]
        tag, body = data[pos+4:pos+8], data[pos+8:pos+8+size]
        assert zlib.crc32(tag + body) == struct.unpack_from('>I', data, pos+8+size)[0]
        if tag == b'IHDR':
            width, height, depth, kind, compression, filtering, interlace = struct.unpack('>IIBBBBB', body)
            assert depth == 8 and kind in (0, 2, 4, 6)
            assert (compression, filtering, interlace) == (0, 0, 0)
            channels = {0: 1, 2: 3, 4: 2, 6: 4}[kind]
        elif tag == b'IDAT':
            compressed.extend(body)
        elif tag == b'IEND':
            assert pos + 12 == len(data)
        pos += size + 12
    raw = zlib.decompress(compressed)
    stride = width * channels
    assert len(raw) == (stride + 1) * height
    pixels, previous = bytearray(), bytearray(stride)
    for y in range(height):
        start = y * (stride + 1)
        mode = raw[start]
        row = bytearray(raw[start+1:start+1+stride])
        assert mode in range(5)
        if mode:
            for x in range(stride):
                left = row[x-channels] if x >= channels else 0
                up = previous[x]
                upper_left = previous[x-channels] if x >= channels else 0
                if mode == 1:
                    prediction = left
                elif mode == 2:
                    prediction = up
                elif mode == 3:
                    prediction = (left + up) // 2
                else:
                    p = left + up - upper_left
                    a, b, c = abs(p-left), abs(p-up), abs(p-upper_left)
                    prediction = left if a <= b and a <= c else up if b <= c else upper_left
                row[x] = (row[x] + prediction) & 255
        pixels.extend(row)
        previous = row
    return width, height, channels, pixels


def fingerprint(path):
    width, height, channels, pixels = decode_png(path.read_bytes())
    assert channels in (3, 4)
    if channels == 4:
        assert all(a == 255 for a in pixels[3::4])
        rgb = bytearray(width * height * 3)
        for channel in range(3):
            rgb[channel::3] = pixels[channel::4]
        pixels = rgb
    return {'width': width, 'height': height, 'rgb_sha256': hashlib.sha256(pixels).hexdigest()}


def fixtures():
    for name in ('reply-sent', 'draft-ready'):
        yield name, (ROOT / f'examples/{name}.pty').read_bytes(), 48, 100, 30
    yield 'blank', b'', 24, 20, 8
    geometry = '─│┌┐└┘╭╮╯╰▀█'
    for px in (1, 9, 24, 47.5, 128, 255):
        yield f'geometry-{px}', (geometry + '\n\x1b[1m' + geometry).encode(), px, 12, 2
    yield 'clipping', 'jÁǺfW\n\x1b[1mÁjWWÁ\x1b[2;5Hf'.encode(), 48, 5, 2
    yield 'cache-collisions', (('AŁɁ́' * 30 + '\n') * 4).encode(), 16, 120, 4
    yield 'missing-glyphs', ('\U0010ffff\u0378 A' * 20).encode(), 16, 100, 1
    yield 'csi', b'ABC\x1b[2;3Hxyz\x1b[s\x1b[1;1HX\x1b[uY\x1b[2D!\x1b[K\x1b[1Bz\x1b[2Cq\x1b[1Aw\x1b[0m.', 20, 20, 5
    yield 'sgr', b'\x1b[1;38;2;250;10;90;48;2;5;30;70mBold\x1b[22mThin\x1b[39;49mReset\x1b[m.\x1b[38;2;8mA\x1b[38;;1mB\x1b[;mC', 24, 40, 2
    yield 'control-strings', b'A\x1b]title\x07B\x1b]title\x1b\\C\x1bPdata\x1b\\D\x1b_hidden\x1b\\E\x1b[?25lF\x1b[2JFinal\x1b[', 24, 20, 4
    rng = random.Random(42)
    log = bytearray()
    for _ in range(600):
        log.extend(f'\x1b[{rng.randrange(1,13)};{rng.randrange(1,41)}H\x1b[{rng.choice([0,1,22])};38;2;{rng.randrange(256)};{rng.randrange(256)};{rng.randrange(256)}m'.encode())
        log.extend(rng.choice('AbgjÉЖ─│╭█').encode())
    yield 'random-colors', bytes(log), 20, 40, 12


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary', type=Path, default=ROOT / 'termshot')
    p.add_argument('--write-goldens', action='store_true', help='Only use with the original reference binary')
    args = p.parse_args()
    actual = {}
    env = {k: v for k, v in os.environ.items() if k != 'TERMSHOT_PROFILE'}
    with tempfile.TemporaryDirectory() as tmp:
        src, out = Path(tmp) / 'input.pty', Path(tmp) / 'output.png'
        for name, data, px, cols, rows in fixtures():
            src.write_bytes(data)
            command = [str(args.binary.resolve()), str(src), str(out), str(ROOT / 'third_party/jetbrains-mono/JetBrainsMono-Regular.ttf'), str(px), str(cols), str(rows)]
            result = subprocess.run(command, env=env, capture_output=True, check=True)
            assert b'termshot-profile ' not in result.stderr
            actual[name] = fingerprint(out)
        if not args.write_goldens:
            result = subprocess.run(command, env={**env, 'TERMSHOT_PROFILE': '1'}, capture_output=True, text=True, check=True)
            records = [json.loads(line.split(' ', 1)[1]) for line in result.stderr.splitlines() if line.startswith('termshot-profile ')]
            assert len(records) == 2
            profile = {key: value for record in records for key, value in record.items()}
            assert all(math.isfinite(v) and v >= 0 for v in profile.values())
            assert profile['png_bytes'] == out.stat().st_size
            assert profile['input_bytes'] == src.stat().st_size
            assert profile['pixel_bytes'] == actual[name]['width'] * actual[name]['height'] * 3
            assert fingerprint(out) == actual[name], 'profiling must not change pixels'
        bad = subprocess.run([str(args.binary.resolve()), str(src), str(Path(tmp) / 'absent/out.png'), command[3]], capture_output=True)
        assert bad.returncode != 0, 'output failure must be reported'
        if not args.write_goldens and Path('/dev/full').exists():
            bad = subprocess.run([str(args.binary.resolve()), str(src), '/dev/full', command[3]], capture_output=True)
            assert bad.returncode != 0, 'short writes must be reported'
    golden = ROOT / 'tests/render-goldens.json'
    if args.write_goldens:
        golden.write_text(json.dumps(actual, indent=2) + '\n')
    else:
        expected = json.loads(golden.read_text())
        assert actual.keys() == expected.keys()
        for name in actual:
            assert actual[name] == expected[name], (name, actual[name], expected[name])
    print(f'{len(actual)} pixel fixtures passed')


if __name__ == '__main__':
    main()
