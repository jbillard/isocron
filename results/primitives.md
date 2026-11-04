# Every form against the machine

Each form of the language measured in Isocron and as the same work in C
written the cheapest way known (its floor), on one CPU at fixed frequency,
the median of 5 runs of 200 rounds (`bench/primitives/run.py`); the
nanoseconds of one call, ranked by Isocron over its floor.

| group | form | Isocron (ns) | floor (ns) | Isocron / floor |
| :-- | :-- | --: | --: | --: |
| strings | `padStart` | 20.57 | 2.11 | 9.7 |
| strings | `slice` | 10.11 | 1.78 | 5.7 |
| strings | `lines.line` | 47.36 | 8.53 | 5.5 |
| strings | `split.6` | 169.93 | 32.08 | 5.3 |
| strings | `template` | 38.44 | 10.54 | 3.6 |
| strings | `concat` | 9.80 | 3.23 | 3.0 |
| strings | `bytes` | 8.39 | 3.09 | 2.7 |
| strings | `indexOf.1` | 7.65 | 3.13 | 2.4 |
| strings | `trim` | 14.62 | 6.01 | 2.4 |
| strings | `endsWith.2` | 2.07 | 0.98 | 2.1 |
| strings | `includes.3` | 25.58 | 12.63 | 2.0 |
| strings | `parseI64.7` | 9.05 | 4.72 | 1.9 |
| strings | `equal` | 5.66 | 3.14 | 1.8 |
| strings | `toUpperCase` | 18.29 | 10.50 | 1.7 |
| strings | `startsWith.1` | 1.97 | 1.16 | 1.7 |
| strings | `startsWith.8` | 1.95 | 1.16 | 1.7 |
| strings | `less` | 5.57 | 3.47 | 1.6 |
| strings | `join.line` | 5.37 | 4.52 | 1.2 |
| strings | `replaceAll.1` | 55.10 | 48.79 | 1.1 |
| strings | `toString.i64` | 11.72 | 11.22 | 1.0 |
