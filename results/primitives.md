# Every form against the machine

Each form of the language measured in Isocron and as the same work in C
written the cheapest way known (its floor), on one CPU at fixed frequency,
the median of 5 runs of 200 rounds (`bench/primitives/run.py`); the
nanoseconds of one call, ranked by Isocron over its floor.

| group | form | Isocron (ns) | floor (ns) | Isocron / floor |
| :-- | :-- | --: | --: | --: |
| time | `format` | 440.51 | 36.44 | 12.1 |
| sys | `envAll` | 4110.27 | 586.36 | 7.0 |
| json | `decode` | 448.10 | 86.81 | 5.2 |
| json | `parse` | 853.71 | 167.68 | 5.1 |
| strings | `padStart` | 9.71 | 1.96 | 4.9 |
| sys | `readText.4k` | 7103.97 | 1951.64 | 3.6 |
| sys | `normalize` | 172.18 | 48.80 | 3.5 |
| sys | `ioError.message` | 48.37 | 14.34 | 3.4 |
| sys | `isAbsolute` | 1.64 | 0.56 | 3.0 |
| sys | `dirname` | 8.64 | 3.20 | 2.7 |
| strings | `split.6` | 77.42 | 32.05 | 2.4 |
| http | `exchange.chunked64` | 48609.88 | 20186.58 | 2.4 |
| strings | `trim` | 14.05 | 6.00 | 2.3 |
| sys | `readFile.4k` | 4241.62 | 1874.59 | 2.3 |
| strings | `indexOf.1` | 6.72 | 3.08 | 2.2 |
| sys | `env` | 29.34 | 13.73 | 2.1 |
| strings | `parseI64.7` | 9.44 | 4.52 | 2.1 |
| strings | `concat` | 6.29 | 3.06 | 2.1 |
| sys | `basename` | 9.10 | 4.53 | 2.0 |
| strings | `includes.3` | 24.92 | 12.54 | 2.0 |
| strings | `bytes` | 1.63 | 0.92 | 1.8 |
| strings | `endsWith.2` | 1.79 | 1.01 | 1.8 |
| time | `formatDateTime` | 84.84 | 48.45 | 1.8 |
| strings | `lines.line` | 14.77 | 8.48 | 1.7 |
| time | `formatIso` | 81.62 | 47.98 | 1.7 |
| strings | `toUpperCase` | 17.71 | 10.45 | 1.7 |
| sys | `extension` | 12.32 | 7.37 | 1.7 |
| http | `serveAction.pipelined` | 1533.95 | 960.63 | 1.6 |
| time | `parseIso` | 36.99 | 23.48 | 1.6 |
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
| sys | `cwd` | 437.40 | 332.58 | 1.3 |
| sys | `makeDir.remove` | 3776.05 | 2913.41 | 1.3 |
| sys | `join` | 6.80 | 5.26 | 1.3 |
| time | `isLeapYear` | 0.73 | 0.59 | 1.2 |
| sys | `listDir.66` | 11126.11 | 9037.07 | 1.2 |
| strings | `join.line` | 5.36 | 4.43 | 1.2 |
| time | `daysBetween` | 13.14 | 11.23 | 1.2 |
| sys | `modified` | 704.34 | 622.27 | 1.1 |
| time | `now` | 19.71 | 17.54 | 1.1 |
| sys | `fileSize` | 699.23 | 622.39 | 1.1 |
| strings | `replaceAll.1` | 54.29 | 48.78 | 1.1 |
| time | `toDateTime` | 3.70 | 3.35 | 1.1 |
| sys | `makeDirs.removeAll` | 22380.42 | 20496.49 | 1.1 |
| json | `stringify` | 423.77 | 390.38 | 1.1 |
| net | `connect` | 25440.76 | 23469.74 | 1.1 |
| time | `weekday` | 8.27 | 7.72 | 1.1 |
| sys | `isFile` | 674.22 | 634.75 | 1.1 |
| net | `roundtrip.64` | 6391.53 | 6028.47 | 1.1 |
| time | `addDays` | 19.23 | 18.14 | 1.1 |
| sys | `appendText.64` | 1837.71 | 1748.40 | 1.1 |
| https | `get.small` | 1242062.21 | 1184573.80 | 1.0 |
| sys | `rename.2` | 4170.97 | 3997.97 | 1.0 |
| sys | `writeFile.4k` | 3233.69 | 3107.32 | 1.0 |
| strings | `toString.i64` | 11.65 | 11.22 | 1.0 |
| sys | `isDir` | 621.03 | 598.92 | 1.0 |
| https | `get.64k` | 1252726.70 | 1227523.31 | 1.0 |
| sys | `writeText.4k` | 3196.89 | 3138.18 | 1.0 |
| sys | `appendFile.64` | 1817.43 | 1794.41 | 1.0 |
| time | `addMonths` | 3.89 | 3.85 | 1.0 |
| time | `dateOf.daysOf` | 19.15 | 19.02 | 1.0 |
| sys | `exists` | 698.73 | 695.47 | 1.0 |
| time | `toInstant` | 8.02 | 8.39 | 1.0 |
| time | `daysInMonth` | 1.06 | 1.12 | 0.9 |
| net | `send.64k` | 11307.68 | 12170.60 | 0.9 |
| sha256 | `hex.32` | 20.41 | 32.37 | 0.6 |
| json | `encode` | 129.22 | 217.75 | 0.6 |
