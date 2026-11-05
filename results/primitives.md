# Every form against the machine

Each form of the language measured in Isocron and as the same work in C
written the cheapest way known (its floor), on one CPU at fixed frequency,
the median of 5 runs of 200 rounds (`bench/primitives/run.py`); the
nanoseconds of one call, ranked by Isocron over its floor.

| group | form | Isocron (ns) | floor (ns) | Isocron / floor |
| :-- | :-- | --: | --: | --: |
| time | `now` | 150.97 | 17.67 | 8.5 |
| time | `formatIso` | 384.39 | 45.75 | 8.4 |
| sys | `normalize` | 339.61 | 50.47 | 6.7 |
| json | `parse` | 1091.56 | 167.86 | 6.5 |
| json | `decode` | 541.51 | 86.52 | 6.3 |
| strings | `padStart` | 12.04 | 1.98 | 6.1 |
| sys | `basename` | 25.50 | 4.83 | 5.3 |
| strings | `template` | 38.76 | 10.56 | 3.7 |
| sys | `env` | 60.23 | 18.20 | 3.3 |
| strings | `concat` | 9.27 | 3.10 | 3.0 |
| strings | `trim` | 14.08 | 6.00 | 2.3 |
| strings | `split.6` | 75.00 | 32.03 | 2.3 |
| sys | `readFile.4k` | 4345.66 | 1921.88 | 2.3 |
| strings | `indexOf.1` | 6.73 | 3.09 | 2.2 |
| strings | `endsWith.2` | 1.90 | 0.93 | 2.0 |
| strings | `bytes` | 1.73 | 0.86 | 2.0 |
| strings | `parseI64.7` | 9.12 | 4.55 | 2.0 |
| strings | `includes.3` | 25.07 | 12.57 | 2.0 |
| strings | `lines.line` | 14.80 | 8.54 | 1.7 |
| strings | `toUpperCase` | 17.66 | 10.48 | 1.7 |
| strings | `slice` | 2.13 | 1.27 | 1.7 |
| strings | `startsWith.1` | 1.84 | 1.12 | 1.6 |
| time | `parseIso` | 38.07 | 23.33 | 1.6 |
| strings | `startsWith.8` | 1.81 | 1.15 | 1.6 |
| json | `stringify` | 604.21 | 390.98 | 1.5 |
| strings | `equal` | 4.84 | 3.15 | 1.5 |
| sha256 | `digest.55` | 425.70 | 282.90 | 1.5 |
| sha256 | `digest.4k` | 25839.88 | 17402.70 | 1.5 |
| strings | `less` | 4.86 | 3.45 | 1.4 |
| sys | `listDir.65` | 10998.99 | 8986.33 | 1.2 |
| json | `encode` | 267.92 | 219.03 | 1.2 |
| strings | `join.line` | 5.35 | 4.46 | 1.2 |
| strings | `replaceAll.1` | 54.38 | 48.75 | 1.1 |
| time | `dateOf.daysOf` | 19.18 | 17.31 | 1.1 |
| time | `toDateTime` | 3.70 | 3.37 | 1.1 |
| sys | `writeFile.4k` | 3268.80 | 3240.59 | 1.0 |
| strings | `toString.i64` | 11.18 | 11.20 | 1.0 |
| sys | `exists` | 664.27 | 702.62 | 0.9 |
| sha256 | `hex.32` | 21.05 | 33.02 | 0.6 |
