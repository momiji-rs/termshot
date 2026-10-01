#!/usr/bin/env python3
"""Build the reference and instrumented 1eaf7dd binaries in a new directory."""
import argparse
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]
p = argparse.ArgumentParser(description=__doc__)
p.add_argument('destination', type=Path, help='new, empty build directory')
p.add_argument('--revision', default='1eaf7dd')
p.add_argument('--profile-patch', type=Path, default=ROOT / 'docs/round-two-baseline.patch')
a = p.parse_args()
dest = a.destination.resolve()
dest.mkdir(parents=True, exist_ok=False)
# Read historical build inputs without switching or modifying the worktree.
names = subprocess.check_output(
    ['git', 'ls-tree', '-r', '--name-only', a.revision, '--', 'build.sh', 'src', 'third_party/stb'],
    cwd=ROOT, text=True,
).splitlines()
for name in names:
    path = dest / name
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(subprocess.check_output(['git', 'show', f'{a.revision}:{name}'], cwd=ROOT))
subprocess.run(['sh', 'build.sh'], cwd=dest, check=True)
(dest / 'termshot').rename(dest / 'original')
subprocess.run(['patch', '-p1', '-i', str(a.profile_patch.resolve())], cwd=dest, check=True)
subprocess.run(['sh', 'build.sh'], cwd=dest, check=True)
(dest / 'termshot').rename(dest / 'baseline')
print(f'Reference: {dest / "original"}\nInstrumented: {dest / "baseline"}')
