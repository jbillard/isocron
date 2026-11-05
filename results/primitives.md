# Every form against the machine

Each form of the language measured in Isocron and as the same work in C
written the cheapest way known (its floor), on one CPU at fixed frequency,
the median of 5 runs of 200 rounds (`bench/primitives/run.py`); the
nanoseconds of one call, ranked by Isocron over its floor.

| group | form | Isocron (ns) | floor (ns) | Isocron / floor |
| :-- | :-- | --: | --: | --: |
| time | `now` | 150.38 | 17.64 | 8.5 |
| json | `parse` | 1098.80 | 168.70 | 6.5 |
| json | `decode` | 542.51 | 86.87 | 6.2 |
| strings | `padStart` | 12.08 | 2.02 | 6.0 |
| time | `formatIso` | 216.46 | 45.77 | 4.7 |
| sys | `normalize` | 229.85 | 51.41 | 4.5 |
| sys | `env` | 46.08 | 11.36 | 4.1 |
| strings | `template` | 36.12 | 10.59 | 3.4 |
| strings | `concat` | 9.18 | 3.15 | 2.9 |
| http | `exchange.chunked64` | 49523.90 | 20107.78 | 2.5 |
| strings | `split.6` | 76.72 | 32.04 | 2.4 |
| strings | `trim` | 14.09 | 6.00 | 2.3 |
| sys | `readFile.4k` | 4291.94 | 1896.68 | 2.3 |
| strings | `indexOf.1` | 6.72 | 3.09 | 2.2 |
| strings | `includes.3` | 25.20 | 12.57 | 2.0 |
| strings | `parseI64.7` | 9.12 | 4.61 | 2.0 |
| strings | `endsWith.2` | 1.79 | 0.93 | 1.9 |
| strings | `bytes` | 1.64 | 0.86 | 1.9 |
| sys | `basename` | 9.07 | 4.83 | 1.9 |
| strings | `lines.line` | 14.79 | 8.45 | 1.7 |
| strings | `toUpperCase` | 17.65 | 10.45 | 1.7 |
| time | `parseIso` | 37.43 | 23.33 | 1.6 |
| strings | `slice` | 1.98 | 1.27 | 1.6 |
| http | `serve.pipelined` | 1439.35 | 933.81 | 1.5 |
| strings | `equal` | 4.83 | 3.13 | 1.5 |
| strings | `startsWith.1` | 1.75 | 1.14 | 1.5 |
| strings | `startsWith.8` | 1.73 | 1.14 | 1.5 |
| sha256 | `digest.55` | 425.45 | 282.94 | 1.5 |
| sha256 | `digest.4k` | 25972.31 | 17353.38 | 1.5 |
| json | `stringify` | 567.08 | 391.13 | 1.4 |
| strings | `less` | 4.85 | 3.45 | 1.4 |
| http | `exchange.small` | 7285.47 | 5699.45 | 1.3 |
| sys | `listDir.65` | 11026.34 | 9025.03 | 1.2 |
| strings | `join.line` | 5.27 | 4.47 | 1.2 |
| time | `dateOf.daysOf` | 19.13 | 17.31 | 1.1 |
| time | `toDateTime` | 3.70 | 3.36 | 1.1 |
| strings | `replaceAll.1` | 53.56 | 48.74 | 1.1 |
| net | `roundtrip.64` | 6207.07 | 5885.30 | 1.1 |
| https | `get.small` | 1238345.36 | 1183687.99 | 1.0 |
| net | `connect` | 24256.48 | 23308.63 | 1.0 |
| https | `get.64k` | 1255747.94 | 1229052.81 | 1.0 |
| strings | `toString.i64` | 11.29 | 11.23 | 1.0 |
| sys | `writeFile.4k` | 3237.54 | 3250.01 | 1.0 |
| sys | `exists` | 662.66 | 706.84 | 0.9 |
| net | `send.64k` | 11245.79 | 12285.39 | 0.9 |
| json | `encode` | 187.98 | 218.20 | 0.9 |
| sha256 | `hex.32` | 21.14 | 32.84 | 0.6 |
