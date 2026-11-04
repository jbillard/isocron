# Every form against the machine

Each form of the language measured in Isocron and as the same work in C
written the cheapest way known (its floor), on one CPU at fixed frequency,
the median of 5 runs of 200 rounds (`bench/primitives/run.py`); the
nanoseconds of one call, ranked by Isocron over its floor.

| group | form | Isocron (ns) | floor (ns) | Isocron / floor |
| :-- | :-- | --: | --: | --: |
| strings | `padStart` | 11.41 | 1.99 | 5.7 |
| strings | `template` | 38.01 | 10.56 | 3.6 |
| strings | `concat` | 9.07 | 3.15 | 2.9 |
| strings | `split.6` | 75.87 | 32.08 | 2.4 |
| strings | `trim` | 13.91 | 6.03 | 2.3 |
| strings | `indexOf.1` | 6.79 | 3.22 | 2.1 |
| strings | `includes.3` | 25.32 | 12.58 | 2.0 |
| strings | `bytes` | 1.67 | 0.83 | 2.0 |
| strings | `endsWith.2` | 1.83 | 0.92 | 2.0 |
| strings | `parseI64.7` | 9.20 | 4.66 | 2.0 |
| strings | `lines.line` | 15.34 | 8.54 | 1.8 |
| strings | `slice` | 2.15 | 1.26 | 1.7 |
| strings | `toUpperCase` | 17.43 | 10.48 | 1.7 |
| strings | `startsWith.1` | 1.78 | 1.13 | 1.6 |
| strings | `equal` | 4.99 | 3.19 | 1.6 |
| strings | `startsWith.8` | 1.75 | 1.15 | 1.5 |
| strings | `less` | 4.88 | 3.51 | 1.4 |
| strings | `join.line` | 5.33 | 4.50 | 1.2 |
| strings | `replaceAll.1` | 56.08 | 48.76 | 1.2 |
| strings | `toString.i64` | 11.14 | 11.22 | 1.0 |
