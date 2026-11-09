# Changelog

## Unreleased

* `--target x86_64-windows-msvc`: Windows programs with Microsoft's ABI, linked by LLD as `link`; static libraries of Visual Studio linked with Microsoft's C runtime, static, found in `LIB` (D542).

* Windows: programs without a C library, importing only `kernel32` (132 functions of 7 DLLs before); their arguments read in UTF-16, any character; the DLLs of the network and of TLS loaded at their first use; a start about 35 % faster (D541).

* `parseI64` reads a sign and up to 8 digits at once, without a loop: -40 % for numbers of 7 digits; up to 16 digits without `str::parse` (D539).

* `serve`: the head of a request checked as UTF-8 once, its parts slices of it (D537).

* `serve`: the headers of a request read in one pass, no array of them: -4.5 % of the instructions of the program a request (D536).

* `serve`: the head of each response made as one text continued in its block, not a text for each header joined then copied: -4 % of the cycles of the program a request (D534).

* `concat(a, b)` grows the block of `a` when `a` is read there for the last time: an array accumulated by `concat` no longer copied whole at each step (100000 steps 3777 -> 432 ms, D533).

* Texts compared by their first 8 (or 4) bytes before a call: `==` and `<` of texts -45 to -55 %, `sort` of texts -28 %, a key of text in a map -29 % (D532).

* A program that added a text to a standard input of 8 MiB or more read from a file (mapped, D257) ended on "invalid free" when the allocator had served a block before: the mapped input has no room, it is copied (D529).

## 0.4.9 — 2026-11-08

Every function of the standard library at most 3 times its floor but two
bound by the representation of texts: `json.parse` 4.04 -> 3.00 and
`json.decode` 3.58 -> 2.80 times their floors. The functions too large to
be inlined decided one at a time, no literal given to the allocator, the
maps of a few heavy elements made where they are. The same results.

### The compiler

* A `map` of at most 4 elements that are loops of their own is made where
  it is when the loops run one task after the other (one thread, or inside
  a parallel loop), without a task: `json.parse` 599 -> 510 ns a record
  (below 3 times its floor), `json.stringify` -13.7 %, `json.encode`
  -7.2 % (D520).

* A text taken out of a tuple, or freed as a part of one, that is a
  literal of the program is not given to the runtime, wherever it is
  freed: `json.decode` 285 -> 263 ns a record (2.87 times its floor),
  `sys.normalize` -3.6 % (D518, D519).

* The functions too large to be inlined are decided one at a time, the
  smallest first: a function no longer counts those it calls that are
  called rather than inlined. The comparison of keys of a decoder of
  `json` inlined: `json.decode` 325 -> 287 ns a record, `json.parse`
  -3.5 %, `sha256.digest` of 55 bytes 425 -> 379 ns (D517).

## 0.4.8 — 2026-11-08

`findIndex`, a search that stops at the first element found, by vectors;
texts appended by the compiled code; a tuple of an `if` taken apart; a
`scan` reading several arrays by vectors. `json.quote` of escaped texts
below 3 times its floor, `json.parse` and `json.decode` faster. The same
results.

### The language

* `findIndex(x, p)`: the first index whose element satisfies `p`, or -1,
  `p` not called past it; by vectors of 64 bytes when `p` cannot fail
  (D505), the first 8 bytes one at a time (D512). The searches of `json`
  by it: `json.decode` 396 -> 346 ns a record, `json.parse` 789 -> 726
  then -4 to -7 %, `json.quote` of a text of 300 bytes 330 -> 93.

### The runtime

* A part of at most 16 bytes appended to a text by one move of 16 when the
  block has room: `json.quote` of an escaped text 81.6 -> 74.2 ns, with
  its closing quote written with its last part (D508).

### The standard library

* The index of a JSON text without a backslash by two sums by prefix, by
  vectors: `json.decode` 346 -> 324 ns a record, `json.parse` 734 -> 698
  (D506).

### The compiler

* The room of the block of an accumulator of text read by the compiled
  code, no call when it holds the parts: `json.quote` of an escaped text
  66.5 -> 56.6 ns (2.86 times its floor), `time.format` 115.7 -> 102.0
  (D511).
