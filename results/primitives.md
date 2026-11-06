# Every form against the machine

Each form of the language measured in Isocron and as the same work in C
written the cheapest way known (its floor), on one CPU at fixed frequency,
the median of 5 runs of 200 rounds (`bench/primitives/run.py`); the
nanoseconds of one call, ranked by Isocron over its floor.

| group | form | Isocron (ns) | floor (ns) | Isocron / floor |
| :-- | :-- | --: | --: | --: |
| json | `parse` | 1006.20 | 167.53 | 6.0 |
| json | `decode` | 459.41 | 86.56 | 5.3 |
| strings | `padStart` | 10.21 | 1.99 | 5.1 |
| sys | `env` | 40.52 | 11.35 | 3.6 |
| sys | `normalize` | 165.41 | 51.52 | 3.2 |
| strings | `split.6` | 77.52 | 32.06 | 2.4 |
| http | `exchange.chunked64` | 48669.11 | 20143.53 | 2.4 |
| strings | `trim` | 14.04 | 6.00 | 2.3 |
| sys | `readFile.4k` | 4240.59 | 1874.34 | 2.3 |
| strings | `indexOf.1` | 6.72 | 3.08 | 2.2 |
| strings | `includes.3` | 25.39 | 12.57 | 2.0 |
| strings | `concat` | 6.28 | 3.13 | 2.0 |
| strings | `endsWith.2` | 1.74 | 0.90 | 1.9 |
| strings | `bytes` | 1.59 | 0.83 | 1.9 |
| sys | `basename` | 9.05 | 4.82 | 1.9 |
| strings | `parseI64.7` | 8.81 | 4.72 | 1.9 |
| time | `formatIso` | 82.08 | 45.67 | 1.8 |
| http | `serveAction.pipelined` | 1703.28 | 962.10 | 1.8 |
| strings | `lines.line` | 14.80 | 8.47 | 1.7 |
| strings | `toUpperCase` | 17.64 | 10.49 | 1.7 |
| time | `parseIso` | 37.00 | 23.32 | 1.6 |
| strings | `startsWith.1` | 1.71 | 1.11 | 1.5 |
| strings | `equal` | 4.83 | 3.13 | 1.5 |
| strings | `slice` | 1.93 | 1.25 | 1.5 |
| sha256 | `digest.55` | 425.25 | 283.11 | 1.5 |
| strings | `startsWith.8` | 1.68 | 1.13 | 1.5 |
| sha256 | `digest.4k` | 25775.50 | 17319.13 | 1.5 |
| http | `serve.pipelined` | 1369.66 | 966.30 | 1.4 |
| strings | `less` | 4.85 | 3.45 | 1.4 |
| strings | `template` | 14.77 | 10.62 | 1.4 |
| http | `exchange.small` | 7517.48 | 5749.13 | 1.3 |
| sys | `listDir.65` | 10872.42 | 8914.76 | 1.2 |
| strings | `join.line` | 5.31 | 4.45 | 1.2 |
| time | `now` | 20.70 | 17.66 | 1.2 |
| json | `stringify` | 451.50 | 390.23 | 1.2 |
| strings | `replaceAll.1` | 54.30 | 48.77 | 1.1 |
| time | `dateOf.daysOf` | 19.15 | 17.30 | 1.1 |
| time | `toDateTime` | 3.70 | 3.36 | 1.1 |
| net | `roundtrip.64` | 6401.79 | 6025.27 | 1.1 |
| https | `get.small` | 1246355.72 | 1186507.30 | 1.1 |
| net | `connect` | 24838.40 | 23815.41 | 1.0 |
| https | `get.64k` | 1254537.18 | 1231134.15 | 1.0 |
| strings | `toString.i64` | 11.39 | 11.18 | 1.0 |
| sys | `writeFile.4k` | 3107.97 | 3073.18 | 1.0 |
| sys | `exists` | 665.72 | 690.00 | 1.0 |
| net | `send.64k` | 11514.24 | 12390.58 | 0.9 |
| sha256 | `hex.32` | 20.48 | 33.02 | 0.6 |
| json | `encode` | 130.06 | 218.08 | 0.6 |
