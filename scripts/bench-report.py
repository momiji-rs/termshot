#!/usr/bin/env python3
"""Markdown tables from scripts/bench.py JSON results, for docs/performance.md.

    python3 scripts/bench-report.py A.json [B.json ...] --label branch

Prints, per case: median/p95 wall, child CPU and peak RSS for each binary,
the paired speedup of every batch, profiling overhead, the stages that
dominate, and the counters that show which path ran. Nothing is measured here.
"""
import argparse
import json
from pathlib import Path
import statistics

# Stages that do not overlap (docs/performance.md, "Stage timings"), so
# their medians can be ranked side by side. foreground_other is foreground
# minus its three child timers; font_load holds every font_* and fallback_* timer.
LEAVES = ['input_read_ms', 'font_load_ms', 'parse_ms', 'face_ms', 'font_setup_ms', 'allocate_ms',
          'background_ms', 'geometry_ms', 'glyph_ms', 'blend_ms', 'foreground_other_ms',
          'deflate_match_emit_ms', 'deflate_checksum_ms', 'png_deflate_other_ms', 'png_pack_ms',
          'output_write_ms']


def leaves(samples):
    """Per-run leaf stages from a columnar profile_samples record."""
    n = len(samples['total_ms'])
    out = {key: [] for key in LEAVES}
    for i in range(n):
        get = lambda key: samples.get(key, [0.0] * n)[i]
        row = {key: get(key) for key in LEAVES if key in samples}
        row['face_ms'] = get('face_ms')
        row['foreground_other_ms'] = get('foreground_ms') - get('geometry_ms') - get('glyph_ms') - get('blend_ms')
        row['png_deflate_other_ms'] = (get('png_deflate_ms') - get('deflate_match_emit_ms') - get('deflate_checksum_ms')
                                       - get('deflate_allocate_ms') - get('deflate_finalize_ms'))
        for key in LEAVES:
            out[key].append(row.get(key, 0.0))
    return {key: statistics.median(values) for key, values in out.items()}


def fmt(x, digits=2):
    return f'{x:.{digits}f}'


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('results', nargs='+', type=Path)
    p.add_argument('--label', default='branch', help='binary whose profile is ranked')
    a = p.parse_args()
    batches = [json.loads(path.read_text()) for path in a.results]
    first = batches[0]
    labels = list(first['binaries'])
    ref = first['reference']
    print(f"host {first['machine']['hostname']}: {first['machine']['cpu']}, {first['machine']['platform']}")
    for b, path in zip(batches, a.results):
        print(f"- {path.name}: seed {b['seed']}, {b['runs']} runs, {b['timestamp_utc']}, "
              f"load {fmt(b['load_average_start'][0])} -> {fmt(b['load_average_end'][0])}, "
              f"binaries {', '.join(f'{k}={v[:12]}' for k, v in b['binary_sha256'].items())}")
    print()
    head = ' | '.join(f'{label} wall med / p95' for label in labels)
    print(f'| case | {head} | ' + ' | '.join(f'{label} CPU' for label in labels)
          + ' | ' + ' | '.join(f'{label} RSS MiB' for label in labels) + ' | PNG bytes |')
    print('| --- |' + ' ---: |' * (3 * len(labels) + 1))
    for name, case in first['cases'].items():
        walls = ' | '.join(f"{fmt(case[l]['wall_ms']['median'])} / {fmt(case[l]['wall_ms']['p95'])}" for l in labels)
        cpus = ' | '.join(fmt(case[l]['child_cpu_ms']['median']) for l in labels)
        rss = ' | '.join(fmt(case[l]['peak_rss_bytes']['median'] / 2**20) if case[l]['peak_rss_bytes'] else '-' for l in labels)
        print(f"| {name} | {walls} | {cpus} | {rss} | {case[labels[0]]['png_bytes']:,} |")
    print()
    others = [l for l in labels if l != ref]
    if ref and others:
        print(f'| case | ' + ' | '.join(f'{ref}/{l} batch {i + 1} [95%]' for l in others for i in range(len(batches)))
              + ' | ' + ' | '.join(f'{l} profiled/plain [95%]' for l in labels) + ' |')
        print('| --- |' + ' ---: |' * (len(others) * len(batches) + len(labels)))
        for name in first['cases']:
            cells = []
            for l in others:
                for b in batches:
                    s = b['cases'][name][l]['paired_wall_speedup']
                    cells.append(f"{s['median']:.3f} [{s['bootstrap_95pct'][0]:.3f}, {s['bootstrap_95pct'][1]:.3f}]")
            for l in labels:
                s = first['cases'][name][l]['profile_overhead']
                cells.append(f"{s['median']:.3f} [{s['bootstrap_95pct'][0]:.3f}, {s['bootstrap_95pct'][1]:.3f}]")
            print(f'| {name} | ' + ' | '.join(cells) + ' |')
        print()
    print(f'Top stages ({a.label}, batch 1 medians, ms) and counters:')
    print()
    print('| case | profiled wall | total_ms | top stages | font_load parts | glyphs: raster / hits / evict / missing / fallback lookups / fallback raster |')
    print('| --- | ---: | ---: | --- | --- | --- |')
    for name, case in first['cases'].items():
        entry = case[a.label]
        med = leaves(entry['profile_samples'])
        top = sorted(med.items(), key=lambda kv: -kv[1])[:3]
        prof = entry['profile']
        m = lambda key: prof[key]['median'] if key in prof else 0.0
        parts = ', '.join(f"{f}: {fmt(m(f + '_allocate_ms'), 3)}/{fmt(m(f + '_read_ms'), 3)}/{fmt(m(f + '_check_ms'), 3)}/{fmt(m(f + '_padding_ms'), 3)}"
                          for f in ('font', 'fallback') if m(f + '_bytes'))
        counters = ' / '.join(str(int(m(k))) for k in ('glyph_rasterizations', 'glyph_cache_hits', 'glyph_cache_evictions',
                                                         'glyph_missing', 'fallback_lookups', 'fallback_rasterizations'))
        print(f"| {name} | {fmt(entry['profiled_wall_ms']['median'])} | {fmt(m('total_ms'))} | "
              + ', '.join(f'{k[:-3]} {fmt(v)}' for k, v in top) + f' | {parts} | {counters} |')
    print()
    hashes = {}
    for name, case in first['cases'].items():
        hashes.setdefault(case[labels[0]]['png_sha256'], []).append(name)
    same = [names for names in hashes.values() if len(names) > 1]
    print('Identical PNGs across cases: ' + ('; '.join(' = '.join(n) for n in same) or 'none'))
    agree = all(b['cases'][n][l]['png_sha256'] == first['cases'][n][labels[0]]['png_sha256']
                for b in batches for n in first['cases'] for l in labels)
    print(f'All binaries and batches give the same PNG per case: {agree}')
    for b, path in zip(batches, a.results):
        for name, cold in b.get('cold', {}).items():
            warm = b['cases'][name]
            print(f"cold {path.name} {name}: " + ', '.join(
                f"{l} {fmt(cold[l]['wall_ms']['median'])} / p95 {fmt(cold[l]['wall_ms']['p95'])} (warm {fmt(warm[l]['wall_ms']['median'])})"
                for l in labels))


if __name__ == '__main__':
    main()
