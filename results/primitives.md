# Every form against the machine

Each form of the language measured in Isocron and as the same work in C
written the cheapest way known (its floor), on one CPU at fixed frequency,
the median of 5 runs of 200 rounds (`bench/primitives/run.py`); the
nanoseconds of one call, ranked by Isocron over its floor.

| group | form | Isocron (ns) | floor (ns) | Isocron / floor |
| :-- | :-- | --: | --: | --: |
| json | `parse` | 1024.22 | 168.08 | 6.1 |
| strings | `padStart` | 11.97 | 1.99 | 6.0 |
| json | `decode` | 473.31 | 86.53 | 5.5 |
| sys | `env` | 47.76 | 11.34 | 4.2 |
| sys | `normalize` | 200.91 | 50.54 | 4.0 |
| strings | `concat` | 9.25 | 3.13 | 3.0 |
| time | `formatIso` | 122.37 | 45.73 | 2.7 |
| strings | `template` | 25.67 | 10.58 | 2.4 |
| http | `exchange.chunked64` | 47786.81 | 19876.30 | 2.4 |
| strings | `trim` | 14.24 | 6.01 | 2.4 |
| strings | `split.6` | 75.97 | 32.06 | 2.4 |
| sys | `readFile.4k` | 4314.99 | 1906.42 | 2.3 |
| strings | `indexOf.1` | 6.73 | 3.11 | 2.2 |
| strings | `includes.3` | 25.08 | 12.58 | 2.0 |
| strings | `parseI64.7` | 9.14 | 4.61 | 2.0 |
| strings | `endsWith.2` | 1.78 | 0.94 | 1.9 |
| strings | `bytes` | 1.61 | 0.85 | 1.9 |
| sys | `basename` | 9.05 | 4.83 | 1.9 |
| http | `serveAction.pipelined` | 1733.18 | 932.69 | 1.9 |
| strings | `lines.line` | 14.86 | 8.51 | 1.7 |
| strings | `toUpperCase` | 17.64 | 10.45 | 1.7 |
| time | `parseIso` | 38.78 | 23.32 | 1.7 |
| strings | `startsWith.1` | 1.75 | 1.12 | 1.6 |
| strings | `slice` | 1.96 | 1.27 | 1.5 |
| strings | `equal` | 4.82 | 3.14 | 1.5 |
| sha256 | `digest.55` | 425.31 | 281.69 | 1.5 |
| http | `serve.pipelined` | 1405.74 | 936.05 | 1.5 |
| strings | `startsWith.8` | 1.71 | 1.14 | 1.5 |
| json | `stringify` | 584.82 | 391.86 | 1.5 |
| sha256 | `digest.4k` | 25768.25 | 17273.44 | 1.5 |
| strings | `less` | 4.82 | 3.45 | 1.4 |
| http | `exchange.small` | 7406.51 | 5681.26 | 1.3 |
| sys | `listDir.65` | 10939.15 | 9014.06 | 1.2 |
| strings | `join.line` | 5.32 | 4.46 | 1.2 |
| time | `now` | 20.84 | 17.68 | 1.2 |
| strings | `replaceAll.1` | 54.55 | 48.75 | 1.1 |
| time | `dateOf.daysOf` | 19.15 | 17.31 | 1.1 |
| time | `toDateTime` | 3.71 | 3.37 | 1.1 |
| net | `roundtrip.64` | 6250.77 | 5854.49 | 1.1 |
| net | `connect` | 24560.44 | 23323.82 | 1.1 |
| https | `get.small` | 1255006.22 | 1192004.20 | 1.1 |
| strings | `toString.i64` | 11.55 | 11.19 | 1.0 |
| https | `get.64k` | 1255785.33 | 1226445.58 | 1.0 |
| sys | `writeFile.4k` | 3242.49 | 3261.46 | 1.0 |
| sys | `exists` | 669.92 | 696.07 | 1.0 |
| net | `send.64k` | 11276.94 | 12211.55 | 0.9 |
| json | `encode` | 183.78 | 218.81 | 0.8 |
| sha256 | `hex.32` | 20.48 | 32.44 | 0.6 |
