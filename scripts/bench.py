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

    def __init__(self, name, src, px, cols, rows, fonts=None, legacy=False, group='legacy', checks=()):
        self.name, self.src, self.px, self.cols, self.rows = name, Path(src), px, cols, rows
        self.fonts = fonts or []
        self.legacy, self.group, self.checks = legacy, group, checks

    def command(self, binary, out):
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


def run_profiled(command, env):
    result = subprocess.run(command, env={**env, 'TERMSHOT_PROFILE': '1'}, capture_output=True, text=True, check=True)
    profile = {}
    for line in result.stderr.splitlines():
        if line.startswith('termshot-profile '):
            profile.update(json.loads(line.split(' ', 1)[1]))
    return profile


def check(case, profile):
    """The counters that show a case exercised the path it is named for."""
    failed = []
    for key, op, value in case.checks:
        if key not in profile:
            continue  # an older binary without the counter
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
    p.add_argument('--suite', choices=('all', 'legacy', 'fonts'), default='all')
    p.add_argument('--cjk-font', type=Path, default=SYSTEM_CJK if SYSTEM_CJK.exists() else None,
                   help=f'full NotoSansCJK-Regular.ttc for the *-full cases (default: {SYSTEM_CJK} if present)')
    p.add_argument('--memory-runs', type=int, default=0, help='separate peak-RSS runs using /usr/bin/time')
    p.add_argument('--cold-runs', type=int, default=0, help='Linux: runs after dropping the page cache (sudo -n)')
    p.add_argument('--cold-case', action='append', help='cases for --cold-runs (default: all selected)')
    p.add_argument('--verify-identical', action='store_true', help='require byte-identical PNGs across binaries')
    p.add_argument('--reference', help='binary label for paired speedup estimates')
    p.add_argument('--seed', type=int, default=0, help='seed for interleaved execution order')
    args = p.parse_args()
    if args.runs < 1 or args.warmups < 0 or args.memory_runs < 0 or args.cold_runs < 0:
        p.error('runs must be positive; warmups, memory-runs and cold-runs must be nonnegative')
    binaries = {label: str(Path(path).resolve()) for label, path in (item.split('=', 1) for item in args.binary)}
    if args.reference and args.reference not in binaries:
        p.error('reference must name one of the binary labels')
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
              'build_flags': {'c': '-O2 -ffp-contract=off (draw.c); -O2 (deflate.c, image.c)',
                              'rust': '--edition 2021 -C opt-level=2'},
              'runs': args.runs, 'warmups': args.warmups, 'memory_runs': args.memory_runs,
              'cold_runs': args.cold_runs, 'verify_identical': args.verify_identical,
              'seed': args.seed, 'reference': args.reference,
              'cache_condition': 'warm: page cache populated by warmups; output files rewritten, no fsync',
              'fonts': {}, 'load_average_start': os.getloadavg(), 'cases': {}, 'cold': {}}
    for path in [FONT, CJK_SUBSET] + ([args.cjk_font] if args.cjk_font else []):
        report['fonts'][str(path)] = {'bytes': path.stat().st_size, 'sha256': sha256(path)}
    env = dict(os.environ)
    env.pop('TERMSHOT_PROFILE', None)
    rng = random.Random(args.seed)
    with tempfile.TemporaryDirectory(prefix='termshot-bench-') as tmp:
        directory = Path(tmp)
        cases = []
        if args.suite in ('all', 'fonts'):
            cases += font_workloads(args.cjk_font)
        if args.suite in ('all', 'legacy'):
            cases += legacy_workloads(directory)
        if args.case:
            unknown = set(args.case) - {case.name for case in cases}
            if unknown:
                p.error('unknown case: ' + ', '.join(sorted(unknown)))
            cases = [case for case in cases if case.name in args.case]
        for case in cases:
            out = {label: directory / f'{label}.png' for label in binaries}
            plain = {label: [] for label in binaries}
            cpu = {label: [] for label in binaries}
            profiled_wall = {label: [] for label in binaries}
            profiles = {label: [] for label in binaries}
            failures = []
            for run in range(-args.warmups, args.runs):
                order = [(label, profiled) for label in binaries for profiled in (False, True)]
                rng.shuffle(order)
                for label, profiled in order:
                    command = case.command(binaries[label], out[label])
                    before = resource.getrusage(resource.RUSAGE_CHILDREN)
                    start = time.perf_counter_ns()
                    if profiled:
                        profile = run_profiled(command, env)
                    else:
                        subprocess.run(command, env=env, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, check=True)
                    elapsed = round((time.perf_counter_ns() - start) / 1e6, 4)
                    after = resource.getrusage(resource.RUSAGE_CHILDREN)
                    if run < 0:
                        continue
                    if profiled:
                        profiled_wall[label].append(elapsed)
                        profiles[label].append(profile)
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
            hashes = {label: sha256(out[label]) for label in binaries}
            if args.verify_identical and len(set(hashes.values())) != 1:
                raise RuntimeError(f'PNG bytes differ in {case.name}: {hashes}')
            entry = report['cases'][case.name] = {
                'workload': {'group': case.group, 'px': case.px, 'cols': case.cols, 'rows': case.rows,
                             'cli': 'positional' if case.legacy else 'options', 'fonts': case.font_files(),
                             'input': str(case.src.relative_to(ROOT)) if case.src.is_relative_to(ROOT) else case.src.name,
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
                    'png_bytes': out[label].stat().st_size, 'png_sha256': hashes[label],
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
                    command = case.command(binaries[label], directory / f'{label}-cold.png')
                    start = time.perf_counter_ns()
                    subprocess.run(command, env=env, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, check=True)
                    cold[label].append(round((time.perf_counter_ns() - start) / 1e6, 4))
            report['cold'][case.name] = {label: {'wall_ms': summary(cold[label]), 'wall_samples_ms': cold[label]}
                                         for label in binaries}
            for label in binaries:
                print(f'{case.name:18} {label:10} cold median={statistics.median(cold[label]):8.2f} ms', flush=True)
    report['load_average_end'] = os.getloadavg()
    args.output.write_text(json.dumps(report, separators=(',', ':')) + '\n')


if __name__ == '__main__':
    main()
