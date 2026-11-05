# Every form against the machine

Each form of the language measured in Isocron and as the same work in C
written the cheapest way known (its floor), on one CPU at fixed frequency,
the median of 5 runs of 200 rounds (`bench/primitives/run.py`); the
nanoseconds of one call, ranked by Isocron over its floor.

| group | form | Isocron (ns) | floor (ns) | Isocron / floor |
| :-- | :-- | --: | --: | --: |
| time | `now` | 150.59 | 17.67 | 8.5 |
| time | `formatIso` | 384.42 | 45.77 | 8.4 |
| sys | `normalize` | 339.20 | 51.12 | 6.6 |
| json | `parse` | 1092.39 | 168.41 | 6.5 |
| json | `decode` | 543.62 | 86.70 | 6.3 |
| strings | `padStart` | 11.92 | 2.03 | 5.9 |
| sys | `basename` | 25.50 | 4.83 | 5.3 |
| strings | `template` | 38.46 | 10.57 | 3.6 |
| sys | `env` | 61.01 | 18.30 | 3.3 |
| strings | `concat` | 9.19 | 3.12 | 2.9 |
| strings | `split.6` | 75.32 | 32.06 | 2.3 |
| strings | `trim` | 14.09 | 6.00 | 2.3 |
| sys | `readFile.4k` | 4338.99 | 1909.95 | 2.3 |
| strings | `indexOf.1` | 6.72 | 3.08 | 2.2 |
| strings | `includes.3` | 25.12 | 12.55 | 2.0 |
| strings | `endsWith.2` | 1.87 | 0.95 | 2.0 |
| strings | `parseI64.7` | 9.12 | 4.66 | 2.0 |
| strings | `bytes` | 1.68 | 0.86 | 2.0 |
| strings | `lines.line` | 14.79 | 8.44 | 1.8 |
| strings | `toUpperCase` | 17.67 | 10.48 | 1.7 |
| time | `parseIso` | 38.28 | 23.34 | 1.6 |
| strings | `slice` | 2.07 | 1.29 | 1.6 |
| strings | `startsWith.1` | 1.80 | 1.13 | 1.6 |
| strings | `startsWith.8` | 1.78 | 1.15 | 1.6 |
| json | `stringify` | 602.60 | 390.60 | 1.5 |
| strings | `equal` | 4.83 | 3.13 | 1.5 |
| strings | `less` | 4.84 | 3.45 | 1.4 |
| json | `encode` | 267.63 | 217.75 | 1.2 |
| sys | `listDir.65` | 11027.44 | 9042.33 | 1.2 |
| strings | `join.line` | 5.37 | 4.47 | 1.2 |
| strings | `replaceAll.1` | 54.47 | 48.81 | 1.1 |
| time | `dateOf.daysOf` | 19.17 | 17.30 | 1.1 |
| time | `toDateTime` | 3.70 | 3.37 | 1.1 |
| strings | `toString.i64` | 11.29 | 11.22 | 1.0 |
| sys | `writeFile.4k` | 3270.61 | 3259.72 | 1.0 |
| sys | `exists` | 665.40 | 698.90 | 1.0 |
