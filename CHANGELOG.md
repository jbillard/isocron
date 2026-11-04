# Changelog

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
