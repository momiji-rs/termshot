host starship: AMD Ryzen 7 8745HS w/ Radeon 780M Graphics, Linux-7.2.5-3-omarchy-x86_64-with-glibc2.44
- performance-2026-10-03-linux-cold.json: seed 41, 5 runs, 2026-10-03T18:12:30.609258+00:00, load 1.49 -> 1.75, binaries main=fefda4d0612d, branch=6a58fd112552

| case | main wall med / p95 | branch wall med / p95 | main CPU | branch CPU | main RSS MiB | branch RSS MiB | output bytes |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| font-builtin | 9.85 / 10.21 | 10.15 / 10.49 | 9.65 | 9.89 | - | - | 205,915 |
| font-file | 9.78 / 9.98 | 9.85 / 10.19 | 9.55 | 9.61 | - | - | 205,915 |
| cjk-subset | 11.13 / 11.47 | 11.12 / 11.19 | 10.93 | 10.90 | - | - | 296,925 |
| cjk-full | 15.09 / 16.22 | 14.81 / 15.05 | 14.80 | 14.51 | - | - | 296,925 |

| case | main/branch batch 1 [95%] | main profiled/plain [95%] | branch profiled/plain [95%] |
| --- | ---: | ---: | ---: |
| font-builtin | 0.987 [0.864, 1.006] | 0.987 [0.919, 1.056] | 0.949 [0.901, 0.997] |
| font-file | 0.961 [0.926, 1.048] | 1.032 [0.973, 1.065] | 1.008 [0.954, 1.041] |
| cjk-subset | 1.022 [0.905, 1.137] | 1.006 [0.904, 1.022] | 1.018 [0.933, 1.024] |
| cjk-full | 1.026 [0.951, 1.095] | 0.970 [0.916, 1.082] | 1.036 [0.945, 1.294] |

Top stages (branch, batch 1 medians, ms) and counters:

| case | profiled wall | total_ms | top stages | font_load parts | glyphs: raster / hits / evict / missing / fallback lookups / fallback raster |
| --- | ---: | ---: | --- | --- | --- |
| font-builtin | 9.59 | 8.48 | deflate_match_emit 3.61, deflate_checksum 1.93, background 0.78 | font: 0.006/0.141/0.161/0.389 | 63 / 399 / 0 / 0 / 0 / 0 |
| font-file | 9.91 | 8.77 | deflate_match_emit 3.57, deflate_checksum 1.92, background 0.80 | font: 0.005/0.141/0.122/0.352 | 63 / 399 / 0 / 0 / 0 / 0 |
| cjk-subset | 11.03 | 9.81 | deflate_match_emit 5.19, font_load 0.94, background 0.79 | font: 0.004/0.107/0.112/0.345, fallback: 0.000/0.013/0.006/0.338 | 37 / 1463 / 6 / 0 / 25 / 25 |
| cjk-full | 14.98 | 13.55 | deflate_match_emit 5.08, font_load 5.08, background 0.79 | font: 0.005/0.144/0.162/0.463, fallback: 0.007/3.842/0.085/0.348 | 37 / 1463 / 6 / 0 / 25 / 25 |

Identical outputs across cases: font-builtin = font-file; cjk-subset = cjk-full
All binaries and batches give the same output per case: True
cold performance-2026-10-03-linux-cold.json font-builtin: main 16.61 / p95 17.30 (warm 9.85), branch 17.01 / p95 18.19 (warm 10.15)
cold performance-2026-10-03-linux-cold.json font-file: main 18.06 / p95 18.87 (warm 9.78), branch 18.07 / p95 20.34 (warm 9.85)
cold performance-2026-10-03-linux-cold.json cjk-subset: main 18.66 / p95 21.03 (warm 11.13), branch 19.00 / p95 19.88 (warm 11.12)
cold performance-2026-10-03-linux-cold.json cjk-full: main 27.18 / p95 28.90 (warm 15.09), branch 27.18 / p95 31.23 (warm 14.81)
