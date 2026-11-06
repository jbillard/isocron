# Changelog

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