* A chain of texts after an accumulator (`acc + a + b`) is written by the
  compiled code, a long part by one call of `memcpy`: `json.quote` of an
  escaped text 73.6 -> 66.6 ns, of a text of 300 bytes 89.6 -> 77.6
  (D509).
* A `scan` whose source reads several arrays by the index (several places
  that can fail) is read by vectors, a vector whose check fails computed
  again one element at a time (the same panics): 1.29 -> 0.24 ns an
  element (D507).
* A tuple made by an `if` whose branches make new tuples is taken apart
  where it is destructured: its parts moved, not copied. `json.decode`
  442 -> 395 ns a record (D504).

## 0.4.7 — 2026-11-08

Maps found by hashes, numbers sorted by their bytes, `toFixed` and `print`
of integers faster, a `map` of a zip made one loop; the audit of every
group of forms against its floor done. The same results.

### The language

* A map keeps an index of hashes of its keys (fixed hashes, the same bits
  everywhere): `get` of an integer 56.1 -> 4.1 ns, of a text 167.7 -> 20.1
  ns; built in O(n) (D492).

### The compiler

* `map(zip(a, b), (p) => ...)` reading its pairs by their components is
  one `map` over `a` and `b`: its sum 0.56 -> 0.11 ns an element (D495).
* `print` of an integer writes its digits and its newline in one call:
  26.7 -> 24.9 ns a line (D498).
* The tasks of a fill that reads a `filter` by windows write 16 KiB too:
  reverse-complement 27.5 -> 23.6 ms (D490).

### The runtime

* `toFixed` from the integer and the power of two of the double, up to 18
  decimals: 1609 -> 49 ns (D487).
* `sort` and `sortBy` of numbers and booleans by their bytes (radix) from
  64 elements: integers -70 %, a key of a few values -63 % (D488).
* `countBy` counts by value the keys below the number of keys, its table
  sized from the start: 9.7 -> 3.5 ns an element (D492, D494).

### The measurements

* New groups of microbenchmarks with their floors in C: numbers, sorts,
  maps, arrays, control and calls, the standard streams (`run.py` gives
  the last its input and output) (D487, D488, D492, D495, D497, D498).
  Every form of them at most 3 times its floor but `sortBy` of texts, bound
  by the representation of texts (1.24 times C with it, D488).
* Floors beaten by Isocron found wrong and made faster (loops of GCC with
  one accumulator, a shift for an addition, scalar Collatz and others).
* A measure of the Benchmarks Game during the maintenance of Plex found
  slower for every language, measured again on a quiet machine (D488,
  D489).

## 0.4.6 — 2026-11-08

A search from the end for texts, the short texts of templates copied in
their code, integers padded by zeros from the table of pairs; the same
results.

### The language

* `s.lastIndexOf(t)`: the byte index of the last `t` in `s`, -1 if none,
  the length of `s` for an empty `t` (as TypeScript); a byte in a short
  text by windows of 16 from the end (D482).

### The compiler

* A text of at most 64 bytes in a template is copied in its code by moves
  that overlap, without a call: `strings.concat` -17.6 %, `time.format`
  125.6 -> 112.1 ns, `json.message` -13 % (D478).
* Integers padded to 3 and 4 digits by zeros are written from the table
  of digit pairs: `time.formatDateTime` -5 % (D477).

### The library

* An HTTPS server on Windows keeps its key in the store of the machine
  when it may: started before a session of its user opened with a
  password (by `ssh` with a key, a task, the first seconds after a start),
  SChannel refused the key of the user, `0x8009030d` (D485).
* `path.basename` -20 %, `path.extension` -14 %: the last `/` found by
  `lastIndexOf` (D482).
* `http.parseUrl` tells its scheme by its bytes, without a text in lower
  case: -4 % (D472).

### The measurements

* `json.parse` and `json.decode` measured against C with the
  representation of Isocron (D471); the waiting of reverse-complement and
  the 17 cycles of n-body measured again on the code of 0.4.5 (D475).
* Rejected after measure, their guards kept as tests: the decoder of
  `json` in one pass (D474), the parts of a path searched from the end
  one byte at a time (D479), the escapes of `json.quote` found by
  `filter` (D481).
