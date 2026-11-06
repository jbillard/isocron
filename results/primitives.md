# Every form against the machine

Each form of the language measured in Isocron and as the same work in C
written the cheapest way known (its floor), on one CPU at fixed frequency,
the median of 5 runs of 200 rounds (`bench/primitives/run.py`); the
nanoseconds of one call, ranked by Isocron over its floor.

| group | form | Isocron (ns) | floor (ns) | Isocron / floor |
| :-- | :-- | --: | --: | --: |
| time | `now` | 149.72 | 17.66 | 8.5 |
| json | `parse` | 1141.17 | 168.15 | 6.8 |
| json | `decode` | 538.13 | 87.01 | 6.2 |
| strings | `padStart` | 11.86 | 1.98 | 6.0 |
| sys | `normalize` | 210.20 | 50.87 | 4.1 |
| sys | `env` | 46.53 | 11.34 | 4.1 |
| strings | `concat` | 10.00 | 3.07 | 3.3 |
| time | `formatIso` | 129.29 | 45.71 | 2.8 |
| strings | `template` | 25.59 | 10.59 | 2.4 |
| strings | `split.6` | 76.49 | 32.06 | 2.4 |
| strings | `trim` | 14.16 | 6.01 | 2.4 |
| http | `exchange.chunked64` | 47925.50 | 20370.79 | 2.4 |
| sys | `readFile.4k` | 4323.14 | 1922.36 | 2.2 |
| strings | `indexOf.1` | 6.72 | 3.08 | 2.2 |
| strings | `endsWith.2` | 1.91 | 0.90 | 2.1 |
| strings | `bytes` | 1.72 | 0.83 | 2.1 |
| strings | `parseI64.7` | 9.12 | 4.53 | 2.0 |
| strings | `includes.3` | 25.22 | 12.54 | 2.0 |
| http | `serveAction.pipelined` | 1804.42 | 933.32 | 1.9 |
| sys | `basename` | 9.16 | 4.83 | 1.9 |
| strings | `lines.line` | 14.85 | 8.47 | 1.8 |
| strings | `startsWith.1` | 1.86 | 1.10 | 1.7 |
| strings | `toUpperCase` | 17.63 | 10.47 | 1.7 |
| strings | `slice` | 2.08 | 1.25 | 1.7 |
| time | `parseIso` | 38.34 | 23.31 | 1.6 |
| strings | `startsWith.8` | 1.83 | 1.13 | 1.6 |
| http | `serve.pipelined` | 1510.32 | 934.85 | 1.6 |
| strings | `equal` | 4.84 | 3.13 | 1.5 |
| sha256 | `digest.55` | 425.50 | 283.28 | 1.5 |
| sha256 | `digest.4k` | 25979.67 | 17300.89 | 1.5 |
| json | `stringify` | 573.09 | 390.98 | 1.5 |
| strings | `less` | 4.86 | 3.45 | 1.4 |
| http | `exchange.small` | 7398.77 | 5657.44 | 1.3 |
| sys | `listDir.65` | 11028.49 | 9062.24 | 1.2 |
| strings | `join.line` | 5.30 | 4.47 | 1.2 |
| strings | `replaceAll.1` | 54.70 | 48.79 | 1.1 |
| time | `dateOf.daysOf` | 19.16 | 17.30 | 1.1 |
| time | `toDateTime` | 3.70 | 3.36 | 1.1 |
| https | `get.small` | 1268918.08 | 1209001.29 | 1.0 |
| net | `roundtrip.64` | 6157.02 | 5891.69 | 1.0 |
| net | `connect` | 24145.73 | 23217.15 | 1.0 |
| strings | `toString.i64` | 11.42 | 11.22 | 1.0 |
| https | `get.64k` | 1262201.33 | 1245043.80 | 1.0 |
| sys | `writeFile.4k` | 3251.28 | 3257.34 | 1.0 |
| sys | `exists` | 678.89 | 693.43 | 1.0 |
| net | `send.64k` | 11181.07 | 12190.08 | 0.9 |
| json | `encode` | 188.27 | 218.26 | 0.9 |
| sha256 | `hex.32` | 21.14 | 33.02 | 0.6 |
