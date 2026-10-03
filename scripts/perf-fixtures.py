#!/usr/bin/env python3
"""Regenerate the committed benchmark logs in tests/perf/ (docs/performance.md).

The output is deterministic: running this again must leave `git status` clean.
--check compares instead of writing. Python is needed only for this script and
the benchmark, not for the build or the tests.
"""
import argparse
from pathlib import Path
import struct
import sys

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'tests/perf'
JETBRAINS = ROOT / 'third_party/jetbrains-mono/JetBrainsMono-Regular.ttf'
# The 22 Han characters NotoSansCJKtc-Subset.otf maps (third_party/noto-sans-cjk/README.md).
SUBSET_HAN = '骨直角永東京台灣測試字型漢字中文繁體簡體龍鬱鑿齉'
COLS, ROWS = 100, 30
CRLF = '\r\n'


def cmap(path):
    """Every codepoint the font's Unicode cmap (format 4 or 12) maps to a glyph."""
    data = path.read_bytes()
    count = struct.unpack_from('>H', data, 4)[0]
    tables = {data[12 + 16 * i:16 + 16 * i]: struct.unpack_from('>I', data, 20 + 16 * i)[0] for i in range(count)}
    base = tables[b'cmap']
    found = set()
    for i in range(struct.unpack_from('>H', data, base + 2)[0]):
        platform, encoding, offset = struct.unpack_from('>HHI', data, base + 4 + 8 * i)
        if (platform, encoding) not in ((0, 3), (0, 4), (3, 1), (3, 10)):
            continue
        sub = base + offset
        fmt = struct.unpack_from('>H', data, sub)[0]
        if fmt == 4:
            segs = struct.unpack_from('>H', data, sub + 6)[0] // 2
            ends = struct.unpack_from(f'>{segs}H', data, sub + 14)
            starts = struct.unpack_from(f'>{segs}H', data, sub + 16 + 2 * segs)
            deltas = struct.unpack_from(f'>{segs}h', data, sub + 16 + 4 * segs)
            range_at = sub + 16 + 6 * segs
            ranges = struct.unpack_from(f'>{segs}H', data, range_at)
            for k in range(segs):
                for cp in range(starts[k], ends[k] + 1):
                    if cp == 0xFFFF:
                        continue
                    if ranges[k] == 0:
                        glyph = (cp + deltas[k]) & 0xFFFF
                    else:
                        at = range_at + 2 * k + ranges[k] + 2 * (cp - starts[k])
                        glyph = struct.unpack_from('>H', data, at)[0]
                        glyph = (glyph + deltas[k]) & 0xFFFF if glyph else 0
                    if glyph:
                        found.add(cp)
        elif fmt == 12:
            groups = struct.unpack_from('>I', data, sub + 12)[0]
            for k in range(groups):
                first, last, _ = struct.unpack_from('>III', data, sub + 16 + 12 * k)
                found.update(range(first, last + 1))
    return found


def glyph_overflow():
    """More distinct drawn glyphs than draw.c's 1,024 cache slots, repeated.

    Every codepoint is one JetBrains Mono maps, narrow, printable, and not
    U+2500-U+259F (painted as geometry, so never cached), in the BMP (the few
    beyond it map to empty glyphs). The screen holds the
    list in order, then again from the start, so a second use finds its slot taken by a later
    codepoint with the same key modulo 1,024."""
    import unicodedata
    cps = sorted(cp for cp in cmap(JETBRAINS)
                 if 0x20 < cp <= 0xFFFF and not 0x2500 <= cp <= 0x259F and not 0x7F <= cp < 0xA0
                 and unicodedata.category(chr(cp))[0] in 'LNPS'
                 and unicodedata.east_asian_width(chr(cp)) not in 'WF'
                 and unicodedata.combining(chr(cp)) == 0)
    if len(cps) <= 1024:
        sys.exit(f'only {len(cps)} usable codepoints, want more than 1,024')
    text = ''.join(chr(cps[i % len(cps)]) for i in range(COLS * ROWS))
    rows = [text[r * COLS:(r + 1) * COLS] for r in range(ROWS)]
    return CRLF.join(rows), len(cps)