* The checks of `publish.sh` read the whole file (D469).

## 0.4.5 — 2026-11-07

Faster texts, dates, paths and numbers of the library, by general changes
of the compiler, the runtime and the modules; the same results.

### The runtime

* A double is written by Schubfach, the text of Rust's `Display` kept
  (`print`, templates, `toString`, `json`): `json.stringify` 368 -> 284
  ns, `json.encode` 115.7 -> 88.4 ns (D467).
* A byte is searched in a text of at most 64 bytes by windows of 16:
  `includes` -19 %, `indexOf` -11 % (D466).
* Texts added to a text are copied by moves, the room of its block read
  inline: `json.quote` of escaped texts -19 %, `time.format` -16 % (D459).
* A parallel loop on one thread runs at once (D460).

### The compiler

* `padStart` and `padEnd` of a text only read write it on the stack:
  10.2 -> 5.7 ns (D458).

### The library

* `http.httpDate` 56.9 -> 37.7 ns: its fields from the table of digit
  pairs, its day of the week from the days (D461).
* `time.format` adds each field with its text in one template (D462).
* `sys.readText` checks ASCII by vectors before its decoder: 6.4 -> 3.2
  µs for 4 KiB (D463).
* `path.normalize` reads its parts by their places: 165 -> 90 ns (D464).

### The measurements

* Floors that did less than their programs, corrected: two that GCC had
  emptied of their blocks (D455), `http.message` and `time.format` that
  wrote constants the programs read at run time (D457, D462); the writes
  of the others kept from the compiler (D456). `sys.isAbsolute` explained:
  Isocron equals C on the layout of an array of strings (D465).

## 0.4.4 — 2026-11-07

A fix of the checks of UTF-8 of the library: one request could end a
server of `http`. Upgrading is advised for every program that serves
HTTP or HTTPS, or reads texts from the network or from files.

### Fixed

* The modules `http`, `sys` and `json` checked bytes as UTF-8 by the
  classes of their bytes only: they took an encoded surrogate (`ED A0 80`),
  the overlong forms of three and four bytes and the characters above
  U+10FFFF, which `stringOf` refuses, and it panicked (D451):
  * the server (`serve`, `serveOn`, `serveAction`, and over TLS): one
    request with such bytes in its head ended the program; it is now
    answered 400;
  * the client: such bytes in the head of a response panicked; now
    `Malformed("the head is not text")`; `text` of such a body panicked;
    now `?` for each byte from 0x80;
  * `sys.readText` of such a file panicked; now the error `not UTF-8`.

### The compiler

* Bytes whose largest value is known below 0x80 (a table of ASCII, `&`,
  `>>`, the branches of an `if`) are given to a text without checking
  them: `sha256.hex` 14.3 -> 7.2 ns (D450).
* A count of bytes takes 64 of them at a time on AVX-512 BW, from a
  boundary of 64 bytes over 1 KiB: `http.text` of 1 KiB 63.2 -> 53.3 ns,
  `json.quote` of a name 6.8 -> 5.0 ns (D452).

## 0.4.3 — 2026-11-07

A fix of the memory of the texts on the stack, and faster texts, dates,
addresses and errors by general changes of the compiler and the runtime.
The Benchmarks Game measured again: the fastest on 10 of 10 (D447).

### Fixed

* A text written into a buffer on the stack (D404) could be freed as a
  block of the heap: the table of these texts was keyed by the names of
  registers, which a task emitted inside the function numbers again; a
  text of the task under the same name took the function's out (D442).

### The compiler

* A component of a new tuple that dies with it is moved out, not copied:
  `http.textResponse` 22.5 -> 13.9 ns (D434).
* The integers of one or two digits of a template, and those padded to two
  digits by zeros, written by their bytes: `http.httpDate` 82.4 -> 57.1
  ns, `time.formatDateTime` 73.8 -> 51.7 ns (D441, D446).
* A count of bytes ends in one masked vector on AVX-512, a short one too:
  `json.quote` of a name 9.7 -> 6.7 ns, `json.message` 13.0 -> 8.4 ns
  (D437, D443).
