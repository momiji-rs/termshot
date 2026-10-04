#!/usr/bin/env python3
"""Dependency-free, interleaved CLI benchmark. JSON is the durable result.

Every round runs each binary twice, without and with TERMSHOT_PROFILE, in a
shuffled order, so wall/CPU samples, profile records and the profiling
overhead all come from the same rounds. docs/performance.md describes the
workloads, the timer boundaries and how to read the result.
"""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import random
import re
import resource
import socket
import statistics
import subprocess
import tempfile
import time
from datetime import datetime, timezone

ROOT = Path(__file__).resolve().parents[1]
FONT = ROOT / 'third_party/jetbrains-mono/JetBrainsMono-Regular.ttf'
CJK_SUBSET = ROOT / 'third_party/noto-sans-cjk/NotoSansCJKtc-Subset.otf'
PERF = ROOT / 'tests/perf'
# Arch's noto-fonts-cjk 20240730-1; face 3 is Noto Sans CJK TC Regular.
SYSTEM_CJK = Path('/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc')
CJK_FACE = '3'


class Case:
    """One workload. legacy cases use the positional CLI with an explicit font
    (as every earlier report did); the others use options, so the built-in
    font and --fallback-font can be measured."""

    def __init__(self, name, src, px, cols, rows, fonts=None, legacy=False, group='legacy', checks=(), text=False):
        self.name, self.src, self.px, self.cols, self.rows = name, Path(src), px, cols, rows
        self.fonts = fonts or []
        self.legacy, self.group, self.checks, self.text = legacy, group, checks, text

    def command(self, binary, out):
        if self.text:
            return [binary, '--size', f'{self.cols}x{self.rows}', '--text', str(out), str(self.src)]
        if self.legacy:
            return [binary, str(self.src), str(out), str(FONT), str(self.px), str(self.cols), str(self.rows)]
        return [binary, *self.fonts, '--px', str(self.px), '--size', f'{self.cols}x{self.rows}', str(self.src), str(out)]

    def font_files(self):
        if self.legacy:
            return {'font': str(FONT)}
        files = {'font': 'built-in JetBrains Mono'}
        for flag, value in zip(self.fonts[::2], self.fonts[1::2]):
            files[flag.lstrip('-').replace('-font', '')] = value
        return files


def legacy_workloads(directory):
    cases = [Case(name, ROOT / f'examples/{name}.pty', 48, 100, 30, legacy=True)
             for name in ('reply-sent', 'draft-ready')]
    cases.extend(Case(f'reply-{px}px', ROOT / 'examples/reply-sent.pty', px, 100, 30, legacy=True) for px in (24, 128))
    cases.extend(Case(f'real-{name}', ROOT / f'tests/vt/real/{name}.log', 48, 80, 24, legacy=True)
                 for name in ('shell', 'less', 'vi'))
    rng = random.Random(13)
    colors = ''.join(
        f'\x1b[{row + 1};{col + 1}H\x1b[38;2;{rng.randrange(256)};{rng.randrange(256)};{rng.randrange(256)};48;2;{rng.randrange(256)};{rng.randrange(256)};{rng.randrange(256)}m{chr(rng.randrange(33, 127))}'
        for row in range(30) for col in range(100)
    ).encode()
    generated = {
        'blank': (b'', 48, 100, 30),
        'color-grid': (colors, 24, 100, 30),
        'ascii-overflow': (b'x' * 4_000_000, 24, 100, 30),
        'rounded-boxes': ('\r\n'.join(['╭╮╰╯' * 25] * 30).encode(), 48, 100, 30),
        'dense': ('\r\n'.join(['The quick brown fox 0123456789! @#$% ' * 3] * 30).encode(), 48, 100, 30),
        'ansi-replay': ((ROOT / 'examples/reply-sent.pty').read_bytes() * 250, 48, 100, 30),
        'large': ('\r\n'.join(['Terminal benchmark 0123456789 ' * 9] * 80).encode(), 48, 240, 80),
        'unicode': ('\r\n'.join([''.join(chr(0x100 + (r * 100 + c) % 800) for c in range(100)) for r in range(30)]).encode(), 24, 100, 30),
    }
    for name, (data, px, cols, rows) in generated.items():
        path = directory / f'{name}.pty'
        path.write_bytes(data)
        cases.append(Case(name, path, px, cols, rows, legacy=True))
    return cases


