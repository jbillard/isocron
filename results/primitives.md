# Every form against the machine

Each form of the language measured in Isocron and as the same work in C
written the cheapest way known (its floor), on one CPU at fixed frequency,
the median of 5 runs of 200 rounds (`bench/primitives/run.py`); the
nanoseconds of one call, ranked by Isocron over its floor.

| group | form | Isocron (ns) | floor (ns) | Isocron / floor |
| :-- | :-- | --: | --: | --: |
| json | `parse` | 1006.36 | 167.90 | 6.0 |
| json | `decode` | 459.87 | 86.75 | 5.3 |
| strings | `padStart` | 10.01 | 1.98 | 5.0 |
| sys | `normalize` | 167.60 | 50.31 | 3.3 |
| strings | `split.6` | 77.47 | 32.04 | 2.4 |
| http | `exchange.chunked64` | 48580.01 | 20474.90 | 2.4 |
| strings | `trim` | 14.10 | 5.99 | 2.4 |
| sys | `readFile.4k` | 4270.84 | 1884.97 | 2.3 |
| strings | `bytes` | 1.70 | 0.83 | 2.0 |
| strings | `indexOf.1` | 6.72 | 3.30 | 2.0 |
| strings | `concat` | 6.23 | 3.10 | 2.0 |
| strings | `includes.3` | 25.11 | 12.57 | 2.0 |
| strings | `endsWith.2` | 1.86 | 0.93 | 2.0 |
| sys | `basename` | 9.04 | 4.83 | 1.9 |
| sys | `env` | 21.17 | 11.34 | 1.9 |
| strings | `parseI64.7` | 8.80 | 4.72 | 1.9 |
| time | `formatIso` | 82.81 | 45.72 | 1.8 |
| http | `serveAction.pipelined` | 1702.26 | 955.35 | 1.8 |
| strings | `lines.line` | 14.83 | 8.48 | 1.7 |
| strings | `toUpperCase` | 17.64 | 10.46 | 1.7 |
| strings | `slice` | 2.06 | 1.27 | 1.6 |
| strings | `startsWith.1` | 1.80 | 1.12 | 1.6 |
| time | `parseIso` | 37.23 | 23.31 | 1.6 |
| strings | `startsWith.8` | 1.78 | 1.14 | 1.6 |
| strings | `equal` | 4.83 | 3.13 | 1.5 |
| sha256 | `digest.55` | 425.03 | 282.82 | 1.5 |
| sha256 | `digest.4k` | 25751.00 | 17353.38 | 1.5 |
| http | `serve.pipelined` | 1371.71 | 956.69 | 1.4 |
| strings | `less` | 4.85 | 3.45 | 1.4 |
| strings | `template` | 14.42 | 10.54 | 1.4 |
| http | `exchange.small` | 7516.36 | 5732.24 | 1.3 |
| sys | `listDir.65` | 10976.49 | 8970.04 | 1.2 |
| strings | `join.line` | 5.35 | 4.48 | 1.2 |
| time | `now` | 20.60 | 17.73 | 1.2 |
| json | `stringify` | 451.60 | 389.73 | 1.2 |
| strings | `replaceAll.1` | 54.19 | 48.79 | 1.1 |
| time | `dateOf.daysOf` | 19.15 | 17.28 | 1.1 |
| time | `toDateTime` | 3.70 | 3.36 | 1.1 |
| net | `roundtrip.64` | 6398.47 | 6065.83 | 1.1 |
| https | `get.small` | 1241795.06 | 1182720.91 | 1.0 |
| net | `connect` | 24787.62 | 23952.96 | 1.0 |
| https | `get.64k` | 1252502.79 | 1228071.84 | 1.0 |
| strings | `toString.i64` | 11.32 | 11.23 | 1.0 |
| sys | `writeFile.4k` | 3116.59 | 3115.67 | 1.0 |
| sys | `exists` | 669.71 | 689.83 | 1.0 |
| net | `send.64k` | 11435.85 | 12516.41 | 0.9 |
| sha256 | `hex.32` | 20.73 | 32.44 | 0.6 |
| json | `encode` | 129.67 | 217.84 | 0.6 |