* A value of a sum type frees the fields of its variant only:
  `http.parseUrl` 108.2 -> 63.2 ns with D445, `sys.ioError.message` 45.6
  -> 12.6 ns (D448).

### The runtime

* The ASCII prefix of a text checked 64 bytes at a time, a short text
  without `from_utf8`: `http.text` of 1 KiB 90.4 -> 74.8 ns, the server
  -1.8 % a request (D436, D440).
* A null pointer freed at once (the fields of the other variants of a sum
  type, D445).
* `sys.envAll` sorts its names measured once (D435).

### The measurements and the tests

* A microbenchmark and a floor for each of the 91 functions of the library,
  by its name: `http.serve`, `https.serveTls` measured by a client, the
  authorities of the system by `SSL_CERT_FILE` (D431).
* `net.lookup` tested on Windows against a server of DNS of the machine
  (D439).
* The waiting of the threads of reverse-complement and the cycles of
  n-body measured again (D432, D433).

## 0.4.2 — 2026-11-07

Faster pidigits, reverse-complement and k-nucleotide by general changes of
the compiler and the runtime, a Windows console in UTF-8, and each
function of the library with a tested example, run on Windows too, and a
microbenchmark against its floor. The Benchmarks Game measured again:
the fastest on 10 of 10, reverse-complement 32 -> 27 ms, k-nucleotide 677 -> 622 ms (D429).

### The compiler

* `z + x * y` and `z - x * y` of bigints in one pass with ADX: pidigits
  337.2 -> 298.3 ms (D406).
* The keys of a count hashed once, in their own vector loop, and only the
  first line of a key probed in the fast loop: k-nucleotide 682.7 ->
  638.8 ms (D417, D418).
* The counts of a later `filter` over the ranges of an array made by
  blocks in the pass of an earlier one over that array, given to a
  variant of the function that filters: reverse-complement reads its
  input twice, 31.9 -> 27.7 ms (D426).
* A join read once in order by a `map` of its bytes is never made: its
  bytes gathered 16 KiB at a time and mapped; k-nucleotide 632.1 -> 615.0
  ms (D428).
* A loop inside a task writes no line that the other threads read: the
  server -3.9 % of user cycles a request (D410).

### The runtime

* A join of 1 MiB or more made by ranges of its strings in parallel
  (D427).
* The output to a Windows console and the input from it in UTF-8
  (`WriteConsoleW`, `ReadConsoleW`, D408, D409).

### The library

* `sys.envAll` in the order of the bytes of the names, as documented: a
  name followed by a byte below `=` came before its own prefix
  (`ProgramFiles(x86)` before `ProgramFiles`, D423).
* `net.resolve` cuts into words only the lines of `/etc/hosts` that hold
  the name: 5609.7 -> 4431.3 ns (D413).

### Windows

* The MSVC ABI at the boundary of a DLL, tested on Windows: functions of
  DLLs built by Microsoft, and import libraries in the format of
  `lib.exe` given by `--link` (D425).

### The documentation

* A tested example in the guide for each of the 91 functions of the
  library (D421); the programs of the guide also run on Windows, with the
  same output (D422).

### The measurements

* A microbenchmark and a floor for each function of the library, by its
  name or through the one that is it: the client of `http` and the
  functions of `net` (D412 to D414), the numbers of `json` (D415), the
  server of `https` (D416), the server API of `net`, `setTimeout`,
  `closeWrite`, `startTls`, `requestWith` (D424); `results/primitives.md`.

## 0.4.1 — 2026-11-07

Faster texts, in the library and in the compiled code: a text that its
scope only reads written on the stack, tables of names kept as data of
the program, and each pure function of `json`, `sha256` and `http`
measured against its floor.

### The compiler

* A text that its scope only reads, made by a concatenation or a
  template, is written into a buffer of 128 bytes on the stack when it
  fits: `http.message` 13.1 -> 6.5 ns, a concatenation 6.2 -> 3.1 ns
  (D404).
* An array literal of string literals is data of the program, read
  without an allocation: `http.httpDate` 138.2 -> 98.8 ns (D401).
* `stringOf` takes the block of its bytes at their last use:
  `sha256.hex` 27.1 -> 20.9 ns (D399).
