# Every form against the machine

Each form of the language measured in Isocron and as the same work in C
written the cheapest way known (its floor), on one CPU at fixed frequency,
the median of 5 runs of 200 rounds (`bench/primitives/run.py`); the
nanoseconds of one call, ranked by Isocron over its floor.

| group | form | Isocron (ns) | floor (ns) | Isocron / floor |
| :-- | :-- | --: | --: | --: |
| json | `parse` | 1017.83 | 168.17 | 6.1 |
| strings | `padStart` | 11.84 | 1.99 | 6.0 |
| json | `decode` | 470.66 | 87.30 | 5.4 |
| sys | `env` | 46.83 | 11.35 | 4.1 |
| sys | `normalize` | 203.07 | 50.55 | 4.0 |
| strings | `concat` | 9.26 | 3.14 | 3.0 |
| http | `exchange.chunked64` | 48056.76 | 19952.94 | 2.4 |
| strings | `trim` | 14.16 | 6.01 | 2.4 |
| strings | `split.6` | 75.37 | 32.05 | 2.4 |
| sys | `readFile.4k` | 4292.09 | 1912.27 | 2.2 |
| strings | `indexOf.1` | 6.75 | 3.09 | 2.2 |
| strings | `parseI64.7` | 9.11 | 4.52 | 2.0 |
| strings | `includes.3` | 25.09 | 12.56 | 2.0 |
| strings | `endsWith.2` | 1.91 | 0.98 | 2.0 |
| strings | `bytes` | 1.74 | 0.90 | 1.9 |
| sys | `basename` | 9.07 | 4.83 | 1.9 |
| http | `serveAction.pipelined` | 1694.02 | 934.07 | 1.8 |
| time | `formatIso` | 81.90 | 45.63 | 1.8 |
| strings | `lines.line` | 14.91 | 8.47 | 1.8 |
| strings | `toUpperCase` | 17.66 | 10.45 | 1.7 |
| time | `parseIso` | 39.02 | 23.34 | 1.7 |
| strings | `startsWith.1` | 1.86 | 1.15 | 1.6 |
| strings | `slice` | 2.09 | 1.30 | 1.6 |
| strings | `startsWith.8` | 1.83 | 1.17 | 1.6 |
| strings | `equal` | 4.83 | 3.13 | 1.5 |
| sha256 | `digest.55` | 424.49 | 283.07 | 1.5 |
| sha256 | `digest.4k` | 25851.04 | 17325.74 | 1.5 |
| json | `stringify` | 573.65 | 391.66 | 1.5 |
| http | `serve.pipelined` | 1363.77 | 933.69 | 1.5 |
| strings | `less` | 4.84 | 3.45 | 1.4 |
| strings | `template` | 14.80 | 10.56 | 1.4 |
| http | `exchange.small` | 7440.36 | 5647.78 | 1.3 |
| sys | `listDir.65` | 10986.89 | 9049.17 | 1.2 |
| strings | `join.line` | 5.34 | 4.46 | 1.2 |
| time | `now` | 20.70 | 17.69 | 1.2 |
| strings | `replaceAll.1` | 54.21 | 48.75 | 1.1 |
| time | `dateOf.daysOf` | 19.18 | 17.30 | 1.1 |
| time | `toDateTime` | 3.70 | 3.37 | 1.1 |
| net | `roundtrip.64` | 6227.12 | 5862.72 | 1.1 |
| https | `get.small` | 1248056.93 | 1189003.68 | 1.0 |
| net | `connect` | 24223.84 | 23236.96 | 1.0 |
| strings | `toString.i64` | 11.39 | 11.18 | 1.0 |
| https | `get.64k` | 1271505.54 | 1254098.16 | 1.0 |
| sys | `writeFile.4k` | 3290.53 | 3255.59 | 1.0 |
| sys | `exists` | 669.30 | 695.22 | 1.0 |
| net | `send.64k` | 11183.66 | 12238.71 | 0.9 |
| json | `encode` | 181.04 | 218.53 | 0.8 |
| sha256 | `hex.32` | 20.57 | 32.96 | 0.6 |
