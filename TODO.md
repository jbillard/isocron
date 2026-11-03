# TODO — Isocron POC (SPEC.md section 15)

- [x] P0 Foundations: Rust workspace with crates `isc-syntax`, `isc-types`, `isc-ir`, `isc-interp`, `isc-codegen`, `isc-runtime`, binary `iscc`; `cargo test` passes
- [x] P1 Front-end: lexer with indentation, parser, AST, errors with position; the 4 POC programs are parsed
- [x] P2 Typing: POC subset, literal typing, size checks; the 4 programs type-check; type and size errors detected on negative cases
- [x] P3 Interpreter: interprets the IR with the fixed reduction form; bit-exact oracle
- [x] P4 LLVM codegen: IR to LLVM IR, F1 fusion, vectorized reduction, in-place update, `bench`; binaries bit-identical to the interpreter; SIMD visible in the assembly
- [x] P5 Reference programs: the 4 kernels in C, Rust, Go, Zig, idiomatic and hand-optimized; identical results or documented floating-point gap
- [x] P6 Measurements: section 14 protocol scripts, table of medians; results report in the repository (`results/poc.md`)

## Improvements after the POC (plan of 2026-10-31)

- [x] 1. Measurement protocol: fixed frequency, isolated cores, interleaved runs, minimum, effective frequency (`bench/machine.sh`, DECISIONS D41-D44)
- [x] 2. Value of V (lanes of the fixed reduction form) measured on an AVX2 / AVX-512 machine (`results/xeon-lanes.md`, recommendation DECISIONS D55; machine.sh adapted, D53)
- [x] 3. Block parallelism (stage 2, phase 6), bit-identical results with 1, 2, 4, 8 cores; parallel references in C, Rust, Go, Zig (`results/xeon.md`, D49, D51)
- [x] 4. Automatic privatization of histogram accumulators: rule P proposed in DECISIONS D50 (to be confirmed for SPEC section 16), skewed input measured (`results/xeon-single.md`, `results/xeon.md`, D52)

## Follow-up of the author's decisions (2026-11-01)

- [x] V = 32 in SPEC section 10, compiler, interpreter, tests and references; passes when registers are short (D56, D57)
- [x] Rule P written into SPEC section 6 (D57)
- [x] Developer guide `docs/language.md`, every example tested (D60); compilation times `results/xeon-compile.md` (D62)
- [x] xeon reports measured again at fixed frequency with V = 32: `xeon-lanes.md`, `xeon-single.md`, `xeon-compile.md` (D58)
- [x] `results/xeon.md` (parallel) measured again at fixed frequency with V = 32 (D63)
- [ ] Celeron campaign (`results/poc.md`) run again with V = 32 and the corrected C reference (D54): not needed for now (decision of the author, 2026-11-01); `bench/campaign.sh poc` on the PC when wanted
- [x] Weaknesses of D63 examined: skewed histogram fixed (4K aliasing of the private copies, D64); large dot product bound by the memory bandwidth (D65)

## Stage 2 — complete language (SPEC section 15)

- [x] 1. Records (stored in columns), sum types, generics, classes `Num/Ord/Eq`, `size` parameters; `f32`; examples of sections 3 and 4 type-check (D66-D73)
  - [x] `f32` (D66)
  - [x] WebAssembly / WASI targets: `wasm32-wasi` (D67) and `wasm32-wasi-threads` (D68), tested under wasmtime 38.0.4
  - [x] arrays of tuples, stored in columns (D69)
  - [x] records (D70)
  - [x] sum types and `match` (D71); recursive types with phase 3
  - [x] `size` parameters (D72)
  - arrays of arrays `[n][m]T`: with views, in phase 5 (D73)
