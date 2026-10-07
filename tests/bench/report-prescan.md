host Lawrences-Mac-Studio.local: Apple M2 Max, macOS-26.6.2-arm64-arm-64bit-Mach-O
- performance-2026-10-03-prescan-macos-a.json: seed 17, 40 runs, 2026-10-04T02:13:44.483956+00:00, load 4.69 -> 4.43, binaries main=345dc4507209, branch=2fb0770600c5
- performance-2026-10-03-prescan-macos-b.json: seed 29, 40 runs, 2026-10-04T02:14:02.379471+00:00, load 4.43 -> 4.05, binaries main=345dc4507209, branch=2fb0770600c5

| case | main wall med / p95 | branch wall med / p95 | main CPU | branch CPU | main RSS MiB | branch RSS MiB | output bytes |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| text-ansi-replay | 18.82 / 19.56 | 13.61 / 14.24 | 17.16 | 12.02 | 6.47 | 6.45 | 3,296 |
| text-ascii-overflow | 8.14 / 8.69 | 5.02 / 5.35 | 6.70 | 3.64 | 5.88 | 5.89 | 3,030 |
| text-dense-sgr | 22.52 / 23.24 | 16.56 / 17.00 | 20.88 | 14.93 | 5.69 | 5.67 | 3,030 |
| text-mixed-unicode | 29.60 / 30.08 | 26.63 / 27.44 | 27.88 | 24.89 | 6.16 | 6.16 | 3,088 |
| text-reply-sent | 3.08 / 3.52 | 3.10 / 3.31 | 1.91 | 1.90 | 1.97 | 1.97 | 3,296 |
| text-kitty | 3.81 / 4.16 | 3.84 / 4.17 | 2.59 | 2.59 | 3.78 | 3.75 | 110 |
| text-sixel | 3.42 / 3.67 | 3.36 / 3.75 | 2.18 | 2.16 | 3.62 | 3.59 | 67 |

| case | main/branch batch 1 [95%] | main/branch batch 2 [95%] | main profiled/plain [95%] | branch profiled/plain [95%] |
| --- | ---: | ---: | ---: | ---: |
| text-ansi-replay | 1.378 [1.364, 1.393] | 1.389 [1.385, 1.395] | 0.998 [0.994, 1.005] | 1.000 [0.987, 1.007] |
| text-ascii-overflow | 1.618 [1.591, 1.650] | 1.611 [1.566, 1.639] | 1.000 [0.988, 1.012] | 1.010 [0.990, 1.021] |
| text-dense-sgr | 1.362 [1.356, 1.371] | 1.359 [1.355, 1.366] | 0.998 [0.994, 1.007] | 1.005 [0.998, 1.010] |
| text-mixed-unicode | 1.111 [1.108, 1.117] | 1.109 [1.104, 1.116] | 1.003 [0.998, 1.008] | 1.003 [0.999, 1.010] |
| text-reply-sent | 0.992 [0.972, 1.019] | 1.023 [0.996, 1.038] | 1.016 [0.977, 1.052] | 1.012 [0.973, 1.028] |
| text-kitty | 0.991 [0.959, 1.018] | 0.971 [0.943, 0.997] | 1.021 [0.992, 1.032] | 0.987 [0.949, 1.005] |
| text-sixel | 1.014 [1.004, 1.031] | 0.991 [0.969, 1.015] | 0.996 [0.977, 1.021] | 1.014 [0.984, 1.022] |

Top stages (branch, batch 1 medians, ms) and counters:

| case | profiled wall | total_ms | top stages | font_load parts | glyphs: raster / hits / evict / missing / fallback lookups / fallback raster |
| --- | ---: | ---: | --- | --- | --- |
| text-ansi-replay | 13.59 | 10.10 | parse 8.22, font_load 0.66, input_read 0.58 |  | 0 / 0 / 0 / 0 / 0 / 0 |
| text-ascii-overflow | 5.00 | 1.86 | font_load 0.53, input_read 0.46, parse 0.35 |  | 0 / 0 / 0 / 0 / 0 / 0 |
| text-dense-sgr | 16.64 | 13.03 | parse 11.83, font_load 0.52, input_read 0.48 |  | 0 / 0 / 0 / 0 / 0 / 0 |
| text-mixed-unicode | 26.72 | 22.97 | parse 21.65, font_load 0.58, input_read 0.55 |  | 0 / 0 / 0 / 0 / 0 / 0 |
| text-reply-sent | 3.08 | 0.24 | parse 0.05, input_read 0.02, font_load 0.01 |  | 0 / 0 / 0 / 0 / 0 / 0 |
| text-kitty | 3.79 | 0.86 | parse 0.44, font_load 0.23, input_read 0.03 | font: 0.000/0.042/0.107/0.074 | 0 / 0 / 0 / 0 / 0 / 0 |
| text-sixel | 3.39 | 0.46 | font_load 0.23, parse 0.05, input_read 0.02 | font: 0.002/0.044/0.109/0.076 | 0 / 0 / 0 / 0 / 0 / 0 |

Identical outputs across cases: text-ansi-replay = text-reply-sent
All binaries and batches give the same output per case: True
