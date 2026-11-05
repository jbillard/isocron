# Every form against the machine

Each form of the language measured in Isocron and as the same work in C
written the cheapest way known (its floor), on one CPU at fixed frequency,
the median of 5 runs of 200 rounds (`bench/primitives/run.py`); the
nanoseconds of one call, ranked by Isocron over its floor.

| group | form | Isocron (ns) | floor (ns) | Isocron / floor |
| :-- | :-- | --: | --: | --: |
| time | `now` | 150.76 | 17.68 | 8.5 |
| time | `formatIso` | 321.05 | 45.78 | 7.0 |
| sys | `normalize` | 335.43 | 50.06 | 6.7 |
| json | `parse` | 1091.21 | 167.86 | 6.5 |
| json | `decode` | 537.17 | 86.69 | 6.2 |
| strings | `padStart` | 11.69 | 2.05 | 5.7 |
| sys | `basename` | 25.64 | 4.83 | 5.3 |
| strings | `template` | 36.25 | 10.55 | 3.4 |
| sys | `env` | 58.67 | 17.72 | 3.3 |
| strings | `concat` | 9.19 | 3.14 | 2.9 |
| http | `exchange.chunked64` | 49843.86 | 19913.90 | 2.5 |
| strings | `trim` | 14.15 | 6.00 | 2.4 |
| strings | `split.6` | 75.28 | 32.02 | 2.4 |
| sys | `readFile.4k` | 4336.71 | 1924.81 | 2.3 |
| strings | `indexOf.1` | 6.73 | 3.08 | 2.2 |
| strings | `bytes` | 1.75 | 0.86 | 2.0 |
| strings | `includes.3` | 25.11 | 12.56 | 2.0 |
| strings | `parseI64.7` | 9.12 | 4.62 | 2.0 |
| strings | `endsWith.2` | 1.92 | 0.99 | 1.9 |
| strings | `lines.line` | 14.83 | 8.50 | 1.7 |
| strings | `toUpperCase` | 17.64 | 10.47 | 1.7 |
| strings | `slice` | 2.11 | 1.27 | 1.7 |
| time | `parseIso` | 38.15 | 23.31 | 1.6 |
| strings | `startsWith.1` | 1.86 | 1.14 | 1.6 |
| strings | `startsWith.8` | 1.84 | 1.19 | 1.5 |
| http | `serve.pipelined` | 1445.20 | 935.24 | 1.5 |
| strings | `equal` | 4.83 | 3.14 | 1.5 |
| sha256 | `digest.55` | 425.39 | 281.98 | 1.5 |
| sha256 | `digest.4k` | 25903.28 | 17306.40 | 1.5 |
| json | `stringify` | 578.38 | 390.77 | 1.5 |
| strings | `less` | 4.84 | 3.47 | 1.4 |
| http | `exchange.small` | 7422.94 | 5668.38 | 1.3 |
| sys | `listDir.65` | 11023.75 | 9046.56 | 1.2 |
| strings | `join.line` | 5.35 | 4.47 | 1.2 |
| time | `dateOf.daysOf` | 19.14 | 17.30 | 1.1 |
| time | `toDateTime` | 3.70 | 3.37 | 1.1 |
| strings | `replaceAll.1` | 53.57 | 48.79 | 1.1 |
| net | `roundtrip.64` | 6226.78 | 5895.62 | 1.1 |
| https | `get.small` | 1258100.13 | 1197391.62 | 1.1 |
| net | `connect` | 24258.04 | 23288.81 | 1.0 |
| https | `get.64k` | 1268887.65 | 1237461.42 | 1.0 |
| sys | `writeFile.4k` | 3278.27 | 3246.86 | 1.0 |
| strings | `toString.i64` | 11.13 | 11.18 | 1.0 |
| sys | `exists` | 668.38 | 690.34 | 1.0 |
| net | `send.64k` | 11161.94 | 12305.31 | 0.9 |
| json | `encode` | 183.31 | 217.90 | 0.8 |
| sha256 | `hex.32` | 21.05 | 32.97 | 0.6 |
