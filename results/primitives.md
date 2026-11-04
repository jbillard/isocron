# Every form against the machine

Each form of the language measured in Isocron and as the same work in C
written the cheapest way known (its floor), on one CPU at fixed frequency,
the median of 5 runs of 200 rounds (`bench/primitives/run.py`); the
nanoseconds of one call, ranked by Isocron over its floor.

| group | form | Isocron (ns) | floor (ns) | Isocron / floor |
| :-- | :-- | --: | --: | --: |
| strings | `lines.line` | 47.22 | 8.45 | 5.6 |
| strings | `padStart` | 11.57 | 2.10 | 5.5 |
| strings | `split.6` | 170.23 | 32.09 | 5.3 |
| strings | `template` | 37.85 | 10.55 | 3.6 |
| strings | `concat` | 9.38 | 3.18 | 2.9 |
| strings | `trim` | 13.79 | 6.03 | 2.3 |
| strings | `indexOf.1` | 6.82 | 3.29 | 2.1 |
| strings | `includes.3` | 25.20 | 12.71 | 2.0 |
| strings | `endsWith.2` | 1.78 | 0.95 | 1.9 |
| strings | `parseI64.7` | 8.85 | 4.71 | 1.9 |
| strings | `bytes` | 1.63 | 0.89 | 1.8 |
| strings | `toUpperCase` | 17.42 | 10.48 | 1.7 |
| strings | `slice` | 1.98 | 1.27 | 1.6 |
| strings | `startsWith.1` | 1.75 | 1.12 | 1.6 |
| strings | `equal` | 4.94 | 3.17 | 1.6 |
| strings | `startsWith.8` | 1.73 | 1.19 | 1.5 |
| strings | `less` | 4.90 | 3.54 | 1.4 |
| strings | `join.line` | 5.33 | 4.47 | 1.2 |
| strings | `replaceAll.1` | 54.57 | 48.78 | 1.1 |
| strings | `toString.i64` | 11.13 | 11.22 | 1.0 |
