# Isocron

Isocron is a purely functional, strict array language whose goal is raw CPU
speed with deterministic, bit-reproducible results. `SPEC.md` is the source of
truth (in French). This repository contains the proof of concept of SPEC
section 15: a compiler written in Rust with an LLVM back-end, a reference
interpreter, the four POC programs, reference implementations in C, Rust, Go
and Zig, and the benchmark protocol of section 14.

* Writing Isocron programs: `docs/language.md` (the language as implemented today).
* Progress: `TODO.md` (tasks P0 to P6).
* Choices made where the specification is silent: `DECISIONS.md`.
* Benchmark results: `results/poc.md` (Celeron J3455, the POC decision point), `results/xeon.md`
  (Xeon E-2388G, block parallelism), `results/xeon-lanes.md` (study of V), `results/xeon-single.md`
  (one core), `results/xeon-compile.md` (compilation times), `results/benchmarks_game.md` (the ten
  programs of the Benchmarks Game against the fastest in C, C++, Rust and Go).

## Layout

| Path | Content |
| --- | --- |
| `crates/isc-syntax` | lexer (offside rule), parser, AST, positioned diagnostics |
| `crates/isc-types` | type checker (literal typing, size polynomials, run-time size tests), standard library `prelude.isc` |
| `crates/isc-ir` | high-level IR, lowering (full inlining, specialization), fusion preparation, ownership analysis |
| `crates/isc-interp` | reference interpreter (bit-exact oracle) |
| `crates/isc-codegen` | IR to LLVM IR (fusion F1, fixed-form vectorized reduction, in-place updates, `bench`), LLVM C API back-end |
| `crates/isc-runtime` | runtime of compiled programs, without the Rust standard library: aligned allocation, I/O actions, printing, clock, panics (its own workspace, built and embedded by `crates/iscc/build.rs`) |
| `crates/isc-runtime-core` | texts and formatting shared by the runtime and the interpreter (`no_std`) |
| `crates/iscc` | the `iscc` driver and the end-to-end tests |
| `examples/poc` | the four POC programs of section 15 |
| `bench` | reference programs, `build.sh`, `run.py` |

## Requirements

* Rust (stable), a C compiler and linker (`cc`), and the LLVM 21 shared
  library (`libLLVM-21.so`, e.g. the Ubuntu package `libllvm21`). Set
  `ISC_LLVM_LIB_DIR` if it is not in `/usr/lib/x86_64-linux-gnu`.
* For the benchmarks: GCC, Go, Zig 0.14 (its `zig cc` is used as Clang),
  `taskset`, Python 3.

## Usage

```sh
cargo build --release
target/release/iscc check examples/poc/sum.isc
target/release/iscc ir    examples/poc/sum.isc            # optimized IR
target/release/iscc run   examples/poc/sum.isc in.f64 out.f64 100    # interpreter
target/release/iscc build examples/poc/sum.isc -o sum --emit-asm sum.s --emit-llvm sum.ll   # --debug keeps symbols
ISC_THREADS=4 ./sum in.f64 out.f64 100   # blocks on 4 threads; same result for any number of threads
ISC_ALLOC=glibc ./sum in.f64 out.f64 100 # glibc's allocator instead of Isocron's
target/release/iscc build examples/poc/sum.isc -o sum64 --lanes 64 --isa sse   # experiment: V = 64 (the language fixes 32), SSE4.2 code
./sum in.f64 out.f64 100      # prints the time of 100 calls in ns, writes the sum
target/release/iscc build examples/poc/sum.isc --target wasm32-wasi   # sum.wasm, a WASI module
wasmtime run --dir=/::/ sum.wasm $PWD/in.f64 $PWD/out.f64 100
target/release/iscc build examples/poc/sum.isc --target wasm32-wasi-threads   # with threads (wasi-threads)
wasmtime run -W threads=y -S threads=y --env ISC_THREADS=8 --dir=/::/ sum.wasm $PWD/in.f64 $PWD/out.f64 100
```

Messages: in colors, with the line of the source under an error, when the
error output is a terminal and `NO_COLOR` is not set; `--color always` or
`--color never` (anywhere on the command line) forces the choice. Piped,
the text is the same as without colors.

## Tests

```sh
cargo test
ISC_ALLOC=glibc cargo test             # the same with glibc's allocator
python3 tools/suite.py . target suite.log 6   # both at once: 3 minutes instead of 13
```

The suite covers the lexer and parser (all examples of the specification),
positive and negative typing cases, the IR passes (fusion, in-place update,
`requireUnique`), the interpreter against an independent implementation of the
fixed reduction form, compiled programs against the interpreter bit for bit
(several sizes, a three-level reduction, panics, printing), SIMD and the
absence of fast-math flags in the generated code, and differential testing on
random programs (`ISC_DIFF_PROGRAMS` and `ISC_DIFF_SEED` widen the campaign).

## Benchmarks

```sh
bench/build.sh        # builds the 53 programs into bench/bin
bench/run.py          # protocol of section 14; writes results/poc.md and results/poc.json
```