* A count of bytes by the masks of its condition, 32 bytes at a time:
  `http.text` 151.1 -> 89.9 ns (D396).

### The library

* `http.headerOf` and `http.header` compare the names of headers byte by
  byte, the case of ASCII ignored (RFC 9110): 378.8 -> 19.1 ns (D395).
* `json.quote` in one pass: a text to escape 273.4 -> 96.3 ns (D397).
* `time.format` in one pass, each field written into the text: 455.2 ->
  203.3 ns (D398, D403); `http.httpDate` writes its fields by their
  digits (D402).
* `sys.envAll` 4634 -> 1882 ns: its entries measured by vectors, checked
  as UTF-8 by words, copied once (D400).

### The runtime

* `strlen` one aligned vector at a time (D400).

### The measurements

* A microbenchmark and a floor for each pure function of `json`, `sha256`
  and `http` (`results/primitives.md`, D394); the floor of `envAll` sorts
  its entries as `envAll` does (D400).

## 0.4.0 — 2026-11-06

One `serve` and one `map` for a pure function or an action, a loop of
events on each core, and faster code. The Benchmarks Game measured again:
the fastest on 10 of 10, each program below three times its floor, against
floors closer to the machine (D390, D391). A version 0.4.0 (as Cargo
counts): names of 0.3 are removed.

### The language

* `mapParallel(x, f)`: an action applied to each element at the same time
  on the threads of the pool, the effects of one application in its order,
  none between two (D373).
* The purity of a lambda is inferred: written where an action is expected,
  it is an action only when its body has an effect (D386).
* `isPure(f)`: whether the function received is pure, a constant of each
  compiled call; in the first branch of `if (isPure(h))`, `h` is a pure
  function (D386).
* `map(x, f)` takes an action in an action: applied to each element in
  order, once each; a pure function still in parallel (D387).
* A lambda that reads a function of its context can be given to a
  recursive function (D387).
* **Removed**: `mapAction` (use `map`).

### The library

* `http.serve`, `serveOn`, `serveServer`, `https.serveTls`, `serveTlsOn`
  take a pure function or an action (D386); under Linux `serve` runs a
  loop of events on each thread of the pool (`SO_REUSEPORT`, D374), with
  either handler (D385). On 2 cores, 104.9 to 116.6 % of nginx from 16
  connections on, a handler that is an action as fast as a pure one
  (`results/http_servers.md`).
* **Removed**: `http.serveAction`, `serveActionOn`, `serveActionServer`,
  `https.serveTlsAction`, `serveTlsActionOn` (use `serve`, `serveOn`,
  `serveServer`, `serveTls`, `serveTlsOn`).
* `time.now` by the vDSO, 149.7 -> 20.7 ns (D360); `sys.env` without a
  block of the heap, -47.8 % (D370).

### The compiler

* Templates written in their one block, their integers never made as
  texts, their literals constants: `formatIso` 215 -> 82 ns (D359, D363).
* Literals kept as strings without a copy (D361); a filter whose predicate
  may panic is vector code (D362); a concatenation with a literal and short
  copies by moves (D368, D369); the payload of a match on a temporary taken
  out of it (D367).
* A row of bytes that reads a table in registers in one vector of 64
  lanes: reverse-complement 40 -> 31 ms (D388).
* The quotient `(a * k + b) / c` of bigints from the leading words:
  pidigits 490 -> 338 ms (D389).
* An `iscc` on Windows links the runtime of the level of its processor:
  regex-redux 158.0 -> 113.4 ms on Windows (D382).
* The memory report names the arrays made by blocks and by windows (D383).

### The runtime

* Each parallel loop chooses the pool or the calling thread by its
  measured runs, the pool asleep sooner (D357).
* Small blocks moved between the caches of the threads by whole lists
  (D371, D372); no block for an allocation of 0 bytes (D379).

### The measurements

* The floors closer to the machine: tables of k-mers of 4 to 256
  counters, trees in the L3 cache, the next permutation of fannkuch-redux
  without a branch, its flips by eight chains (D390).
* A microbenchmark and a floor for every function of `sys`, `path` and
  `time` (`results/primitives.md`, D392).

