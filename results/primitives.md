# Every form against the machine

Each form of the language measured in Isocron and as the same work in C
written the cheapest way known (its floor), on one CPU at fixed frequency,
the median of 5 runs of 200 rounds (`bench/primitives/run.py`); the
nanoseconds of one call, ranked by Isocron over its floor.

| group | form | Isocron (ns) | floor (ns) | Isocron / floor |
| :-- | :-- | --: | --: | --: |
| time | `now` | 150.32 | 17.52 | 8.6 |
| time | `formatIso` | 321.27 | 45.75 | 7.0 |
| sys | `normalize` | 328.03 | 50.21 | 6.5 |
| json | `parse` | 1088.62 | 168.16 | 6.5 |
| json | `decode` | 536.69 | 86.68 | 6.2 |
| strings | `padStart` | 11.53 | 2.01 | 5.7 |
| sys | `basename` | 25.68 | 4.82 | 5.3 |
| strings | `template` | 36.05 | 10.56 | 3.4 |
| sys | `env` | 61.26 | 17.98 | 3.4 |
| strings | `concat` | 9.27 | 3.10 | 3.0 |
| http | `exchange.chunked64` | 49662.17 | 20010.18 | 2.5 |
| strings | `trim` | 14.09 | 6.00 | 2.3 |
| strings | `split.6` | 74.62 | 32.03 | 2.3 |
| sys | `readFile.4k` | 4351.60 | 1903.20 | 2.3 |
| strings | `indexOf.1` | 6.72 | 3.22 | 2.1 |
| strings | `includes.3` | 25.05 | 12.55 | 2.0 |
| strings | `parseI64.7` | 9.12 | 4.59 | 2.0 |
| strings | `endsWith.2` | 1.85 | 0.97 | 1.9 |
| strings | `bytes` | 1.69 | 0.89 | 1.9 |
| strings | `lines.line` | 14.80 | 8.53 | 1.7 |
| strings | `toUpperCase` | 17.63 | 10.51 | 1.7 |
| time | `parseIso` | 38.35 | 23.33 | 1.6 |
| strings | `slice` | 2.07 | 1.30 | 1.6 |
| strings | `startsWith.1` | 1.80 | 1.15 | 1.6 |
| http | `serve.pipelined` | 1444.40 | 934.81 | 1.5 |
| strings | `equal` | 4.84 | 3.14 | 1.5 |
| strings | `startsWith.8` | 1.78 | 1.17 | 1.5 |
| sha256 | `digest.55` | 424.88 | 282.27 | 1.5 |
| sha256 | `digest.4k` | 25784.56 | 17286.16 | 1.5 |
| json | `stringify` | 577.40 | 391.09 | 1.5 |
| strings | `less` | 4.84 | 3.45 | 1.4 |
| http | `exchange.small` | 7410.46 | 5719.79 | 1.3 |
| sys | `listDir.65` | 10987.82 | 8999.22 | 1.2 |
| strings | `join.line` | 5.31 | 4.47 | 1.2 |
| time | `dateOf.daysOf` | 19.13 | 17.32 | 1.1 |
| time | `toDateTime` | 3.70 | 3.37 | 1.1 |
| strings | `replaceAll.1` | 53.52 | 48.79 | 1.1 |
| net | `roundtrip.64` | 6150.57 | 5863.43 | 1.0 |
| net | `connect` | 24291.29 | 23162.50 | 1.0 |
| strings | `toString.i64` | 11.32 | 11.20 | 1.0 |
| sys | `writeFile.4k` | 3247.05 | 3224.72 | 1.0 |
| sys | `exists` | 684.67 | 693.45 | 1.0 |
| net | `send.64k` | 11100.33 | 12064.07 | 0.9 |
| json | `encode` | 182.29 | 217.96 | 0.8 |
| sha256 | `hex.32` | 21.05 | 32.97 | 0.6 |