- [ ] 2. Fusion F2 to F4, materialization rules M1 to M3, memory report
- [ ] 3. Complete uniqueness analysis, fallback reference counting, `require_unique`; histogram without allocation; no leak (AddressSanitizer)
- [x] 4. `require_no_alloc`, `require_fused`, `require_tail`, with positive and negative tests (D106)
- [x] 5. `filter`, `scan`, `window`, `zip`, slices, IEEE 754-2019 `min`/`max`, canonical NaN (sections 6 and 10) (D88 to D95, D97; a `map` building rows: below)
- [x] 6. Block parallelism, thread pool: identical results with 1, 2, 4, 8 cores (D49, D50, D63, D64)
- [ ] 7. Complete benchmark suite (section 14), comparisons A and B
- [x] Start-up cost of the thread pool: the tasks of rule P no longer allocate (D77); first call as costly as OpenMP's
- [x] Histogram: the fastest of Isocron, OpenMP and Rust in the eight cases (1 and 8 threads, small and large, uniform and skewed) (D78)
- [x] Regenerate results/xeon.md and results/xeon-single.md with D77 and D78 (histograms measured again and merged, bench/merge_hist.py)
- [x] An allocator for Isocron instead of glibc's: the default since D81 (`ISC_ALLOC=glibc` for glibc's); faster than glibc on every allocation pattern measured, memory within 2 % of glibc's on growing phases, less on the stress test
- [x] The start of a parallel job: context in the job line, plain-write publication, pool lines (D81); results of `reduce` tasks in their own lines (D82)
- [x] Regenerate results/xeon.md and results/xeon-single.md with D81 and D82 (all languages, one session; compilation times in results/xeon-compile.md, summary in results/xeon-summary.md)

## The cases lost on 2026-11-02, then the complete language (plan of 2026-11-02)

