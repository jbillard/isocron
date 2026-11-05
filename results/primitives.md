# Every form against the machine

Each form of the language measured in Isocron and as the same work in C
written the cheapest way known (its floor), on one CPU at fixed frequency,
the median of 5 runs of 200 rounds (`bench/primitives/run.py`); the
nanoseconds of one call, ranked by Isocron over its floor.

| group | form | Isocron (ns) | floor (ns) | Isocron / floor |
| :-- | :-- | --: | --: | --: |
| sys | `normalize` | 335.56 | 49.95 | 6.7 |
| strings | `padStart` | 11.56 | 2.03 | 5.7 |
| sys | `basename` | 25.51 | 4.82 | 5.3 |
| strings | `template` | 38.09 | 10.54 | 3.6 |
| sys | `env` | 61.64 | 19.02 | 3.2 |
| strings | `concat` | 8.93 | 3.14 | 2.8 |
| strings | `split.6` | 75.83 | 32.03 | 2.4 |
| strings | `trim` | 13.77 | 6.00 | 2.3 |
| sys | `readFile.4k` | 4316.62 | 1929.61 | 2.2 |
| strings | `indexOf.1` | 6.75 | 3.08 | 2.2 |
| strings | `endsWith.2` | 1.93 | 0.93 | 2.1 |
| strings | `bytes` | 1.74 | 0.85 | 2.0 |
| strings | `includes.3` | 25.05 | 12.56 | 2.0 |
| strings | `parseI64.7` | 9.12 | 4.64 | 2.0 |
| strings | `lines.line` | 14.77 | 8.51 | 1.7 |
| strings | `startsWith.1` | 1.87 | 1.12 | 1.7 |
| strings | `toUpperCase` | 17.34 | 10.50 | 1.7 |
| strings | `slice` | 2.10 | 1.28 | 1.6 |
| strings | `startsWith.8` | 1.84 | 1.14 | 1.6 |
| strings | `equal` | 4.86 | 3.13 | 1.6 |
| strings | `less` | 4.86 | 3.47 | 1.4 |
| sys | `listDir.65` | 11000.35 | 9011.29 | 1.2 |
| strings | `join.line` | 5.24 | 4.47 | 1.2 |
| strings | `replaceAll.1` | 54.70 | 48.77 | 1.1 |
| sys | `writeFile.4k` | 3317.43 | 3232.49 | 1.0 |
| strings | `toString.i64` | 11.15 | 11.18 | 1.0 |
| sys | `exists` | 666.28 | 691.52 | 1.0 |
