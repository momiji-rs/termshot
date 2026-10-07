host host: cpu, platform
- fake-report.json: seed 3, 4 runs, 2026-10-07T00:00:00.123456+00:00, load 1.50 -> 1.50, binaries a=a1955811422f, b=a1955811422f

| case | a wall med / p95 | b wall med / p95 | a CPU | b CPU | a RSS MiB | b RSS MiB | output bytes |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| font-builtin | 7.47 / 9.27 | 4.59 / 8.91 | 1611.11 | 1611.11 | - | - | 37 |
| reply-sent | 7.28 / 8.18 | 5.84 / 8.54 | 1111.11 | 1611.11 | - | - | 51 |
| color-grid | 4.93 / 9.97 | 3.31 / 9.62 | 1611.11 | 1611.11 | - | - | 51 |
| text-kitty | 3.30 / 4.57 | 7.44 / 8.16 | 1611.11 | 1611.11 | - | - | 36 |

| case | a/b batch 1 [95%] | a profiled/plain [95%] | b profiled/plain [95%] |
| --- | ---: | ---: | ---: |
| font-builtin | 1.315 [0.490, 4.778] | 0.708 [0.301, 4.126] | 1.473 [0.273, 2.259] |
| reply-sent | 1.370 [0.494, 1.472] | 0.426 [0.164, 1.321] | 0.694 [0.450, 1.763] |
| color-grid | 2.000 [0.289, 3.183] | 1.110 [0.221, 2.686] | 2.417 [0.551, 5.050] |
| text-kitty | 0.494 [0.400, 0.644] | 1.657 [0.527, 2.694] | 1.291 [0.162, 1.950] |

Top stages and counters: not in this report (slim; bench.sh --full-profile keeps them)

| case | total_ms (b, batch 1 median) |
| --- | ---: |
| font-builtin | 6.000 |
| reply-sent | 5.500 |
| color-grid | 6.000 |
| text-kitty | 4.000 |

Identical outputs across cases: none
All binaries and batches give the same output per case: True
