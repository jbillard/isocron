# Every form against the machine

Each form of the language measured in Isocron and as the same work in C
written the cheapest way known (its floor), on one CPU at fixed frequency,
the median of 5 runs of 200 rounds (`bench/primitives/run.py`); the
nanoseconds of one call, ranked by Isocron over its floor.

| group | form | Isocron (ns) | floor (ns) | Isocron / floor |
| :-- | :-- | --: | --: | --: |
| time | `now` | 150.48 | 17.73 | 8.5 |
| time | `formatIso` | 314.00 | 45.80 | 6.9 |
| json | `parse` | 1097.42 | 168.41 | 6.5 |
| json | `decode` | 542.14 | 86.84 | 6.2 |
| strings | `padStart` | 11.60 | 2.01 | 5.8 |
| sys | `normalize` | 228.61 | 50.44 | 4.5 |
| sys | `env` | 47.71 | 11.34 | 4.2 |
| strings | `template` | 36.17 | 10.59 | 3.4 |
| strings | `concat` | 9.23 | 3.14 | 2.9 |
| http | `exchange.chunked64` | 49531.59 | 20218.07 | 2.4 |
| strings | `split.6` | 75.72 | 32.04 | 2.4 |
| strings | `trim` | 14.10 | 6.01 | 2.3 |
| sys | `readFile.4k` | 4306.20 | 1922.61 | 2.2 |
| strings | `indexOf.1` | 6.73 | 3.09 | 2.2 |
| strings | `includes.3` | 25.07 | 12.56 | 2.0 |
| strings | `parseI64.7` | 9.14 | 4.72 | 1.9 |
| strings | `bytes` | 1.63 | 0.86 | 1.9 |
| strings | `endsWith.2` | 1.79 | 0.95 | 1.9 |
| sys | `basename` | 9.07 | 4.84 | 1.9 |
| strings | `lines.line` | 14.73 | 8.45 | 1.7 |
| strings | `toUpperCase` | 17.63 | 10.47 | 1.7 |
| time | `parseIso` | 37.90 | 23.31 | 1.6 |
| strings | `startsWith.1` | 1.76 | 1.12 | 1.6 |
| strings | `slice` | 2.00 | 1.28 | 1.6 |
| strings | `equal` | 4.83 | 3.13 | 1.5 |
| http | `serve.pipelined` | 1437.96 | 934.06 | 1.5 |
| sha256 | `digest.55` | 425.77 | 283.34 | 1.5 |
| strings | `startsWith.8` | 1.73 | 1.16 | 1.5 |
| sha256 | `digest.4k` | 25807.10 | 17371.29 | 1.5 |
| json | `stringify` | 568.77 | 391.47 | 1.5 |
| strings | `less` | 4.86 | 3.45 | 1.4 |
| http | `exchange.small` | 7332.61 | 5671.96 | 1.3 |
| sys | `listDir.65` | 10994.49 | 9032.43 | 1.2 |
| strings | `join.line` | 5.28 | 4.46 | 1.2 |
| time | `dateOf.daysOf` | 19.14 | 17.29 | 1.1 |
| time | `toDateTime` | 3.70 | 3.36 | 1.1 |
| strings | `replaceAll.1` | 53.48 | 48.76 | 1.1 |
| net | `roundtrip.64` | 6270.86 | 5895.48 | 1.1 |
| https | `get.small` | 1249892.76 | 1184228.82 | 1.1 |
| net | `connect` | 24514.81 | 23424.90 | 1.0 |
| strings | `toString.i64` | 11.47 | 11.21 | 1.0 |
| https | `get.64k` | 1256286.84 | 1233765.55 | 1.0 |
| sys | `writeFile.4k` | 3267.76 | 3242.48 | 1.0 |
| sys | `exists` | 663.55 | 697.74 | 1.0 |
| net | `send.64k` | 11383.58 | 12286.53 | 0.9 |
| json | `encode` | 186.32 | 217.73 | 0.9 |
| sha256 | `hex.32` | 20.88 | 32.46 | 0.6 |