## 0.3.2 — 2026-11-06

Actions as values, a server whose handler can act, and a server that
holds many connections. The Benchmarks Game measured again: still the
fastest on 10 of 10, each program below three times its floor (D356).

### The language

* An action can be given as an argument: its type is `action (T) => U`; a
  pure function goes where one is expected, a lambda written there may
  print, read or write; actions can have type parameters; the call of such
  a value is an effect, only in an action; each call compiled for the
  action it receives, no closure at run time (D352).
* The prelude: `forEach(x, f)` and `mapAction(x, f)`, an action applied to
  each element in order (D352).
* The message of an effect where none is allowed names the action and
  says why (a pure lambda, a `function`) (D352).

### The library

* `http.serveAction`, `serveActionOn`, `https.serveTlsAction`,
  `serveTlsActionOn`: a handler that is an action (it may print, read
  files, call other services), the requests answered one at a time in
  their order (D352).
* The server with many connections: a table of the connections by their
  numbers, only those that received bytes read: 1024 connections kept
  alive 67277 -> 158735 requests a second, as lighttpd; measured against
  the ceiling of the machine (the cheapest server known for the same work)
  under wrk: 57 to 85 % of the ceiling of one loop of events
  (`results/http_servers.md`, D353 to D355).

### The compiler

* Rule A: the accumulator of a tail recursion and the state of a loop
  owned, the left text of a concatenation written after it in its block: a
  string built piece by piece in linear time (100000 pieces 122.2 -> 3.7
  ms), `path.normalize` 229.85 -> 208.28 ns (D351, D354).

## 0.3.1 — 2026-11-06

A correction of 0.3.0: the example of the server in the README, and a
port listened to twice.

### Fixes

* The server of the README says where it listens and why it cannot
  serve, and returns 0 or 1 (it printed nothing: `serve` answers without
  end, its `done` could never show). Every example of the README is now
  run by the tests (D349).
* A port already listened to is `AlreadyExists` on Linux, on Windows and
  in the interpreter; under Windows a second server took the same port
  (`SO_REUSEADDR`, now `SO_EXCLUSIVEADDRUSE`) (D349).

### Faster

* `==` against a short literal compiled in place; the last index of a
  `fold` found from the end; `getenv` without a block of the heap:
  `path.basename` 25.64 -> 9.07 ns, `path.normalize` 335.43 -> 228.61 ns
  (D346).
* `toString(x).padStart(n, c)` of an integer written in one block:
  `time.formatIso` 314.00 -> 216.46 ns (D348).

## 0.3.0 — 2026-11-05

A standard library, to take programs of computation out of their input
file: modules shipped inside `iscc`, written in Isocron over a closed list
of primitives of the runtime, the same results in the interpreter and in
the compiled program, on Linux and Windows, refused at compile time for
WebAssembly where the target lacks what they reach (D322 to D343).
The Benchmarks Game measured again with them: still the fastest on 10
of 10, each program below three times its floor (D344).

### The library

* `sys`: the environment, the current folder, folders and files (exists,
  made, removed, renamed, listed, read and written), the errors as values
  (`IoError`); `path` (D323, D330 to D333).
* `time`: instants, dates and times of the calendar, ISO 8601, offsets
  (D334).
* `json`: read and written, `json.decode<T>` straight into a type of the
  program (its fields checked at compile time) and `json.encode` (D335 to
  D337).
* `sha256`: FIPS 180-4, the vectors of the NIST (D338).
* `net`: TCP, client and server; connections and listeners linear values,
  closed exactly once on every path, checked at compile time; names
  resolved (`/etc/hosts`, DNS) (D339).
* `http`: a client of HTTP/1.1 and a server `serve(port, handler)` whose
  handler is a pure function, the requests ready at once given to it by
  `map` in parallel (D340, D342).
* `https`: TLS by the library of the system, OpenSSL loaded at run time
  under Linux (only by the programs that use it, never among their
  libraries), SChannel under Windows; the client checks the certificate
  and the name, `ca` names a private authority (D343).

### The language

* A payload of a sum type holds an array of its own size; arrays of
  their own size, `T[]`, in records, tuples and function types (D335,
  D341).
