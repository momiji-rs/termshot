#!/usr/bin/env python3
"""Dependency-free, interleaved CLI benchmark. JSON is the durable result."""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import random
import statistics
import subprocess
import tempfile
import time
from datetime import datetime, timezone

ROOT = Path(__file__).resolve().parents[1]
FONT = ROOT / 'third_party/jetbrains-mono/JetBrainsMono-Regular.ttf'


def workloads(directory):
    cases = [(name, ROOT / f'examples/{name}.pty', 48, 100, 30)
             for name in ('reply-sent', 'draft-ready')]
    generated = {
        'blank': (b'', 48, 100, 30),
        'dense': ('\n'.join(['The quick brown fox 0123456789! @#$% ' * 3] * 30).encode(), 48, 100, 30),
        'ansi-replay': ((ROOT / 'examples/reply-sent.pty').read_bytes() * 250, 48, 100, 30),
        'large': ('\n'.join(['Terminal benchmark 0123456789 ' * 9] * 80).encode(), 48, 240, 80),
        'unicode': ('\n'.join([''.join(chr(0x100 + (r * 100 + c) % 800) for c in range(100)) for r in range(30)]).encode(), 24, 100, 30),
    }
    for name, (data, px, cols, rows) in generated.items():
        path = directory / f'{name}.pty'
        path.write_bytes(data)
        cases.append((name, path, px, cols, rows))
    return cases


def summary(values):
    values = sorted(values)
    return {'median': statistics.median(values),
            'p95': values[math.ceil(len(values) * .95) - 1],
            'min': values[0], 'max': values[-1]}


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary', action='append', required=True, help='label=/path/to/binary (repeatable)')
    p.add_argument('--runs', type=int, default=10)
    p.add_argument('--warmups', type=int, default=2)
    p.add_argument('--output', type=Path, required=True)
    p.add_argument('--case', action='append')
    args = p.parse_args()
    if args.runs < 1 or args.warmups < 0:
        p.error('runs must be positive and warmups nonnegative')
    binaries = dict(item.split('=', 1) for item in args.binary)
    report = {'platform': platform.platform(), 'machine': platform.machine(),
              'timestamp_utc': datetime.now(timezone.utc).isoformat(),
              'binary_sha256': {label: hashlib.sha256(Path(path).read_bytes()).hexdigest() for label, path in binaries.items()},
              'toolchain': {tool: subprocess.check_output([tool, '--version'], text=True).splitlines()[0] for tool in ('rustc', 'cc', 'python3')},
              'runs': args.runs, 'warmups': args.warmups,
              'binaries': binaries, 'cases': {}}
    env = dict(os.environ)
    env.pop('TERMSHOT_PROFILE', None)
    rng = random.Random(0)
    with tempfile.TemporaryDirectory(prefix='termshot-bench-') as tmp:
        directory = Path(tmp)
        for name, src, px, cols, rows in workloads(directory):
            if args.case and name not in args.case:
                continue
            samples = {label: [] for label in binaries}
            profiles = {label: [] for label in binaries}
            for run in range(-args.warmups, args.runs):
                labels = list(binaries)
                rng.shuffle(labels)
                for label in labels:
                    command = [str(Path(binaries[label]).resolve()), str(src), str(directory / f'{label}.png'), str(FONT), str(px), str(cols), str(rows)]
                    start = time.perf_counter_ns()
                    subprocess.run(command, env=env, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, check=True)
                    elapsed = (time.perf_counter_ns() - start) / 1e6
                    if run >= 0:
                        samples[label].append(elapsed)
            # Profile separately so timer overhead never biases normal CLI timings.
            for label in binaries:
                command = [str(Path(binaries[label]).resolve()), str(src), str(directory / f'{label}.png'), str(FONT), str(px), str(cols), str(rows)]
                for _ in range(args.runs):
                    result = subprocess.run(command, env={**env, 'TERMSHOT_PROFILE': '1'}, capture_output=True, text=True, check=True)
                    profile = {}
                    for line in result.stderr.splitlines():
                        if line.startswith('termshot-profile '):
                            profile.update(json.loads(line.split(' ', 1)[1]))
                    profiles[label].append(profile)
            report['cases'][name] = {}
            for label in binaries:
                keys = profiles[label][0].keys()
                report['cases'][name][label] = {
                    'wall_ms': summary(samples[label]), 'wall_samples_ms': samples[label],
                    'profile': {key: summary([s[key] for s in profiles[label]]) for key in keys},
                    'profile_samples': profiles[label],
                    'png_bytes': (directory / f'{label}.png').stat().st_size,
                }
                print(f'{name:12} {label:12} median={statistics.median(samples[label]):8.2f} ms p95={summary(samples[label])["p95"]:8.2f} ms', flush=True)
    args.output.write_text(json.dumps(report, indent=2) + '\n')


if __name__ == '__main__':
    main()
