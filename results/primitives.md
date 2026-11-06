# Every form against the machine

Each form of the language measured in Isocron and as the same work in C
written the cheapest way known (its floor), on one CPU at fixed frequency,
the median of 5 runs of 200 rounds (`bench/primitives/run.py`); the
nanoseconds of one call, ranked by Isocron over its floor.

| group | form | Isocron (ns) | floor (ns) | Isocron / floor |
| :-- | :-- | --: | --: | --: |
| json | `decode` | 448.10 | 86.81 | 5.2 |
| json | `parse` | 853.71 | 167.68 | 5.1 |
| strings | `padStart` | 9.71 | 1.96 | 4.9 |
| sys | `normalize` | 168.97 | 50.24 | 3.4 |
| strings | `split.6` | 77.42 | 32.05 | 2.4 |
| http | `exchange.chunked64` | 48609.88 | 20186.58 | 2.4 |
| strings | `trim` | 14.05 | 6.00 | 2.3 |
| sys | `readFile.4k` | 4270.94 | 1869.87 | 2.3 |
| strings | `indexOf.1` | 6.72 | 3.08 | 2.2 |
| strings | `parseI64.7` | 9.44 | 4.52 | 2.1 |
| strings | `concat` | 6.29 | 3.06 | 2.1 |
| strings | `includes.3` | 24.92 | 12.54 | 2.0 |
| sys | `env` | 21.63 | 11.33 | 1.9 |
| sys | `basename` | 9.04 | 4.82 | 1.9 |
| time | `formatIso` | 81.67 | 45.68 | 1.8 |
| strings | `bytes` | 1.63 | 0.92 | 1.8 |
| strings | `endsWith.2` | 1.79 | 1.01 | 1.8 |
| strings | `lines.line` | 14.77 | 8.48 | 1.7 |
| strings | `toUpperCase` | 17.71 | 10.45 | 1.7 |
| http | `serveAction.pipelined` | 1533.95 | 960.63 | 1.6 |
| time | `parseIso` | 37.19 | 23.32 | 1.6 |
| strings | `equal` | 4.90 | 3.13 | 1.6 |
| strings | `slice` | 1.99 | 1.32 | 1.5 |
| sha256 | `digest.55` | 424.64 | 282.98 | 1.5 |
| sha256 | `digest.4k` | 25750.35 | 17306.26 | 1.5 |
| strings | `startsWith.1` | 1.74 | 1.17 | 1.5 |
| strings | `startsWith.8` | 1.72 | 1.19 | 1.4 |
| strings | `less` | 4.93 | 3.45 | 1.4 |
| strings | `template` | 14.76 | 10.55 | 1.4 |
| http | `serve.pipelined` | 1334.19 | 960.57 | 1.4 |
| http | `exchange.small` | 7509.81 | 5665.14 | 1.3 |
| sys | `listDir.65` | 10829.30 | 8929.62 | 1.2 |
| strings | `join.line` | 5.36 | 4.43 | 1.2 |
| time | `now` | 20.28 | 16.92 | 1.2 |
| strings | `replaceAll.1` | 54.29 | 48.78 | 1.1 |
| time | `dateOf.daysOf` | 19.15 | 17.32 | 1.1 |
| time | `toDateTime` | 3.70 | 3.35 | 1.1 |
| json | `stringify` | 423.77 | 390.38 | 1.1 |
| net | `connect` | 25440.76 | 23469.74 | 1.1 |
| net | `roundtrip.64` | 6391.53 | 6028.47 | 1.1 |
| https | `get.small` | 1242062.21 | 1184573.80 | 1.0 |
| strings | `toString.i64` | 11.65 | 11.22 | 1.0 |
| https | `get.64k` | 1252726.70 | 1227523.31 | 1.0 |
| sys | `writeFile.4k` | 3145.17 | 3106.03 | 1.0 |
| sys | `exists` | 681.92 | 687.62 | 1.0 |
| net | `send.64k` | 11307.68 | 12170.60 | 0.9 |
| sha256 | `hex.32` | 20.41 | 32.37 | 0.6 |
| json | `encode` | 129.22 | 217.75 | 0.6 |