- [x] Large `sum` and `std_dev`, one core: the first call cleared a huge page, large arrays started 64 bytes after a page (D83); the same huge pages for every language during the measurements (D84)
- [x] The test suite hung in `tests/parallel.rs`: LLVM's targets registered once per process (D85)
- [x] Small `std_dev` against Zig, large `sum` against Rust: block results of `reduce` on the stack, last elements of a block through memory, four rows per iteration from memory (D86); `tests/reduce_paths.rs`; compilation time recovered, runtime built with LTO (D87)
- [x] Official campaign with D83 to D87: Isocron has the lowest median in the 10 cases on one core and the 40 cases with 1, 2, 4 and 8 threads; compilation times not degraded (D87); `results/xeon-summary.md`
- [ ] Stage 2, phases 2, 3, 4, 5 and 7, and `allow_reassoc` (SPEC sections 10 and 15), in the order their dependencies impose:
  - [ ] Phase 5 (`zip` D88, `min`, `max` D89, `count` D90, slices D91, `scan` D92, `filter` D93: done; slices are copies until F4; parallel `scan` not faster yet; `filter` sequential; existential result sizes of functions D94; arrays of arrays and `window` D95, without a `map` building rows yet): `zip`, slices `x[a..b]`, `window`, arrays of arrays `[n][m]T` with views; `filter` and bounded arrays `[k <= n]T`; `scan` (parallel for the predefined associative operators on integers and booleans); `count`, `min`, `max` (IEEE 754-2019), canonical NaN
  - [ ] Phase 2: fusion F2 (filter), F3 (independent `reduce` of one array), F4 (views never materialized); materialization M1 to M3; the memory report (`--report`, text and JSON)
    - [x] F4: slices of arrays of scalars and zips are views, never materialized, even shared (D97)
    - [ ] Open question (D97): a view of a part of an array of tuples needs a stride in the representation; until then such a slice is a copy
    - [x] Views of producers: a slice of a producer computes only its elements; a window of a producer materializes it by M1 (D101)
    - [x] Memory report `--report`, text and JSON (D98); a length test no longer materializes a producer
    - [x] F2: a filter read in order by `reduce`, `fold` or `map` in one pass; masked reductions for exact operators (D99)
    - [x] Phase 7: vectorized stream for float `+` of a filter (compress into a buffer, same order) (D110)
    - [x] F3: independent `reduce`s of one array in one pass, results exact (D100)
    - [ ] Bounds proved on the IR (the report lists every indexing as checked at run time)
    - [ ] Generator scratch in the report (block results of `reduce`, counts of `filter`, block sums of `scan`)
  - [x] Memory decisions checked at every compilation, a failure is a compilation error (D96); `tests/verify.rs`, `tests/memory.rs` (Valgrind)
  - [x] Phase 3: uniqueness analysis on the IR (D96, D105), recursion (D102) and recursive sum types with reference counting (D103); `require_unique`; histogram without allocation (`report.rs`); no leak under Valgrind (`memory.rs`). Open points below
    - [x] Recursive functions: not inlined, real functions in the IR and the code; direct tail recursion as a loop; stack overflow is a panic; reported in the memory report (D102)
    - [x] Uniqueness in the branches of an `if`: the last use on each path moves, a branch that does not consume frees (D105)
    - [x] A recursive function with a function parameter, specialized per function given (D128), a lambda's captures as parameters (D131); not yet a lambda that reads a size of its context
    - [x] Recursive sum types on the heap with reference counting (non-atomic outside tasks, atomic inside) (D103)
    - [x] `==` on recursive values (D129)
    - [x] Arrays of a recursive type (D130)
    - [ ] A constructor without payload (`Nil`) without an allocation (a shared node, or a null pointer with its tag)
    - [ ] Question (D103): fallback reference counting for arrays; decided provisionally: arrays are copied, each copy in the report
    - [x] Defunctionalization: a function chosen at run time (`let f = if c then g else h`) becomes a branch on its condition (D107)
    - [x] A function chosen by `match` (D127), reading the variables of a pattern of one level (D132)
  - [x] Phase 4: `require_no_alloc`, `require_fused`, `require_tail`, positive and negative tests for each (D106; `tests/phase4.rs`)
  - [x] Mathematical functions `exp`, `log`, `sin`, `cos`, `pow`, `fma` (f64, f32), `x ^ y` with a float `y` (D108)
  - [ ] Question (D108, section 16): precision of the transcendental functions; decided provisionally: musl's algorithms, the same bits everywhere; vectorized versions of the same algorithms for speed
  - [x] `allow_reassoc` (section 10, D109); mathematical functions `fma`, `pow`, `exp`, `log`, `sin`, `cos` (D108)
  - [x] Phase 7: kernels moving average (window 64), filter then sum, `i64` scan, matrix-vector 4096 x 4096; idiomatic and optimized versions in C, Rust, Go, Zig; comparisons A and B
    - [x] Isocron programs in `examples/phase7/`, inputs and references in `bench/run.py`, `bench/build.sh`
    - [x] C, Rust, Go and Zig: idiomatic, opt, parallel and reassociated variants; every opt identical to Isocron (`bench/<lang>/PHASE7.md`)
    - [x] Vectorized stream for the float `+` of a filter (D110)
    - [x] Sliding-window reductions by their shared levels (D115), found by the C and Go agents
    - [x] Comparison B (moving average in O(n)) in every language; Isocron's B is the windows themselves (D118)
    - [x] Lost cases found by quick measurements and fixed: integer `scan` in vectors (D116), output pages touched before the timer in every language (D117), filter compacted by chunks (D119), parallel `reduce` of a filter (D120), `map` of loops cut per thread (D121), non-temporal stores for results larger than the caches (D122)
    - [x] Campaign on the tuned machine (D124, `results/stage2-summary.md`): the lowest median in 20/20 cases on one core, 12/12 with `allow_reassoc`, 79/80 with threads; the 80th (large `filter_sum`, 8 threads, within the dispersion) explained in D120
    - [ ] Compilation time of a `reduce` of a filter (0.28 s for `filter_sum`, about 0.1 s for the others): the sequential and parallel streams are both emitted
