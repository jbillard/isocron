# Every form against the machine

Each form of the language measured in Isocron and as the same work in C
written the cheapest way known (its floor), on one CPU at fixed frequency,
the median of 5 runs of 200 rounds (`bench/primitives/run.py`); the
nanoseconds of one call, ranked by Isocron over its floor.

| group | form | Isocron (ns) | floor (ns) | Isocron / floor |
| :-- | :-- | --: | --: | --: |
| time | `now` | 150.96 | 17.73 | 8.5 |
| json | `parse` | 1097.87 | 168.02 | 6.5 |
| json | `decode` | 540.59 | 86.68 | 6.2 |
| strings | `padStart` | 12.02 | 2.04 | 5.9 |
| time | `formatIso` | 214.51 | 45.69 | 4.7 |
| sys | `normalize` | 208.28 | 50.69 | 4.1 |
| sys | `env` | 45.40 | 11.45 | 4.0 |
| strings | `template` | 32.74 | 10.58 | 3.1 |
| strings | `concat` | 9.33 | 3.13 | 3.0 |
| http | `exchange.chunked64` | 49702.32 | 20256.68 | 2.5 |
| strings | `split.6` | 75.87 | 32.04 | 2.4 |
| strings | `trim` | 14.09 | 6.01 | 2.3 |
| sys | `readFile.4k` | 4301.29 | 1913.12 | 2.2 |
| strings | `indexOf.1` | 6.74 | 3.11 | 2.2 |
| strings | `includes.3` | 25.14 | 12.72 | 2.0 |
| strings | `endsWith.2` | 1.90 | 0.98 | 1.9 |
| strings | `bytes` | 1.72 | 0.89 | 1.9 |
| sys | `basename` | 9.26 | 4.83 | 1.9 |
| http | `serveAction.pipelined` | 1764.63 | 937.09 | 1.9 |
| strings | `parseI64.7` | 9.13 | 5.00 | 1.8 |
| strings | `lines.line` | 14.86 | 8.48 | 1.8 |
| strings | `toUpperCase` | 17.66 | 10.43 | 1.7 |
| strings | `slice` | 2.16 | 1.29 | 1.7 |
| strings | `startsWith.1` | 1.85 | 1.14 | 1.6 |
| time | `parseIso` | 37.66 | 23.36 | 1.6 |
| http | `serve.pipelined` | 1474.56 | 940.54 | 1.6 |
| strings | `startsWith.8` | 1.83 | 1.18 | 1.5 |
| strings | `equal` | 4.85 | 3.14 | 1.5 |
| sha256 | `digest.55` | 426.02 | 282.83 | 1.5 |
| sha256 | `digest.4k` | 25799.76 | 17337.27 | 1.5 |
| json | `stringify` | 556.00 | 391.26 | 1.4 |
| strings | `less` | 4.90 | 3.46 | 1.4 |
| http | `exchange.small` | 7372.77 | 5744.58 | 1.3 |
| sys | `listDir.65` | 11005.50 | 9032.33 | 1.2 |
| strings | `join.line` | 5.28 | 4.42 | 1.2 |
| strings | `replaceAll.1` | 53.99 | 48.83 | 1.1 |
| time | `dateOf.daysOf` | 19.13 | 17.31 | 1.1 |
| time | `toDateTime` | 3.70 | 3.37 | 1.1 |
| https | `get.small` | 1256498.37 | 1192157.65 | 1.1 |
| net | `roundtrip.64` | 6181.60 | 5905.90 | 1.0 |
| net | `connect` | 24288.34 | 23298.59 | 1.0 |
| https | `get.64k` | 1268311.24 | 1234335.62 | 1.0 |
| strings | `toString.i64` | 11.30 | 11.19 | 1.0 |
| sys | `writeFile.4k` | 3277.03 | 3251.43 | 1.0 |
| sys | `exists` | 672.86 | 698.37 | 1.0 |
| net | `send.64k` | 11232.60 | 12233.13 | 0.9 |
| json | `encode` | 186.47 | 218.15 | 0.9 |
| sha256 | `hex.32` | 20.71 | 32.45 | 0.6 |
