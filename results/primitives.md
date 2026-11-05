# Every form against the machine

Each form of the language measured in Isocron and as the same work in C
written the cheapest way known (its floor), on one CPU at fixed frequency,
the median of 5 runs of 200 rounds (`bench/primitives/run.py`); the
nanoseconds of one call, ranked by Isocron over its floor.

| group | form | Isocron (ns) | floor (ns) | Isocron / floor |
| :-- | :-- | --: | --: | --: |
| time | `now` | 150.85 | 17.64 | 8.6 |
| time | `formatIso` | 384.31 | 45.87 | 8.4 |
| sys | `normalize` | 340.14 | 50.59 | 6.7 |
| json | `parse` | 1092.22 | 167.99 | 6.5 |
| json | `decode` | 540.76 | 86.58 | 6.2 |
| strings | `padStart` | 11.89 | 1.96 | 6.1 |
| sys | `basename` | 25.50 | 4.83 | 5.3 |
| sys | `env` | 63.40 | 17.34 | 3.7 |
| strings | `template` | 38.36 | 10.58 | 3.6 |
| strings | `concat` | 9.24 | 3.13 | 3.0 |
| strings | `trim` | 14.13 | 6.00 | 2.4 |
| strings | `split.6` | 75.10 | 32.06 | 2.3 |
| sys | `readFile.4k` | 4327.23 | 1898.90 | 2.3 |
| strings | `indexOf.1` | 6.73 | 3.34 | 2.0 |
| strings | `includes.3` | 25.02 | 12.58 | 2.0 |
| strings | `endsWith.2` | 1.84 | 0.94 | 2.0 |
| strings | `bytes` | 1.65 | 0.86 | 1.9 |
| strings | `parseI64.7` | 9.12 | 4.74 | 1.9 |
| strings | `lines.line` | 14.80 | 8.50 | 1.7 |
| strings | `toUpperCase` | 17.64 | 10.46 | 1.7 |
| time | `parseIso` | 38.05 | 23.31 | 1.6 |
| strings | `slice` | 2.08 | 1.27 | 1.6 |
| strings | `startsWith.1` | 1.80 | 1.12 | 1.6 |
| strings | `equal` | 4.86 | 3.13 | 1.6 |
| strings | `startsWith.8` | 1.76 | 1.15 | 1.5 |
| json | `stringify` | 604.44 | 393.38 | 1.5 |
| sha256 | `digest.55` | 426.95 | 282.63 | 1.5 |
| sha256 | `digest.4k` | 25951.62 | 17340.39 | 1.5 |
| strings | `less` | 4.94 | 3.45 | 1.4 |
| sys | `listDir.65` | 11101.98 | 9006.48 | 1.2 |
| json | `encode` | 267.97 | 218.62 | 1.2 |
| strings | `join.line` | 5.28 | 4.45 | 1.2 |
| strings | `replaceAll.1` | 54.66 | 48.80 | 1.1 |
| time | `dateOf.daysOf` | 19.15 | 17.30 | 1.1 |
| time | `toDateTime` | 3.70 | 3.36 | 1.1 |
| net | `roundtrip.64` | 6167.09 | 5869.86 | 1.1 |
| net | `connect` | 24175.46 | 23192.41 | 1.0 |
| sys | `writeFile.4k` | 3254.80 | 3213.64 | 1.0 |
| strings | `toString.i64` | 11.29 | 11.24 | 1.0 |
| sys | `exists` | 665.03 | 692.49 | 1.0 |
| net | `send.64k` | 11204.86 | 12105.38 | 0.9 |
| sha256 | `hex.32` | 21.00 | 32.35 | 0.6 |
