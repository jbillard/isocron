# Every form against the machine

Each form of the language measured in Isocron and as the same work in C
written the cheapest way known (its floor), on one CPU at fixed frequency,
the median of 5 runs of 200 rounds (`bench/primitives/run.py`); the
nanoseconds of one call, ranked by Isocron over its floor.

| group | form | Isocron (ns) | floor (ns) | Isocron / floor |
| :-- | :-- | --: | --: | --: |
| json | `parse` | 1131.37 | 167.62 | 6.7 |
| json | `decode` | 534.19 | 86.58 | 6.2 |
| strings | `padStart` | 11.71 | 1.97 | 5.9 |
| sys | `env` | 48.25 | 11.34 | 4.3 |
| sys | `normalize` | 203.85 | 50.78 | 4.0 |
| strings | `concat` | 9.38 | 3.07 | 3.1 |
| time | `formatIso` | 129.47 | 45.74 | 2.8 |
| strings | `template` | 25.81 | 10.54 | 2.4 |
| strings | `trim` | 14.29 | 6.01 | 2.4 |
| http | `exchange.chunked64` | 48201.24 | 20369.08 | 2.4 |
| strings | `split.6` | 74.65 | 32.02 | 2.3 |
| sys | `readFile.4k` | 4283.44 | 1908.73 | 2.2 |
| strings | `indexOf.1` | 6.72 | 3.09 | 2.2 |
| strings | `bytes` | 1.71 | 0.83 | 2.1 |
| strings | `endsWith.2` | 1.88 | 0.91 | 2.1 |
| strings | `includes.3` | 25.09 | 12.55 | 2.0 |
| strings | `parseI64.7` | 9.12 | 4.59 | 2.0 |
| sys | `basename` | 9.34 | 4.84 | 1.9 |
| http | `serveAction.pipelined` | 1787.39 | 937.68 | 1.9 |
| strings | `lines.line` | 14.77 | 8.45 | 1.7 |
| strings | `toUpperCase` | 17.69 | 10.48 | 1.7 |
| strings | `startsWith.1` | 1.83 | 1.10 | 1.7 |
| time | `parseIso` | 38.01 | 23.30 | 1.6 |
| strings | `slice` | 2.05 | 1.27 | 1.6 |
| strings | `startsWith.8` | 1.80 | 1.13 | 1.6 |
| http | `serve.pipelined` | 1491.87 | 935.84 | 1.6 |
| strings | `equal` | 4.84 | 3.13 | 1.5 |
| sha256 | `digest.55` | 424.75 | 283.35 | 1.5 |
| sha256 | `digest.4k` | 25899.74 | 17345.14 | 1.5 |
| json | `stringify` | 575.86 | 391.42 | 1.5 |
| strings | `less` | 4.85 | 3.45 | 1.4 |
| http | `exchange.small` | 7438.28 | 5722.41 | 1.3 |
| sys | `listDir.65` | 11117.57 | 9032.18 | 1.2 |
| strings | `join.line` | 5.30 | 4.47 | 1.2 |
| time | `now` | 20.70 | 17.56 | 1.2 |
| strings | `replaceAll.1` | 54.80 | 48.77 | 1.1 |
| time | `dateOf.daysOf` | 19.15 | 17.31 | 1.1 |
| time | `toDateTime` | 3.70 | 3.37 | 1.1 |
| net | `roundtrip.64` | 6211.76 | 5897.88 | 1.1 |
| https | `get.small` | 1254319.66 | 1203880.14 | 1.0 |
| net | `connect` | 24235.29 | 23362.58 | 1.0 |
| https | `get.64k` | 1266425.07 | 1232785.26 | 1.0 |
| sys | `writeFile.4k` | 3283.35 | 3239.72 | 1.0 |
| strings | `toString.i64` | 11.24 | 11.20 | 1.0 |
| sys | `exists` | 671.47 | 690.27 | 1.0 |
| net | `send.64k` | 11348.21 | 12438.41 | 0.9 |
| json | `encode` | 184.88 | 217.59 | 0.8 |
| sha256 | `hex.32` | 20.29 | 32.46 | 0.6 |
