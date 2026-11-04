# Every form against the machine

Each form of the language measured in Isocron and as the same work in C
written the cheapest way known (its floor), on one CPU at fixed frequency,
the median of 5 runs of 200 rounds (`bench/primitives/run.py`); the
nanoseconds of one call, ranked by Isocron over its floor.

| group | form | Isocron (ns) | floor (ns) | Isocron / floor |
| :-- | :-- | --: | --: | --: |
| strings | `padStart` | 61.64 | 1.99 | 31.0 |
| strings | `toUpperCase` | 186.64 | 10.48 | 17.8 |
| strings | `replaceAll.1` | 825.43 | 48.82 | 16.9 |
| strings | `slice` | 10.16 | 1.65 | 6.2 |
| strings | `lines.line` | 47.16 | 8.43 | 5.6 |
| strings | `split.6` | 166.44 | 32.05 | 5.2 |
| strings | `template` | 38.20 | 10.54 | 3.6 |
| strings | `concat` | 9.83 | 3.10 | 3.2 |
| strings | `bytes` | 8.47 | 2.97 | 2.8 |
| strings | `indexOf.1` | 7.66 | 3.12 | 2.5 |
| strings | `trim` | 14.62 | 6.01 | 2.4 |
| strings | `endsWith.2` | 1.99 | 0.97 | 2.1 |
| strings | `includes.3` | 25.94 | 12.61 | 2.1 |
| strings | `parseI64.7` | 9.06 | 4.69 | 1.9 |
| strings | `equal` | 5.67 | 3.14 | 1.8 |
| strings | `startsWith.1` | 1.90 | 1.13 | 1.7 |
| strings | `startsWith.8` | 1.88 | 1.16 | 1.6 |
| strings | `less` | 5.55 | 3.46 | 1.6 |
| strings | `join.line` | 5.41 | 4.48 | 1.2 |
| strings | `toString.i64` | 11.68 | 11.18 | 1.0 |
