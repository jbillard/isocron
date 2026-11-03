# Isocron

**A purely functional, strict array language for raw CPU speed, with
deterministic, bit-reproducible results.**

> [!IMPORTANT]
> The same program gives the same bits on every machine and for any number
> of threads: no fast-math, a fixed form of reduction (SPEC section 10), and
> every optimization of the compiler keeps the floating-point operations as
> they are written.

`SPEC.md` is the source of truth (in French). This repository holds the
compiler, written in Rust with an LLVM back-end, a reference interpreter,
the programs of the proof of concept and of the Benchmarks Game, reference
implementations in C, C++, Rust, Go and Zig, and the benchmark protocol of
section 14.

## Results

<!-- results:begin -->

**Isocron is the fastest on 7 of the 10 programs of the Benchmarks Game** (commit `de50af3`, DECISIONS D233; 4 threads, Intel Xeon E-2388G; [details](results/benchmarks_game.md), [floors](results/floors.md)).

| | benchmark | Isocron | fastest other | Isocron / fastest | floor | Isocron / floor |
| :-: | :-- | --: | :-- | :-: | --: | :-: |
| 🟢 | binary-trees | 686 ms | Rust 874 ms | $\color{green}{\textbf{0.78}}$ | 394.7 ms | $\color{orange}{1.7}$ |
| 🟢 | fannkuch-redux | 1666 ms | C 2464 ms | $\color{green}{\textbf{0.68}}$ | 883.5 ms | $\color{orange}{1.9}$ |
| 🟢 | fasta | 49 ms | Rust 755 ms | $\color{green}{\textbf{0.06}}$ | 2.3 ms | $\color{red}{21.2}$ |
| 🟡 | k-nucleotide | 1151 ms | C++ 1170 ms | $\color{orange}{\textbf{0.98}}$ | 268.0 ms | $\color{red}{4.3}$ |
| 🟢 | mandelbrot | 640 ms | C++ 782 ms | $\color{green}{\textbf{0.82}}$ | 436.3 ms | $\color{green}{1.5}$ |
| 🔴 | n-body | 2113 ms | Rust 1947 ms | $\color{red}{\textbf{1.09}}$ | 1315.5 ms | $\color{orange}{1.6}$ |
| 🔴 | pidigits | 886 ms | C 681 ms | $\color{red}{\textbf{1.30}}$ | 194.4 ms | $\color{red}{4.6}$ |
| 🟢 | regex-redux | 476 ms | Rust 789 ms | $\color{green}{\textbf{0.60}}$ | 29.7 ms | $\color{red}{16.0}$ |
| 🔴 | reverse-complement | 104 ms | Rust 67 ms | $\color{red}{\textbf{1.55}}$ | 16.8 ms | $\color{red}{6.2}$ |
| 🟡 | spectral-norm | 195 ms | Rust 205 ms | $\color{orange}{\textbf{0.95}}$ | 107.1 ms | $\color{orange}{1.8}$ |

🟢 Isocron the fastest · 🟡 within 5 % of the fastest · 🔴 behind. Over the floor: $\color{green}{\text{up to 1.5}}$, $\color{orange}{\text{up to 3}}$, $\color{red}{\text{beyond}}$.

<!-- results:end -->

## Contents

