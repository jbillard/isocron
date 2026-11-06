# Every form against the machine

Each form of the language measured in Isocron and as the same work in C
written the cheapest way known (its floor), on one CPU at fixed frequency,
the median of 5 runs of 200 rounds (`bench/primitives/run.py`); the
nanoseconds of one call, ranked by Isocron over its floor.

| group | form | Isocron (ns) | floor (ns) | Isocron / floor |
| :-- | :-- | --: | --: | --: |
| time | `now` | 150.09 | 17.68 | 8.5 |
| json | `parse` | 1096.83 | 167.53 | 6.5 |
| json | `decode` | 544.53 | 86.48 | 6.3 |
| strings | `padStart` | 11.77 | 2.00 | 5.9 |
| time | `formatIso` | 215.06 | 45.77 | 4.7 |
| sys | `normalize` | 208.55 | 50.54 | 4.1 |
| sys | `env` | 46.07 | 11.34 | 4.1 |
| strings | `template` | 32.79 | 10.55 | 3.1 |
| strings | `concat` | 9.38 | 3.18 | 3.0 |
| http | `exchange.chunked64` | 47846.28 | 20081.38 | 2.4 |
| strings | `split.6` | 76.07 | 32.11 | 2.4 |
| strings | `trim` | 14.13 | 6.01 | 2.4 |
| sys | `readFile.4k` | 4294.28 | 1901.04 | 2.3 |
| strings | `indexOf.1` | 6.74 | 3.19 | 2.1 |
| strings | `includes.3` | 25.04 | 12.60 | 2.0 |
| strings | `endsWith.2` | 1.83 | 0.94 | 1.9 |
| strings | `bytes` | 1.65 | 0.85 | 1.9 |
| sys | `basename` | 9.14 | 4.83 | 1.9 |
| http | `serveAction.pipelined` | 1768.81 | 936.49 | 1.9 |
| strings | `parseI64.7` | 9.11 | 4.83 | 1.9 |
| strings | `lines.line` | 14.80 | 8.52 | 1.7 |
| strings | `toUpperCase` | 17.70 | 10.48 | 1.7 |
| time | `parseIso` | 37.78 | 23.35 | 1.6 |
| strings | `slice` | 2.04 | 1.27 | 1.6 |
| http | `serve.pipelined` | 1480.00 | 937.29 | 1.6 |
| strings | `startsWith.1` | 1.76 | 1.12 | 1.6 |
| strings | `startsWith.8` | 1.78 | 1.14 | 1.6 |
| strings | `equal` | 4.82 | 3.15 | 1.5 |
| sha256 | `digest.55` | 425.54 | 283.10 | 1.5 |
| sha256 | `digest.4k` | 25748.20 | 17360.86 | 1.5 |
| json | `stringify` | 555.99 | 390.98 | 1.4 |
| strings | `less` | 4.84 | 3.55 | 1.4 |
| http | `exchange.small` | 7516.98 | 5653.21 | 1.3 |
| sys | `listDir.65` | 10974.78 | 9024.92 | 1.2 |
| strings | `join.line` | 5.34 | 4.46 | 1.2 |
| time | `dateOf.daysOf` | 19.15 | 17.30 | 1.1 |
| time | `toDateTime` | 3.70 | 3.36 | 1.1 |
| strings | `replaceAll.1` | 53.59 | 48.84 | 1.1 |
| net | `roundtrip.64` | 6206.43 | 5884.33 | 1.1 |
| https | `get.small` | 1249364.55 | 1190410.38 | 1.0 |
| net | `connect` | 24137.61 | 23428.13 | 1.0 |
| https | `get.64k` | 1261035.67 | 1236869.02 | 1.0 |
| strings | `toString.i64` | 11.43 | 11.23 | 1.0 |
| sys | `writeFile.4k` | 3265.43 | 3219.44 | 1.0 |
| sys | `exists` | 672.88 | 692.48 | 1.0 |
| net | `send.64k` | 11226.69 | 12071.54 | 0.9 |
| json | `encode` | 189.40 | 218.35 | 0.9 |
| sha256 | `hex.32` | 20.73 | 32.38 | 0.6 |
