# Every form against the machine

Each form of the language measured in Isocron and as the same work in C
written the cheapest way known (its floor), on one CPU at fixed frequency,
the median of 5 runs of 200 rounds (`bench/primitives/run.py`); the
nanoseconds of one call, ranked by Isocron over its floor.

| group | form | Isocron (ns) | floor (ns) | Isocron / floor |
| :-- | :-- | --: | --: | --: |
| time | `now` | 151.10 | 17.59 | 8.6 |
| time | `formatIso` | 383.95 | 45.70 | 8.4 |
| sys | `normalize` | 340.10 | 50.14 | 6.8 |
| json | `parse` | 1090.64 | 167.90 | 6.5 |
| json | `decode` | 543.38 | 86.21 | 6.3 |
| strings | `padStart` | 11.75 | 2.04 | 5.8 |
| sys | `basename` | 25.52 | 4.84 | 5.3 |
| strings | `template` | 38.79 | 10.55 | 3.7 |
| sys | `env` | 61.17 | 17.75 | 3.4 |
| strings | `concat` | 9.31 | 3.12 | 3.0 |
| http | `exchange.chunked64` | 49754.47 | 20027.70 | 2.5 |
| strings | `split.6` | 75.35 | 32.08 | 2.3 |
| strings | `trim` | 14.11 | 6.02 | 2.3 |
| sys | `readFile.4k` | 4342.48 | 1918.43 | 2.3 |
| strings | `indexOf.1` | 6.74 | 3.09 | 2.2 |
| strings | `includes.3` | 25.09 | 12.59 | 2.0 |
| strings | `parseI64.7` | 9.18 | 4.96 | 1.8 |
| strings | `bytes` | 1.58 | 0.86 | 1.8 |
| strings | `endsWith.2` | 1.74 | 0.98 | 1.8 |
| strings | `lines.line` | 14.79 | 8.55 | 1.7 |
| strings | `toUpperCase` | 17.66 | 10.42 | 1.7 |
| time | `parseIso` | 37.96 | 23.34 | 1.6 |
| json | `stringify` | 603.43 | 391.02 | 1.5 |
| strings | `equal` | 4.83 | 3.14 | 1.5 |
| sha256 | `digest.55` | 426.81 | 282.52 | 1.5 |
| strings | `slice` | 1.94 | 1.30 | 1.5 |
| sha256 | `digest.4k` | 25897.87 | 17336.98 | 1.5 |
| strings | `startsWith.1` | 1.71 | 1.15 | 1.5 |
| strings | `startsWith.8` | 1.68 | 1.17 | 1.4 |
| strings | `less` | 4.84 | 3.46 | 1.4 |
| http | `exchange.small` | 7531.66 | 5624.50 | 1.3 |
| json | `encode` | 267.65 | 218.24 | 1.2 |
| sys | `listDir.65` | 11028.62 | 9023.56 | 1.2 |
| strings | `join.line` | 5.35 | 4.47 | 1.2 |
| strings | `replaceAll.1` | 54.69 | 48.83 | 1.1 |
| time | `dateOf.daysOf` | 19.14 | 17.32 | 1.1 |
| time | `toDateTime` | 3.70 | 3.37 | 1.1 |
| net | `roundtrip.64` | 6119.19 | 5849.24 | 1.0 |
| net | `connect` | 24298.15 | 23301.30 | 1.0 |
| strings | `toString.i64` | 11.38 | 11.18 | 1.0 |
| sys | `writeFile.4k` | 3252.02 | 3254.68 | 1.0 |
| sys | `exists` | 663.24 | 709.66 | 0.9 |
| net | `send.64k` | 11198.48 | 12238.83 | 0.9 |
| sha256 | `hex.32` | 21.11 | 33.12 | 0.6 |
