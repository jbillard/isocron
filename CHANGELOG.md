# Changelog

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
