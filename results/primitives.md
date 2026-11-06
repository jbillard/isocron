# Every form against the machine

Each form of the language measured in Isocron and as the same work in C
written the cheapest way known (its floor), on one CPU at fixed frequency,
the median of 5 runs of 200 rounds (`bench/primitives/run.py`); the
nanoseconds of one call, ranked by Isocron over its floor.

| group | form | Isocron (ns) | floor (ns) | Isocron / floor |
| :-- | :-- | --: | --: | --: |
| json | `parse` | 1007.02 | 168.10 | 6.0 |
| strings | `padStart` | 11.81 | 2.04 | 5.8 |
| json | `decode` | 459.17 | 86.67 | 5.3 |
| sys | `env` | 39.05 | 11.34 | 3.4 |
| sys | `normalize` | 165.78 | 50.51 | 3.3 |
| http | `exchange.chunked64` | 48810.09 | 19790.86 | 2.5 |
| strings | `split.6` | 77.17 | 32.09 | 2.4 |
| strings | `trim` | 14.05 | 5.99 | 2.3 |
| sys | `readFile.4k` | 4265.90 | 1889.61 | 2.3 |
| strings | `indexOf.1` | 6.73 | 3.13 | 2.2 |
| strings | `concat` | 6.27 | 3.10 | 2.0 |
| strings | `includes.3` | 25.14 | 12.56 | 2.0 |
| sys | `basename` | 9.04 | 4.82 | 1.9 |
| strings | `endsWith.2` | 1.74 | 0.94 | 1.9 |
| strings | `bytes` | 1.56 | 0.86 | 1.8 |
| strings | `parseI64.7` | 8.82 | 4.86 | 1.8 |
| time | `formatIso` | 82.18 | 45.69 | 1.8 |
| strings | `lines.line` | 14.94 | 8.46 | 1.8 |
| http | `serveAction.pipelined` | 1701.04 | 963.73 | 1.8 |
| strings | `toUpperCase` | 17.65 | 10.44 | 1.7 |
| time | `parseIso` | 36.94 | 23.31 | 1.6 |
| strings | `equal` | 4.84 | 3.13 | 1.5 |
| strings | `startsWith.1` | 1.70 | 1.12 | 1.5 |
| strings | `slice` | 1.92 | 1.27 | 1.5 |
| sha256 | `digest.55` | 425.31 | 282.72 | 1.5 |
| sha256 | `digest.4k` | 26050.51 | 17329.96 | 1.5 |
| strings | `startsWith.8` | 1.67 | 1.14 | 1.5 |
| http | `serve.pipelined` | 1369.19 | 963.99 | 1.4 |
| strings | `template` | 14.96 | 10.59 | 1.4 |
| strings | `less` | 4.85 | 3.46 | 1.4 |
| http | `exchange.small` | 7544.76 | 5711.60 | 1.3 |
| sys | `listDir.65` | 10946.64 | 8962.84 | 1.2 |
| strings | `join.line` | 5.30 | 4.45 | 1.2 |
| time | `now` | 20.85 | 17.64 | 1.2 |
| json | `stringify` | 451.63 | 390.35 | 1.2 |
| strings | `replaceAll.1` | 54.66 | 48.74 | 1.1 |
| time | `dateOf.daysOf` | 19.15 | 17.31 | 1.1 |
| time | `toDateTime` | 3.71 | 3.36 | 1.1 |
| net | `roundtrip.64` | 6497.90 | 6086.39 | 1.1 |
| https | `get.small` | 1250526.72 | 1187123.45 | 1.1 |
| net | `connect` | 24955.27 | 23782.87 | 1.0 |
| https | `get.64k` | 1261801.47 | 1231155.97 | 1.0 |
| sys | `writeFile.4k` | 3163.20 | 3110.55 | 1.0 |
| strings | `toString.i64` | 11.42 | 11.24 | 1.0 |
| sys | `exists` | 670.69 | 691.29 | 1.0 |
| net | `send.64k` | 11472.15 | 12541.39 | 0.9 |
| sha256 | `hex.32` | 20.61 | 32.44 | 0.6 |
| json | `encode` | 128.17 | 217.37 | 0.6 |
