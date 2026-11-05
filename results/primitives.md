# Every form against the machine

Each form of the language measured in Isocron and as the same work in C
written the cheapest way known (its floor), on one CPU at fixed frequency,
the median of 5 runs of 200 rounds (`bench/primitives/run.py`); the
nanoseconds of one call, ranked by Isocron over its floor.

| group | form | Isocron (ns) | floor (ns) | Isocron / floor |
| :-- | :-- | --: | --: | --: |
| time | `now` | 150.78 | 17.59 | 8.6 |
| time | `formatIso` | 380.90 | 45.81 | 8.3 |
| sys | `normalize` | 336.77 | 50.00 | 6.7 |
| strings | `padStart` | 11.50 | 1.98 | 5.8 |
| sys | `basename` | 25.49 | 4.85 | 5.3 |
| strings | `template` | 37.97 | 10.55 | 3.6 |
| sys | `env` | 59.01 | 19.01 | 3.1 |
| strings | `concat` | 8.93 | 3.09 | 2.9 |
| strings | `split.6` | 75.66 | 32.06 | 2.4 |
| strings | `trim` | 13.83 | 6.00 | 2.3 |
| sys | `readFile.4k` | 4327.81 | 1925.03 | 2.2 |
| strings | `indexOf.1` | 6.73 | 3.15 | 2.1 |
| strings | `bytes` | 1.68 | 0.83 | 2.0 |
| strings | `includes.3` | 25.14 | 12.58 | 2.0 |
| strings | `endsWith.2` | 1.87 | 0.94 | 2.0 |
| strings | `parseI64.7` | 9.12 | 4.62 | 2.0 |
| strings | `lines.line` | 14.80 | 8.52 | 1.7 |
| strings | `toUpperCase` | 17.40 | 10.48 | 1.7 |
| strings | `slice` | 2.08 | 1.25 | 1.7 |
| strings | `startsWith.1` | 1.81 | 1.12 | 1.6 |
| time | `parseIso` | 37.43 | 23.31 | 1.6 |
| strings | `startsWith.8` | 1.79 | 1.14 | 1.6 |
| strings | `equal` | 4.85 | 3.17 | 1.5 |
| strings | `less` | 4.85 | 3.48 | 1.4 |
| sys | `listDir.65` | 10979.17 | 9076.39 | 1.2 |
| strings | `join.line` | 5.26 | 4.46 | 1.2 |
| strings | `replaceAll.1` | 54.67 | 48.77 | 1.1 |
| time | `dateOf.daysOf` | 19.15 | 17.30 | 1.1 |
| time | `toDateTime` | 3.70 | 3.37 | 1.1 |
| sys | `writeFile.4k` | 3263.66 | 3264.72 | 1.0 |
| strings | `toString.i64` | 11.08 | 11.21 | 1.0 |
| sys | `exists` | 662.81 | 701.35 | 0.9 |
