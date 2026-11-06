# Every form against the machine

Each form of the language measured in Isocron and as the same work in C
written the cheapest way known (its floor), on one CPU at fixed frequency,
the median of 5 runs of 200 rounds (`bench/primitives/run.py`); the
nanoseconds of one call, ranked by Isocron over its floor.

| group | form | Isocron (ns) | floor (ns) | Isocron / floor |
| :-- | :-- | --: | --: | --: |
| json | `parse` | 1120.94 | 167.59 | 6.7 |
| json | `decode` | 526.05 | 87.11 | 6.0 |
| strings | `padStart` | 11.78 | 2.09 | 5.6 |
| sys | `env` | 47.15 | 11.34 | 4.2 |
| sys | `normalize` | 201.42 | 51.78 | 3.9 |
| strings | `concat` | 9.20 | 3.16 | 2.9 |
| time | `formatIso` | 122.41 | 45.70 | 2.7 |
| http | `exchange.chunked64` | 47845.24 | 19643.80 | 2.4 |
| strings | `template` | 25.58 | 10.54 | 2.4 |
| strings | `trim` | 14.29 | 6.00 | 2.4 |
| strings | `split.6` | 75.22 | 32.06 | 2.3 |
| sys | `readFile.4k` | 4331.52 | 1916.70 | 2.3 |
| strings | `indexOf.1` | 6.73 | 3.08 | 2.2 |
| strings | `endsWith.2` | 1.87 | 0.92 | 2.0 |
| strings | `bytes` | 1.67 | 0.83 | 2.0 |
| strings | `includes.3` | 25.09 | 12.57 | 2.0 |
| http | `serveAction.pipelined` | 1812.75 | 936.11 | 1.9 |
| strings | `parseI64.7` | 9.13 | 4.72 | 1.9 |
| sys | `basename` | 9.06 | 4.83 | 1.9 |
| strings | `lines.line` | 14.90 | 8.49 | 1.8 |
| strings | `toUpperCase` | 17.65 | 10.42 | 1.7 |
| time | `parseIso` | 38.71 | 23.35 | 1.7 |
| strings | `slice` | 2.04 | 1.27 | 1.6 |
| strings | `startsWith.1` | 1.82 | 1.14 | 1.6 |
| http | `serve.pipelined` | 1486.44 | 937.52 | 1.6 |
| strings | `startsWith.8` | 1.79 | 1.13 | 1.6 |
| strings | `equal` | 4.85 | 3.13 | 1.5 |
| sha256 | `digest.55` | 426.05 | 282.65 | 1.5 |
| sha256 | `digest.4k` | 26000.65 | 17314.55 | 1.5 |
| json | `stringify` | 577.07 | 391.18 | 1.5 |
| strings | `less` | 4.86 | 3.46 | 1.4 |
| http | `exchange.small` | 7409.59 | 5634.52 | 1.3 |
| sys | `listDir.65` | 11043.66 | 9066.25 | 1.2 |
| strings | `join.line` | 5.40 | 4.49 | 1.2 |
| time | `now` | 20.70 | 17.64 | 1.2 |
| strings | `replaceAll.1` | 54.58 | 48.77 | 1.1 |
| time | `dateOf.daysOf` | 19.17 | 17.30 | 1.1 |
| time | `toDateTime` | 3.70 | 3.36 | 1.1 |
| net | `roundtrip.64` | 6270.86 | 5884.10 | 1.1 |
| net | `connect` | 24508.94 | 23205.39 | 1.1 |
| https | `get.small` | 1250154.90 | 1198968.50 | 1.0 |
| https | `get.64k` | 1256922.60 | 1226612.55 | 1.0 |
| strings | `toString.i64` | 11.46 | 11.26 | 1.0 |
| sys | `writeFile.4k` | 3274.51 | 3240.38 | 1.0 |
| sys | `exists` | 669.24 | 697.85 | 1.0 |
| net | `send.64k` | 11256.17 | 12124.45 | 0.9 |
| json | `encode` | 185.59 | 217.66 | 0.9 |
| sha256 | `hex.32` | 20.68 | 33.10 | 0.6 |
