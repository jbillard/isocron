# Changelog

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
