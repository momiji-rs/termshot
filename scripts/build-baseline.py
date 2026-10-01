#!/usr/bin/env python3
"""Build the original and instrumented 8e1110e binaries in a new directory."""
import argparse
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]
p = argparse.ArgumentParser(description=__doc__)
p.add_argument('destination', type=Path, help='new, empty build directory')
a = p.parse_args()
dest = a.destination.resolve()
dest.mkdir(parents=True, exist_ok=False)
# Read only the files required for a baseline build; never switch the worktree.
for name in ('build.sh', 'src/main.rs', 'src/draw.c', 'src/deflate.c',
             'third_party/stb/stb_truetype.h', 'third_party/stb/stb_image_write.h'):
    path = dest / name
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(subprocess.check_output(['git', 'show', f'8e1110e:{name}'], cwd=ROOT))
subprocess.run(['sh', 'build.sh'], cwd=dest, check=True)
(dest / 'termshot').rename(dest / 'original')
subprocess.run(['patch', '-p1', '-i', str(ROOT / 'docs/profiling-baseline.patch')], cwd=dest, check=True)
subprocess.run(['sh', 'build.sh'], cwd=dest, check=True)
(dest / 'termshot').rename(dest / 'baseline')
print(f'Original: {dest / "original"}\nInstrumented: {dest / "baseline"}')