* Type arguments of a call (`decode<Point[]>(text)`); sizes bound by the
  arrays of records and tuples of a parameter; branches of arrays of two
  sizes (D336, D337).

### The compiler

* `concat` of arrays made into memory by two copies; chains of strings
  and template literals in one block (D340, D342).
* Fixes found by the library: a slice of an array of tuples that own
  memory freed them twice (each element now copied); `count(rows, (r) =>
  r.id ...)` of records; `r.length` of a record with a field `length`
  (D342).

## 0.2.2 — 2026-11-05

Every program of the Benchmarks Game below three times its floor, the
cost of the machine (`results/floors.md`), and the fastest on 10 of 10:
regex-redux 129 -> 83 ms, fasta 11 -> 6 ms, k-nucleotide 863 -> 680 ms,
reverse-complement 42 -> 41 ms (D319: the lowest of 12 runs now).

### The compiler

* Rows of a `map` (lines of fasta): the whole rows at constant places, in
  one vector of 64 lanes when they are 33 to 64 bytes wide, an equality
  with the index and the remainders of their tables taken out of their
  loop; a table read at `x % c` copied with its period (D272 to D274,
  D318).
* An affine recurrence modulo a constant (`v.with(t + 1, (v[t] * a + c) %
  m)`) computed in parts from powers of its step (D292).
* A `filter` read only by arrays written by blocks made by windows; the
  index of a `fold` in vector code checked before its loop (D293, D297).
* String literals and the parts and bytes of strings read where they are;
  reductions of integers in one accumulator (D301, D303, D304).
* The text of a replacement by a regular expression given to it at its
  last use, written where it is when each match is replaced by as many
  bytes (D315).

### The runtime

* No C called by the runtime under Linux: the system called directly, the
  functions of memory in Rust (`isc-mem`), static programs without a C
  library, their start, threads and thread-local storage (D287 to D289).
* Regular expressions: fixed alternatives searched by lanes and groups of
  blocks of AVX-512, the parts of a search stopped at the next part,
  several parts per thread, each thread its range first, the sizes of a
  replacement from its search (D275 to D284, D306 to D314).
* `countBy` by tables of counts only, each key with its count in one word,
  looked for by lines of 8 slots (D317); the blocks of small keys counted
  in one loop (D296).
* The standard output by blocks into files; the decimal text of integers
  written directly; case, padding and replacement written once; the lines
  and parts of a text in one block (D298 to D305).
* The allocator: small blocks carved from spans of huge pages; the last
  2 MiB of a region without huge pages (0.25 ms less at the start of a
  program, D294, D316).

### The repository

* `bench/primitives/`: every form of the language against its floor in C,
  `results/primitives.md` (D301).
* The Benchmarks Game measured with the lowest of 12 runs (D319).

## 0.2.1 — 2026-11-04

The editors: `isc-lsp`, the language server, beside the compiler. The
compiler and the programs it makes are those of 0.2.0.

### The language server

* `isc-lsp`, beside `iscc` in the archives: the language server of
  Isocron, for every editor that speaks the Language Server Protocol. On
  the text as it is typed: the errors of the checker at their place, on
  hover the type of a name and the signature of a function with the
  comments above its declaration, the documentation of the functions of
  the standard library with a link to it, the definitions (also in an
  imported module), the outline (`docs/language.md`, "Editors").
* VS Code: the extension of `tools/vscode` (colors, the errors of `iscc
  check`, run, build, the IR, outline, definitions, the hover of
  `isc-lsp`): `isocron-0.2.1.vsix`, beside the archives
  (`code --install-extension isocron-0.2.1.vsix`, or **Install from
  VSIX...**).
* Neovim 0.10: `tools/neovim/isocron.lua`.

### The repository

* The README laid out again, with the logo and the icons of the project.
* `tools/release/publish.sh`: a release pushed, tagged and published on
  GitHub by one command, checked first.

## 0.2.0 — 2026-11-04