def cjk_dense():
    """A full screen of the subset's Han characters with ASCII labels: every
    Han cell is a fallback glyph when the subset (or full Noto CJK) is the
    fallback, and a missing-glyph box without one."""
    rows = []
    for r in range(ROWS):
        label = f'{r:02d} '
        han = ''.join(SUBSET_HAN[(r * 7 + k) % len(SUBSET_HAN)] for k in range((COLS - len(label)) // 2))
        rows.append(f'\x1b[3{1 + r % 7}m{label}\x1b[0m{han}')
    return CRLF.join(rows)


def cjk_overflow():
    """1,500 distinct Han characters (U+4E00 onward), more than the cache's
    1,024 slots, for a full CJK fallback font; all are in Noto Sans CJK TC."""
    per_row = COLS // 2
    return CRLF.join(''.join(chr(0x4E00 + r * per_row + k) for k in range(per_row)) for r in range(ROWS))


def mixed_script():
    """What a CI log in several languages looks like: a prompt, ASCII output,
    Latin, Greek and Cyrillic (in JetBrains Mono), Han (from the fallback),
    Hiragana, Hangul and an emoji (in neither font: missing-glyph boxes), box
    drawing, colours, bold and italic."""
    lines = [
        '\x1b[1;32muser@host\x1b[0m:\x1b[1;34m~/src/termshot\x1b[0m$ make test LANG=zh_TW.UTF-8',
        '\x1b[2m[build]\x1b[0m cc -O2 -ffp-contract=off -c src/draw.c -o draw.o',
        'Résumé naïve façade — coöperate über Ærøskøbing; Øresund ﬁnal “quotes” ‘ok’',
        'Ελληνικά: Γειά σου κόσμε · Русский: Привет, мир · Українська: Добрий день',
        '\x1b[33m警告\x1b[0m：\x1b[3m測試字型\x1b[0m 東京 台灣 漢字 中文 繁體 簡體 — 骨直角永 龍鬱鑿齉',
        '日本語のテキスト ひらがな カタカナ · 한국어 텍스트 · emoji 🙂 🚀 ✅ (not in either font)',
        '╭──────────────┬──────────────────────────────╮',
        '│ \x1b[1mstage\x1b[0m        │ \x1b[1mresult\x1b[0m                       │',
        '├──────────────┼──────────────────────────────┤',
        '│ parse        │ \x1b[32m✓ ok\x1b[0m 中文 測試                 │',
        '│ render       │ \x1b[31m✗ fail\x1b[0m Ελληνικά Привет         │',
        '╰──────────────┴──────────────────────────────╯',
        '\x1b[38;2;255;128;0mtruecolor\x1b[0m \x1b[48;5;24m 256-colour \x1b[0m ░▒▓█ ▁▂▃▄▅▆▇█ ←↑→↓ ∀∂∈∑√∞≠≤≥',
    ]
    rows = [lines[r % len(lines)] for r in range(ROWS)]
    return CRLF.join(rows)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--check', action='store_true', help='fail if a committed log differs')
    a = p.parse_args()
    overflow, distinct = glyph_overflow()
    logs = {'glyph-overflow.pty': overflow, 'cjk-dense.pty': cjk_dense(),
            'cjk-overflow.pty': cjk_overflow(), 'mixed-script.pty': mixed_script()}
    OUT.mkdir(parents=True, exist_ok=True)
    stale = []
    for name, text in logs.items():
        path = OUT / name
        data = text.encode()
        if a.check:
            if not path.exists() or path.read_bytes() != data:
                stale.append(name)
        else:
            path.write_bytes(data)
    if stale:
        sys.exit('stale: ' + ', '.join(stale))
    print(f'glyph-overflow: {distinct} distinct codepoints over {COLS * ROWS} cells')


if __name__ == '__main__':
    main()