- [ ] The points of SPEC section 16 deferred to v1 or after phase 7 (`require_vectorized`, parallel fixed-form float `scan`, `scatter`, functions stored in structures, mutual tail recursion, automatic layout)
  - [x] `scatter` in the prelude, collisions in the order of k (D111); parallel `scatter` in general: not yet
  - [x] Parallel fixed-form float `scan` by blocks of 4096 (D112)
  - [x] Mutual tail recursion with `tailcc` (D113)
  - [x] Functions stored in tuples, records and arrays; an array of functions indexed at run time is a branch (D114)
  - [ ] Functions in sum types, functions returned by functions
  - [ ] `require_vectorized` (outside the specification for now), automatic layout
- [x] `extern` (SPEC section 12, D125)
- [x] A `map` building arrays of arrays (D126)
- [x] String literals and comparison (not in SPEC v0's grammar: part 3; D134)
- [ ] The ARM64 target (SPEC section 1: "x86-64, then ARM64"; not a phase of stage 2)
- [ ] Part 3 (designs decided provisionally in D133, to be confirmed by the author)
  - [x] Strings: literals, `++`, order by bytes, `len`, `bytes`, `string_of`, `to_string`, `parse_f64`; owned and freed as arrays; `reduce` and `scan` of owned values (D134)
  - [x] Text input/output: `read_text`, `write_text`, `lines`, `split`, `trim`, `join`; `len` of arrays (D135)
  - [x] CSV: `parse_csv_f64`, `read_csv_f64` with header lines skipped (D135)
  - [x] `Result` and the fallible actions; components of new tuples taken without copies (D138)
  - [x] Modules: `import`, qualified names, `$ISC_PATH` (D139)
  - [ ] Arrays of arrays of strings (refused at compile time for now)
- [ ] Benchmarks Game programs written idiomatically (D136)
  - [x] `++` on arrays, bitwise operators, `print_bytes`; `examples/benchmarks_game/mandelbrot.isc` (D137)
  - [x] `mandelbrot` vectorized without changing the program: SPMD calls of tail-recursive functions (D140); `--isa avx` (D141)
  - [x] Their fastest C++, C, Rust and Go `mandelbrot` built and checked in a Docker image of ours (`bench/benchmarks_game/`, D142); 8 chunks per thread for maps of loops (D143); cheaper SPMD loops (D144)
  - [x] The gap in `ivybridge` mode: dynamic chunks (D145), uniform branches in SPMD bodies (D146), the program testing every 4 iterations as theirs do (D147): first on one thread in both modes, first with threads in native mode
  - [ ] Their measurement with the protocol of SPEC section 14 (machine.sh at a fixed frequency, the container in `isc-bench.slice`): the ranking with threads in `ivybridge` mode is within the noise of the turbo
  - [ ] Their architecture emulated: `--isa avx` (Ivy Bridge: AVX without AVX2 or FMA) for Isocron, `-march=ivybridge` / `target-cpu=ivybridge` / `GOAMD64=v2` for the others, 4 physical cores, lowest of 12 runs as they report it; a real i5-3330 only to compare with their published numbers
  - [ ] `spectral-norm`, `n-body`, `binary-trees`, `fannkuch-redux`; `fasta`, `k-nucleotide`, `reverse-complement` with text
- [x] Syntax 2, inspired by TypeScript (D148, decided by the author)
  - [x] Lexer and parser lowered to the same AST; method calls, `.length`, `.slice`, `.with`, `{ ...r, f: v }`, decorators, `import * as m from "./m";`, `declare function` (D148)
  - [x] Every program of the repository converted with the same AST checked (380 programs), the old parser and the converter then removed (D149)
  - [x] Messages and the memory report in syntax 2; a value given by `return`, in actions too (D149)
  - [x] SPEC section 3 (grammar, precedence of JavaScript) and the guide rewritten; examples written idiomatically (D149)
  - [ ] Named imports `import { f } from "./m";`, template literals

## v1 — the language finalized (goal of 2026-11-02)

- [x] (1) Syntax: named imports `import { f, T } from "./m";`, `export` (what is not exported is private), modules in subdirectories, template literals `${...}` (D150)
  - [x] Type aliases `type X = { ... };` and constants of the top level `const X = ...;`, found missing while writing (4) (D159)
- [x] (2) Known holes: functions in sum types, functions returning functions, a recursive function given a lambda that reads a size, arrays of arrays of strings; "Not implemented yet" of the guide holds only what DECISIONS defers after v1 (D152, D153, D154)
- [x] (3) Standard library: `sort`, `sort_by`, `abs`, `floor`, `ceil`, `round`, `trunc`, `tan`, `atan`, `atan2`, `asin`, `acos`, seeded random numbers, standard input, `to_fixed`, `pad_start`, string functions, `Map<K, V>`, `bigint`, regular expressions
  - [x] `Math` functions, string functions of TypeScript, `toFixed`, standard input, `random`, `sort`, `sortBy`, `toReversed`, `iota(n)` of any integer (D155)
  - [x] `bigint`; pure parsing; `Number`, `parseFloat`, `parseInt` (D156)
  - [x] `Map<K, V>`, `Option`, `??`, `countBy` (D158)
  - [x] Regular expressions (D157)
- [x] (4) The ten Benchmarks Game programs, outputs identical to the references in our Docker image (D160)
- [x] (5) SPEC section 16 "avant la v1" decided and written; grammar = parser; every feature in the guide with a tested example (D161)
- [x] (6) Interpreter and compiled tests, Valgrind, compile-time checks, `cargo test` with both allocators (D162; 90 suites, 0 failures with both allocators on 2026-11-03)
- [x] (7) One commit per part, without any reference to the assistant

## Performance — the ten programs at the level of the fastest (goal of 2026-11-03)

Method: every optimization from a profile and a model of cost of the primitive, never from a difference with another language alone; every gap left explained by a measurement. First survey (4 threads, the sizes of the Benchmarks Game): binarytrees 34.4 s, fannkuchredux 29.9 s, knucleotide 29.2 s, pidigits 29.7 s, nbody 19.2 s, regexredux 2.62 s; fasta, mandelbrot, revcomp, spectralnorm at the level of the fastest.

- [ ] (1) Allocation
  - [x] Components of a tuple accumulator owned, fields in the order of the program: nbody 27.6 s -> 9.9 s (D164)
  - [x] An array of at most one block filled without a task: nbody 8.1 s -> 6.9 s (D172)
  - [x] An array literal of scalar constants is data of the program: nbody 6.9 s -> 6.4 s (D173)
  - [ ] Parameters of a tail-recursive function owned (nbody: 3 copies per step)
  - [x] Fast path of the allocator: 63 -> 25 instructions per allocation, 43 -> 28 per free; nbody 9.9 s -> 8.4 s (D165)
  - [ ] Small arrays of a fixed size without the heap
  - [x] Nodes in cells of their size without a header, a free list per thread: binarytrees 21 on 4 threads 43.3 s -> 4.3 s (D166)
  - [x] A constructor without payload of a recursive type without allocation: binarytrees 21 on 4 threads 4.3 s -> 2.5 s (D169)
  - [ ] Non-atomic reference counts in a task none of whose nodes can leave it; the allocation of nodes inlined in the compiled code; allocation by regions (binarytrees)
- [x] (2) `Map` and `countBy` by a table of hashes: knucleotide 36.9 s -> 3.4 s on 4 threads (D167)
- [ ] (3) `bigint` in limbs of 64 bits (pidigits: 78 % in the conversion of base 128)
  - [x] Stored in bytes in the compiled programs, converted by words: pidigits 10000 42.6 s -> 11.6 s (D168)
  - [x] The common operations (add, subtract, multiply, divide, compare) on the bytes themselves: pidigits 10000 11.6 s -> 2.5 s (D171)
- [ ] (4) Code of scalar recursions (fannkuchredux)
  - [x] The program by chunks, without divisions, one pass: 41.7 s -> 4.3 s on 4 threads (D170)
- [ ] (5) Copies of strings, cache of regular expressions (regexredux)
- [ ] (6) Audit of the representations chosen without measurement, each with a microbenchmark and its model of cost
- [ ] (7) The ten programs measured against the fastest C, C++, Rust and Go in Docker; `results/benchmarks_game.md`