def kitty_image(width, height, cols, rows, z, seed):
    """A kitty direct transmission (a=T, f=32) of a width x height RGBA
    gradient with some transparency, scaled to cols x rows cells at z, sent in
    the 4096-byte chunks the protocol allows."""
    import base64
    rng = random.Random(seed)
    pixels = bytearray()
    for y in range(height):
        for x in range(width):
            pixels += bytes((x * 255 // width, y * 255 // height, rng.randrange(256),
                             255 if (x // 16 + y // 16) % 3 else 128))
    data = base64.b64encode(bytes(pixels))
    chunks = [data[i:i + 4096] for i in range(0, len(data), 4096)]
    out = b''
    for i, chunk in enumerate(chunks):
        more = int(i + 1 < len(chunks))
        keys = f'a=T,f=32,s={width},v={height},c={cols},r={rows},z={z},q=2,m={more}' if i == 0 else f'm={more}'
        out += b'\x1b_G' + keys.encode() + b';' + chunk + b'\x1b\\'
    return out


def draw_workloads(directory):
    """Painting and geometry (#22): box, block and rounded grids, the same at
    other sizes, every geometry character at once, large sparse and colored
    screens, and an image below, under and over text."""
    crlf = '\r\n'
    # Every row fits its screen exactly, so none wraps.
    table = []
    for row in range(30):
        kind = row % 3
        if row % 6 == 0:
            table.append('┌' + '──────┬' * 12 + '──────┐' + '━━━┳━━━┓')
        elif row % 6 == 5:
            table.append('└' + '──────┴' * 12 + '──────┘' + '━━━┻━━━┛')
        elif kind == 1:
            table.append('│' + ' cell │' * 13 + '═══╬═══╣')
        else:
            table.append('├' + '──────┼' * 12 + '──────┤' + '║  ╠═══╣')
    assert all(len(line) == 100 for line in table)
    blocks = '█▀▄▌▐░▒▓▖▗▘▝▚▞▙▟▁▂▃▅▆▇▏▎▍▋▊▉▔▕'
    rng = random.Random(22)
    block_grid = crlf.join(
        ''.join(f'\x1b[38;5;{rng.randrange(256)}m{blocks[(r * 7 + c) % len(blocks)]}' for c in range(100))
        for r in range(30))
    every = [chr(cp) for cp in range(0x2500, 0x25a0)]
    geometry_all = crlf.join(
        ''.join(('\x1b[1m' if (r + c) % 2 else '\x1b[22m') + every[(r * 240 + c) % len(every)] for c in range(240))
        for r in range(80))
    panes = []
    for r in range(30):
        band = r % 10
        if band == 0:
            panes.append('╭' + '─' * 23 + '╮' + ('╭' + '─' * 23 + '╮') * 3)
        elif band == 9:
            panes.append('╰' + '─' * 23 + '╯' + ('╰' + '─' * 23 + '╯') * 3)
        else:
            panes.append(('│ ' + f'item {r:02} value {r * 37 % 1000:4}'.ljust(21) + ' │') * 4)
    assert all(len(line) == 100 for line in panes)
    rounded = crlf.join(['╭╮╰╯' * 25] * 30)
    sparse = crlf.join(['$ termshot --px 48 --size 240x80 large.log out.png', 'done in 42 ms', '$ '] + [''] * 77)
    colored = crlf.join(
        ''.join(f'\x1b[48;5;{(r * 3 + c // 10) % 216 + 16}m' + 'Benchmark ' for c in range(0, 240, 10))
        for r in range(80))
    # Every third row on a background of its own, which hides an image below
    # the backgrounds.
    text = crlf.join((f'\x1b[48;5;{17 + r}m' if r % 3 == 0 else '') + ('The quick brown fox 0123456789! @#$% ' * 3)[:100]
                     + '\x1b[0m' for r in range(30))
    image = lambda z: ('\x1b[H'.encode() + kitty_image(128, 128, 60, 20, z, 22) + b'\x1b[H' + text.encode())
    generated = {
        'box-grid': (crlf.join(table).encode(), 48, 100, 30),
        'block-grid': (block_grid.encode(), 48, 100, 30),
        'rounded-panes': (crlf.join(panes).encode(), 48, 100, 30),
        'rounded-24px': (rounded.encode(), 24, 100, 30),
        'rounded-128px': (rounded.encode(), 128, 100, 30),
        'geometry-all': (geometry_all.encode(), 48, 240, 80),
        'large-sparse': (sparse.encode(), 48, 240, 80),
        'large-color': (colored.encode(), 48, 240, 80),
        'image-below': (image(-1073741825), 48, 100, 30),
        'image-under': (image(-1), 48, 100, 30),
        'image-over': (image(1), 48, 100, 30),
    }
    cases = []
    for name, (data, px, cols, rows) in generated.items():
        path = directory / f'{name}.pty'
        path.write_bytes(data)
        cases.append(Case(name, path, px, cols, rows, group='draw'))
    return cases


PARSER_CASES = ('dense-sgr', 'cursor-moves', 'scrolling', 'mixed-unicode', 'thai-combining')


def parser_logs():
    """Long logs for the parser (#21), each about 4-5 MB so parsing is a
    large part of the run. Deterministic: the same bytes on every host. The
    logs share one random sequence, so they are made together, in order."""
    rng = random.Random(21)
    pick = rng.randrange

    def dense_sgr():
        # Every character in its own SGR: palette, 256-colour, truecolour in
        # both separators, attributes on and off, resets.
        forms = [
            lambda: f'{30 + pick(8)};{40 + pick(8)}',
            lambda: f'{90 + pick(8)};{100 + pick(8)}',
            lambda: f'38;5;{pick(256)};48;5;{pick(256)}',
            lambda: f'38;2;{pick(256)};{pick(256)};{pick(256)}',
            lambda: f'48;2;{pick(256)};{pick(256)};{pick(256)}',
            lambda: f'38:2::{pick(256)}:{pick(256)}:{pick(256)}',
            lambda: f'1;3;4;{30 + pick(8)}',
            lambda: '22;23;24;39;49',
            lambda: f'4:{pick(4)};7',
            lambda: '0',
            lambda: '',
        ]
        line = lambda: ''.join(f'\x1b[{rng.choice(forms)()}m{chr(pick(33, 127))}' for _ in range(100))
        return '\r\n'.join(line() for _ in range(3_000)).encode()

    def cursor_moves():
        # Absolute and relative moves, each followed by a character, with the
        # C0 moves (CR, BS, HT) in between.
        def move():
            k = pick(13)
            if k < 4:
                return f'\x1b[{pick(1, 31)};{pick(1, 101)}H'
            return [f'\x1b[{pick(1, 9)}A', f'\x1b[{pick(1, 9)}B', f'\x1b[{pick(1, 30)}C', f'\x1b[{pick(1, 30)}D',
                    f'\x1b[{pick(1, 101)}G', f'\x1b[{pick(1, 31)}d', '\r', '\b\b', '\t'][k - 4]
        return ''.join(move() + chr(pick(33, 127)) for _ in range(700_000)).encode()

    def scrolling():
        # Lines that scroll the whole screen, then a scroll region with IND,
        # RI, IL, DL, SU and SD, then the margins reset.
        parts = []
        for block in range(2_000):
            parts += [f'line {block}.{n} ' + 'abcdefghij' * 4 + '\r\n' for n in range(30)]
            parts.append(f'\x1b[{pick(2, 10)};{pick(15, 30)}r\x1b[{pick(10, 15)};1H')
            parts += ['scrolled text\x1bD', '\x1bM', '\x1b[2L', '\x1b[3M', '\x1b[2S', '\x1b[T', 'region\n' * 8]
            parts.append('\x1b[r')
        return ''.join(parts).encode()

    def mixed_unicode():
        # ASCII words between Latin-1, Greek, Cyrillic, box drawing, CJK and
        # a rare emoji, so UTF-8 decoding and the width lookup take turns.
        words = ['terminal', 'output', 'naïve', 'café', 'Ελληνικά', 'Кириллица', '│', '├──', '漢字', '日本語',
                 '한국어', '→', '✓', '🙂', '42', 'ß', 'µs']
        line = lambda: ' '.join(rng.choice(words) for _ in range(14))
        return '\r\n'.join(line() for _ in range(40_000)).encode()

    def thai_combining():
        # Thai syllables with above and below vowels and tone marks (kept as
        # marks: no precomposed form), Latin with a composing acute, and
        # Hebrew with points.
        consonants = [chr(c) for c in range(0x0e01, 0x0e2f)]
        above = ['ั', 'ิ', 'ี', 'ึ', 'ื', '็']
        tones = ['่', '้', '๊', '๋']
        below = ['ุ', 'ู']

        def syllable():
            s = rng.choice(consonants)
            k = pick(4)
            if k == 0:
                s += rng.choice(above) + rng.choice(tones)
            elif k == 1:
                s += rng.choice(below) + rng.choice(tones)
            elif k == 2:
                s += rng.choice(above)
            return s
        words = lambda: ''.join(syllable() for _ in range(pick(2, 6)))
        line = lambda: ' '.join([words() for _ in range(10)] + ['café', 'שָׁלוֹם'])
        return '\r\n'.join(line() for _ in range(16_000)).encode()

    return {'dense-sgr': dense_sgr(), 'cursor-moves': cursor_moves(), 'scrolling': scrolling(),
            'mixed-unicode': mixed_unicode(), 'thai-combining': thai_combining()}


def parser_workloads(directory, wanted=None):
    """The parser matrix (#21), at 24 px. With ansi-replay and ascii-overflow
    from the legacy suite it covers each kind of input the parser handles.
    Making the logs takes seconds, so when --case names none of them
    (wanted), they are not made."""
    if wanted and not set(wanted) & set(PARSER_CASES):
        return []
    cases = []
    logs = parser_logs()
    assert tuple(logs) == PARSER_CASES
    for name, data in logs.items():
        path = directory / f'{name}.pty'
        path.write_bytes(data)
        cases.append(Case(name, path, 24, 100, 30, legacy=True, group='parser'))
    return cases


TEXT_CASES = ('text-ansi-replay', 'text-ascii-overflow', 'text-dense-sgr', 'text-mixed-unicode',
              'text-reply-sent', 'text-kitty', 'text-sixel')


def text_workloads(directory, wanted=None):
    """Text-only runs (--text, no PNG). Fonts are read only when the log has
    an image that needs cell metrics, so most of these read none; text-kitty
    (an a=T upload) and text-sixel do. font_builtin says which happened.
    The two parser logs are made only when --case names one of them (or none)."""
    no_font = (('font_builtin', '==', 0), ('font_bytes', '==', 0))
    font = (('font_builtin', '==', 1),)
    logs = {
        'text-ansi-replay': ((ROOT / 'examples/reply-sent.pty').read_bytes() * 250, no_font),
        'text-ascii-overflow': (b'x' * 4_000_000, no_font),
        'text-reply-sent': ((ROOT / 'examples/reply-sent.pty').read_bytes(), no_font),
        'text-kitty': (b'\x1b[H' + kitty_image(128, 128, 60, 20, 1, 22) + b'\x1b[H' + b'text\r\n' * 20, font),
        'text-sixel': ((ROOT / 'tests/fixtures/sixel-magick.pty').read_bytes(), font),
    }
    if not wanted or set(wanted) & {'text-dense-sgr', 'text-mixed-unicode'}:
        parser = parser_logs()
        logs['text-dense-sgr'] = (parser['dense-sgr'], no_font)
        logs['text-mixed-unicode'] = (parser['mixed-unicode'], no_font)
    cases = []
    for name in TEXT_CASES:
        if name not in logs:
            continue
        data, checks = logs[name]
        path = directory / f'{name}.pty'
        path.write_bytes(data)
        cases.append(Case(name, path, 48, 100, 30, group='text', checks=checks, text=True))
    return cases


def font_workloads(cjk):
    """The font-path matrix (#19). checks are (counter, op, value) on the
    profile record, verified on every profiled run of a binary that has it."""
    sub = ['--fallback-font', str(CJK_SUBSET)]
    full = ['--fallback-font', f'{cjk}#{CJK_FACE}'] if cjk else None
    reply = ROOT / 'examples/reply-sent.pty'
    fallback_used = (('fallback_rasterizations', '>', 0), ('fallback_lookups', '>', 0))
    cases = [
        Case('font-builtin', reply, 48, 100, 30, [], group='font', checks=(('font_builtin', '==', 1),)),
        Case('font-file', reply, 48, 100, 30, ['--font', str(FONT)], group='font', checks=(('font_builtin', '==', 0),)),
        Case('cjk-none', PERF / 'cjk-dense.pty', 24, 100, 30, [], group='fallback',
             checks=(('fallback_lookups', '==', 0), ('glyph_missing', '>', 0))),
        Case('cjk-subset', PERF / 'cjk-dense.pty', 24, 100, 30, sub, group='fallback',
             checks=fallback_used + (('glyph_missing', '==', 0),)),
        Case('cjk-cff-primary', PERF / 'cjk-dense.pty', 24, 100, 30, ['--font', str(CJK_SUBSET)], group='fallback',
             checks=(('fallback_lookups', '==', 0), ('glyph_missing', '==', 0))),
        Case('mixed-subset', PERF / 'mixed-script.pty', 24, 100, 30, sub, group='mixed', checks=fallback_used),
        Case('glyph-overflow', PERF / 'glyph-overflow.pty', 24, 100, 30, [], group='cache',
             checks=(('glyph_cache_evictions', '>', 0), ('glyph_missing', '==', 0))),
    ]
    if full:
        cases += [
            Case('cjk-full', PERF / 'cjk-dense.pty', 24, 100, 30, full, group='fallback',
                 checks=fallback_used + (('glyph_missing', '==', 0),)),
            Case('mixed-full', PERF / 'mixed-script.pty', 24, 100, 30, full, group='mixed', checks=fallback_used),
            Case('cjk-overflow-full', PERF / 'cjk-overflow.pty', 24, 100, 30, full, group='cache',
                 checks=fallback_used + (('glyph_cache_evictions', '>', 0), ('glyph_missing', '==', 0))),
        ]
    return cases


def summary(values):
    values = sorted(values)
    return {'mean': statistics.mean(values), 'median': statistics.median(values),
            'p95': values[math.ceil(len(values) * .95) - 1],
            'min': values[0], 'max': values[-1]}


def paired_ratio(numerator, denominator):
    """Ratio for each interleaved round; bootstrap whole pairs, without trimming."""
    ratios = [a / b for a, b in zip(numerator, denominator)]
    rng = random.Random(42)
    estimates = sorted(statistics.median(rng.choices(ratios, k=len(ratios)))
                       for _ in range(2000))
    return {'median': statistics.median(ratios),
            'bootstrap_95pct': [estimates[49], estimates[1949]]}


def relative(path):
    """The path from the repository root, or just the name of a generated file.
    (Path.is_relative_to needs Python 3.9.)"""
    try:
        return str(path.relative_to(ROOT))
    except ValueError:
        return path.name


def sha256(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def first_line(command):
    try:
        return subprocess.check_output(command, text=True, stderr=subprocess.STDOUT).splitlines()[0]
    except (OSError, subprocess.CalledProcessError, IndexError):
        return None


def machine():
    info = {'platform': platform.platform(), 'machine': platform.machine(), 'hostname': socket.gethostname()}
    if platform.system() == 'Darwin':
        info['cpu'] = subprocess.check_output(['sysctl', '-n', 'machdep.cpu.brand_string'], text=True).strip()
        info['ram_bytes'] = int(subprocess.check_output(['sysctl', '-n', 'hw.memsize'], text=True))
        info['os'] = first_line(['sw_vers', '-productVersion'])
    else:
        text = Path('/proc/cpuinfo').read_text()
        model = re.search(r'model name\s*:\s*(.+)', text)
        info['cpu'] = model[1].strip() if model else None
        info['ram_bytes'] = os.sysconf('SC_PAGE_SIZE') * os.sysconf('SC_PHYS_PAGES')
        governor = Path('/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor')
        info['cpufreq_governor'] = governor.read_text().strip() if governor.exists() else None
        info['libc'] = ' '.join(platform.libc_ver())
    info['logical_cpus'] = os.cpu_count()
    return info


def source():
    def git(*args):
        try:
            return subprocess.check_output(['git', *args], cwd=ROOT, text=True, stderr=subprocess.DEVNULL).strip()
        except (OSError, subprocess.CalledProcessError):
            return None
    # The hash of the files the build reads, so a tarball without .git is identified too.
    digest = hashlib.sha256()
    for path in sorted([ROOT / 'build.sh', *(ROOT / 'src').glob('*'), *(ROOT / 'third_party').rglob('*')]):
        if path.is_file():
            digest.update(str(path.relative_to(ROOT)).encode() + b'\0' + path.read_bytes())
    return {'git_head': git('rev-parse', 'HEAD'), 'git_dirty': bool(git('status', '--porcelain', '--untracked-files=no')),
            'build_inputs_sha256': digest.hexdigest(), 'build_sh_sha256': sha256(ROOT / 'build.sh')}


def profile_records(stderr):
    """The termshot-profile records in a run's stderr, merged."""
    profile = {}
    for line in stderr.decode(errors='replace').splitlines():
        if line.startswith('termshot-profile '):
            profile.update(json.loads(line.split(' ', 1)[1]))
    return profile


def check(case, profile):
    """The counters that show a case exercised the path it is named for. A
    missing counter fails: only --unchecked exempts a binary."""
    if not profile:
        return ['no termshot-profile records']
    failed = []
    for key, op, value in case.checks:
        if key not in profile:
            failed.append(f'no {key} in the profile')
            continue
        have = profile[key]
        if not {'>': have > value, '==': have == value}[op]:
            failed.append(f'{key} {op} {value} (got {have})')
    return failed


def drop_caches():
    """Linux only: write back and drop the page cache (needs passwordless sudo)."""
    subprocess.run(['sync'], check=True)
    subprocess.run(['sudo', '-n', 'sh', '-c', 'echo 3 > /proc/sys/vm/drop_caches'], check=True)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary', action='append', required=True, help='label=/path/to/binary (repeatable)')
    p.add_argument('--describe', action='append', default=[], help='label=text, e.g. the revision a binary is built from')
    p.add_argument('--runs', type=int, default=10)
    p.add_argument('--warmups', type=int, default=2)
    p.add_argument('--output', type=Path, required=True)
    p.add_argument('--case', action='append')
    p.add_argument('--suite', choices=('all', 'legacy', 'fonts', 'draw', 'parser', 'text'), default='all')
    p.add_argument('--cjk-font', type=Path, default=SYSTEM_CJK if SYSTEM_CJK.exists() else None,
                   help=f'full NotoSansCJK-Regular.ttc for the *-full cases (default: {SYSTEM_CJK} if present)')
    p.add_argument('--memory-runs', type=int, default=0, help='separate peak-RSS runs using /usr/bin/time')
    p.add_argument('--cold-runs', type=int, default=0, help='Linux: runs after dropping the page cache (sudo -n)')
    p.add_argument('--cold-case', action='append', help='cases for --cold-runs (default: all selected)')
    p.add_argument('--verify-identical', action='store_true', help='require byte-identical outputs (PNG or text) across binaries')
    p.add_argument('--reference', help='binary label for paired speedup estimates')
    p.add_argument('--unchecked', action='append', default=[],
                   help='binary label exempt from the path checks, for one that predates the counters (repeatable)')
    p.add_argument('--seed', type=int, default=0, help='seed for interleaved execution order')
    args = p.parse_args()
    if args.runs < 1 or args.warmups < 0 or args.memory_runs < 0 or args.cold_runs < 0:
        p.error('runs must be positive; warmups, memory-runs and cold-runs must be nonnegative')
    binaries = {label: str(Path(path).resolve()) for label, path in (item.split('=', 1) for item in args.binary)}
    if args.reference and args.reference not in binaries:
        p.error('reference must name one of the binary labels')
    if set(args.unchecked) - set(binaries):
        p.error('--unchecked must name binary labels')
    if args.cold_case and not args.cold_runs:
        p.error('--cold-case needs --cold-runs')
    if args.cold_runs and platform.system() != 'Linux':
        p.error('--cold-runs drops the Linux page cache; it is not supported here')
    if args.cjk_font and not args.cjk_font.exists():
        p.error(f'{args.cjk_font}: no such file')
    report = {'machine': machine(), 'source': source(),
              'timestamp_utc': datetime.now(timezone.utc).isoformat(),
              'binaries': binaries,
              'binary_sha256': {label: sha256(path) for label, path in binaries.items()},
              'describe': dict(item.split('=', 1) for item in args.describe),
              'toolchain': {tool: first_line([tool, '--version']) for tool in ('rustc', 'cc', 'python3')},
              'build_flags': {'c': '-O2 -ffp-contract=off (draw.c); -O2 (image.c)',
                              'rust': '--edition 2021 -C opt-level=2'},
              'runs': args.runs, 'warmups': args.warmups, 'memory_runs': args.memory_runs,
              'cold_runs': args.cold_runs, 'verify_identical': args.verify_identical,
              'seed': args.seed, 'reference': args.reference, 'unchecked': args.unchecked,
              'cache_condition': 'warm: page cache populated by warmups; output files rewritten, no fsync',
              'fonts': {}, 'load_average_start': os.getloadavg(), 'cases': {}, 'cold': {}}
    for path in [FONT, CJK_SUBSET] + ([args.cjk_font] if args.cjk_font else []):
        report['fonts'][str(path)] = {'bytes': path.stat().st_size, 'sha256': sha256(path)}
    env = dict(os.environ)
    env.pop('TERMSHOT_PROFILE', None)
    profiled_env = {**env, 'TERMSHOT_PROFILE': '1'}
    rng = random.Random(args.seed)
    with tempfile.TemporaryDirectory(prefix='termshot-bench-') as tmp:
        directory = Path(tmp)
        cases = []
        if args.suite in ('all', 'fonts'):
            cases += font_workloads(args.cjk_font)
        if args.suite in ('all', 'legacy'):
            cases += legacy_workloads(directory)
        if args.suite in ('all', 'draw'):
            cases += draw_workloads(directory)
        if args.suite in ('all', 'parser'):
            cases += parser_workloads(directory, args.case)
        if args.suite in ('all', 'text'):
            cases += text_workloads(directory, args.case)
        if args.case:
            unknown = set(args.case) - {case.name for case in cases}
            if unknown:
                p.error('unknown case: ' + ', '.join(sorted(unknown)))
            cases = [case for case in cases if case.name in args.case]
        if args.cold_case:
            unknown = set(args.cold_case) - {case.name for case in cases}
            if unknown:
                p.error('--cold-case names a case not selected: ' + ', '.join(sorted(unknown)))
        for case in cases:
            out = {label: directory / f'{label}.{"txt" if case.text else "png"}' for label in binaries}
            plain = {label: [] for label in binaries}
            cpu = {label: [] for label in binaries}
            profiled_wall = {label: [] for label in binaries}
            profiles = {label: [] for label in binaries}
            failures = []
            seen = {label: set() for label in binaries}
            for run in range(-args.warmups, args.runs):
                order = [(label, profiled) for label in binaries for profiled in (False, True)]
                rng.shuffle(order)
                for label, profiled in order:
                    command = case.command(binaries[label], out[label])
                    before = resource.getrusage(resource.RUSAGE_CHILDREN)
                    start = time.perf_counter_ns()
                    # Plain and profiled runs are spawned alike; parsing the
                    # profile happens after the clock stops.
                    result = subprocess.run(command, env=profiled_env if profiled else env,
                                            stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, check=True)
                    elapsed = round((time.perf_counter_ns() - start) / 1e6, 4)
                    after = resource.getrusage(resource.RUSAGE_CHILDREN)
                    profile = profile_records(result.stderr) if profiled else None
                    # Every run's output, outside the timed interval.
                    seen[label].add(sha256(out[label]))
                    if run < 0:
                        continue
                    if profiled:
                        profiled_wall[label].append(elapsed)
                        profiles[label].append(profile)
                        if label not in args.unchecked:
                            failures += [f'{label}: {failure}' for failure in check(case, profile)]
                    else:
                        plain[label].append(elapsed)
                        cpu[label].append(round(1000 * (after.ru_utime + after.ru_stime - before.ru_utime - before.ru_stime), 4))
            if failures:
                raise RuntimeError(f'{case.name} did not exercise its path: {sorted(set(failures))}')
            rss = {label: [] for label in binaries}
            darwin = platform.system() == 'Darwin'
            for _ in range(args.memory_runs):
                labels = list(binaries)
                rng.shuffle(labels)
                for label in labels:
                    command = case.command(binaries[label], out[label])
                    result = subprocess.run(['/usr/bin/time', '-l' if darwin else '-v', *command], env={**env, 'LC_ALL': 'C'}, capture_output=True, text=True, check=True)
                    pattern = r'(\d+)\s+maximum resident set size' if darwin else r'Maximum resident set size \(kbytes\):\s*(\d+)'
                    match = re.search(pattern, result.stderr)
                    if not match:
                        raise RuntimeError('Unable to read peak RSS: ' + result.stderr)
                    rss[label].append(int(match[1]) * (1 if darwin else 1024))
                    seen[label].add(sha256(out[label]))
            varied = {label: sorted(hashes) for label, hashes in seen.items() if len(hashes) != 1}
            if varied:
                raise RuntimeError(f'Output bytes vary between runs in {case.name}: {varied}')
            hashes = {label: next(iter(seen[label])) for label in binaries}
            if args.verify_identical and len(set(hashes.values())) != 1:
                raise RuntimeError(f'Output bytes differ in {case.name}: {hashes}')
            entry = report['cases'][case.name] = {
                'workload': {'group': case.group, 'px': case.px, 'cols': case.cols, 'rows': case.rows,
                             'cli': 'positional' if case.legacy else 'options', 'fonts': case.font_files(),
                             'output': 'text' if case.text else 'png',
                             'input': relative(case.src),
                             'input_bytes': case.src.stat().st_size, 'input_sha256': sha256(case.src),
                             'checks': [' '.join(map(str, c)) for c in case.checks]},
            }
            for label in binaries:
                keys = profiles[label][0].keys()
                entry[label] = {
                    'wall_ms': summary(plain[label]), 'wall_samples_ms': plain[label],
                    'child_cpu_ms': summary(cpu[label]), 'child_cpu_samples_ms': cpu[label],
                    'profiled_wall_ms': summary(profiled_wall[label]), 'profiled_wall_samples_ms': profiled_wall[label],
                    'profile_overhead': paired_ratio(profiled_wall[label], plain[label]),
                    'profile': {key: summary([s[key] for s in profiles[label]]) for key in keys},
                    'profile_samples': {key: [s[key] for s in profiles[label]] for key in keys},
                    # output_* for every case; png_* too for a PNG, as in
                    # earlier reports (a text case's output is no PNG).
                    'output_bytes': out[label].stat().st_size, 'output_sha256': hashes[label],
                    **({} if case.text else {'png_bytes': out[label].stat().st_size, 'png_sha256': hashes[label]}),
                    'peak_rss_bytes': summary(rss[label]) if rss[label] else None,
                    'peak_rss_samples_bytes': rss[label],
                }
                if args.reference and label != args.reference:
                    entry[label]['paired_wall_speedup'] = paired_ratio(plain[args.reference], plain[label])
                    entry[label]['paired_cpu_speedup'] = paired_ratio(cpu[args.reference], cpu[label])
                print(f'{case.name:18} {label:10} median={statistics.median(plain[label]):8.2f} ms '
                      f'p95={summary(plain[label])["p95"]:8.2f} ms '
                      f'profiled x{entry[label]["profile_overhead"]["median"]:.3f}', flush=True)
        # Cold: each run starts with an empty page cache, so the binary, the
        # input and any font file come from storage.
        for case in cases:
            if not args.cold_runs or (args.cold_case and case.name not in args.cold_case):
                continue
            cold = {label: [] for label in binaries}
            for _ in range(args.cold_runs):
                labels = list(binaries)
                rng.shuffle(labels)
                for label in labels:
                    drop_caches()
                    cold_out = directory / f'{label}-cold.{"txt" if case.text else "png"}'
                    command = case.command(binaries[label], cold_out)
                    start = time.perf_counter_ns()
                    subprocess.run(command, env=env, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, check=True)
                    cold[label].append(round((time.perf_counter_ns() - start) / 1e6, 4))
                    if sha256(cold_out) != report['cases'][case.name][label]['output_sha256']:
                        raise RuntimeError(f'{case.name}: a cold run of {label} wrote a different output')
            report['cold'][case.name] = {label: {'wall_ms': summary(cold[label]), 'wall_samples_ms': cold[label]}
                                         for label in binaries}
            for label in binaries:
                print(f'{case.name:18} {label:10} cold median={statistics.median(cold[label]):8.2f} ms', flush=True)
    report['load_average_end'] = os.getloadavg()
    args.output.write_text(json.dumps(report, separators=(',', ':')) + '\n')


if __name__ == '__main__':
    main()
