# Every form against the machine

Each form of the language measured in Isocron and as the same work in C
written the cheapest way known (its floor), on one CPU at fixed frequency,
the median of 5 runs of 200 rounds (`bench/primitives/run.py`); the
nanoseconds of one call, ranked by Isocron over its floor.

| group | form | Isocron (ns) | floor (ns) | Isocron / floor |
| :-- | :-- | --: | --: | --: |
| json | `parse` | 1017.51 | 167.79 | 6.1 |
| strings | `padStart` | 11.88 | 1.97 | 6.0 |
| json | `decode` | 474.71 | 86.58 | 5.5 |
| sys | `normalize` | 209.57 | 50.45 | 4.2 |
| sys | `env` | 40.90 | 11.35 | 3.6 |
| strings | `concat` | 9.28 | 3.09 | 3.0 |
| strings | `split.6` | 76.62 | 32.08 | 2.4 |
| http | `exchange.chunked64` | 48591.01 | 20429.57 | 2.4 |
| strings | `trim` | 14.12 | 6.00 | 2.4 |
| sys | `readFile.4k` | 4260.51 | 1892.40 | 2.3 |
| strings | `indexOf.1` | 6.72 | 3.17 | 2.1 |
| strings | `includes.3` | 25.28 | 12.55 | 2.0 |
| strings | `parseI64.7` | 9.10 | 4.59 | 2.0 |
| strings | `endsWith.2` | 1.78 | 0.90 | 2.0 |
| strings | `bytes` | 1.62 | 0.83 | 2.0 |
| sys | `basename` | 9.05 | 4.82 | 1.9 |
| time | `formatIso` | 81.81 | 45.68 | 1.8 |
| http | `serveAction.pipelined` | 1716.95 | 962.83 | 1.8 |
| strings | `lines.line` | 14.80 | 8.49 | 1.7 |
| strings | `toUpperCase` | 17.64 | 10.47 | 1.7 |
| time | `parseIso` | 38.23 | 23.30 | 1.6 |
| strings | `startsWith.1` | 1.74 | 1.11 | 1.6 |
| strings | `slice` | 1.95 | 1.25 | 1.6 |
| strings | `equal` | 4.82 | 3.13 | 1.5 |
| strings | `startsWith.8` | 1.72 | 1.13 | 1.5 |
| sha256 | `digest.55` | 424.95 | 282.99 | 1.5 |
| sha256 | `digest.4k` | 25832.83 | 17346.60 | 1.5 |
| json | `stringify` | 576.81 | 390.56 | 1.5 |
| http | `serve.pipelined` | 1387.60 | 967.36 | 1.4 |
| strings | `template` | 15.12 | 10.55 | 1.4 |
| strings | `less` | 4.83 | 3.46 | 1.4 |
| http | `exchange.small` | 7556.99 | 5711.40 | 1.3 |
| sys | `listDir.65` | 10962.60 | 8918.50 | 1.2 |
| strings | `join.line` | 5.31 | 4.44 | 1.2 |
| time | `now` | 20.61 | 17.73 | 1.2 |
| strings | `replaceAll.1` | 54.28 | 48.75 | 1.1 |
| time | `dateOf.daysOf` | 19.16 | 17.30 | 1.1 |
| time | `toDateTime` | 3.70 | 3.35 | 1.1 |
| net | `roundtrip.64` | 6481.04 | 6064.96 | 1.1 |
| https | `get.small` | 1248169.37 | 1192565.70 | 1.0 |
| net | `connect` | 24836.43 | 23850.52 | 1.0 |
| strings | `toString.i64` | 11.50 | 11.22 | 1.0 |
| sys | `writeFile.4k` | 3189.55 | 3115.39 | 1.0 |
| https | `get.64k` | 1261375.53 | 1232670.87 | 1.0 |
| sys | `exists` | 672.05 | 694.50 | 1.0 |
| net | `send.64k` | 11478.77 | 12337.36 | 0.9 |
| json | `encode` | 181.66 | 218.02 | 0.8 |
| sha256 | `hex.32` | 20.39 | 33.05 | 0.6 |