Isocron is the fastest on the 10 programs of the Benchmarks Game, measured
in Docker against the fastest programs in C, C++, Rust and Go on an Intel
Xeon E-2388G (`results/benchmarks_game.md`): n-body 1.897 s against C's
1.945 s, k-nucleotide 1.070 s against C++'s 1.176 s, reverse-complement
0.061 s against Rust's 0.067 s (they were behind in 0.1.0: 2.021 s, 1.603 s
and 0.104 s). The time each program still spends above the floor of the
machine is attributed in `results/floors.md`.

### Faster programs

* A `fold` over a small constant range whose body computes roots and
  divisions that do not read its accumulator computes them first, two
  elements at a time (n-body 1.979 -> 1.861 s on one core).
* `countBy` with many keys counts them by rounds scattered by their hash;
  the lines of a text are made by parts in parallel; the small blocks of
  the allocator are refilled by batches (k-nucleotide 1.607 -> 1.075 s).
* A large regular file given as the standard input is mapped, not copied,
  under a read lease: a program that writes the file meanwhile does not
  change what the reader sees (`docs/language.md`); the loops that fill an
  array give each thread a contiguous range; a `filter` skips the blocks
  that keep nothing (reverse-complement 0.104 -> 0.061 s).
* The functions and loops of the runtime are aligned, so that its speed no
  longer depends on the code placed before them.

The same results, to the bit, as with 0.1.x.

## 0.1.1 — 2026-11-04

* Programs for Windows keep only the functions of the runtime they use: a
  hello was 1.5 MB, it is 54 KB (regex-redux, which uses the engine of
  regular expressions: 1.52 MB -> 1.23 MB). The runtime for Windows is now
  built with one section per function, which the linker can drop one by
  one, as on Linux.
* `iscc --version`.
* Programs for Linux and WebAssembly are the same as with 0.1.0: the
  measurements of 0.1.0 stand.

## 0.1.0 — 2026-11-04

The first release of Isocron: a purely functional, strict array language for
raw CPU speed, with deterministic, bit-reproducible results.

### The compiler

* `iscc` for Linux (x86-64, any glibc from 2.28 on) and `iscc.exe` for
  Windows (10 and 11, x86-64), each self-contained: LLVM 21.1.8 and its
  linker are inside, with what they link programs with. Nothing to install.
* `iscc check`, `iscc ir`, `iscc run` (the reference interpreter) and
  `iscc build`, for the native system, `--target x86_64-windows`,
  `wasm32-wasi` and `wasm32-wasi-threads` (WebAssembly with WASI, with or
  without threads); `--isa` chooses the instruction set (SSE4.2, AVX, AVX2,
  AVX-512; by default, the CPU of the machine).
* Messages in colors with the line of the source; completion for bash, zsh
  and fish (`iscc completions`).

### The language

* Arrays, element-wise operations and reductions in a fixed order: the same
  bits for any number of threads, on every machine, and between the
  interpreter, native code and WebAssembly. No fast-math.
* Maps, strings in UTF-8, regular expressions, big integers, records, sum
  types and `match`, recursive types, modules, functions as values,
  generics, `Result`, C functions (`declare function`), actions for
  input and output. See [`docs/language.md`](docs/language.md).

### Performance

The ten programs of the Benchmarks Game against the fastest programs in C,
C++, Rust and Go, measured together in Docker (4 threads, Intel Xeon
E-2388G at fixed frequency): Isocron is the fastest on 7 of the 10
(binary-trees, fannkuch-redux, fasta, mandelbrot, pidigits, regex-redux,
spectral-norm), within 5 % of C on n-body, behind on k-nucleotide (1.36
times C++) and reverse-complement (1.58 times Rust). The results, and the
floor of each program (what its result requires at the cost of the
cheapest way the machine has to do it): [`results/benchmarks_game.md`](results/benchmarks_game.md),
[`results/floors.md`](results/floors.md).

### Known limits

* x86-64 only (Linux and Windows); WebAssembly for any WASI engine.
* Windows: programs use the base x86-64 runtime (the AVX2 and AVX-512
  versions of the runtime, which speed up the searches of texts and of
  regular expressions, are for Linux for now); no measurements on Windows
  yet; the GNU ABI (MinGW-w64), not MSVC's.
* The compilers are large (about 125 MB for Linux, 110 MB for Windows):
  LLVM, its linker and six runtimes are inside.