* [Getting started](#getting-started): requirements, building, the commands
* [Tests](#tests)
* [Benchmarks](#benchmarks)
* [Layout of the repository](#layout-of-the-repository)
* Documents:
  * writing Isocron programs: [`docs/language.md`](docs/language.md) (the language as implemented today);
  * progress: [`TODO.md`](TODO.md) (tasks P0 to P6);
  * choices made where the specification is silent: [`DECISIONS.md`](DECISIONS.md);
  * benchmark results: [`results/`](results/README.md).

## Getting started

### Requirements

* Rust (stable), a C compiler and linker (`cc`), and the LLVM 21 shared
  library (`libLLVM-21.so`, e.g. the Ubuntu package `libllvm21`). Set
  `ISC_LLVM_LIB_DIR` if it is not in `/usr/lib/x86_64-linux-gnu`.
* For the benchmarks: GCC, Go, Zig 0.14 (its `zig cc` is used as Clang),
  `taskset`, Python 3; Docker for the Benchmarks Game.

### Building and running

```sh
cargo build --release
target/release/iscc check examples/poc/sum.isc
target/release/iscc ir    examples/poc/sum.isc                          # the optimized IR
target/release/iscc run   examples/poc/sum.isc in.f64 out.f64 100       # the interpreter
target/release/iscc build examples/poc/sum.isc -o sum                   # a native program
ISC_THREADS=4 ./sum in.f64 out.f64 100   # on 4 threads: the same result for any number of threads
```

<details>
<summary><b>More options</b>: assembly and LLVM IR, allocators, lanes, instruction sets, WebAssembly</summary>

```sh
target/release/iscc build examples/poc/sum.isc -o sum --emit-asm sum.s --emit-llvm sum.ll   # --debug keeps symbols
ISC_ALLOC=glibc ./sum in.f64 out.f64 100 # glibc's allocator instead of Isocron's
target/release/iscc build examples/poc/sum.isc -o sum64 --lanes 64 --isa sse   # experiment: V = 64 (the language fixes 32), SSE4.2 code
./sum in.f64 out.f64 100      # prints the time of 100 calls in ns, writes the sum
target/release/iscc build examples/poc/sum.isc --target wasm32-wasi   # sum.wasm, a WASI module
wasmtime run --dir=/::/ sum.wasm $PWD/in.f64 $PWD/out.f64 100
target/release/iscc build examples/poc/sum.isc --target wasm32-wasi-threads   # with threads (wasi-threads)
wasmtime run -W threads=y -S threads=y --env ISC_THREADS=8 --dir=/::/ sum.wasm $PWD/in.f64 $PWD/out.f64 100
```

</details>

### In the shell

> [!TIP]
> **Completion** of the commands, options, their values and the `.isc`
> files: `source <(iscc completions bash)` in `~/.bashrc` (or `zsh`, in
> `~/.zshrc`; `iscc completions fish | source` in fish), with `iscc` on the
> `PATH` (`cargo install --path crates/iscc`, or a link to
> `target/release/iscc`).

Messages are in colors, with the line of the source under an error, when
the error output is a terminal and `NO_COLOR` is not set; `--color always`
or `--color never` (anywhere on the command line) forces the choice. Piped,
the text is the same as without colors.

## Tests

```sh
cargo test
ISC_ALLOC=glibc cargo test                     # the same with glibc's allocator
python3 tools/suite.py . target suite.log 6    # both at once: 3 minutes instead of 13
```

The suite covers the lexer and parser (all examples of the specification),
positive and negative typing cases, the IR passes (fusion, in-place update,
`requireUnique`), the interpreter against an independent implementation of
the fixed reduction form, compiled programs against the interpreter bit for
bit (several sizes, a three-level reduction, panics, printing), SIMD and the
absence of fast-math flags in the generated code, memory under valgrind, and
differential testing on random programs (`ISC_DIFF_PROGRAMS` and
`ISC_DIFF_SEED` widen the campaign).

## Benchmarks

```sh
bench/build.sh                              # the programs of the proof of concept into bench/bin
bench/run.py                                # protocol of section 14: results/poc.md and results/poc.json
bench/benchmarks_game/run.sh measure 4 3 native > results/benchmarks_game.tsv   # the ten programs, in Docker
bench/floor/run.sh > results/machine_floor.txt                                    # the costs of the machine
```

> [!NOTE]
> The reference is the machine, not another language: the *floor* of a
> program is what its result requires at the cost of the cheapest way the
> machine has to do it ([`results/floors.md`](results/floors.md)).

## Layout of the repository

| Path | Content |
| :-- | :-- |
| `crates/isc-syntax` | lexer (offside rule), parser, AST, positioned diagnostics |
| `crates/isc-types` | type checker (literal typing, size polynomials, run-time size tests), standard library `prelude.isc` |
| `crates/isc-ir` | high-level IR, lowering (full inlining, specialization), the passes (fusion, tables of functions, lengths known before running), ownership analysis |
| `crates/isc-interp` | reference interpreter (bit-exact oracle) |
| `crates/isc-codegen` | IR to LLVM IR (fusion F1, fixed-form vectorized reduction, in-place updates, vector code, `bench`), LLVM C API back-end |
| `crates/isc-runtime` | runtime of compiled programs, without the Rust standard library: its allocator, the thread pool, I/O actions, printing, clock, panics (its own workspace, built and embedded by `crates/iscc/build.rs`) |
| `crates/isc-runtime-core` | texts, formatting, big integers and regular expressions shared by the runtime and the interpreter (`no_std`) |
| `crates/iscc` | the `iscc` driver and the end-to-end tests |
| `examples` | the programs of the proof of concept (`poc`) and of the Benchmarks Game (`benchmarks_game`) |
| `bench` | reference programs, the protocols, the floors of the machine (`bench/floor`) |
| `tools` | `suite.py`: the complete suite in a few minutes |
