# Decisions

Choices made while implementing the POC (SPEC.md section 15) where the
specification was silent, ambiguous, or could not be followed on the test
machine. Each entry says what was decided and why. Questions that the
specification itself must settle are also mirrored in SPEC.md section 16 terms
("to be settled") when relevant.

## Environment

- **D1 — Toolchains installed in user space.** The machine had no Rust, no
  Zig, no clang binary and no LLVM development files (only the runtime library
  `libLLVM.so.21.1`), and `sudo` requires a password. Rust 1.99.1 was installed
  with the official `rustup-init` (profile `minimal`, `--no-modify-path`, in
  `~/.cargo` and `~/.rustup`); Zig 0.14.1 was downloaded from ziglang.org
  (SHA-256 checked) and extracted to `~/.local/opt/zig-x86_64-linux-0.14.1`.
  Scripts find tools through `PATH` first, then these locations.
- **D2 — Clang is `zig cc`.** No clang binary is installed; Zig 0.14.1 ships
  clang 19.1.7, used through `zig cc` as the "Clang" C compiler of the
  benchmark protocol (section 14). Its default target is musl; the benchmark
  scripts pass `-target x86_64-linux-gnu` so it links against the same glibc as
  GCC.
- **D3 — LLVM back-end through the LLVM C API, not inkwell.** inkwell needs
  `llvm-config` and the LLVM headers, which are absent. The system library
  `libLLVM-21.so` exports the full C API, so `isc-codegen` emits textual LLVM IR
  and compiles it with a few hand-written `extern "C"` declarations
  (`LLVMParseIRInContext`, `LLVMRunPasses("default<O3>")`,
  `LLVMTargetMachineEmitToFile`). Section 13 marks inkwell as a
  recommendation, so this stays within the specification. Textual IR also makes
  the generated code easy to test and inspect.
- **D4 — Test CPU has no AVX.** The machine is an Intel Celeron J3455
  (Goldmont): SSE4.2, no AVX/AVX2/FMA. Consequences: SIMD in the assembly is
  SSE2 (`addpd`, `mulpd` on `xmm` registers); FMA contraction is impossible on
  this CPU for every language, so the strict/contracting distinction of section
  14 cannot change any result here; Go is built with `GOAMD64=v2` instead of
  `v3` (a `v3` binary refuses to start on this CPU).

## Front-end (P1)

- **D5 — Layout rule.** The grammar's INDENT/DEDENT are realized by an
  offside rule on columns (documented at the top of
  `crates/isc-syntax/src/parser.rs`): a block (a `fn`/`action` body or a `let`
  chain) has the column of its first token; a line starting at that column
  starts the next item, a line starting further left closes the block, a line
  starting further right continues the current expression, and line breaks
  inside `()`, `[]`, `{}` are ignored. This accepts every example of the
  specification, including `  = let y = x * 2.0` followed by `    y - mean(y)`.
- **D6 — Tabs.** A tab in indentation is a lexical error (column semantics
  would otherwise be ambiguous).
- **D7 — Unary minus on literals.** `-5` is the literal −5 (so `-128i8` is
  valid), but the folding happens after `^` is parsed: `-2 ^ 2` is `-(2 ^ 2)`,
  as section 3 requires.

## Typing (P2)

- **D8 — Literal typing.** An integer literal may take any integer type and
  also `f64` (the value `2` is exactly representable as a float; it is typed
  directly, not converted). A floating-point literal may only take a float
  type. Without context: `i64` and `f64` (section 4). An integer literal out of
  the range of its final type is a compile error. In `x ^ k` the exponent is
  typed independently of the base, so `x ^ 2` uses the integer exponentiation
  of section 10.
- **D9 — When a size mismatch is a compile error.** Section 4 says that sizes
  not provably equal get a run-time test. We refine: if the difference of the
  two normalized polynomials is a non-zero constant (`[n]` vs `[n + 1]`,
  `[3]` vs `[4]`) the sizes can never be equal, so it is a compile error.
  Run-time tests are inserted only for element-wise operations on two arrays
  and for function arguments (the test of section 4 for `dot(a, b)`); other
  positions (result type, both branches of `if`, `let` annotations) require
  provable equality.
- **D10 — Size variables of a signature (POC).** Each size variable must
  appear alone as the size of a top-level array parameter (`x: [n]T`), which
  binds it; a variable that only appears in the result type
  (existential result of a user function) and `size` parameters are not
  part of the POC subset. Existential sizes do exist for the primitives
  `read_f64` and `read_bytes`.
- **D11 — Classes.** `Num` implies `Eq`, `Ord` implies `Eq`; `Num` does not
  imply `Ord`. Integer and float types are in all three; `bool` is `Eq` and
  `Ord`; arrays and tuples are `Eq` when their elements are.
- **D12 — Array equality.** `a == b` on arrays compares by value
  (section 5); arrays of different lengths are unequal (no size test, no
  panic).
- **D13 — Lambdas are pure.** A lambda body is checked as a `fn` body: no
  action may be called inside, including the function given to `bench`.
- **D14 — Recursion.** Not in the POC subset (section 15 lists no
  recursion); a call cycle is a compile error. Everything is inlined.
- **D15 — `iota(n)` and `fill(n, v)`.** Their size argument must be a size
  expression (integer literals and size variables with `+ - *`), since the
  result size is part of the type.
- **D16 — Standard library.** `sum`, `product`, `mean` and `dot` are written in
  Isocron (`crates/isc-types/src/prelude.isc`); user declarations shadow them.
  `sqrt` and the conversions are primitives.
- **D17 — Not in the POC.** `f32`, records, sum types, `match`, slices,
  `extern`, modifiers, `filter`/`scan`/`window`/`zip`, `pow` (`x ^ y` with a
  float exponent) and arrays of non-scalars are parsed but rejected by the
  checker with an explicit "not supported in the POC" message.

## IR and interpreter (P3)

- **D18 — Where size tests run.** Section 4 places the test "at function
  entry". Since everything is inlined, the test of a call is emitted right
  after its arguments are evaluated, and the test of an element-wise operation
  right before the operation. In a `fn` the evaluation order is free
  (section 5), so the only observable difference would be which panic is
  reported, which section 5 leaves unspecified.
- **D19 — `and` / `or` short-circuit.** They are lowered to
  `if a then b else false` / `if a then true else b`, so `i < n and x[i] > 0`
  does not panic. The specification does not say; this is the behavior of C,
  Rust, Go and Zig.
- **D20 — Panics.** Message `panic at line L, column C: <message>` on standard
  error, exit status 101 (as Rust). Standard output is flushed first. The
  interpreter and compiled programs use the same messages (`isc-runtime`).
- **D21 — `print`.** Integers in decimal, floats in the shortest decimal form
  that reads back to the same value (Rust's `Display` algorithm: positional,
  no exponent; `NaN`, `inf`, `-inf`, `-0`), booleans `true`/`false`, tuples
  `(a, b)`, arrays `[a, b]`, then a newline.
- **D22 — `bench(r, f)` with `r < 1`** panics: there is no last result to
  return.
- **D23 — `args`** does not include the program name: `args[0]` is the first
  argument, as in the programs of section 15.
- **D24 — The interpreter runs on the optimized IR.** The passes never change
  a value (fusion only changes where elements are computed, ownership only
  adds moves, copies and frees). The interpreter ignores fusion and frees but
  honors moves, so that `fold` accumulators are updated in place: copying a
  256-entry histogram at each of 10⁷ steps would make the oracle unusable on
  the large inputs.
- **D25 — Ownership analysis of the POC.** The uniqueness analysis is the
  restricted one described at the top of `crates/isc-ir/src/passes.rs`: a
  value is moved at its last use if that use is in the same region (not inside
  a lambda body or an `if` branch entered after its definition); otherwise a
  consuming use (`with`, `fold` initial value, results) copies. It is enough
  for the "obvious uniqueness" required by section 15 (fold accumulator) and is
  safe in all other cases; the step-by-step algorithm of section 16 remains
  open.

## Code generation (P4)

- **D26 — Back-end pipeline.** `isc-codegen` emits textual LLVM IR, which is
  parsed, verified and optimized by `libLLVM-21` with the `default<O3>`
  pipeline for the host CPU and its features (the equivalent of
  `-O3 -march=native`), then emitted as a PIC object file and linked by the
  system `cc` with `libisc_runtime.a`. No fast-math flag is ever emitted
  (section 13; a test checks the generated IR).
- **D27 — Explicit vectors.** The 16 accumulators of the fixed reduction form
  are one `<16 x T>` value, so the reduction loop is SIMD by construction and
  the order of operations is exactly that of section 10, whatever LLVM does
  (it cannot reassociate without fast-math flags). Element-wise loops are also
  emitted 16 elements at a time when every operation of the element
  computation has a vector form (arithmetic, comparisons, conversions, `sqrt`,
  `^` with a loop-invariant exponent, `if` with side-effect-free branches as a
  `select`); otherwise a scalar loop is emitted. A scalar value used in a
  vector loop is the same in every lane and is broadcast.
- **D28 — Partial results of `reduce`.** When `n > B`, the block results are
  stored in a heap buffer of `ceil(n / B)` elements, reduced in place level
  by level, then freed. This allocation belongs to the implementation of the
  fixed form; it will appear in the memory report (stage 2, phase 2).
- **D29 — `bench`.** The body is compiled into a separate `noinline`
  function, called `r` times through a function pointer laundered by an empty
  `asm` statement: LLVM sees an unknown callee and can neither merge, hoist nor
  delete the calls (section 12). The results of the first `r - 1` calls are
  freed inside the timed region (they are part of the cost of the body).
- **D30 — Action statements are never removed.** The value of each `let` of an
  action is passed to an empty `asm` with a memory clobber, so LLVM can neither
  delete it nor move it across another statement (section 12).
- **D31 — Bounds checks.** Every index is checked; checks that LLVM proves
  useless are removed by LLVM (for instance a `u8` index into a `[256]` array,
  visible in the histogram's assembly). The report of remaining checks is part
  of the memory report (stage 2, phase 2).
- **D32 — Runtime.** Written in Rust (`isc-runtime`, built as `rlib` for the
  interpreter and `staticlib` for compiled programs). Arrays are allocated with
  `aligned_alloc(64, ...)`. The driver looks for the most recent
  `libisc_runtime.a` next to itself (or in `$ISC_RUNTIME`).

## Reference programs (P5)

- **D33 — Variants.** Each language has, per kernel, an *idiomatic* variant
  (what one writes first: a sequential loop or iterator) and an *opt*
  variant, hand-optimized: for the three float kernels it implements the fixed
  form of section 10 (16 accumulators, blocks of 4096, tree), hence
  bit-identical results; for the histogram it uses four sub-histograms merged
  at the end (a classic way to break the store-to-load dependency). C and Rust
  express the 16 accumulators as a small array the compilers vectorize; Zig
  uses explicit `@Vector(16, f64)`; Go has no SIMD (without assembly) and uses
  16 scalar accumulators.
- **D34 — FMA in Go.** Go allows fusing `x*y + z`; every product is written
  `float64(a * b)`, which the Go specification defines as a rounding point, so
  no fusion can happen whatever `GOAMD64` is. Rust never contracts; Zig does
  not in the default float mode; C uses `-ffp-contract=off`. On this CPU there
  is no FMA instruction anyway (D4).
- **D35 — Reassociated variants.** Following the table of section 14: C (GCC
  and Clang, `-fassociative-math -fno-signed-zeros -fno-trapping-math`) and Zig
  (`@setFloatMode(.optimized)`) on the idiomatic code; none for Rust and Go.
  They are reported separately, for information: `allow_reassoc` is not part
  of the POC.
- **D36 — Same program interface everywhere.** Every reference program takes
  the arguments of the Isocron program (`<inputs> <output.f64> <reps>`),
  times only the kernel, prints nanoseconds and writes the result in the same
  binary format, so that `bench/run.py` treats all implementations alike.

## Measurements (P6)

- **D37 — Sizes.** Small: 32 768 `f64` per array (256 KiB; two arrays for the
  dot product, 512 KiB) and 262 144 bytes for the histogram, all within the
  1 MiB of L2 cache shared by each pair of cores. Large: 10⁷ elements.
- **D38 — One measurement.** One process run that times `r` calls, `r` being
  calibrated so that the run lasts at least 100 ms; 5 warm-up runs are
  discarded, 30 are recorded; the process is pinned with `taskset -c 3`.
- **D39 — Frequency.** The governor (`ondemand`) and the turbo cannot be
  changed without root. The warm-up runs bring the core to its frequency; the
  IQR in the report shows the remaining noise.
- **D40 — "Beats" and "equals".** Section 14 says Isocron beats a language
  when its median is lower than that language's best median by more than the
  measured dispersion. The dispersion is taken as the larger of the two IQRs.
  "Equals" (decision point of section 15) means the two medians are within that
  dispersion.

## Measurement protocol, revision of 2026-11-01

- **D41 — Fixed frequency and isolated cores** (replaces D39). With root
  access, `bench/machine.sh apply` sets the governor to `performance`, turns
  the turbo off (the maximum frequency becomes the base 1.5 GHz) and allows the
  CPU counters for `perf`. It also confines every other process (the
  `user.slice`, `system.slice` and `init.scope` cgroups) to cores 0-1, which
  share one L2 cache; `bench/machine.sh run` starts the benchmarks in their own
  slice on cores 2-3, the other L2 pair, and each program is pinned to core 3.
  All of it is `--runtime` and undone by `bench/machine.sh restore`. Without
  the isolation, a browser and the desktop shared the measured core and its
  L2: the IQR reached several percent; with it, it is below 1 % for most runs.
- **D42 — Interleaved measurements.** Each round runs every implementation of
  a kernel once, in an order that rotates from round to round, so that a slow
  drift of the machine affects them alike. The minimum is reported next to the
  median and the IQR.
- **D43 — Cycles.** `perf stat` counts the whole process, data loading
  included, so it is not used for the time per call. One extra run per
  implementation measures the effective frequency (user-space cycles divided
  by task clock); the report converts the median time into cycles per element
  with it.
- **D44 — Zig reference fixed.** The fixed form of the Zig `opt` variants
  allocated its partial results with `std.heap.page_allocator`, that is two
  system calls per call, which penalized them unfairly (small sum: 46.8 µs
  before, 21.7 µs after). They now use a stack buffer up to 4096 partial
  results, like the C and Rust variants use `malloc`.
- **D45 — `GOAMD64` follows the CPU.** `bench/build.sh` uses `GOAMD64=v3`
  (the value of section 14) when the CPU has AVX2, FMA and BMI2, and `v2`
  otherwise (the Celeron test PC, D4). From 2026-11-01 the project lives on
  the server xeon (Intel Xeon E-2388G, AVX2 and AVX-512); the results of the Celeron PC stay in
  `results/poc.md` and `results/poc.json`.

## Small executables (2026-11-01)

- **D46 — A runtime without the Rust standard library** (replaces D32). The
  runtime of compiled programs (`crates/isc-runtime`) uses only `core` (number
  formatting, including the shortest round-trip printing of floats, the same
  algorithm as `std`) and direct calls to the C library (`open`, `read`,
  `write`, `aligned_alloc`, `clock_gettime`, `strerror`). Standard output is
  buffered and flushed at each newline and before a panic. Because a `no_std`
  static library must use `panic = "abort"`, which cannot be set for one crate
  of a workspace, the runtime is its own workspace; `crates/iscc/build.rs`
  builds it and `iscc` embeds it, so `iscc` needs no file at run time
  (`$ISC_RUNTIME` may still name another archive). The precompiled `core` is
  built for unwinding and its unwind tables refer to `rust_eh_personality`:
  the runtime defines it empty, since nothing ever unwinds. The texts shared
  with the interpreter (panic messages, OS errors in the form of
  `std::io::Error`, printing of floats) live in the `no_std` crate
  `isc-runtime-core`.
- **D47 — Linking.** Programs are linked with `-Wl,--gc-sections` and
  stripped; `iscc build --debug` keeps the symbols. They need only libc and
  libm (`fmod`, for `%` on floats). The four POC programs went from about
  5.4 MB to 34 976 bytes; the kernel code is unchanged and interleaved
  measurements show no difference beyond noise.

## Improvements after the POC, on the server xeon (2026-11-01)

- **D48 — V and the instruction set as experiment options.** `iscc build
  --lanes V --isa native|avx2|avx512` and `iscc run --lanes V` change V, the
  number of accumulators of the fixed reduction form, and the instruction set
  of the back-end. They exist to study SPEC section 16 ("Valeurs de V (16) et
  B (4 096)"); results depend on V, so a program built with another V than
  the one of the specification (16, then 32 since D57) is not a conforming
  Isocron program. `avx2` removes
  every AVX-512 feature of the host; `avx512` adds the function attributes
  `min-legal-vector-width=512` and `prefer-vector-width=512`, so that explicit
  512-bit vectors stay in `zmm` registers. The element-wise loops keep 16
  elements per iteration whatever V (speed only). The hand-optimized C, Rust
  and Zig references take V at compile time (`-DLANES`, `ISC_LANES`,
  `lanes.zig`); `bench/build_lanes.sh` and `bench/lanes.py` build and measure
  the matrix; Go has no SIMD and is left out.
- **D49 — Block parallelism (stage 2, phase 6).** The first level of every
  `reduce` and every materialized element-wise loop are cut into blocks of B
  elements computed by the tasks of `isc_parallel_for`, a pool of POSIX
  threads in the runtime (spinning briefly, then sleeping on a futex). Task j
  writes only its own result slot or output range, and the upper levels of the
  tree stay sequential, so the result does not depend on the number of
  threads; tasks are distributed statically (thread k runs k, k + T, ...).
  Loops inside a task stay sequential (no nested parallelism). `fold` stays
  sequential, except under rule P (D50). The number of threads is
  `$ISC_THREADS`, by default the number of CPUs the process may run on, so a
  program pinned to one CPU runs single-threaded. Tests run the POC programs,
  a three-level reduction and materialized maps with 1, 2, 3, 4 and 8 threads,
  and the differential test runs every random program with 1 and 3 threads.
- **D50 — Rule P, privatized accumulation (proposed rule for SPEC section 16,
  "`scatter` et règle de collision").** A `fold(x, init, f)` whose function is
  `(c, v) -> c with [g(v)] = c[g(v)] + h(v)` (or `h(v) + c[g(v)]`), where the
  elements of `c` are integers and neither `g` nor `h` uses `c`, may be
  computed by cutting `x` into consecutive chunks, accumulating each chunk
  into a private copy of zeros of the length of `init`, and adding the copies
  to `init` element by element. Integer addition modulo 2^bits is associative
  and commutative, so the result is identical to the sequential `fold` for any
  chunking, hence for any number of threads. Index checks are kept: an index
  out of bounds panics with the position of the read `c[g(v)]`, as the
  sequential order of section 8 (rule 2) would. The compiler uses one chunk
  per thread and, inside each chunk, four interleaved copies (four
  independent dependency chains when values repeat). The rule does not apply
  to float accumulators (addition is not associative) nor to any other shape
  of update. Accepted by the author of the specification and written into
  SPEC section 6 (D57).
- **D51 — Parallel references.** C with OpenMP (GCC only: `zig cc` does not
  ship the OpenMP runtime, so Clang has no parallel variant), Rust with rayon
  (fetched from crates.io), Go with goroutines (`GOMAXPROCS`), Zig with
  `std.Thread.Pool`. *Idiomatic*: the usual parallel reduction of the language
  (OpenMP `reduction(+)`, rayon `par_iter().sum()`, one chunk per goroutine or
  thread); its float result depends on the number of threads. *Opt*: the blocks
  of the fixed form in parallel, bit-identical to Isocron for any number of
  threads. Histograms: one private histogram per thread (with four
  sub-histograms in *opt*), merged.
- **D52 — Skewed histogram input.** `histogram_skewed` runs the histogram
  programs on bytes of geometric law (value k with probability 2^-(k+1): half
  the bytes are 0), the case where successive increments hit the same counter
  and wait for each other.
- **D53 — Measurement conditions on xeon.** `bench/machine.sh` now derives
  the isolation from the topology (`single`: the second half of the physical
  cores for the benchmarks; `parallel`: the first hardware thread of every
  core), but applying it (governor, turbo, cgroups) was refused on this server
  by the permission system of the session, which runs services (nginx, Plex,
  Docker). The xeon measurements therefore run with the turbo enabled and
  without confinement, pinned with `taskset` (single-thread: CPU 7; T threads:
  the first hardware thread of the first T physical cores), and the reports
  say so. With interleaved runs the IQR stayed small (most below 1 %).
  Superseded by D58: the measurements were run again at fixed frequency.
- **D54 — C reference fixed (fairness).** The hand-optimized C dot product
  and standard deviation read their inputs through a context structure at
  every element; GCC did not vectorize that loop with V = 16 (dot on 2048
  elements: 1.13 µs instead of about 120 ns). The macro `FIXED_SUM_OF` now
  loads the context into local `restrict` pointers once per block; the result
  is unchanged (bit-identical) and GCC vectorizes it. **Caveat:** the Celeron
  results of `results/poc.md` were measured before this fix, so their
  "C (GCC), opt" rows for dot and std_dev are pessimistic; that campaign must
  be run again on the Celeron to compare Isocron with the corrected
  reference (the xeon reports use the corrected one).
- **D55 — Recommendation on V (SPEC section 16, "Valeurs de V (16) et
  B (4 096)").** Measured on xeon (`results/xeon-lanes.md`), Isocron's
  time relative to V = 16 with the same instruction set:

  | Kernel, size | AVX-512 V=32 | AVX-512 V=64 | AVX2 V=32 | AVX2 V=64 |
  | --- | --- | --- | --- | --- |
  | sum, L1 (2048) | 0.64 | 0.61 | 0.64 | 0.62 |
  | dot, L1 | 1.01 | 1.01 | 1.01 | 1.36 |
  | std_dev, L1 | 0.81 | 0.81 | 0.86 | 0.85 |
  | sum, L2 (16384) | 0.74 | 0.78 | 1.03 | 1.09 |
  | dot, L2 | 0.99 | 1.04 | 1.06 | 1.08 |
  | std_dev, L2 | 0.90 | 0.92 | 1.01 | 1.04 |

  (AVX2 columns: 72/112 ns, 124/123 ns, 256/298 ns, 976/951 ns,
  2.03/1.91 µs, 2.38/2.35 µs for V = 32; 69/112 ns, 167/123 ns, 254/298 ns,
  1.04 µs/951 ns, 2.07/1.91 µs, 2.45/2.35 µs for V = 64.)
  V = 16 leaves compute-bound reductions latency-bound on AVX2 and AVX-512:
  16 doubles are only 2 `zmm` or 4 `ymm` independent chains, fewer than the
  latency of a floating-point addition times the number of adders. V = 32
  gives up to 36 % less time when the data are in L1, and with AVX-512 also in
  L2; it costs at most 6 % (dot product in L2 with AVX2), where the loop is
  bound by its loads. V = 64 brings nothing more and costs more. The hand-
  optimized C, Rust and Zig references show the same trends, so the gain
  belongs to the reduction form, not to Isocron's code generator.
  **Recommendation: V = 32**, provided that it is first measured on the
  targets not covered here — the SSE2-only Celeron of the POC (16 `xmm`
  registers: 32 accumulators of 2 doubles would use all of them, so spills
  are possible) and ARM64 (NEON, 2 doubles per register, 32 registers). B =
  4096 stays a multiple of 32. Since V changes results, it must stay one
  value for every target (section 10); the decision belongs to the author of
  the specification, and SPEC.md is not modified. Follow-up: the SSE
  measurement showed a cost that the code generator now removes (D56), and
  the author adopted V = 32 (D57).

## Decisions of the author and follow-up (2026-11-01)

- **D56 — Reductions in several passes when registers are short (speed
  only).** Measured before adopting V = 32 (D55 required it), the code of the
  Celeron J3455 of the POC (SSE4.2 without AVX, 16 `xmm` registers of 2
  doubles; `iscc build --isa sse`, LLVM CPU `goldmont`, run on xeon) was
  much slower with V = 32: 32 accumulators take all 16 registers, and LLVM
  spilled accumulators inside the loop (dot product on 2048 elements: 699 ns
  instead of 370 ns with V = 16, at fixed frequency). The code generator now
  splits the V accumulators of `reduce_block` into groups, one pass over the
  block per group (`reduce_passes`): the registers of one group, plus one per
  array read (one less with AVX, which folds a load into the arithmetic
  instruction), must fit in the vector registers of the target
  (`Isa::vector_registers`). Pass p updates only the accumulators of group
  p, each with its elements in increasing order, so the result is exactly
  the fixed form of section 10 for any number of passes; the tail and the
  tree are unchanged. The rows are taken in chunks of at most 16 KiB of
  input (`reduce_chunk`), every pass running over a chunk before the next
  one, so that cache lines fetched for one pass are still in L1 for the
  others; between passes the accumulators go through memory with volatile
  accesses (kept in registers, LLVM spilled the accumulators of the next
  pass inside its loop). The rule was fitted on measurements at fixed
  frequency (A/B against one pass, same binaries otherwise): it splits only
  SSE with V >= 32 and AVX2 dot products with V = 64; time relative to one
  pass from 0.54 (dot, V = 64, SSE, L1) to 1.04 (sum, V = 32, SSE, L2), and
  0.57 for the dot product with V = 32 on SSE in L1 (699 ns -> 401 ns). AVX-512 and
  the native target of xeon (32 registers) never split for V <= 64. The
  test `reductions_in_passes_match_interpreter` compiles the POC kernels for
  SSE (V = 32, 64) and AVX2 (V = 64) and compares them bit for bit with the
  interpreter, tails included. For fairness, the hand-optimized references
  (C `GROUP1`/`GROUP2`, Rust `GROUP1`/`GROUP2`, Zig `group1`/`group2`) apply
  the same grouping (without the chunks); Go keeps its accumulators in
  memory anyway (no SIMD), so V only changes the length of its inner loop.
  `bench/build_lanes.sh` and `bench/lanes.py` add the `sse` column (CPU
  `goldmont` for GCC, Clang, Rust and Zig).
- **D57 — V = 32 and rule P adopted by the author of the specification.**
  SPEC section 10 now fixes V = 32 (B = 4096 stays provisional, section 16),
  and rule P (D50) is part of SPEC section 6 (`fold` is parallel under rule
  P). The compiler, the interpreter, the independent oracle of the tests and
  every reference program use V = 32 by default; `--lanes` stays an
  experiment option. Every result of a floating-point reduction changes
  (another fixed form), so every report was measured again with V = 32.
  ARM64 (NEON, 32 registers of 2 doubles) has not been measured.
- **D58 — Fixed frequency on xeon.** The author allowed `bench/machine.sh`
  with a local permission rule. The xeon reports
  are now measured with the governor `performance`, turbo off, minimum =
  maximum = 3.2 GHz, other processes confined: `single` mode (the benchmarks
  on cores 4-7, pinned to CPU 7) for `lanes.py` and `run.py`, `parallel`
  mode (the first hardware thread of every core for the benchmarks, the
  services on the sibling threads) for `parallel.py`; `machine.sh restore`
  afterwards. At fixed frequency, with the passes of D56 and the
  references of D56 and D59, Isocron's time with V = 32 relative to V = 16
  (`results/xeon-lanes.md`):

  | Kernel, size | SSE | AVX2 | AVX-512 |
  | --- | --- | --- | --- |
  | sum, L1 (2048) | 1.08 | 0.64 | 0.65 |
  | dot, L1 | 1.07 | 1.01 | 1.00 |
  | std_dev, L1 | 1.02 | 0.86 | 0.81 |
  | sum, L2 (16384) | 0.96 | 1.02 | 0.74 |
  | dot, L2 | 1.01 | 1.07 | 0.99 |
  | std_dev, L2 | 0.99 | 1.01 | 0.90 |

  So V = 32 costs at most 8 % on the SSE code of the Celeron (run on the
  Xeon; to be measured on the Celeron itself) and 7 % on AVX2 dot products,
  and saves up to 36 % elsewhere.
- **D59 — Rust reference vectorized (fairness).** The hand-optimized Rust
  fixed form indexed the rows given by `chunks_exact`; LLVM did not vectorize
  that loop in most configurations (scalar `vmulsd` in the dot product; in
  the study of V, dot on 2048 elements with V = 16 and AVX2: 621 ns, Zig 174
  ns). The rows are now fixed-size arrays (`slice::as_chunks`) and the
  accumulators are updated through iterators, without bounds checks: 213 ns
  for the same case, 167 ns with V = 32 and AVX-512 (956 ns before). The
  result is unchanged (bit-identical). The Rust "opt" rows of the reports
  measured before this fix (the Celeron `results/poc.md`, and the xeon
  reports before the re-run of D58) are therefore pessimistic. Stable Rust
  has no portable SIMD (`std::simd` is unstable), so the Rust reference still
  depends on the auto-vectorizer, which remains a little behind C and Zig in
  a few configurations of the study of V.
- **D60 — Developer guide.** `docs/language.md` describes the language as the
  compiler implements it (not the whole design of SPEC.md), with a list of
  what is not implemented yet. The test `crates/iscc/tests/docs.rs` runs
  every example of the guide with the interpreter and compiled, and compares
  both outputs with the output printed in the guide (blocks `isc`,
  `isc-check`, `isc-error`, `isc-panic`, `output`).
- **D61 — Three checker fixes found while writing the guide.** (1) The class
  `Ord` now allows `<`, `<=`, `>`, `>=` on a type parameter (the comparisons
  required a scalar type, which no class implied; every ordered type of the
  POC is a scalar). (2) An integer literal of a generic function is range
  checked once its type is known (`x[0] + 300` with `T = u8` was accepted and
  wrapped to 45; it is now the error of section 4). (3) An array whose
  element type nothing fixes (`print([])`) is a compile error instead of an
  array of `()`, which crashed the interpreter.
- **D62 — Compilation times.** `bench/compile.py` builds every benchmark
  program with the options of `bench/build.sh`, from copies of the sources
  (the repository is never modified), in two states: *cold* (every cache
  empty: a new Cargo target directory, `GOCACHE`, Zig cache) and *warm* (the
  caches filled, the program's own source changed by an appended comment,
  built again). It reports the median wall time and CPU time (user + system
  of the compiler and its children, `RUSAGE_CHILDREN`) of 3 cold and 5 warm
  builds, at fixed frequency on the 4 benchmark cores
  (`results/xeon-compile.md`). GCC and Isocron have no cache, so one
  measurement serves for both states. The POC programs are tiny, so these
  times say little about large programs: Isocron inlines every call, and
  its compilation time on large programs is an open question of SPEC
  section 16 (code growth).
- **D63 — Block parallelism at fixed frequency (`results/xeon.md`).**
  Measured again with V = 32, at 3.2 GHz, T threads on the first hardware
  thread of T physical cores, the services on the sibling threads. In the 10
  cases (5 kernels, 2 sizes), Isocron and every *opt* variant give the
  same result for 1, 2, 4 and 8 threads; the idiomatic floating-point
  variants of the four languages do not (6 cases each). With 8 threads,
  Isocron is the fastest on the small floating-point kernels (sum 913 ns,
  OpenMP 1.70 µs). Two weaknesses remain, recorded in TODO.md: the large dot
  product is slower with 8 threads than with 4 (3.829 ms, 3.448 ms; OpenMP
  3.499 ms with 8), and the skewed histogram is behind OpenMP (large: 612.87
  µs, OpenMP 473.37 µs; small: 18.40 µs, 15.42 µs).
- **D64 — Private copies of rule P in one block (fixes the skewed
  histogram).** With 8 threads, the skewed histogram took 612 µs, OpenMP 472
  µs. Hardware counters (`perf stat`) showed 39 % more cycles than with one
  thread for the same instructions, and 54.1 million loads blocked by "4K
  aliasing" (`ld_blocks_partial.address_alias`) per 100 calls. Cause,
  reproduced in a C program: each task allocated its four copies with
  separate `aligned_alloc` calls; glibc recycles the chunks between threads
  (the copy freed by the main thread goes to its cache), and the main thread
  received copies that were at the same offset in heaps aligned on 64 MiB,
  so three of its four copies had the same address modulo 4096 (2304, 2304,
  2304). The same hot counter of two copies then has the same 12 low address
  bits: every load waits for the store to the other copy, and the four
  independent chains become one; the other threads wait for the main one.
  Now each task allocates one block, with the copies `stride` bytes apart:
  the size of a copy rounded up to an odd multiple of 4096 / K bytes, so that
  the same counter of two copies is at least 1024 bytes apart modulo 4096.
  The stride is computed in the IR (a call to the runtime cost about 7 %:
  LLVM no longer saw a constant). Measured on the skewed input with 8
  threads: 495 µs (612 before), uniform input 488 µs (495), one thread
  unchanged (3836 µs, 3837 before). The result is unchanged.
- **D65 — The large dot product is bound by the memory bandwidth.** With 8
  threads, Isocron (and every *opt* variant: C 3.727 ms, Rust 3.757 ms,
  Zig 3.834 ms) is a little slower than with 4. The server has two DDR4-3200
  modules in dual channel, 51.2 GB/s in theory; the large sum already reads
  80 MB in 1.582 ms with 4 threads, about 50.6 GB/s, so 4 threads saturate the
  memory and more threads only add contention. Distributing contiguous ranges
  of blocks instead of `k, k + T, ...` does not help (dot with 8 threads:
  3.644 ms instead of 3.593 ms), so the distribution is unchanged. No change
  to the language or the compiler.

## Stage 2 — complete language (from 2026-11-01)

- **D66 — `f32` (stage 2, phase 1).** IEEE 754 binary32 everywhere: type
  `f32`, literals (`1.5f32`, or an unsuffixed literal whose context is
  `f32`), conversions `f32(x)` (integer and `f64` to `f32` rounded to
  nearest; `f32` to integer truncated and saturated, NaN 0; `f32` to `f64`
  exact), arithmetic, comparisons, `^`, `sqrt` (now `T -> T` for both float
  types), reductions in the fixed form (V = 32 lanes of 4 bytes), printing
  (shortest form that reads back to the same `f32`), `read_f32`/`write_f32`
  (little-endian; canonical NaN `0x7FC00000`, sign 0, quiet bit 1, payload 0,
  like the `f64` one). A float literal is rounded directly from its decimal
  text to binary32 (the lexer parses the text twice): going through `f64`
  would round twice and could differ in the last bit. `mean` stays
  `[n]f64 -> f64`: the classes of section 4 cannot express a conversion of
  `n` to a generic float type.
- **D67 — WebAssembly / WASI target (request of the author).** `iscc build
  --target wasm32-wasi` (`Isa::Wasm32`): LLVM triple `wasm32-unknown-wasi`,
  CPU `generic` with `simd128`, `nontrapping-fptoint`, `bulk-memory`,
  `sign-ext` (WebAssembly 2.0), static relocation model. The runtime is the
  same crate built for the Rust target `wasm32-wasip1` (embedded in `iscc`
  next to the native one), linked with wasi-libc (by `zig cc` at first, by `rust-lld` since D68); it differs only by the constants of the C library (`O_*`,
  `clockid_t` is a pointer in wasi-libc, `long` is 32 bits) and by the thread
  pool: WASI preview 1 has no threads, so `isc_parallel_for` runs its tasks
  in order, which gives the same result (section 10). The generated code
  does not use inline assembly there: the barriers of `bench` are volatile
  accesses. The functions of the runtime that produce an array
  (`isc_read_*`, `isc_args`) now write it through an out-pointer on every
  target, so that the generated code and the C ABI of the target cannot
  disagree on how a structure is returned. The reductions use the register
  model of SSE (16 registers of 16 bytes: the engine maps `v128` to `xmm`
  registers). Main stack 8 MiB (wasm-ld's default is 64 KiB). The tests
  compile every POC kernel and every example of the guide for WebAssembly and,
  when a WASI engine is installed (`wasmtime`, or `$ISC_WASI_RUNNER`), run
  them and compare them bit for bit with the interpreter. On xeon:
  wasmtime 38.0.4 (official release, in `~/.local/opt`, linked from
  `~/.wasmtime/bin`); the sum, dot product and standard deviation on 5 sizes,
  a panic (message and exit code 101) and every example of the guide give the
  same output as the interpreter. The modules import only
  `wasi_snapshot_preview1` functions and export `_start` and `memory`.
- **D68 — WebAssembly with threads; one toolchain for WebAssembly.**
  `iscc build --target wasm32-wasi-threads` (`Isa::Wasm32Threads`): the same
  code with the `atomics` feature, linked with a shared, imported memory (4
  GiB at most) and `wasi_thread_start` exported, as wasi-threads requires.
  Separate from `wasm32-wasi` because such a module only runs on engines
  with the threads proposal and wasi-threads (wasmtime: `-W threads=y -S
  threads=y`, marked experimental). The runtime keeps its thread pool there:
  threads of wasi-libc (`pthread_create`, which imports `wasi
  thread-spawn`), and the futex replaced by `memory.atomic.wait32` /
  `memory.atomic.notify`; these instructions are unstable in Rust, so the
  generated module defines `isc_wasm_wait32` and `isc_wasm_notify` with the
  LLVM intrinsics, and the runtime calls them. rustc gives `wasm32-wasip1`
  and `wasm32-wasip1-threads` the same `cfg` (`atomics` is an unstable
  feature, absent from `cfg` on stable), so `crates/iscc/build.rs` builds
  the threads runtime with `--cfg isc_wasm_threads`. The number of threads
  is `ISC_THREADS`, 4 by default (WASI has no way to know the number of
  CPUs). Both WebAssembly targets are now linked by `rust-lld` (wasm-ld)
  with the wasi-libc shipped by the Rust targets (`crt1-command.o`,
  `libc.a`), found through `rustc --print sysroot` (`$ISC_RUST_SYSROOT`):
  the wasi-libc of Zig has no threads, and one toolchain for both targets
  avoids two versions of the C library; Zig is no longer needed for
  WebAssembly. `main` is named `__main_argc_argv` there, as clang names it.
  OS errors are shown with their Linux number and text on WASI too
  (`ENOENT` is 44 in WASI, 2 on Linux; a directory, which WASI engines open
  and then refuse to read with `EBADF`, is reported `EISDIR`), so a
  program panics with the same message natively, as WebAssembly and in the
  interpreter. Tests: sum, dot product and standard deviation on 9 blocks
  plus a tail with 1, 2, 3, 4 and 8 threads, the histogram of rule P with
  1, 2, 3 and 8 threads (its tasks allocate through the shared allocator),
  and the messages of a missing file and of a directory, all equal to the
  interpreter under wasmtime 38.0.4.
- **D69 — Arrays of tuples, stored in columns (stage 2, phase 1).** An
  array whose elements are tuples (of scalars, strings or tuples) is one
  block: the column of each scalar leaf, in the order of the components,
  starts where the previous one ends, rounded up to 64 bytes (a cache line;
  every column is aligned for vector loads); the value is still `{ ptr,
  i64 }`, and the offsets are computed from the length. The interpreter
  stores the same arrays as a list of columns. The checker has an internal
  class "array element" (scalar, string, tuple of these, type parameter)
  for array literals, `map` results, `fill`, `reduce` and inferred arrays.
  `reduce` over tuples follows the fixed form with tuple accumulators, one
  element at a time (the vectorized path needs scalar elements); the block
  results of a parallel reduction use the same column layout. There is no
  `t.0`: SPEC section 3 has only `.field` (for records); tuples are taken
  apart with `let`. Arrays of arrays (`[n][m]T`) come later. Tests:
  `crates/iscc/tests/tuples.rs` (an `argmin`, columns of 1 and 8 bytes, 3
  blocks plus a tail, 1, 3 and 8 threads, WebAssembly) and an example of the
  guide.
- **D70 — Records (stage 2, phase 1).** Record types are structural: a
  record type is its set of fields, kept sorted by name, so `{a: T, b: U}`
  and `{b: U, a: T}` are the same type and a literal may give its fields in
  any order. In the IR a record is the tuple of its fields in that order:
  layout, equality, copies and arrays of records (stored in columns, D69)
  are those of tuples. `r.f` needs the type of `r` to be known where it is
  read (no inference of a record from its use; the error asks for an
  annotation); `r with .f = v` builds a new record; a pattern `{f: p, ...}`
  may name only some fields. `print` shows a record as `{price: 2.5,
  volume: 3}`, its fields in the order of their names: the names are kept
  for printing only, in a table of shapes of the IR program
  (`Program::shapes`, one per `print`), read by the interpreter and the code
  generator. `require_unique` applies to array updates only.
- **D71 — Sum types and `match` (stage 2, phase 1).** `type Name<T, ...> =
  A | B(T, ...)`: type parameters without class; constructor names are
  unique in the program and may not be function names; a constructor
  without payload is a value (`None`), one with a payload is applied
  (`Some(x)`), never used as a function value yet. A type that contains
  itself (directly or through other types) is rejected until reference
  counting exists (phase 3). Representation (section 4, "à plat, sans
  allocation"): an IR tuple, the tag (`u32`, the index of the constructor)
  then the slots of every constructor one after the other; the slots of the
  other constructors hold the zero value of their type (new IR node
  `Zero`), so equality is that of tuples, and an array of a sum type is
  stored in columns (D69). `match` is lowered to a chain of tests, first
  arm first; its arms must be exhaustive, by a simple rule checked at
  compile time: an arm that always matches, or one arm per constructor whose
  payload patterns always match, or `true` and `false` (deeper analyses,
  for example on tuples of constructors, are not done: such a `match` needs
  a last arm `_ -> ...`). The last arm is taken without a test. Patterns:
  `_`, names, integer, float (IEEE equality: a NaN never matches) and
  boolean literals, tuples, records, constructors. `let` patterns must
  always match. `print` shows a constructor and its payload: `Some(0.25)`,
  `Rect(2, 3)`, `None` (shape `Shape::Sum`). The parser now ends the
  scrutinee of `match` with its line: an arm that starts with `(` or `[`
  was taken as a call or an index of the scrutinee.
- **D72 — `size` parameters (stage 2, phase 1).** `k: size` is an `i64`
  local and a size of the signature (it can appear in the parameter and
  result types). At a call, its argument must be a size expression (literal,
  size variable, sum or product of these), whose value is then known in the
  caller's types, or a variable of type `i64`, which then stands for the
  size in the caller's types for as long as it is in scope; any other
  expression must be bound with `let` first (the size of the result must
  stay computable after the call). A negative argument is a panic, `negative
  size: -2`, at the position of the call (SPEC section 4: a size is positive
  or zero; new IR node `CheckNonNegative`, runtime function
  `isc_panic_negsize`).
- **D73 — Scope of phase 1.** Phase 1 is complete except for arrays of
  arrays `[n][m]T` (section 4): rows of an array are views of it (`window`
  is "une vue, jamais copiée", section 6), which needs the ownership rules of
  views; they come with `window` and slices in phase 5. The example of
  section 3 type-checks without its `require_no_alloc`, which is verified in
  phase 4 (a modifier is not accepted before the compiler can verify it:
  section 1, "garanties vérifiées à la compilation"); `[k <= n]T` is the
  type of `filter` (phase 5). The test `examples_of_sections_3_and_4` checks
  the other examples.
- **D74 — Why 8 threads are slower than 5 on large streaming reductions
  (investigation asked by the author).** Measured on xeon at 3.2 GHz,
  parallel mode, with the memory-controller counters
  (`uncore_imc/data_reads`, system-wide, the traffic of 4 calls subtracted
  from that of 44 so that only the timed loop counts). Time per call of the
  dot product on 10^7 doubles (160 MB read per call) and DRAM read rate:

  | Threads | Isocron | OpenMP opt | OpenMP idiomatic | C probe (fast-math) | DRAM reads (Isocron) |
  | --- | --- | --- | --- | --- | --- |
  | 1 | 6.793 ms | 6.790 ms | 12.829 ms | 6.881 ms | 22.4 GB/s |
  | 4 | 3.419 ms | 3.411 ms | 3.653 ms | 3.380 ms | 45.1 GB/s |
  | 5 | 3.396 ms | 3.369 ms | 3.379 ms | 3.364 ms | 44.8 GB/s |
  | 8 | 3.753 ms | 3.697 ms | 3.458 ms | 3.661 ms | 41.4 GB/s |

  The C probe (`a[i] * b[i]` summed with 8 vector accumulators, OpenMP,
  one contiguous chunk per thread, nothing from Isocron) shows the same
  curve, so the slowdown is a property of the machine, not of Isocron's
  code or thread pool: the DRAM itself delivers less (45 GB/s with 5
  threads, 41 GB/s with 8, out of 51.2 GB/s in theory for two DDR4-3200
  channels) when more threads stream at once. The core frequency stays at
  3.195 GHz for every count of threads (no power limit), and the services
  on the sibling threads used under 3 % of their CPUs during the runs. The
  effect grows with the number of concurrent streams: the sum (one array,
  8 streams with 8 threads) loses 4 % from 5 to 8 threads (1.570 ms ->
  1.635 ms), the dot product (two arrays, 16 streams) 11 %. This matches
  DRAM bank and row-buffer contention: more interleaved streams than the
  banks can keep open. The idiomatic OpenMP version loses less because its
  scalar loop issues its loads more slowly (it is twice as slow with few
  threads). Nothing changes in the compiler: capping the number of threads
  of memory-bound loops would need a rule that depends on the machine and
  would slow down compute-bound loops (the histogram scales up to 7.7x with
  8 threads); a program can set `ISC_THREADS`. Distributing contiguous
  ranges of blocks instead of interleaving them was already measured not to
  help (D65).
- **D75 — Where OpenMP was faster: the histograms (investigation asked by
  the author).** In `results/xeon.md`, the OpenMP histogram was ahead of
  Isocron by about 2 % (small) to 5 % (large), already with one thread;
  `perf` showed 9.52 instead of 6.01 billion instructions for the same work.
  The loop of rule P checked every index against the number of counters,
  which a task reads from its context, so LLVM could not remove the check
  (two instructions per element). An unsigned index of b bits is below
  2^b: the task now tests once whether the counters are more than 2^b - 1
  and then runs a copy of the loop without checks; otherwise every index is
  checked (SPEC section 5: the compiler removes the checks it proves
  useless). The loop is then the one GCC generates for the C reference,
  instruction for instruction. Fixed frequency, median of 7 runs, before ->
  after (OpenMP opt): small uniform, 1 thread 100.77 -> 98.16 µs (95.30),
  8 threads 15.99 -> 15.81 µs (15.80); large uniform, 1 thread 3785.77 ->
  3750.22 µs (3664.58), 8 threads 501.73 -> 492.50 µs (496.76); large
  skewed, 1 thread 3882.00 -> 3850.92 µs (3752.04), 8 threads 509.81 ->
  508.86 µs (484.87). Isocron is still slower than OpenMP with one thread
  in every case (2 to 3 %) and with 8 threads on the skewed input (small:
  16.05 against 15.86 µs; large: 5 %); with 8 threads on the uniform input
  the differences are under 1 %, not significant with 7 runs. What remains
  is not explained by the counters measured: no more instructions (1.11
  billion against 1.21 for 40 calls), as many 4K-aliasing blocks or fewer,
  a few dozen TLB walks per call in both, no store-buffer or
  store-forwarding stall, the loop within one 64-byte line; it is
  proportional to the number of elements (not a fixed cost per call).
  Investigation continued in D76.
- **D76 — The histogram gap, measured again properly.** The A/B runs of
  D75 made only 12 calls per process for the large input; Isocron pays a
  start-up cost at its first parallel call (creating the threads, and the
  first allocation of each thread creates its glibc arena): with 8 threads
  on the large input, 500.5 µs per call with 12 calls and 477.6 µs with 400
  (about 275 µs once per process), against 473.2 and 467.2 µs for OpenMP
  (about 70 µs). Measured again like `bench/parallel.py` (at least 200 ms
  per measure), Isocron and OpenMP alternated, 15 rounds, median and
  quartiles in µs:

  | Case | Isocron | OpenMP opt |
  | --- | --- | --- |
  | small uniform, 1 thread | 97.28 [96.31, 98.53] | 95.61 [94.13, 96.13] |
  | small skewed, 1 thread | 99.24 [98.81, 99.44] | 97.20 [96.20, 98.22] |
  | large uniform, 1 thread | 3694.47 [3691.13, 3703.16] | 3605.41 [3599.89, 3614.45] |
  | large skewed, 1 thread | 3789.22 [3775.26, 3797.90] | 3703.84 [3697.77, 3724.79] |
  | small uniform, 8 threads | 15.36 [15.19, 15.43] | 15.57 [15.39, 16.05] |
  | small skewed, 8 threads | 16.16 [15.58, 16.58] | 15.48 [15.32, 18.39] |
  | large uniform, 8 threads | 483.48 [475.38, 495.12] | 470.71 [467.32, 525.90] |
  | large skewed, 8 threads | 494.47 [488.41, 501.97] | 479.86 [472.30, 489.09] |

  With 8 threads, Isocron is level with OpenMP or faster, except on the
  large skewed input (3 %); the 5 to 7 % of D75 came from the method. With
  one thread, Isocron is 1.7 to 2.5 % slower, a real difference (the
  quartiles do not overlap) that remains unexplained: the instructions of
  the loop are the same and both come from the loop stream detector
  (`lsd.uops`: 4.505 against 4.754 per element, Isocron issuing fewer);
  aligning the loop on 64 bytes instead of 16 changes nothing (assembly
  edited by hand); a C program with Isocron's layout (one heap block per
  task, copies 1024 bytes apart, input aligned or not) runs as fast as
  OpenMP; cache and TLB misses are negligible in all of them; no context
  switch or migration. Open: the start-up cost of the thread pool (TODO).
- **D77 — Rule P: no allocation in the tasks, one panic, the sequential
  panic.** (1) The private copies of all the tasks are allocated at once by
  the calling thread (each task zeroes its own block, in parallel; 128 bytes
  between two blocks, so that the L2 prefetcher of a core, which completes
  aligned pairs of 64-byte lines, never fetches a line of another thread's
  block): the threads of the pool never allocate, so they never create a
  glibc arena. The extra cost of the first parallel call (8 threads, large
  histogram) went from 221.1 µs to 119.5 µs, that of OpenMP being 117.1 µs;
  with 8 threads on the small input Isocron is now faster than OpenMP
  (14.29 against 15.70 µs uniform, 14.59 against 15.64 µs skewed). (2) This
  made several tasks meet an index out of bounds at the same time, and their
  messages were printed mixed: the runtime now lets only the first panicking
  thread print and end the program (`PANICKING`), the others wait silently.
  (3) A task of rule P that meets an index out of bounds records it and
  stops; the calling thread then reports the failure of the first task that
  failed, which holds the first such element in the order of the elements:
  the panic is the one of the sequential `fold` and of the interpreter, for
  any number of threads (before, the message could depend on which thread
  was first). Remaining gaps, measured like `bench/parallel.py`: with 8
  threads on the large input Isocron is about 4 % behind OpenMP (487.65
  against 467.87 µs uniform, 493.01 against 471.44 µs skewed). With one
  thread, the same C source is 3.4 % slower built by GCC without OpenMP
  than with it (3733.33 against 3610.87 µs, large input), and the same loop
  takes from 3610.87 µs (GCC with OpenMP) to 3825.91 µs (Zig) depending on
  the compiler; Isocron (3694.51 µs) is faster than GCC alone, Clang
  (3823.04) and Zig, and behind GCC with OpenMP and Rust (3627.57): the
  one-thread difference is a variation between code generators, not a
  defect of the algorithm.
- **D78 — Rule P: 8 copies, 32 elements per iteration, one page-aligned
  block per task.** Measured at fixed frequency, 15 alternated rounds of at
  least 200 ms (medians, µs per call):

  | histogram | Isocron | OpenMP (GCC) | Rust |
  |---|---|---|---|
  | small uniform, 1 thread | 91.41 | 95.01 | 94.69 |
  | small uniform, 8 threads | 13.51 | 15.54 | 19.64 (rayon) |
  | small skewed, 1 thread | 92.06 | 96.83 | 97.77 |
  | small skewed, 8 threads | 13.74 | 15.46 | 20.11 (rayon) |
  | large uniform, 1 thread | 3510.07 | 3610.23 | 3624.94 |
  | large uniform, 8 threads | 446.86 | 468.61 | 478.72 (rayon) |
  | large skewed, 1 thread | 3544.02 | 3695.25 | 3734.27 |
  | large skewed, 8 threads | 453.20 | 476.81 | 489.75 (rayon) |

  Isocron is the fastest in the eight cases; before D78 it was 3697.53,
  495.28, 3779.90 and 494.09 µs on the large inputs. The last sentence of
  D77 is withdrawn: the one-thread gap was not a variation between code
  generators but a loop that could be made faster. The changes:
  (1) The task knows the number of counters as a constant when the initial
  value says it (`zeros(256)`, an array literal): LLVM then addresses the
  copies with constant offsets from one register.
  (2) 32 elements per iteration (measured against 8, 16 and 64): the loop
  instructions become a small part of the work.
  (3) 8 private copies instead of 4: on a skewed input the same few counters
  are increased again and again, and each copy's chain of dependent
  increments (load, add, store, forwarded to the next load) is half as long;
  large skewed with one thread 3707.56 µs with 4 copies, 3543.04 with 8,
  the uniform input unchanged (3543.63 and 3531.28). With 4 copies this case
  depended on the exact placement of the code: two builds with the same
  instructions (other registers, other addresses) ran at 1.1645 and 1.1989
  cycles per element with the same counts of micro-operations (from the
  micro-operation cache in both), of 4K-aliasing blocks and of memory
  ordering clears; moving the block of copies within its page did not
  change it (3692.69 to 3738.62 µs for 13 offsets). Shorter chains removed
  this sensitivity.
  (4) The block of each task is made of whole pages, aligned on a page, and
  holds the task's failure record after its copies; the arrays of pointers
  and of failure records shared by all the tasks are gone. With blocks
  4224 bytes apart, thread 0 (whose block has no neighbour before it) was
  always the fastest: the L2 streamer of a core prefetches ahead up to the
  end of the page, into the start of the next task's block. Timed in each
  thread (large uniform, 8 threads): threads 1 to 7 worked 449.36 µs
  against 441.05 µs for thread 0; with aligned blocks 439.16 against
  438.15, the time of one thread alone on an eighth of the input
  (438.89 µs); the call went from 472.16 to 461.79 µs.
  (5) The thread pool wakes its workers only when some are asleep
  (`SLEEPERS`): a job given to spinning workers costs no system call.
  Findings that guided this: timed inside the threads, the pool costs
  little: the threads start 0.25 µs after `isc_parallel_for` is entered,
  it returns 0.14 µs after the last one ends, and 0.9 µs pass between two
  calls; OpenMP does the same in 0.85 and 0.6 µs. The time of a parallel
  call is the time of its slowest thread: with 8 threads one of them is
  about 20 µs slower than the mean at each call, for both. The same C
  program with the loop of Isocron (32 elements, 4 copies in one block)
  under OpenMP ran at 456.64 to 462.36 µs where its own loop took 481.29 to
  556.89 µs: a faster loop does give a faster parallel call. The profile of
  `perf record` was misleading here: it put 72.57 % of the samples of the
  OpenMP threads in the loop, which timing inside the threads contradicts.
  Rejected: `FINISHED` alone in its cache lines (no measurable effect).
  Kept: `ISC_LLVM_ARGS`, options for LLVM's code generator, for experiments
  only (`-align-all-functions=12` does move `isc_task_1` to 0x3000;
  `-align-loops=64` left this loop where it was).
- **D79 — An allocator of Isocron, as an experiment (`ISC_ALLOC=isc`).**
  `crates/isc-runtime/src/heap.rs`; glibc's `aligned_alloc` and `free` stay
  the default until the author decides. Size classes
  (`isc_runtime_core::alloc_class`: multiples of 64 bytes up to 1 KiB, then
  8 per power of two); blocks up to 256 KiB carved from chunks of 4 MiB;
  larger blocks in their own mapping, in huge pages from 2 MiB on. Freed
  large blocks go to a cache of at most max(64 MiB, the bytes in use);
  beyond, the blocks freed the longest ago are unmapped. Without this bound
  (first version) eight arrays of 10 to 80 MB processed one after the other
  reached a maximum resident size of 701 MB against 154 MB with glibc; with
  it, 199 MB. A first version counted the cache in bytes of mappings
  (array plus 2 MiB of alignment) against a bound in bytes of arrays: the
  temporary of `temp.isc` was unmapped at every call (40.0 page faults per
  call). The huge blocks are colored: they start 0, 1, ... 7 steps of
  256 KiB + 64 bytes after the start of their first huge page. Large `dot`,
  one thread, by the distance between its two arrays: 8013 µs at the same
  offset, 7838 µs 4160 bytes apart, 6578 µs 65600 bytes apart, 6517 µs
  262208 bytes apart, 6887 µs with glibc (glibc's arrays are also in huge
  pages: 153600 kB of 157640 kB resident). Measured at fixed frequency,
  15 alternated rounds, glibc against `isc`, µs per call:
  `temp.isc` (a temporary of the size of the input at every call, until
  fusion exists) small 12.24 / 10.72 (1 thread), 3.45 / 3.73 (8 threads),
  large 22392.63 / 15795.53 (560.9 and 0.0 page faults per call) and
  35302.29 / 14179.56 (887.6 and 0.0); `dot` small 6.94 / 3.72 and 1.22 /
  1.26, large 6950.96 / 6563.42 and 3618.96 / 3451.43; `sum`, `std_dev` and
  `histogram` within 1 % except small with 8 threads (`sum` 1.00 / 1.05,
  `std_dev` 2.33 / 2.37). One allocation and free costs 12.2 to 13.2 ns
  with `isc` whatever the size, 8.3 to 8.7 ns with glibc up to 1 KiB and
  32.5 to 33.1 ns from 20480 bytes on: not enough to explain the small
  inputs with 8 threads, which stay open. Not done: per-thread caches,
  giving back the chunks of small blocks.
- **D80 — `bench/machine.sh restore` did not give the CPUs back.** It ran
  `systemctl set-property --runtime <unit> AllowedCPUs=`: an empty value
  removes systemd's property but leaves the cgroup's `cpuset.cpus` as it
  was, and `status` printed the property only. Found on 2026-11-01 evening:
  `user.slice` and `system.slice` still had CPUs 8-15, and so had nginx,
  Plex, dockerd, transmission-daemon and systemd, since the first `apply`.
  `restore` now gives every online CPU explicitly, even when nothing is
  saved, `apply` does the same without SMT, and `status` prints the CPUs
  the cgroup really has. After the fix every one of these processes has
  CPUs 0-15. The measurements are not affected: `apply` writes explicit
  CPU lists, which systemd does apply.
- **D81 — The allocator of Isocron by default; a shorter start of a
  parallel job.** The author asked for an allocator "as functional as glibc
  and much faster" (2026-11-01). `crates/isc-runtime/src/heap.rs` is now
  used by every compiled program on Linux; `ISC_ALLOC=glibc` uses glibc's
  instead; WebAssembly keeps wasi-libc's. This supersedes the experimental
  status of D79. Its design:
  - Small blocks (up to 256 KiB) in size classes, carved from chunks of
    4 MiB; each thread keeps freed blocks in its thread-local storage
    (local-exec TLS, `isc_heap_thread`, no lock, no atomic operation); empty
    or full caches exchange half of their limit with the global lists, one
    lock per class; a thread that ends gives its cache back (pthread key).
    Arrays (16 KiB and more) and smaller blocks come from different chunks:
    after the end of an array, the cells a call allocates and its tasks
    write were fetched by the prefetcher of the core reading that end
    (`dot`, small, 8 threads: 1118.6 against 1098.8 ns once separated, glibc
    1091.8 ns).
  - Medium blocks (up to 32 MiB) cut by a two-level segregated fit (TLSF)
    from regions of 256 MiB, merged with their free neighbours; each thread
    keeps its last two medium blocks of at most 8 MiB.
  - Large blocks in their own mapping, colored (steps of 256 KiB + 2048
    bytes: two arrays read together at least 64 KiB apart modulo 2 MiB, D79)
    and cached; a request that finds no cached block first unmaps the cached
    blocks smaller than it. Huge pages only for the huge pages the array
    covers whole: the first and last ones were otherwise allocated whole
    (two arrays of 80 MB: 163840 kB resident against 156264 kB with glibc,
    then 156208 kB).
  - Every array of at least 16 KiB starts 64 or 2112 bytes after a page
    boundary (two arrays 0 or 2048 bytes apart modulo 4096): blocks carved
    one after the other were 64 bytes apart, the worst case of 4K aliasing
    for `y = x * 2.0` (`temp`, small, 8 threads: 3.614 µs, then 3.468 µs,
    glibc 3.468 µs).
  - Memory given back: the free medium blocks with written pages go back
    when the idle bytes exceed 64 MiB plus the bytes in use, or when they
    stayed idle while the program had to get more than 64 MiB of new memory
    (medium blocks never written, new large mappings). Only whole huge pages
    go back: giving back parts of them made the kernel split them (283
    splits per stress run, then 0), and each later use faulted 4 KiB pages.
    A region all free and stale is unmapped. Free small blocks are kept,
    like glibc's bins.
  - A block never written is zero: a zeroed request writes one byte per page
    instead of clearing it. Not writing it at all was wrong: a program that
    reads a `zeros` array first got the shared zero page, then a fault per
    4 KiB page at each write (a block of 40 MiB read then written: 9729
    faults and 2048 kB of huge pages; now 30 faults and 40960 kB; glibc 541
    and 38912 kB).
  - A double free, or the free of a pointer it did not give, is a panic
    ("invalid free"), tested with a stress test of 8 threads (blocks of 16
    bytes to 40 MiB, blocks freed by other threads, threads that end, zeroed
    blocks checked) in `crates/iscc/tests/heap.rs`, run with both allocators.
  Measured at fixed frequency, glibc against Isocron's: one allocation and
  free 12.6 / 7.6 ns (64 bytes), 48.6 / 8.5 ns (16 KiB), 50.0 / 7.9 ns
  (1 MiB), 4913.5 / 28.1 ns (64 MiB); 1024 live blocks of 16 bytes to
  64 KiB renewed, 86.3 / 18.7 ns per step with one thread, 90.9 / 76.5 ns
  with 8; producer and consumer 813.1 / 68.9 ns per block; stress test
  16.71 / 16.56 s, 561764 / 44979 page faults, 1086 / 895 MB, 4.3 / 2.6 s of
  system time; eight arrays of 10 to 80 MB one after the other, 153 / 156 MB
  at most; `temp.isc` (a temporary of the size of the input at every call)
  large, 22339.07 / 15546.51 µs with one thread (624.9 / 0.0 page faults per
  call) and 34925.98 / 9612.96 µs with 8.
  The start of a parallel job (`isc_parallel_for`) was the cause of the
  differences that remained on the small inputs with 8 threads: the gap was
  always in the time from the entry of the caller to the publication of the
  job, a chain of transfers of cache lines between the cores. Three changes
  shorten it, for both allocators:
  - The context of the tasks (at most 64 bytes) is copied into the line of
    the job (`isc_parallel_for` takes its size, a new argument): a worker
    reads job and context in one aligned pair of lines, and the caller no
    longer takes back the line of its stack the workers read.
  - The job is published by a plain write of `gen` and a fence, instead of a
    locked add, which asked for the line of `gen` only once every earlier
    write was done.
  - The variables of the pool are in three exported lines of 128 bytes
    (`isc_pool_gen`, `isc_pool_job`, `isc_pool_sleep`), never shared with
    other data (`gen` had shared its line with the output buffer). Other
    groupings were measured and are slower: each variable alone (`sum`,
    small, 8 threads, glibc 840.3 then 880.5 ns), `gen` with the job (878.4
    ns).
  Small inputs, 8 threads, glibc / Isocron's, ns per call: `sum` 853.6 /
  902.7 at first, 831.1 / 828.9 at the end; `std_dev` 2055.9 / 2165.4, then
  2001.9 / 1961.5; `dot` 1198.3 / 1216.8, then 1091.8 / 1098.8.
- **D82 — The results of the tasks of a `reduce` in their own cache lines.**
  The tasks of the first level of a `reduce` wrote their results side by
  side: 8 results of `f64` in one cache line that the 8 cores took from each
  other when the tasks ended (false sharing). With at most 32 blocks
  (`PAD_BLOCKS`), each task writes its result into its own 128 bytes (128
  divided by the smallest leaf: `u8` results are 128 elements apart), then
  the caller copies them into `parts`; beyond, the copy costs more than it
  saves. `dot`, small, 8 threads: 1182.4 ns (glibc) and 1257.7 ns
  (Isocron's allocator) side by side, 1154.4 and 1154.4 ns apart; 32
  blocks 2596.7 against 2486.0 ns; 64 blocks 18101.6 against 18269.8 ns;
  large `sum`, 8 threads, 1530.69 against 1540.05 ns with cells for every
  block. The result does not change: the tree of the reduction is the same.

- **D83 — Large inputs on one core: the first huge page and the offset of
  the arrays (investigation of the cases lost on 2026-11-02).** In the
  official campaign of 2026-11-02, Rust was ahead of Isocron on the large
  `sum` (3.490 against 3.608 ms) and `std_dev` (7.025 against 7.176 ms), one
  core. Rust computes the same blocks with the same tree, so the difference
  is not the algorithm. Measured on xeon (fixed frequency, the protocol of
  `bench/machine.sh`):
  - *Huge pages are a confounder of the protocol.* With 977948 kB free and
    26017196 kB of page cache, Isocron (`MADV_HUGEPAGE`) had its 80 MB
    input in huge pages (81920 kB), GCC most of it, Rust and Zig none (0 to
    4096 kB): only a program that asks for them makes the kernel compact
    memory. With the page cache dropped, Rust got them too (75776 kB) and
    was still ahead (3530598 against 3648428 ns); with 4 KiB pages for
    both, Isocron was ahead (3688773 against 3703441 ns).
  - *The gap was not in the steady state.* Total time against repetitions:
    3544.1 µs per call for Isocron, 3517.7 for Rust, but a fixed cost of
    395.7 against 75.7 µs, and calls that got faster: 3955 µs for the
    first, then from 3672 down to 3525 over 40 calls. Between the 41st and
    110th calls Isocron was faster than Rust (3465130 against 3526729 ns).
  - *The first call cleared a huge page.* The block results (19 KiB) were
    the first array of their group: a new chunk in huge pages, and 2 MiB
    cleared inside the measurement. With one untimed call first, the first
    timed call took 3566 instead of 3869 µs. The block results are now on
    the stack (D86). Rejected: the first chunk of each group in pages of
    4 KiB (first call 3601 µs, Rust 3580), because an array of that chunk is
    then on scattered physical pages: the small `std_dev`, whose input is
    carved from it, had runs up to 8129 ns and an interquartile range of
    312.7 ns, against 21.2 ns in huge pages.
  - *The offset of an array in its page.* The same C loop over the same
    data, only the offset of the array changing: 0 or 64 bytes after a
    4 KiB boundary, 3593 and 3599 µs per call (calls 2 to 10); 1024, 2112
    and 3072 bytes, 3492, 3504 and 3510 µs. Only the offset in the 4 KiB
    page counts (64, 4160 and 2097216 bytes are slow; 1088, 2112 and 6208
    fast); the gap stays with a loop without blocks, with each hardware
    prefetcher disabled in turn (MSR 0x1A4), and the memory reads the same
    volume per call (67.11 against 66.43 MiB). The mechanism is not
    identified. Isocron's large arrays started 64 bytes after a huge
    page; they now start `LARGE_OFFSET` (1024) further, 1088 or 3136 bytes
    after a page boundary, two arrays still 0 or 2048 bytes apart (D81).
  - *What remains.* A slower start over about ten calls, also measured in
    Rust when it reads its file straight into its array (3557 then 3508 µs)
    and not with its usual loading, which converts a buffer of bytes
    (3513 to 3541 µs). It is not the uncore frequency (2600 MHz during the
    whole run), nor page migrations (none), nor the end of the previous
    process (the same after a pause of one second). Still open.
  - *Tried and rejected:* a software prefetch in the loop, slower at every
    distance from 512 to 8192 bytes (3524 to 3616 µs against 3476).
  Large inputs, one core, 15 interleaved rounds, the repetitions of the
  campaign (before, after; best strict variant of the others):
  `sum` 3647092.7 then 3536531.9 ns (Rust 3535883.0, GCC 3602567.8, Zig
  3566397.4); `std_dev` 7201618.2 then 7028854.0 ns (Rust 7085921.2); `dot`
  6550458.3 then 6517349.7 ns (GCC 6925102.1, Rust 7071207.4). The results
  are bit for bit the same.
- **D84 — The same huge pages for every language during the measurements.**
  `bench/machine.sh apply` now drops the page cache and compacts memory
  (`drop_caches`, `vm.compact_memory`) before the measurements, as it fixes
  the frequency. On xeon the page cache fills the memory (26017196 kB of
  32754772): only a program that asks for huge pages with `madvise` makes
  the kernel compact memory for it, so that Isocron had its large inputs
  in huge pages and Rust and Zig in pages of 4 KiB, and which program got
  the few free huge pages depended on the order of the runs (D83). With
  free memory every program gets huge pages where the kernel gives them
  (`enabled` is `always` on xeon). The cache fills again by itself; the
  programs of the server lose nothing but the time to read their files
  again.
- **D85 — LLVM's targets registered once per process.** The test suite
  hung in `tests/parallel.rs`: its three tests, each compiling from its own
  thread, spun for minutes in `llvm::TargetRegistry::lookupTarget` (83 % of
  the samples of `perf`). `compile_with` called the `LLVMInitialize*`
  functions at each compilation, and LLVM's registration of a target is not
  safe from two threads at once: both can link the target into the list of
  targets, which then loops. The command `iscc` compiles once per process
  and never met it. The targets (and `ISC_LLVM_ARGS`) are now registered
  once, behind a `std::sync::Once`.
- **D86 — The scratch arrays of a `reduce` on the stack, its last elements
  through memory, and four rows per iteration from memory.** Three changes
  of the code of `reduce`, none of its result (the tree is the same; the
  outputs of the POC programs are bit for bit the same):
  - *Block results on the stack.* The block results (`parts`) were
    allocated and freed at each call. They are now on the stack when they
    take at most 32 KiB (`STACK_SCRATCH`: 4096 results of `f64`), between
    `llvm.stacksave` and `llvm.stackrestore`, else on the heap as before.
    The cells of D82 were on the stack too at first, then went back to the
    heap (D87).
    Small `std_dev`, one core: 5830.8 then 5803.1 ns, interquartile range
    4.3 ns; it also removes the huge page the first call of a large `sum`
    cleared (D83). This is a provisional answer, for the arrays the
    compiler makes itself, to the threshold of placement on the stack of
    SPEC section 16.
  - *The last elements of a block.* Fewer than V elements were added to
    their lane with an `extractelement` and an `insertelement` at a
    variable lane, which LLVM compiled into a store and a reload of every
    accumulator register for each element; the upper levels of a reduction
    of a few blocks (8 blocks for the small inputs) are only such elements.
    They now go through a slot of the vector type, stored and reloaded once,
    the elements updated one at a time (lane by lane in the register for
    `bool`, packed in memory). A first version used an array of scalars:
    aligned to 8 bytes, where the store of the vector assumes its own
    alignment, it faulted for sizes not a multiple of V (the tests of
    `codegen.rs`; the sizes of the benchmarks are multiples of V). Small
    `std_dev` 5806.3 then 5672.2 ns, small `sum` 1818.9 then 1758.2 ns.
  - *Four rows per iteration from memory.* When the first level reads a
    materialized array (no element-wise map: D87) of at least 32 MiB
    (`STREAM_BYTES`, twice the L3 of xeon), the
    loop of a block takes four rows of V elements per iteration
    (`STREAM_UNROLL`), each accumulator still receiving its rows in order;
    the size is tested at run time, both loops are emitted. Large `sum`,
    one core, 3527408.4 against 3483863.0 ns (Rust 3531169.0); `std_dev`
    and `dot` the same within their dispersion. From the caches it is
    slower (small `sum` 1866.1 against 1760.5 ns, small `dot` 3903.4
    against 3698.6, `sum` over 8 MB 176172.2 against 174722.3), hence the
    threshold; two rows everywhere were slower too (small `sum` +2.79 %).
    The cause is not identified: the unrolled loop does not make fewer
    demand reads of L2 (955902 against 937902 per call), only fewer cycles
    with a pending miss (116751246 against 121874874).
  - *Tests.* Both thresholds are options of the code generator
    (`stream_bytes`, `stack_scratch`); `tests/reduce_paths.rs` lowers them
    to 0 so that small arrays take the unrolled loop and the heap, and
    compares `sum`, `dot` and `std_dev` with the interpreter and the fixed
    form, with 1 and 4 threads, for sizes around V, four rows and a block.
  Rejected: reading a large file through a buffer of 1 MiB that the
  program copies (the first passes 1 % faster, but 25.3 instead of about
  22 ms to load 80 MB); vectors of 256 bits on AVX-512 (small inputs 17 to
  58 % slower, large ones not faster). One core, 15 interleaved rounds,
  before (D83) and after, against the best strict variant of the others:
  small `sum` 1756.2 / 1759.0 ns (Zig 1852.5), small `std_dev` 5672.2 /
  5671.6 (Zig 5796.5), small `dot` 3696.2 / 3698.2 (Zig 7046.4), large
  `sum` 3512865.2 / 3458468.1 (Rust 3494407.5), large `std_dev` 7039787.4 /
  7012434.4 (Rust 7083788.7), large `dot` 6484662.7 / 6475671.5 (Clang
  6892077.0).
- **D87 — The compilation time of D86 recovered.** The official campaign
  after D86 showed slower compilations (warm, `bench/compile.py`): `sum`
  0.080 s against 0.065 the night before, `dot` 0.081 against 0.066,
  `std_dev` 0.115 against 0.081 (`histogram`, without `reduce`, 0.088
  against 0.089). Measured by switching each change of D86 off in turn
  (median of 9 builds, `bench/machine.sh`), then repaired:
  - *The loop of the last elements* cost most: LLVM vectorized that loop
    of at most V - 1 elements, with its run-time checks and its epilogue
    (`std_dev` 0.1150 s with it, 0.0895 with the old code). It is now a
    `for_range_scalar`, whose back edge carries `llvm.loop` metadata that
    forbids vectorization and unrolling: 0.0894 s, the speed of D86 kept.
  - *The runtime* is built with `lto = "fat"` and `codegen-units = 1`: one
    object for the runtime and the parts of `core` it uses, an archive of
    4882372 bytes instead of 7791336, which `iscc` writes and the linker
    reads at each build: every program 4 to 5 ms faster to compile
    (`histogram` 0.0897 then 0.0849 s).
  - *Four rows per iteration only for a materialized array*: with a map in
    the loop it was not faster (large `dot`, `std_dev`), and each unrolled
    loop is code to compile (`dot` 0.0671 then 0.0639 s).
  - *The cells of D82 back on the heap*: on the stack they cost 1.5 ms of
    compilation of `std_dev` and saved 12 ns per call (small `std_dev`
    5677.6 against 5689.9 ns, Zig 5788.8).
  Tried and rejected: the cells in the frame at their largest size (slower
  to compile, `sum` 0.0682 s), one branch only in `alloc_scratch` (no
  change), linking with `lld` (21864 µs against 16507 for `ld`), the
  runtime reduced to one relocatable object (linked in 17454 µs against
  16412). After D87, median of 9 builds: `sum` 0.0634 s, `dot` 0.0633,
  `std_dev` 0.0808, `histogram` 0.0845, each at most its time of the night
  of 2026-11-02.

## Stage 2 — the complete language (plan of 2026-11-02)

- **D88 — `zip`.** `zip(a, b)` has the type `[n](A, B)`; the sizes of `a`
  and `b` are unified like the operands of an element-wise operator: proved
  different, a compilation error that names both types; not provable, a
  run-time test, and a difference is a panic at the call. It is lowered to
  a `Map` over both arrays whose lambda builds the pair, so that a consumer
  (`map`, `reduce`, `fold`, indexing) fuses it by F1 and reads `a` and `b`
  directly; the interpreter and the code generator needed no change. A
  `zip` that is materialized (a variable used more than once, M1) is still
  copied into an array of tuples in columns: SPEC section 6 calls it a view
  of zero cost in columns, which rule F4 (phase 2) will give.
  `tests/phase5.rs` compares it with the interpreter, with 1 and 4 threads,
  and checks both size errors; `docs/language.md` describes it.
- **D89 — `min` and `max`.** SPEC section 6 names them both as predefined
  associative operators (for `scan`) and as functions of the standard
  library over an array (like `sum`); they are one primitive each, chosen
  by the number of arguments (an ambiguity resolved here): `min(a, b)` of
  two numbers of the same type, `min(x)` of an array of numbers, and `min`
  passed as a function is the one of two numbers (`reduce(x, min, e)`).
  `min(x)` is `reduce(x, min, e)` in the fixed form of section 10, with `e`
  = `inf` for floats (`-inf` for `max`) and the largest (smallest) value of
  an integer type, which is the result for an empty array (the
  specification does not say; a panic was the other choice). On floats
  they are IEEE 754-2019 `minimum` and `maximum` (section 10): the code
  generator calls `llvm.minimum` / `llvm.maximum` (scalar and vector), the
  interpreter `isc_runtime_core::minimum_f64` and its siblings, so that a
  NaN gives NaN and -0 is below +0 everywhere; integers use `llvm.smin`,
  `umin`, `smax`, `umax`. Two operations `Min` and `Max` were added to
  `BinOp`, without infix syntax. `tests/phase5.rs` compares random arrays
  with NaNs, zeros of both signs and infinities, with 1 and 4 threads, and
  checks the type errors; `docs/language.md` describes them.
- **D90 — `count`.** `count(x, p)` (SPEC section 6, standard library) is
  written in Isocron in the prelude: `sum(map(x, (v) -> if p(v) then 1
  else 0))`, an `i64`, fused into one loop and computed in the fixed form
  (the result of an integer sum does not depend on the order anyway).
- **D91 — Slices `x[a..b]`.** The parser already read them. The size of a
  slice is an existential size symbol, known at run time like the size of
  the result of `read_f64` (SPEC writes `[b - a]T`; sizes have no
  subtraction yet); `a` and `b` are integers of any type, converted to
  `i64`. A slice with `a < 0`, `a > b` or `b > len(x)` is a panic,
  `slice out of bounds: [a..b] of an array of length n` (`Panic::Slice`,
  the same text in the interpreter and the runtime). A new IR node `Slice`
  copies the elements, column by column for an array of tuples: SPEC
  section 6 makes a slice a view that shares the memory of its source,
  which needs the lifetime of the source tracked while the view lives
  (SPEC section 8, uniqueness rule 7); the copy is replaced by a view with
  rule F4 and the uniqueness analysis (phases 2 and 3). The source is only
  read (a borrowing slot). Tested in `tests/phase5.rs` (sizes, tuples, 1
  and 4 threads, the three panics) and described in `docs/language.md`.
- **D92 — `scan`.** `scan(x, op, e)` (SPEC section 6) is the inclusive
  cumulation from left to right, `out[i] = op(out[i - 1], x[i])` with
  `out[-1] = e`: a new IR node `Scan`, always materialized (rule M2), its
  source fused like the source of a `fold` (F1). When its lambda is a
  predefined associative operator on integers (`+`, `*`, `min`, `max`) or
  booleans (`and`, `or`, which the lowering turns into `if`), `(a, b) -> a
  op b` or `b op a` (they commute), and there are more than one block and
  more than one thread, it is parallel: each task reduces a block from its
  first element, the caller computes from `e` the value before each block,
  then each task cumulates its block again from that value. Associativity
  alone makes the result the sequential one for any number of threads (no
  neutral element needed: `e` is applied once, before the first block).
  Every other lambda, and every operator on floats, is sequential (section
  10). Measured on 10^7 `i64` on xeon: sequential 7934832 ns, parallel
  7685866 to 8819979 ns with 2 to 8 threads: the parallel form reads the
  input twice, and the cumulation is bound by memory; it is to be improved
  with the benchmark of phase 7 (cumulation of `i64`). `tests/phase5.rs`
  compares integers that overflow, `min`, `max`, a non-neutral `e`,
  booleans and floats with the interpreter, with 1, 3 and 8 threads.
- **D93 — `filter`.** `filter(x, p)` (SPEC section 6) has an existential
  size (SPEC writes `[k <= n]T`; the bound is not used yet). A new IR node
  `Filter`, its source fused like any consumer (F1). The code generator
  counts the elements kept, allocates an array of exactly that size (the
  columns of an array of tuples are laid out for its length), then writes
  them: the predicate is pure, so that evaluating it twice is not
  observable. Still to do: the parallel form that SPEC asks for (counts per
  block in parallel, then each block written at its offset), rule F2 (one
  pass with its consumer, phase 2), and the type `[k <= n]T` written in a
  signature, with the existential sizes of results of user functions (SPEC
  section 4, which the POC refused).
- **D94 — Existential sizes in the results of functions.** SPEC section 4
  lets a size variable that appears only in the type of a result be fixed
  at run time by the function (`[k]T`), and `[k <= n]T` add that it does
  not exceed `n`; the POC refused both. Now: a new size variable in the
  result of a signature is existential, and must appear alone as the size
  of an array; `[k <= n]T` is allowed only there, `k` new and `n` made of
  sizes of the parameters (the bound is not used by the compiler yet). The
  body is checked against the result with each existential size opened (a
  fresh size variable, which unifies with the size the body computes, a
  `filter` or a slice for instance); each call gives each existential size
  of the callee a new existential size of the caller, whose value is the
  length of the result once it is bound by `let`, as for `read_f64`. No
  change to the lowering: the inlined body computes its own array.
  `tests/phase5.rs` checks a bounded result, a generic one, nested calls
  and four errors.
- **D95 — Arrays of arrays and `window`.** `[n][m]T` is allowed when `T` is
  a scalar (the class `ELEM` of an array of scalars), which completes phase
  1 (D73). In compiled code an array of arrays is `{ ptr, i64, i64, i64 }`:
  its block, its rows, its columns and the distance in elements from a row
  to the next. A literal `[[1, 2], [3, 4]]` copies its rows into one block
  (stride = columns; rows of different lengths known only at run time are
  a panic `size mismatch`). `window(x, k)`, of type `[n - k + 1][k]T`, is a
  view of `x` with stride 1, never copied (SPEC section 6); `k > n` is a
  panic `window of k elements over an array of length n`. A row `w[i]` is
  a view `{ ptr, i64 }` of the block; the ownership pass knows both are
  aliases of their source. A row of a temporary array of arrays (a literal,
  a `window` of a `map`) is copied, the temporary bound outside the copy,
  so that it is freed after it. `map`, `reduce`, `fold`, `print` read the
  rows; a deep copy makes a window a matrix. Not compiled yet: a `map`
  whose function returns an array (a new array of arrays), the equality and
  the update of arrays of arrays. The interpreter keeps the rows as
  separate arrays (`Array::Rows`), which cannot be told from views since
  values are never mutated. `tests/memory.rs` (new) runs programs compiled
  for AVX2 under Valgrind with glibc's allocator (Valgrind does not decode
  AVX-512) and finds no leak and no invalid access; it found the arguments
  of `main` never freed (now freed at the end of `main`) and the rows of
  temporaries freed before their copy (fixed as above).
- **D96 — The memory decisions checked at every compilation.** The author
  asked that memory problems be detected at compilation, as Rust does. A
  program never allocates or frees memory itself: the compiler decides
  where (SPEC section 8), so that a memory error is a defect of the
  compiler. `isc_ir::verify` checks the IR after the ownership pass, at
  every compilation, and a failure is a compilation error naming the line:
  every new value is freed exactly once (bound to a variable freed at the
  end of its scope, moved into a slot that takes it, or taken by a tuple, a
  literal, an update, a `fold`), no variable is used after it was moved, a
  variable is moved only in its own region (not inside a loop body or a
  branch entered after its definition), a value borrowed is never given
  away, and no view outlives the variable it borrows (rule 7 of section
  8). It is written independently of the ownership pass: it follows the
  values in evaluation order (new memory, views of owned variables, moves,
  frees) instead of recomputing aliases from use counts, so that a defect
  of the pass is not repeated by the check. `tests/verify.rs` changes one
  decision of the IR of real programs (a free removed, a free added after
  a move, a move turned into a borrow, a copy removed, a temporary left
  unbound, a view returned past its source, `args` not freed or freed
  after a move) and each is rejected. The decisions about the values of
  the program that the code generator took alone are now in the IR: `main`
  owns `args` (`Program::drop_args`, set by the ownership pass: freed at
  the end of `main`, or moved at its last use and not freed), and a
  literal takes its rows. On its first program outside the tests, the
  check found a defect of the ownership pass: in `let m = [a, [3, 4]]`
  followed by a use of `a`, the row `a` was given to the literal without a
  copy, freed by the literal and again at the end of its scope (with the
  compiler of before: Valgrind reports a read of freed memory, then the
  program stops on the panic `invalid free`). The pass now copies a row
  still used, or moves it at its last use, like a tuple component and the
  new element of an update. What the check cannot see: memory that is not
  a value of the program (the block results of `reduce`, the scratch of
  `scan` and `filter`) and decisions below the IR (LLVM, the runtime);
  `tests/memory.rs` (Valgrind) and the panic `InvalidFree` of the runtime
  stay for them. Compilation time: within the dispersion (`sum` 0.0637 s,
  `std_dev` 0.0812 s).
- **D97 — Rule F4: slices and zips are views.** A slice `x[a..b]` of an
  array of scalars (or strings) is a view `{ x + a, b - a }`: no copy, no
  free, even shared or sliced again. The ownership pass treats it as an
  alias of `x`, as `window` and the rows of an array of arrays (D95): uses
  of the view are uses of `x`, so `x` is not moved or updated in place
  while a view of it is alive (rule 7 of section 8), and a view that must
  be owned (the target of `with`, a tuple component, the result of a
  block that outlives `x`) is copied there. The memory check (D96) treats
  a slice as a view and rejects a slice of new memory, whose pointer
  inside the block could not be freed. Element loads assume the alignment
  of one element only, so an offset view needs nothing else. Exception, an
  open question: the columns of an array of tuples are placed according
  to its length (`columns`), so a view of a part of one needs another
  representation (a stride); a slice of an array of tuples stays a copy
  until then. A zip is never materialized, even shared (`inline_zips`): a
  `let`-bound zip used more than once, or inside a lambda, is replaced at
  each use by a copy of itself with parameters of its own, which the
  consumer fuses (F1); an input that is not a variable (a `map`) is bound
  once (a shared result, rule M1); the size test stays where it was, once.
  An action that receives a view is given the view itself when its
  elements are contiguous (a slice of a materialized array), so rule M3
  is met by the memory of the source, and a zip given to an action is
  materialized there (M3). What remains of F4 with F1: a view of a
  producer (`map(x, f)[a..b]`, `window(map(...), k)`) still materializes
  the producer. Tests: `isc-ir/tests/ir.rs` (`slices_are_views`,
  `shared_zip_is_not_materialized`), `phase5.rs` (`slices_as_views`,
  `shared_zip`, against the interpreter), `memory.rs` (Valgrind),
  `verify.rs` (`a_slice_of_new_memory_is_rejected`).
- **D98 — The memory report.** `iscc check --report` prints it, `iscc
  build --report` prints it and writes it as JSON next to the executable
  (`<output>.report.json`: `{"file", "entries": [{"line", "column",
  "kind", "what", "bytes", "rule", "why"}]}`); SPEC section 8 asks for
  text and JSON without saying where. `isc_ir::report` reads the
  optimized IR: an array expression in a fusable slot of its consumer is
  fused (no entry), any other array expression that allocates gets an
  `alloc` entry whose reason comes from its place: bound to a variable
  read more than once or inside a loop (M1, with the lines), the result of
  `scan` or `fold` and the accumulator of a `fold` (M2), given to an
  action (M3), read from a file (an input), a literal; a `Copy` gets a
  `copy` entry with the reason the ownership pass recorded when it
  inserted it (`Program::copy_reasons`, the same text as the error of
  `require_unique`); every indexing and update gets a `bounds` entry and
  every size test a `size` entry, since the code generator emits a test
  for each (LLVM removes some; proving bounds on the IR is an open task).
  An entry inside a lambda or a `bench` says it happens at each
  iteration. An entry with no rule says so: it is a materialization the
  rules of section 7 do not allow, to be removed. The first ones it
  showed: (1) the size test of a `zip` whose sizes are not proved equal
  read `length(zip_a)` of a bound `map`, a second use that materialized
  it; `producer_lengths` now replaces the length of a `let`-bound
  producer by the length of its input (or the size given to `iota`,
  `zeros`, `fill`), so the test reads `a` and the `map` is fused; (2) a
  view of a producer, and the result of `filter` (F2 to come). Two
  defects of positions found on the way, both fixed: the parameters of a
  lambda were all named `p` in the IR (the report showed `p[p]`; they now
  have the names of the program), and a use inside an inlined function of
  the prelude (`sum(y)`) had the line of the prelude: the error of
  `require_unique` said "used again at line 4" for a use at line 11;
  uses now take the innermost position in the program's file. Generator
  scratch (the block results of `reduce`, the counts of `filter`, the
  block sums of a parallel `scan`) is not a value of the program and is
  not in the report yet. Tests: `tests/report.rs`, `isc-ir/tests/ir.rs`
  (`require_unique_names_the_line_of_the_program`).
- **D99 — Rule F2: a filter read in order is one pass.** A stream is a
  `filter`, or a `map` of one input that is a stream; it is fused into the
  array slot of a `reduce` or a `fold` and into the input of a `map` of one
  input (`is_stream`, `is_stream_slot`); `fuse` moves a `let`-bound filter
  used once into a fusable slot, `bind_temporaries` leaves a filter in a
  fusable slot, and `place_streams` (after the last fusion) binds to a
  `let` every filter that its consumer would read by index (an indexing, a
  `scan`, a `map` of several inputs or read by index, a `filter`). The
  memory check (D96) accepts a filter only where it is read in order. Code
  generation: `fold_stream` changes the accumulator only for the kept
  elements; `materialize_stream` counts the kept elements, allocates
  exactly, then maps them (the predicate runs twice, the maps once); and
  `reduce_stream` computes the form of section 10 on the kept elements
  for any operator and any `e`, in one sequential pass: the k-th kept
  element goes to accumulator k mod V of its block, a block of B elements
  is closed only when one more element comes (so that exactly B elements
  give `reduce_block`, not a reduction of one block result), its result
  goes the same way to the level above, and at the end the first level
  that has received no block result gives the result, after the partial
  levels below have given theirs (`STREAM_LEVELS` = 5: B^5 = 2^60
  elements). Checked against the interpreter up to 25 000 000 elements
  (22 500 000 kept: three levels), with a non-associative operator and a
  non-neutral `e`. **Masked reductions**: when the operator is exact
  (integer `+`, `*`, `min`, `max` of integers and floats: the result does
  not depend on order or grouping) and `e` is its exact identity, and
  neither the predicate nor the maps can panic (`cannot_panic`, now also
  used by the code generator, which also covers updates, slices and
  windows, and accepts a division by a non-zero literal),
  `masked_reductions` rewrites `reduce(filter(x, p) then f, op, e)` into
  `reduce(map(x, (v) -> if p(v) then f(v) else e), op, e)`: the same
  result, vectorized and parallel. On one core, 25 000 000 `f64`:
  `max(filter(x, (v) -> v < 0.9))` 53 330 755 ns before (filter
  materialized, then `reduce`), 9 601 464 ns after; `sum(filter(x, (v) ->
  v > 0.1))` 53 448 300 ns before, 53 981 798 ns after (the stream: `+` of
  floats depends on the order). Open, for phase 7 (the kernel "filter
  then sum"): a vectorized stream for float `+`, compressing the kept
  elements of each vector (AVX-512 `vcompress`) into a buffer added 32
  at a time to the 32 accumulators, which is the same order. The LLVM of
  the four POC kernels is unchanged.
- **D100 — Rule F3: independent reductions of one array, one pass.**
  `horizontal` looks, in each spine of non-statement `let`s and in each
  expression, for the `reduce`s evaluated whenever it is (not inside a
  lambda, a branch, the rest of an action after a statement, or another
  `reduce`) whose array is one variable read through `map`s of one input;
  those of one variable whose free variables are all defined before the
  place of the first become `let reduced = reduce(map(x, (v) -> (f_1(v),
  ..., f_k(v))), op, (e_1, ..., e_k))`, each replaced by a projection. The
  operator works component by component (`let a_i = A.i in let b_i = B.i
  in op_i`): each component gets its elements in the lanes and blocks of
  section 10, so each result is that of its own `reduce`, which the
  interpreter computes from this same IR. The code generator recognizes
  this form (`columns_of`) and reads the array once (`reduce_columns`):
  each block of B elements is reduced for every column in turn while it
  is in the cache (block j by task j in parallel), then the block results
  of each column as in `reduce`. Not grouped: two statements of an action
  (section 12: a `let` is computed before the next statement; grouping
  would move the second before the statements between), and a `reduce` that
  uses the result of another (`dep` in the tests). Measured on one core,
  `max(x) - min(x)` and `sum(x)` with `sum(map(x, (v) -> v * v))`: 25 000
  000 `f64`, 17 458 534 ns to 12 774 073 ns and 17 941 444 ns to 10 542 955
  ns; 32 768 `f64` (in L2), 10 814 ns to 10 791 ns and 4 410 ns to 4 288
  ns (the data was in the cache for the second pass already). The block
  results of a column are on the heap and not padded against false
  sharing (D81), unlike those of one `reduce`; the LLVM of the four POC
  kernels is unchanged (none has independent reductions).
- **D101 — Views of producers.** A slice of a producer is a producer
  (`is_producer`): its source slot is fusable, so `map(x, f)[a..b]` read by
  a consumer computes only the elements `a` to `b - 1`, through
  `Src::Offset` (the bounds tested first, `slice_check`), and evaluated
  into a value it computes only those (`materialize`): rule F1 lists the
  views among the producers, and no rule M allows materializing the whole
  `map`. A window of a producer materializes the producer: each element is
  read by up to `k` windows, so rule M1 applies (a result used more than
  once), which the report now says. Found on the way: the new arm of the
  code generator for a slice of a producer came after the arm of every
  slice and was never reached (the compiler warned); a slice evaluated as
  a value was then a view into a materialized `map`, freed as a block of
  its own, and the runtime stopped on `invalid free`. The memory check
  could not see it, since the IR decision was right and the generator did
  not follow it (the limit stated in D96); the panic `InvalidFree` did.
  Tests: `phase5.rs` (`slices_of_producers`), `memory.rs`, `report.rs`
  (`views_of_producers`).
- **D102 — Recursive functions.** The type checker accepts recursion and
  marks the functions in a cycle of calls (`TProgram::recursive`). Such a
  function is not inlined (SPEC section 11): the lowering makes one
  `isc_ir::Function` per function and type arguments, lowered once, with
  `ExprKind::Call` for its calls; its body tests on entry the size of
  every array parameter that another parameter fixes, since one call site
  cannot prove it for every call. Not yet: a recursive function with a
  function parameter (an error that says so; specializing it per function
  argument is open). Every pass runs on every body; in the ownership pass
  and the memory check, the parameters are borrowed and the result is new
  (a parameter returned is copied; the check rejects a function that
  returns borrowed memory), and the arguments of a call are borrowing slots
  (a new array passed is bound to a `let` and freed after the call). Code:
  one `define internal` per function. **Tail calls** (section 11: a direct
  tail call is a loop, without growth of the stack): `tail_calls` finds
  the calls of the function to itself in tail position (through `let`
  bodies, `if` branches, size tests) whose array arguments are the same
  parameter or a temporary freed after the call; the code keeps the
  parameters in slots, jumps to the start of the body, moves such a
  temporary into the loop (a flag per array parameter says the loop owns
  its value: the previous one is freed when replaced, and on return), and
  frees the other pending temporaries before the jump. The interpreter
  runs the same calls as a loop, so that both agree on a recursion of
  any length (3 000 000 calls in the tests). **Stack overflow** (a
  panic): at the entry of each function the code compares its frame
  address with the floor of the thread's stack (`isc_stack_floor`:
  `pthread_getattr_np`, with a margin of 256 KiB for the frames that do
  not test it), kept in a thread-local variable, and calls
  `isc_panic_stack`, "stack overflow: too many nested calls of `f`", at the
  position of the function. With recursive functions, the body of `main`
  runs on a thread with a stack of 1 GiB (`isc_run_big_stack`) rather
  than the 8 MiB of the main thread: `grow`, which is not a tail call,
  reached a depth of 100 000 on 8 MiB, beyond 10 000 000 on 1 GiB. The
  interpreter runs on a thread with a stack of 4 GiB and stops when less
  than 16 MiB are left (about 2.2 KiB per call of a small function:
  depths of about 1 800 000), so that the compiled program goes at least
  as deep as the interpreter for the same program, and both give the same
  panic beyond. **Nested parallel loops**: a recursive function with a
  parallel `reduce`, called by a parallel `map`, would call
  `isc_parallel_for` from a task; the runtime now runs the tasks of a
  loop started while another runs on the calling thread, in order
  (`BUSY`), which gives the same result (blocks do not depend on the
  number of threads). The memory report lists each call of a recursive
  function: `loop` for a direct tail call, `stack` otherwise; an array
  result, and an array argument, are materialized (rule M2: a recursive
  function is not inlined). A reason of a copy is now given in this
  order: a borrowed variable (an input, a parameter) first, then a use
  that comes later. Tests: `tests/phase3.rs`, `memory.rs`
  (`recursion_frees_everything`), `report.rs`, `verify.rs`, `isc-ir/tests/ir.rs`.
- **D104 — The language is named Isocron.** At the request of the author
  (2026-11-02), the language formerly named Isochrone is Isocron
  everywhere its name is written: the documentation, SPEC.md (its name
  only: no rule changes), the comments, the displayed messages, the
  scripts and the result files (the key `"kind": "isocron"` of the
  measurements, read by the scripts). The technical names stay, since
  they fit the new name too: the crates `isc-*`, the compiler `iscc`, the
  extension `.isc`, the prefix `isc_` of the runtime and the variables
  `ISC_*`; the directory of the repository is unchanged.
- **D103 — Recursive sum types, reference counting.** A sum type that
  contains itself (`SumDef::recursive`, found by the type checker, which
  now accepts it) is a node on the heap: `Type::Rec(i)` in the IR, a
  pointer, with `Program::recs[i]` its layout (the tuple of a sum: the
  tag, then the slots of every variant) and its shape for `print`, laid
  out once per type arguments (`rec_index`, which breaks the cycle). A
  constructor is `Box(tuple)` (the node takes the parts), a pattern reads
  `Unbox(x)` (the tuple, borrowed from the node). In the ownership pass a
  node is a value like an array: a borrowed node that must be owned gets a
  `Copy`, which for a node is one owner more (its count incremented), not
  a copy; the memory report lists it as `refcount`, and each constructor
  as `refcount` too (SPEC section 8 shows such an entry). Code: a node is
  `{ i64 count, tuple }` from `isc_alloc`; one owner more is an increment,
  atomic in the task of a parallel loop (other threads may hold the
  node) and plain otherwise (SPEC section 8: the compiler knows which);
  one owner less calls `isc_rec_drop_<i>` (or its atomic variant): the
  last owner frees the parts, then the node, and the last part that is a
  node by a tail call, so that freeing a list of 10 000 000 nodes uses no
  stack. A cycle cannot exist (section 8). The type checker's test of a
  class (`Eq`, element of an array) took a recursive type into itself
  without end: the compiler overflowed its own stack; a sum type whose
  test is in progress now meets it inside itself. Not yet (errors that
  say so): an array of a recursive type, `==` on recursive values.
  **Fallback reference counting for arrays** (section 8: "une valeur
  partagée dont la durée de vie n'est pas déterminable à la compilation
  passe en comptage de références"), decided provisionally: arrays are not
  counted. A view (a slice, a row, a window) points inside a block, so a
  count would need the base of the block in every view; where the
  ownership pass cannot prove a single owner, it copies, and the report
  gives each copy with its reason, so that every such case is visible.
  Listed in TODO as a question. Tests: `tests/phase3.rs`
  (`recursive_types`, `recursive_types_in_parallel_tasks`, the errors),
  `memory.rs` (Valgrind, with 4 threads), `report.rs`, `verify.rs` (a node
  returned without a new owner is rejected), `isc-types/tests/check.rs`.
- **D105 — Uniqueness in the branches of an `if`.** The ownership pass
  moved a variable only at its last use in its own region, so that `if c
  then a with [0] = 1.0 else a` copied `a` in both branches. Its regions are
  now a tree (`region_parent`; each branch of an `if` knows its `if` and its
  side, a lambda body has none), and a use is a move candidate when it is
  the last on its path (`last_on_its_path`): it is in the region of the
  variable or in branches below it (never across a lambda: a loop would
  move it more than once), and every later use is in the other branch of
  an `if` (`exclusive`). After the rewrite, `consumes` decides for each
  variable moved somewhere whether every path moves it: a branch that does
  not consume a variable the other branch moves gets `ExprKind::Free(v,
  body)` (the body, then the variable freed), and the `let` frees it at
  the end of its scope only when no path consumes it. The memory check
  evaluates both branches from the same state and requires the variables
  defined before the `if` to be consumed in both or in neither; `Free`
  consumes its variable after its body, which must not borrow it. The
  reason of a copy (and the error of `require_unique`) names a later use
  on the same path; the report counts the uses of a value on one path for
  rule M1, and gives the reason of the use that asks the most of the
  memory. Tests: `isc-ir/tests/ir.rs` (`moves_in_branches`: no copy, one
  `Free`, `require_unique` in a branch), `phase3.rs` and `memory.rs`
  (against the interpreter, under Valgrind), `verify.rs` (a `Free`
  removed is rejected).
- **D106 — The requirements of section 9.** The type checker accepts
  `require_no_alloc`, `require_fused` and `require_tail` (the parser only
  takes them on a `fn`); `allow_reassoc` stays an error until it is done.
  `require_tail` is checked on the IR after the ownership pass: every call
  of the function to itself must be one of its direct tail calls compiled
  as a loop (`tail_calls`, D102); the error says, for each other call,
  whether it is not in tail position, passes an array that is neither the
  same parameter nor a new array, or belongs to a mutual recursion (not a
  loop in v0, section 11). On a function that is not recursive (inlined),
  it holds trivially. `require_fused` and `require_no_alloc` are checked
  on the function lowered alone (`lower::probe`), once per type arguments
  it is used with (and once for a function that is not generic, used or
  not; a generic function never used is not checked): its parameters are
  borrowed inputs, and its result is the destination its caller gives
  (section 8), so an array that is the result is not counted: the memory
  report now says "a result" for anything that computes the result of a
  function (through `let` bodies and branches, the accumulator of a final
  `fold`, the array a final `with` updates, a copy into the result), and
  the result of a lambda is a value at each iteration, not a result. The
  memory report of that body (`entries_of`) then gives what breaks them:
  for `require_fused`, any other array or copy; for `require_no_alloc`,
  also the nodes of recursive types and, in the recursive functions it
  calls, everything they allocate (an increment of a count is not an
  allocation; the call frames are not arrays). The verdicts depend on the
  rules of the specification only (section 9: portable verdicts). Found on
  the way: after F3 groups the `reduce`s of a value, the value is read
  once, but the fusion had already run: it runs again after F3 and the
  masked reductions, so that `let y = map(...) in sum(y) + max(y)` builds
  no array (the LLVM of the POC kernels is unchanged). Tests:
  `tests/phase4.rs` (positive and negative for each, the examples of
  section 9), `docs/language.md`.
- **D107 — Defunctionalization.** A function value chosen at run time
  (`if c then g else h`, nested or not, of named functions, operators or
  lambdas with captures) is `FuncVal::Choice` in the lowering: each call
  binds its arguments once and becomes `if c then g(args) else h(args)`,
  with no closure and no indirect call (SPEC section 11). Where the
  function is bound by `let`, its conditions are moved into variables
  computed once there (`hoist_choices`), so that a `map(x, f)` does not
  evaluate the condition per element; a choice written as an argument
  keeps its condition in place (pure, so the result is the same). Section
  11 speaks of a tag per possible function; with two functions per `if`,
  the tag is the boolean of the condition, and nested choices nest the
  branches. The memory report lists each choice (`switch`, from
  `Program::choices`). `match` choosing a function is not yet supported
  (an error says so). Tests: `phase3.rs` (`functions_chosen_at_run_time`,
  four conditions against the interpreter), `docs/language.md`.
- **D108 — The mathematical functions.** `exp`, `log`, `sin`, `cos`, `pow`
  and `fma`, for `f64` and `f32` (arguments of one float type), are
  primitives (`Prim::Math`, `ExprKind::Math`), also usable as function
  values (`map(x, exp)`); `x ^ y` with a floating-point `y` is `pow(x, y)`
  (SPEC section 10). The transcendental functions come from the crate
  `libm` (musl's algorithms in Rust, without `std`) through
  `isc_runtime_core`: the interpreter calls them there, the generated code
  calls `isc_exp`, `isc_expf`, ... of the runtime, which call the same
  code; checked to give the same bits natively, interpreted and as
  WebAssembly (`math_functions`). `fma` is the instruction `llvm.fma`
  (vectorized in a `map`), and `libm::fma` in the interpreter: both round
  once, so they agree. The other functions are calls, one element at a
  time. **Open question** of section 16 ("précision des fonctions
  transcendantes : reproductibilité contre vitesse"), decided
  provisionally: reproducibility, with musl's precision (under one unit in
  the last place, not correctly rounded); a vectorized implementation of
  the same algorithms would keep the bits and gain speed. Listed in TODO.
- **D109 — `allow_reassoc`.** The modifier is accepted on a `fn`; the
  lowering marks every expression of its inlined body (`Expr::reassoc`,
  `mark_reassoc`), including the functions it calls (`sum` of the prelude:
  the case that matters), and lists the function (`Program::reassoc_fns`)
  for the memory report (`reassoc`, as section 10 asks). The code
  generator gives the floating-point arithmetic of a marked expression the
  flags `reassoc nsz contract`: what `-fassociative-math` allows (with the
  `-fno-signed-zeros` it requires), not `-ffast-math` (no `nnan`, `ninf`,
  `arcp`, `afn`), as section 10 says the fair comparison is; `a * b + c`
  is then contracted into an FMA (checked in the assembly of `fast_dot`).
  The form of reduction stays the fixed one (section 10 allows another,
  does not require it; with the flags, LLVM may reorder it). The
  interpreter computes strictly; tests compare within a relative 1e-12.
  Nodes built by later passes (the combined `reduce` of F3) are not
  marked: they keep the strict operations, which is allowed. Tests:
  `phase4.rs` (`allow_reassoc`: flags only in the function, the strict
  one keeps its bits, the report), `docs/language.md`.
- **D110 — A vector path for the reduction of a stream.** `sum(filter(x,
  p))` of floats kept the order of section 10 through a scalar loop with
  a lane counter (D99): 116 043 ns for the 32 768 elements of the small
  input of `filter_sum`. When the source, the predicate and the maps are
  vectorizable (and the maps cannot panic: they run on the rejected
  elements too) and the operator is too, `stream_vector` takes each line
  of 64 bytes (8 `f64`), compares it at once, compresses its kept
  elements in order (`llvm.experimental.vector.compress`: `vcompresspd`
  on AVX-512) at the end of a buffer of V + 8 elements, and adds each
  group of V kept elements to the V accumulators as one vector: the k-th
  kept element still goes to accumulator k mod V, since a block holds a
  whole number of groups; a full block is closed (its tree to level 1, as
  in `rs_push`) when the next group comes, and the last elements go one
  at a time. Same bits as the interpreter on every test (to 25 000 000
  elements); 5 732 ns on the small input and 3 805 853 ns on 10^7 `f64`
  (instead of about 36 ms), measured without the machine settings. Found
  on the way: a store of the vector of accumulators into the array of the
  levels gave no alignment, so LLVM assumed that of the vector (256
  bytes) and the program stopped on a general protection fault when the
  array was not aligned so (the tests of F2 had passed by chance of
  alignment); the stores say `align 8`.
- **D111 — `scatter` (section 16, v1), decided provisionally.**
  `scatter(dest, idx, vals, op)`, in the prelude, is the `fold` over
  `zip(idx, vals)` that combines each value into `dest`: the rule of
  collisions is the order of k (several values at one index are combined
  in that order, whatever the operator). Indices are `i64` (the
  specification has no class of the integers). An index out of bounds is a
  panic at the call: the positions inside an inlined function of the
  prelude are now those of the call (`relocate_prelude`), so that the
  message names a line of the program. The parallel form of section 6
  (rule P) applies when the update has its form; the general parallel
  scatter stays open (TODO).
- **D112 — The fixed form of a scan of floats (section 16, v1), decided
  provisionally.** Section 10 makes a scan of floats sequential in v0; the
  v1 point asks for a parallel fixed form. The form chosen: the first
  block of B = 4096 elements is cumulated from `e`, from left to right;
  each later block is cumulated from its own first element, and its value
  i is op(carry, local[i]), the carry being the last value of the block
  before. It applies to every scan whose elements are floats; up to B
  elements it is the left-to-right cumulation, beyond it differs from it
  by the rounding of the carries. The interpreter computes it; the code
  computes it in one pass with one thread or inside a task
  (`scan_blocked_seq`), in three passes in parallel otherwise
  (`scan_parallel_float`: the last value of each block, the carries from
  left to right, the blocks written), with the same values for any number
  of threads. Tests: `tests/v1.rs`.
- **D113 — Mutual tail recursion (section 16, v1), decided
  provisionally.** Section 11 does not guarantee it in v0. Now: a call in
  tail position of a recursive function to another one reuses the frame,
  when the arrays it passes are parameters of the caller (alive after the
  caller is gone) and the caller's loop owns no array (D102: it would have
  to free it after the call); the temporaries freed after the call are
  freed before it (`other_tail_calls`). The code gives every recursive
  function the calling convention `tailcc` and emits such a call as `tail
  call tailcc` followed by `ret`, which LLVM guarantees to compile as a
  jump. The interpreter runs every tail call, of any function, in the loop
  of the call in progress (the variables of each function met saved once,
  restored at the end). `require_tail` accepts such calls; the memory
  report lists them (`tail`). Checked on 3 000 000 calls of `even`/`odd`
  and of two functions passing an array (`tests/v1.rs`). Not on
  WebAssembly: its LLVM back end takes no other calling convention than
  C's, so these calls stay calls there (the tail-call proposal of
  WebAssembly would allow them).
- **D114 — Functions stored in structures (section 16, v1), decided
  provisionally.** A tuple or a record of functions, and an array of
  functions, are allowed (the type checker accepts a function as an
  element of an array); they exist only when the program is compiled
  (`FuncVal::Tuple`, `FuncVal::Array`): a field, a pattern `let (f, g) =
  ops` and an argument of an inlined function take their functions; an
  array of functions indexed at run time (`fs[i](x)`, `map(x, fs[i])`) is a
  branch on the index (`FuncVal::Table`): the index is first checked by
  `iota(n)[i]` (fused: no array, the panic of an index out of bounds) and
  each call binds its arguments once; the memory report lists it as a
  choice at run time (section 11). Not yet (errors that say so): a
  function in a sum type, an array of functions used otherwise than by an
  index, a structure of functions chosen at run time.
- **D115 — The reductions of sliding windows by their shared levels.**
  Found by the agents that wrote the C and Go kernels of phase 7: in the
  fixed form of a window of k elements (k a multiple of V, at most B: one
  block), the value of lane j of window i depends only on i + j: it is
  L(i + j), with L(p) = op(...op(z, x[p]), x[p + V]...), and after the
  level d of the tree it is M_d(i + j) with M_d(p) = op(M_2d(p), M_2d(p +
  d)); the result of window i is M_1(i). The C kernel computes each level
  once per position (6 additions per window instead of 63). The code
  generator now does the same for `map(rows, (w) -> ... reduce(w, op, z)
  ...)` when the row is used only by that `reduce` (`window_reduce_of`)
  and, at run time, the rows are windows (a view of stride 1) of a width
  multiple of V and at most B: by blocks of 1 024 windows, in two buffers
  on the stack, in parallel block by block (`window_cascade`); the first
  element of each lane is combined with z in the first pass, each other
  element in its own pass (plain loops, vectorized), and the last level
  with the rest of the lambda. Other widths and strides take the former
  path. The interpreter keeps one `reduce` per window, so that the tests
  compare two different computations: the same bits for windows of 64,
  96, 32 and a non-associative operator, with 1, 4 and 16 threads, on the
  inputs of the kernel `moving_average` and in `window_reductions`.
  Measured once while other programs were running (to be measured
  properly): 32 768 `f64`, 274 127 ns per call before, 34 939 ns after.
  Then, on the tuned machine (one core, fixed frequency), three changes,
  each with the same bits: (1) the first level L in one pass when a lane
  holds 1 to 4 elements (windows of V to 4 V), chosen at run time (the
  width is a variable of the IR: `let tmp = window(x, 64)`), each element
  of the lane in its order; wider lanes keep one pass per element; (2) the
  levels two at a time (d and d/2 in one pass, from the four values of the
  level 2d: one store per position instead of two, the operands of the tree
  unchanged); (3) the task of the cascade prefers 512-bit vectors when the
  host has AVX-512 (attribute group `#1`; the rest of the program keeps
  LLVM's default tuning, 256-bit vectors): its passes over buffers in the
  L1 cache execute half as many instructions. Per call, 32 768 `f64`:
  39 493 ns before, 28 035 ns with (1) and (2), 19 752 ns with (3); GCC's
  `opt` (the same algorithm): 30 015 ns. 10^7 `f64`: 17 241 407 ns,
  13 647 649 ns, 11 693 611 ns; GCC's `opt`: 14 495 452 ns.
- **D116 — The scans of integers in vector registers.** The phase 7
  kernel `prefix_sum` was a scalar loop (one dependent addition per
  element). A `scan` whose operator is `+`, `*`, `min` or `max` on integers
  (exact, associative, commutative) and whose input is read by vectors now
  takes 64 bytes at a time (8 `i64`, 64 `u8`): the inclusive scan in the
  register by log2(W) shifts, the lanes shifted in holding the neutral
  value (0, 1, the largest or the smallest value of the type), then
  op(carry, x); the carry of the next vector is op(carry, last lane of x),
  so that the permutation is off the chain from one vector to the next (as
  the Rust and Zig `opt` kernels do; the first version permuted the result,
  3 cycles more per vector on the chain). The same values as the scalar
  loop (`integer_scans_in_vectors`: every width, the four operators,
  wrapping, lengths around the multiples of the lanes). Sequential path and
  the blocks of the parallel scan. Measured on the tuned machine, one core,
  32 768 `i64` (minimum of 7 rounds): 11 313.6 ns before, 6 596.0 ns after;
  Zig's `opt` 7 652.0 ns, GCC's 8 232.7 ns. 10^7 `i64` (30 calls in one
  process, D117): 7 968 733 ns, Zig 7 862 239 ns: limited by memory.
- **D117 — The output pages touched before the timer, in every language.**
  With one call per process, the large `prefix_sum` took 15 910 530 ns in
  Isocron, 16 027 793 ns in C and 16 251 261 ns in Go, but 7 903 327 ns in
  Zig and 8 091 754 ns in Rust: Rust and Zig write their output buffer
  before starting the timer, the others paid the first touch of 80 MB of
  new pages in the first timed call (the protocol of section 14 starts one
  process per measurement, so this cost stays in every measurement,
  divided by the repetitions). The rule is now the same for all: only the
  kernel is timed (section 14), the pages of the output are touched
  before. C writes its buffer with `memset` (not zeros, which GCC may turn
  into `calloc`), Go clears it with a loop; Isocron's programs make one
  call before the timed one, `let (_, _) = bench(1, f)` (the language is
  unchanged: `bench` calls `f` r times). For the block of that call to be
  given again to the timed calls, two changes of the compiler: a `_` of a
  component of a tuple binds nothing (it kept the tuple alive to the end
  of the scope), and an owned value the rest of its scope never reads is
  freed right after its definition instead of at the end of the scope
  (the code generator; the same effects, less memory). After: Isocron
  7 983 293 ns, C 8 153 047 ns, Go 8 446 777 ns with one call per process,
  as with 30.
- **D118 — Isocron's program for comparison B of the moving average.**
  Comparison B lets each language use the algorithm an experienced
  developer would write. In C, Rust, Go and Zig it is a running sum, O(n),
  which rounds differently from the sums of the windows. In Isocron it is
  the windows themselves, `map(window(x, 64), (w) -> sum(w) / 64.0)`: with
  D115 about 7 operations per window, exact, and faster than the O(n)
  version by prefix sums, kept as `examples/phase7/moving_average_prefix.isc`
  (32 768 `f64`, the same session: 77 096 ns for the prefix sums,
  19 752 ns for the windows; Rust's running sum 44 249 ns). So
  `moving_average_b.isc` is the same program as `moving_average.isc`.
- **D119 — The stream of a filter compacted by chunks.** The vectorized
  stream of D110 tested after each compression of 8 elements whether the
  buffer held a group of V kept elements, a branch that depends on the
  data. As the C kernel of phase 7 does, the inputs now go by chunks of
  1 024: their kept elements compressed into a buffer of the stack without
  any branch, then every full group of V added to the accumulators (a
  branch only to close a block, every B / V groups), the fewer than V left
  moved to the front. The kept elements reach the accumulators in the same
  order: the same bits (tests of `filter`, the kernel's references, sizes
  from 7 to 270 000 against the interpreter). (A loop of step 1 024 of the
  generator only runs whole steps: the chunks are counted instead.) On
  the tuned machine, one core: 32 768 `f64`, 8 045 ns per call before,
  5 453 ns after (GCC's `opt` 5 720 ns); 10^7, 5 115 165 ns before,
  4 327 380 ns after (Rust's `opt` 4 415 955 ns). Tried and left out:
  the store form of the compression (`llvm.masked.compressstore`, one
  `vcompresspd` to memory): 5 444 ns against 5 522 ns on the small size,
  4 017 541 ns against 3 973 068 ns on the large one, and very slow on AMD [a hypothesis: D375]
  Zen 4 and 5; 512-bit vectors for the whole function: 5 551 ns against
  5 431 ns.
- **D120 — `reduce` of a filter on several threads.** The stream of D99,
  D110 and D119 ran on one thread: `filter_sum` did not scale (5 470 ns
  with 1 thread, 5 519 ns with 8, against 4 745 ns for GCC's OpenMP
  `opt`). By section 10, the reduction of the kept elements is the
  reduction of the results of their blocks of B kept elements (one block:
  its result; none: the tree of z), so the blocks can be computed anywhere,
  each with its own elements in order: the result does not depend on the
  number of threads. The inputs are cut into chunks of at most 32 Ki
  inputs (one per thread for small inputs), taken by rounds of one chunk
  per thread: each chunk counts its kept elements; from the counts, the
  index of the first kept element of each chunk; a chunk owns the blocks
  that start in its range of indices, skips the kept elements of a block
  started before it and reads past its end to finish its last block
  (`stream_blocks`, the compression of D119 with a number of elements to
  skip and a number to take); at the end, the block results are reduced in
  the fixed form (`reduce_mat_seq`). The rounds keep the inputs of a chunk
  in the L2 cache of its thread between the count and the blocks (the
  pool gives task j to thread j); the blocks of round r and the count of
  round r + 1 are one task, so that memory is read while other threads
  compute. Measured on the tuned machine (parallel mode), 10^7 `f64`, per
  call with 1, 2, 4 and 8 threads: chunks of 128 Ki inputs, two tasks per
  round, 3 926 458, 3 110 424, 2 661 171 and 2 646 984 ns; 32 Ki inputs,
  1 818 050 ns with 8 threads; one task per round, 3 934 176, 3 052 815,
  1 999 649 and 1 763 260 ns; GCC's OpenMP idiomatic loop (one pass, its
  own order of additions) 12 741 052, 6 374 629, 3 248 895 and
  1 701 510 ns, GCC's OpenMP `opt` 4 383 798, 4 445 875, 3 173 066 and
  3 108 891 ns. With 8 threads, the loop of OpenMP and Isocron both read
  the 80 MB near the bandwidth of the memory (47 and 45 GB/s). 32 768
  `f64`: 2 940 ns with 8 threads against 4 668 ns. The same bits as the
  interpreter with 1, 2, 3, 4, 7, 8 and 16 threads (`parallel_streams`:
  densities from none to all, a non-associative operator, a map after the
  filter, negative zeros, 8 193 to 1 000 003 inputs), no error under
  Valgrind.
- **D121 — A `map` whose elements are loops, one chunk per thread.** The
  materialized `map` ran by blocks of B elements: `matvec` (4 096 rows,
  each a `reduce` of 4 096 products) was one task, 6.3 ms with any number
  of threads. When the function of the `map` contains a loop (`reduce`,
  `fold`, `scan`, `filter`), the elements are cut into one chunk per
  thread; the elements of a `map` do not depend on the cut. Measured
  (parallel mode, 1, 2, 4, 8 threads): 128 x 256, 2 858, 1 663, 1 073 and
  884 ns (GCC OpenMP `opt` 5 887, 3 403, 2 158, 1 805 ns); 4 096 x 4 096,
  6 348 626, 3 536 056, 2 768 299 and 2 790 869 ns (the best of the others:
  6 659 424 Zig, 3 636 641 Zig, 2 773 980 Rust, 2 828 320 Rust).
- **D122 — Non-temporal stores for results larger than the caches.** An
  output of more than STREAM_BYTES (32 MiB, twice the L3 cache) is not read
  again before the end of its loop: its lines are written with
  non-temporal stores (`!nontemporal`, 64-byte aligned vectors, then
  `sfence` before the end of the task), which do not read each line from
  memory before writing it. For now the vector `scan` of integers (D116)
  and the last pass of the window cascade (D115, now by explicit vectors;
  the `reduce` given counts as a vector in `vec_ok_given`). Not on
  WebAssembly. Measured, 10^7 elements, 1, 2, 4, 8 threads: `prefix_sum`
  7 888 570, 7 724 422, 6 878 790, 7 325 317 ns before, 5 275 685,
  5 731 898, 5 146 487, 5 193 087 ns after (Rust's `par_opt` 8 032 436,
  7 653 057, 6 749 846, 7 347 864 ns); `moving_average` 11 626 038,
  7 241 308, 5 707 117, 5 306 608 ns before, 8 901 259, 5 300 721,
  4 105 974, 3 748 033 ns after (the running sums of comparison B: GCC
  OpenMP 15 073 566, 7 711 721, 5 582 859, 5 815 836 ns).
- **D120 (continued) — The 4 % left with 8 threads.** On the large
  `filter_sum`, Isocron takes 1 758 516 ns with 8 threads, GCC's OpenMP
  idiomatic loop 1 707 955 ns (one pass, its own order of additions, a
  result that is not the fixed form). The memory is not the limit: an
  Isocron `count` of the same predicate (one read of the 80 MB, no rounds)
  takes 1 531 735 ns. The difference is the computation of the blocks (the
  compression from the L2 cache), which the rounds overlap only partly
  with the reads: all threads compute, then all read. Tried and left out,
  each slower with 8 threads: the next chunk prefetched during the
  compression (1 927 759 ns), the odd tasks counting first (1 911 076 ns),
  two chunks of 16 Ki per thread and round (1 853 752 ns), chunks of 16 Ki
  (1 792 001 ns) and 64 Ki (2 009 369 ns): the cache of each core (512
  KiB of L2) cannot hold the chunk being computed and the next one [contradicted in part: D375]. With
  2 and 4 threads Isocron is about twice as fast as that loop (3 024 826
  against 6 375 759 ns, 2 009 870 against 3 244 530 ns).
- **D123 — `allow_reassoc` in the measurements.** Section 14 compares the
  reassociated variants of C and Zig with Isocron's `allow_reassoc`, which
  no program used: `examples/reassoc/` holds the six kernels that have
  such variants (`sum`, `dot`, `std_dev`, `moving_average`, `filter_sum`,
  `matvec`), built as `isc-reassoc_<kernel>` and reported apart by
  `bench/run.py`. Two changes of the compiler for this mode: (1) the
  masked reductions of D99 also take `+` and `*` of floats under
  `allow_reassoc` (a rejected element counts as 0.0 or 1.0; the sign of a
  zero is ignored, `nsz`): `reduce` of a filter without compaction
  (`reassociated_filters_are_masked`); (2) the multiplications of `^`
  carry the flags of the mode, so that `(x - m) ^ 2` and the sum are
  contracted into an FMA. One core, tuned machine, 32 768 `f64`:
  `filter_sum` 1 772.6 ns against 2 640.5 ns for Zig reassociated (median
  of 7 rounds), `std_dev` 5 694.9 ns before (2), 4 410 ns after, against
  5 253 ns for Clang and 5 268 ns for Zig reassociated; 10^7: `filter_sum`
  3 564 430.0 against 3 658 667.0 ns, `std_dev` 7 053 122 against
  7 191 194 ns. The strict programs are unchanged (the same bits).
- **D121 (continued) — The chunks start at different rows.** In the
  campaign of stage 2, `matvec` 4 096 x 4 096 with 8 threads took
  2 890 656 ns in Isocron against 2 876 656 ns in Rust's rayon `par_opt`.
  Alternated runs (41 rounds) showed a steady difference (3 311 669
  against 3 223 133 ns in one session) with equal minimums (about 2.86 ms
  for all), the same with 300 calls per process (3 297 752 against
  3 197 709 ns): not the start of the process; not the TLB (Isocron has
  fewer misses and page faults, its huge pages); not the unrolled loop
  of the arrays read from memory (3 277 318 ns). The 8 chunks of one
  thread each started exactly 16 MiB apart, so that the 8 streams read
  addresses a power of two apart at the same time; rayon's threads do
  not. Each chunk of a `map` of loops now starts at row 37 i of its range
  (then wraps), the order of the elements of a `map` being free. Measured
  in two sessions: 2 824 692 against 2 904 950 ns (Rust 2 891 300 ns), and
  3 244 319 against 3 294 015 ns (Rust 3 221 223 ns; the machine changes
  between sessions more than the languages differ). Smaller chunks did not
  help (16 per thread: 3 292 943 ns against 3 266 789 for Rust). The
  official measurement of `matvec` with the change (`bench/parallel.py
  --kernels matvec`, merged into `results/stage2.md` by
  `bench/merge_kernels.py`): 2 851 920 ns with 8 threads, Rust 2 884 380 ns.
- **D120 (continued).** Also tried: counting the next chunk 1 024 inputs at
  a time between the chunks of the compression, so that each thread reads
  memory while it computes: 1 964 997 ns with 8 threads against 1 822 261
  ns, and slower with 2 and 4 threads. The campaign of stage 2 keeps the
  version of D120: 1 813 718 ns with 8 threads, GCC's OpenMP idiomatic loop
  1 744 216 ns, a difference within the dispersion (85 783 ns).
- **D124 — The campaign of stage 2.** Every language and every kernel (the
  five of the POC, the five of phase 7 with comparison B, the six with
  `allow_reassoc`), one session on the tuned machine
  (`results/stage2-single.md`, `results/stage2.md`,
  `results/stage2-compile.md`; the earlier campaign stays in
  `results/xeon*.md`). One core: Isocron has the lowest median in the 20
  strict cases and its `allow_reassoc` programs in the 12 cases that have
  reassociated variants. Parallel (1, 2, 4, 8 threads): the lowest median
  in 79 of the 80 cases; the 80th is the large `filter_sum` with 8 threads
  (D120). Compilation: 0.08 s per program (median), GCC 0.07 s.
- **D125 — `extern` (SPEC section 12).** `extern name(params) -> type`
  declares a C function (System V x86-64 ABI), called only from an action
  (an action of the IR, `ActionOp::Extern`: run in its place, never
  removed). Decided where SPEC is silent: parameters are scalars (the C
  scalar of the same width; `i8`/`i16` with their C extension; `bool` as C's
  `bool`), `string` (a NUL-terminated copy, freed after the call; a string
  with a NUL byte is a panic) and arrays of scalars (two C parameters: a
  pointer to the elements, read only, and their number as an `int64_t`);
  the result is a scalar or `()`; at most 6 parameters in integer registers
  (an array takes two) and 8 in floating-point registers, so that every
  argument is in a register; variadic C functions are not supported; a name
  starting with `isc_` or `llvm`, or `main`, is reserved; an extern function
  is not a value. The compiled call carries `nobuiltin` (LLVM computed
  `cbrt(27.0)` itself, `3`, where glibc gives `3.0000000000000004`). The
  libraries of the extern functions, then libm and libc, come before the
  runtime on the command line of the linker: the runtime's archive holds
  weak copies of C functions (Rust's `compiler_builtins`, `cbrt` among
  them) which otherwise replaced glibc's. `iscc build --link name` adds
  `-lname` or a library by its path; `iscc run --link name` loads it into
  the interpreter, which finds each function with `dlsym` before the
  program starts and calls it through one function type of 6 integers and 8
  doubles (System V gives each class its own registers; an `f32` in the low
  half of its register). WebAssembly: the same code, with WASI's C library
  (its `cbrt(27.0)` is `3`: the result of a C function is that of the C
  library of the target). Tests: `tests/extern.rs` (libc, libm and a
  library built from `tests/data/extern_helper.c`: every class and width,
  slices, the errors).
- **D126 — A `map` that builds an array of arrays.** `map(x, f)` where `f`
  returns an array of scalars is an array of arrays (SPEC sections 4 and
  6): its rows have the same size, which the type guarantees (a size that
  does not depend on the element: `map(iota(i % 3 + 2), ...)` in the
  function is refused by the type checker), in one block, rows after rows.
  The length of the rows is that of row 0; each row is written in its
  place (`row_into`): a literal element by element, a `map`, `iota`,
  `fill` or `zeros` filled in place (no temporary array), a variable, a
  slice or a row of another array of arrays copied; a variable of a `let`
  of the function moved to the end of its body is copied then freed. Other
  kinds of rows (an `if`, a call) are refused with a message. In parallel,
  one chunk of rows per thread. The memory check (D96) took the result of
  the function of a `map` as given to the `map`; for rows, a new row is
  built in place and a borrowed one copied: no double free, no leak
  (Valgrind with 1 and 3 threads). Test: `map_builds_arrays_of_arrays`
  (literals, `map`s, slices, a variable of the program, a `let`, integers,
  floats, booleans, no row; the same bits as the interpreter).
- **D127 — A function chosen by `match`.** `let f = match x ...` whose arms
  are functions: the choices of D107, one per arm (`if` the pattern of arm
  0 matches then its function, else ...), the first arm that matches, the
  conditions computed once where `f` is bound; an arm that matches every
  value ends the choice. An arm whose pattern binds a variable is refused
  for now (its function would read the payload of a variant that may not be
  the value's: invalid for a recursive type). Test:
  `functions_chosen_by_match`. Also found while testing in release mode:
  the interpreter built with optimizations holds about 600 000 nested calls
  that are not tail calls (300 000 and 600 000 run, 1 000 000 is the panic
  "stack overflow" of D102), fewer than when it is built without them (its
  frames are larger); the 4 GiB of its stack stay (a larger reservation can
  fail on a smaller machine), and `recursive_types` uses lists of 500 000
  elements, held in both builds.
- **D128 — A recursive function with a function parameter.** SPEC section
  11 specializes each call for the function it receives; a recursive
  function is not inlined (D102), so it is now specialized: the key of its
  IR function is (function, type arguments, functions given), the function
  given is bound in the body (its calls inlined there) and passed again to
  the recursive calls, which then reach the same specialization; tail
  calls stay loops. Supported: a function of the program, an operator, a
  primitive, a lambda that reads no variable or size of its context (known
  by its expression). A lambda with captures would need its captured
  values as parameters of the specialization: refused for now, with a
  message. Tests: `recursive_functions_with_function_parameters` (the same
  bits as the interpreter, 1 and 4 threads), `recursive_functions_are_functions`
  (one specialization per function given, the refusal).
- **D129 — `==` on values of a recursive type.** Structural (SPEC section
  5): the lowering keeps one `==` of two nodes (expanding it as for tuples
  would never end), both sides bound by a `let`, freed after the comparison
  that only reads them (the memory check found the nodes of
  `Cons(1, Nil) == Cons(2, Nil)` never freed without it); the code generator
  emits one function per type, `isc_rec_eq_<i>`: component by component,
  the tag first (the slots of the other variants are zero, D71), floats
  with IEEE equality, no shortcut for the same node (NaN differs from
  itself, as in the interpreter), the last component that is a node by a
  tail call (a list of 1 000 000 compared without stack); the interpreter
  compares the values. A node that holds a string is refused (no equality
  of strings yet). Test: `equality_of_recursive_values`.
- **D130 — Arrays of a recursive type.** An array whose elements are (or
  hold) nodes of a recursive type: an array of pointers, each element one
  owner of its node (D103). Freeing the array takes one owner from each
  node; copying it adds one to each; `with` takes one from the element it
  replaces; `filter` (materialized or a stream) adds one to each element
  it keeps; the function of a `map` that returns an element of another
  array (borrowed) is copied by the ownership pass (one owner more), as a
  component of a tuple or a literal already was; reading an element and
  keeping it elsewhere adds an owner the same way. The interpreter has an
  array of nodes (`Array::Nodes`). Found by the memory check (D96: a
  borrowed element given to a `map`) and by Valgrind (the nodes of a freed
  array never freed; `filter` without the extra owner: a read after free).
  Tests: `arrays_of_recursive_types` (literals, `map`s, `with`, `filter`,
  `fold`, slices, `==`, a recursive function, tuples of nodes; the same as
  the interpreter, 1 and 4 threads), `arrays_of_nodes_free_everything`
  (Valgrind).
- **D131 — A recursive function given a lambda with captures.** D128
  refused a lambda that reads variables of its context. Its captured locals
  (in their order of first reading) now become parameters of the
  specialization, after the others: the caller passes the values of its
  variables (borrowed, as any argument), and in the specialization the
  lambda is bound with its captures bound to these parameters, so that the
  recursive calls pass them again and reach the same specialization (the
  key stays the lambda's expression). Still refused: a lambda that reads a
  size of its context, or a function of it. Test:
  `recursive_functions_given_lambdas_with_captures` (scalars, an array,
  two captures, values computed at run time; the same as the interpreter;
  no error under Valgrind).
- **D132 — A function chosen by `match` that reads its pattern.** D127
  refused an arm whose pattern binds a variable. The variables of a pattern
  of one level (the value itself, the components of a tuple or of a
  variant, each a variable, `_` or a literal) are now read from the value
  whatever its variant (the slots of the other variants are zero, D71; a
  node's tuple through its pointer, never null) and bound where the
  function is bound, with the conditions of D107 (`FuncVal::Bound`; around
  the call when the function is used in place). Below the first level the
  value read could be the null slot of a node of another variant: refused,
  with a message. Test: `functions_chosen_by_match` (a payload, a tuple
  with a literal, the head of a list read from `Nil` too, the whole value;
  no error under Valgrind).
- **D133 — Part 3, the design decided provisionally (to be confirmed by
  the author).** SPEC v0 has `string` as an opaque type of actions and
  `extern`, no string literal, binary input/output only, no modules and no
  `Result`. Decided, in this order of work (each step tested against the
  interpreter, documented, then measured where it matters):
  1. **Strings**: literals `"..."` (UTF-8; escapes `\n`, `\t`, `\r`,
     `\\`, `\"`, `\u{hex}`), immutable values usable in any `fn` (pure);
     `==`, `!=`, `<`, `<=`, `>`, `>=` (by bytes, lexicographic), `++`
     (concatenation), `len(s)` (bytes), `bytes(s) -> [k]u8`,
     `string_of(b) -> string` (a panic if not UTF-8), `to_string(v)` for
     the scalars (the text of `print`), `parse_f64(s)`; `split(s, sep) ->
     [k]string`, `trim(s)`, `lines(s) -> [k]string` in the prelude or the
     runtime. Memory: a string is an array of bytes (`{ptr, len}`), owned
     and freed as arrays are; a literal is a constant, never freed.
  2. **Text input/output**: actions `read_text(path) -> string`,
     `write_text(path, s)`, `print` of a string without quotes.
  3. **CSV**: `parse_csv_f64(text) -> [r][c]f64` (comma-separated decimal
     numbers, one row per line, every row of the same length: a panic
     otherwise) and `read_csv_f64(path)`; text cells through `split`.
  4. **`Result`**: `type Result<T, E> = Ok(T) | Err(E)` in the prelude;
     fallible forms of the actions that panic today:
     `try_read_f64(path) -> Result<[k]f64, string>`, `try_read_text`,
     `try_parse_i64(s) -> Result<i64, string>`, `try_parse_f64`; the panic
     forms stay.
  5. **Modules**: `import name` at the top of a file: the file `name.isc`
     of the same directory (then of the directories of `ISC_PATH`); its
     declarations are reached as `name.decl` (types, functions, actions,
     constructors); no cycle (an error); one module compiled once whatever
     the number of imports; everything is public for now.
- **D134 — Strings (part 3, step 1 of D133).**
  - Syntax: literals `"..."` on one line, escapes `\n \t \r \0 \\ \"
    \u{hex}` (lexer, `TokKind::Str`, `ExprKind::Str`); `a ++ b` at the
    precedence of `+`, parsed as a call of the primitive named `++`, so a
    function `concat` of the program never replaces it.
  - Primitives: `concat`/`++`, `len` (bytes), `bytes`, `string_of`
    (UTF-8 checked, panic `NotUtf8` with the index of the first invalid
    byte), `to_string` of a scalar (the text of `print`), the action
    `parse_f64` (Rust's `f64::from_str`, panic `ParseF64`). A function of
    the program with one of these names takes precedence, as for every
    primitive. Strings are ordered by their bytes (class `ORD` without
    `SCALAR`); `==` on strings, and on recursive types that hold strings
    (the refusal of D129 is gone), compares bytes.
  - IR: `ExprKind::StrLit` (a new string at each evaluation) and
    `ExprKind::StrOp` (operands only read); `Type::Str` now `has_heap`: a
    string is owned and freed as an array of bytes. The runtime copies the
    program arguments (`isc_args`), which `main` frees with their array.
    Runtime: `isc_str_new`, `isc_str_concat`, `isc_str_cmp`,
    `isc_str_from_bytes`, `isc_to_string_*`, `isc_parse_f64`.
  - Arrays of values that own memory (`holds_owned`, formerly
    `holds_node`): each element freed and copied (a string copy is a new
    allocation, a node copy one owner more). A producer of such elements
    (`owns_elems`) is never fused into its consumer and a `map` of them is
    never a stream: it is evaluated and freed as a temporary (the fused
    form computed elements that nobody freed; this also held for nodes).
  - `reduce` and `scan` of values that own memory: the ownership pass makes
    the neutral element and each result of the operator new values; the
    generator has `reduce_owned` (the fixed form of section 10, blocks of
    B, V lanes and the tree, level by level, every replaced value freed)
    and `scan_owned`. Measured nothing: these paths are scalar and only
    used for owned values.
  - Not supported, refused at compile time: arrays of arrays of strings
    (an array of arrays holds scalars).
  - Tests: `crates/iscc/tests/strings.rs` (interpreter and compiled
    output identical, Valgrind clean); the guide has a section *Strings*.
- **D135 — Text files and CSV (part 3, steps 2 and 3 of D133).**
  - Actions: `read_text(path) -> string` (UTF-8 checked, panic
    `TextNotUtf8` with the first invalid byte), `write_text(path, s)`,
    `parse_csv_f64(text, skip) -> [r][c]f64` and
    `read_csv_f64(path, skip) -> [r][c]f64`. D133 had no `skip`: a CSV
    file usually starts with a header, and guessing it (a first line that
    is not numeric) would be implicit; the header is read as text with
    `lines` and `split`. CSV rule (`isc_runtime_core::parse_csv_f64`,
    shared by the runtime and the interpreter): one row per line, `,`
    between fields, each field trimmed and read by `parse_f64`, blank lines
    skipped, every row with the fields of the first; the panics
    `CsvField` and `CsvRagged` give the line (from 1, header included).
    The compiled form reads twice: checked and counted, then written into
    one block (rows contiguous, the layout of an array of arrays).
  - Functions (pure): `lines` (`\n`, a `\r` before it removed, no empty
    last line, none in `""`), `split` (Rust's `str::split`; an empty
    separator is a panic `SplitEmpty`), `trim` (Unicode white space),
    `join`. `len` also gives the number of elements of an array: a result
    of existential size (`words(s) -> [k]string`) has no size the caller
    can name, and SPEC had no length function.
  - `trim`, `len` (of a string), `++`, `to_string`, `string_of` and `join`
    can be used as values (`map(xs, trim)`); `lines`, `split` and `bytes`
    cannot (their result has a size of its own).
  - Tests: `crates/iscc/tests/text.rs`; the guide documents the actions
    and has a CSV example.
- **D136 — Planned: what idiomatic Benchmarks Game programs need.** A
  prototype of `mandelbrot` (Benchmarks Game, P4 bitmap) gives the same
  bytes as the classic C program; idiomatic Isocron still lacks the
  concatenation of arrays (`++`), bitwise operators on integers and an
  action writing bytes to the standard output. Rough timing (unmeasured
  protocol, untuned machine, N = 4000): 1069 ms on one thread against
  1038 ms for scalar C `-O3`, 236 ms on 8 threads; the best entries of the
  Benchmarks Game compute 4 to 8 pixels at once in SIMD.
- **D137 — D136 done: bitwise operators, `++` on arrays, `print_bytes`.**
  - `&`, `|`, `xor` (a word, as `and` and `or`; recognized only between
    two operands, so `xor` stays a valid name), `<<`, `>>`: integers of
    one type, element-wise on arrays as `+`. Precedence, from low to high:
    comparisons, `|`, `xor`, `&`, `<<` `>>`, `+` `-` `++`, `*` `/` `%`
    (as Rust and Python: above the comparisons, unlike C). `<<` and `>>`
    are two adjacent `<` or `>` tokens, so `Opt<Opt<i64>>` still parses.
    `>>` is arithmetic on signed types, logical on unsigned ones; the
    shift amount is taken modulo the number of bits (as Java; not as the x86
    instructions for 8 and 16 bits: D375): one `and`, no check, vectorizable. `(&)`, `(|)` and
    `(xor)` are functions.
  - `a ++ b` of arrays: `[n]T ++ [m]T -> [n + m]T` (an existential size
    when a size is unknown), lowered to `map(iota(len(a) + len(b)), (i) ->
    if i < len(a) then a[i] else b[i - len(a)])`: every element type, the
    interpreter and the ownership rules for free; a copy by blocks
    (`memcpy`) is left for when a measurement asks for it.
  - `print_bytes(x: [n]u8)`: the bytes into the buffer of `print`, in
    order with it; the generated `main` now flushes that buffer at its end
    (`isc_flush`), a panic already did.
  - `examples/benchmarks_game/mandelbrot.isc`, idiomatic: the same bytes
    as the classic C program and as an independent implementation in
    `crates/iscc/tests/bitwise.rs` (N = 1, 7, 8, 9, 200, 201, 1000).
    Rough timing (untuned machine, N = 4000): 1023 ms on one thread, C
    scalar `-O3` 1038 ms; 226 ms on 8 threads.
- **D138 — `Result` and the fallible actions (part 3, step 4 of D133).**
  - The prelude declares `type Result<T, E> = Ok(T) | Err(E)` and the
    actions `try_parse_i64`, `try_parse_f64`, `try_read_bytes`,
    `try_read_f64`, `try_read_text`, `try_read_csv_f64`, `try_write_text`,
    `try_write_bytes`, written in Isocron over the primitives
    `<action>_or_error` (`Fallible`, `ActionOp::OrError`): the value and
    the message the panic would print, `""` when the action succeeds (on
    an error the value is empty or zero). No sum value is built by the
    code generator: tuples only. The runtime functions return `Result<_,
    Panic>` (`try_read_file`, `try_write_file`), the panicking forms die
    on `Err`; on an error the buffer is freed and the file closed.
  - The prelude may declare types: they come after the program's, and a
    program that declares the name of a prelude type or of one of its
    constructors (`Result`, `Ok`, `Err`) keeps its own; the prelude type
    and the prelude functions whose signature mentions it are left out.
    The functions of a program take precedence over the primitives even
    in the prelude, so the prelude compares with `""` instead of calling
    `len`.
  - The parameters of a `type` declaration take no class (a payload can
    be an array of arrays, `Result<[r][c]f64, string>`); a sum stored in
    an array still constrains its payloads. The existential sizes of a
    result are opened inside the arguments of a sum type too
    (`Result<[k]f64, string>`). `len` of a value whose type is not known
    yet is decided at the end of the checking (a string or an array).
  - No copy of what is read: `Take` (IR) takes the components out of a
    new tuple destructured right after its definition (`let (a, b) =
    f(x)`), each then owned on its own; an `if` whose two branches borrow
    is made new branch by branch (a move of the variable used for the
    last time in each branch, D105). Both are checked by the memory
    verifier (`taken`). Before them, `try_read_f64` copied the array and
    `let (h, d) = f(x) in if b then h else d` copied the result.
  - Tests: `crates/iscc/tests/result.rs` (outputs of both modes, Valgrind,
    no copy in the memory report); the guide has *Errors as values*.
- **D139 — Modules (part 3, step 5 of D133).**
  - `import name` at the top of a file (a word recognized there only, so
    `import` stays a valid name elsewhere): the file `name.isc` of the
    importing file's directory, else of a directory of `$ISC_PATH`
    (separated by `:`). The driver (`iscc::frontend`, `load_modules`)
    reads each module once, gives it a file id for the diagnostics,
    refuses a cycle (`import cycle: a -> b -> a`), a missing module and a
    module that declares `main`, renames its declarations `name.f`,
    `name.T`, `name.C` (a dot: no program can write such a name otherwise)
    and adds them to the program; an `extern` keeps its C name.
  - In a file that imports `m`, the parser reads `m.x` as the name `m.x`
    in expressions (calls, constructors, values), types and patterns, even
    where a variable has the module's name.
  - Inside a module, an unqualified name is looked for in the module first
    (`FnCx::q`, `SigCtx::module`), then among the program's and the
    prelude's names: a module can declare its own `sum`.
  - Everything is public; no subdirectories (`a.b.c`), no renaming
    (`import m as n`), no private declarations: later if needed.
  - Tests: `crates/iscc/tests/modules.rs`; the guide has *Modules*.
- **D140 — SPMD calls: tail-recursive functions computed lane by lane in
  vector code.** A recursive function whose recursive calls are all tail
  calls to itself, whose parameters and result are scalars and whose body
  cannot panic whatever its arguments (`cannot_panic`; the lanes that have
  stopped keep any state), with a body of `let`s and `if`s whose leaves
  are values of vector code or such tail calls (`spmd_fns`, `spmd_shape`),
  is accepted by `vec_ok`. In vector code (`ex` at width w > 1) its call
  is a loop (`spmd_call`): the state of each lane in vectors; each round
  computes the body for every lane under the mask of its path
  (`spmd_body`), a lane reaching a value keeps it and stops, a lane
  reaching a tail call takes its new arguments; the loop ends when no lane
  is active (`llvm.vector.reduce.or`). The lanes not on the call's path
  start stopped: `if`s in vector code now carry the mask of each branch
  (`Gen::mask`, `branch_masks`), so a call guarded by `x < n and ...`
  never runs for the lanes outside its guard. A `fold` over the same
  elements in every lane (`iota(8)`, an array of the context) is a scalar
  loop of a vector accumulator (`fold_vec`). A map whose element calls a
  function is a heavy map (one chunk per thread, D121).
  - The idiomatic `examples/benchmarks_game/mandelbrot.isc`, unchanged,
    gives the same bytes and goes from 1023 ms to 199 ms on one thread
    (5.14 times faster), 46 ms on 8 threads (rough timing, untuned
    machine, N = 4000, MAP_WIDTH = 16 lanes).
  - `--isa avx2` (the target of the Valgrind runs) also removes GFNI,
    VAES and VPCLMULQDQ, which Valgrind does not decode and LLVM uses for
    the masks.
  - Tests: `crates/iscc/tests/spmd.rs` (Collatz, Newton, a call under a
    condition, `fold`s nested, Euclid and a non-tail recursion left
    scalar; the vector loop present in the code of mandelbrot).
- **D141 — `--isa avx` (Ivy Bridge).** LLVM CPU `ivybridge`: AVX without
  AVX2 or FMA, the instruction set of the i5-3330 of the Benchmarks Game,
  to compare with its programs built `-march=ivybridge`; 16 registers of
  32 bytes for the cost model.
- **D142 — The Benchmarks Game programs, built and run in a Docker image
  of ours** (`bench/benchmarks_game/`; the author's condition: nothing
  installed on the host, their code isolated). `fetch.sh` downloads their
  source archive (sha256 kept in `build/`) and extracts the programs of
  `programs.txt` with their license (BSD 3-clause); `image.sh` builds the
  image `isocron-bg` (Debian trixie: gcc and g++ 14.2 as on their site,
  Rust 1.84.1 their version, Go 1.24, `libllvm21` from trixie-backports
  and the iscc of the host); `run.sh build|check` runs `build_programs.sh`
  and `check_programs.sh` in it without network, as the user of the host.
  Two modes: `ivybridge`, their command lines (AVX without AVX2 or FMA),
  Isocron `--isa avx`; `native`, the same with this CPU, where C and C++
  add `-ffp-contract=off` (with AVX-512, `-mno-fma` no longer stops gcc
  from contracting into FMAs, which changed the output) and C++ #4 adds
  `-mno-avx512f`: its AVX-512 branch returns 0 for a byte as soon as one
  of its 8 pixels escapes (`_mm512_cmp_pd_mask(...) != 0`) where its other
  branches wait for all 8, so its output is wrong on an AVX-512 machine
  (theirs has none). C++ #4 also rounds its number of rows up to a
  multiple of the number of CPUs: at N = 200 its output differs on this
  machine of 16 CPUs; N = 16000 (their size) is the check: every program,
  in both modes, gives the same bytes as Isocron there.
- **D143 — A `map` whose elements are loops: 8 chunks per thread.** D121
  cut it into one chunk per thread; with elements of very different costs
  (mandelbrot: the set fills the middle rows) the threads waited for the
  one with the middle rows: 1105 ms with 4 threads, 807 ms with 8 chunks
  per thread dealt in turn (`HEAVY_CHUNKS`; 32 or 64 per thread: 613 and
  615 ms against 622, not more). The elements never depend on the cut.
  The remaining gap of the scaling (3.1 against 3.9 for C++ from 1 to 4
  threads) is the frequency: `perf stat` gives 4.58 GHz on one core and
  3.48 GHz on 4 cores with 512-bit vectors (4.59 and 4.28 with 256-bit
  ones); 256-bit SPMD code with the masks of AVX-512VL (prefer-256-bit) is
  still slower: 875 ms against 640 ms with 4 threads. Kept: 512 bits.
- **D144 — The SPMD loop (D140), made cheaper.**
  - Width: as many lanes as the registers hold, about 8 bytes per
    parameter of the function but one, at least 4 (`spmd_width`); the loop
    is bound by the latency of its chain, so independent lanes pay until
    the registers are full. Measured on mandelbrot, one thread, N = 8000:
    AVX-512 4/8/16/32/64/128 lanes: 1830/1046/767/608/539/751 ms; AVX
    4/8/16/32/64: 1591/1049/806/897/2932 ms; SSE 8/16: 2209/2219 ms. Hence
    64, 16 and 8 lanes.
  - Uniform parameters (`spmd_uniform`): a parameter uniform at the call
    and given by every tail call the same expression of uniform parameters
    and literals (the counter `i + 1`) keeps one value for all the active
    lanes: a scalar, its tests scalar.
  - The first tail call reached gives the next state to every lane, the
    others only to their lanes: a lane that stops keeps no state.
  - Without mask registers, a value leaf is computed only in the rounds
    where a lane reaches it (AVX 804 to 731 ms, SSE 1316 to 1121 ms; with
    AVX-512 the test costs more than the select: 507 against 522 ms).
  - Tried and dropped: the masks as one `i64` per lane without AVX-512
    (AVX 1640 ms against 804, AVX2 1303 against 766, SSE 1762 against
    1316: LLVM's own `i1` masks are better).
  - Result on this machine (rough timing, the Docker image, N = 16000):
    one thread native, Isocron 1914 ms, C++ #4 2151, Rust #4 2428; 4
    threads native 568 ms, C++ #4 557, Rust #4 621; `ivybridge` one
    thread 3143 ms against C++ 2169 (before D144's leaf rule), the gap
    being the masks without mask registers and the checks of each
    iteration, which the C++ and Rust programs make every 4 or 5
    iterations by hand.
- **D145 — Dynamic chunks for the maps whose elements are loops.** The
  runtime has `isc_parallel_for_dynamic`: the threads take the next chunk
  not taken (an atomic counter in a line of its own, `NextLine`) as they
  finish the one before; no chunk is taken twice, and each chunk writes
  only its elements, so nothing depends on which thread runs it. Used for
  the heavy maps (D121, D143), 8 chunks per thread (32 and 64 measured
  slower: 577 and 576 ms against 560 with 4 threads). Gain on mandelbrot
  (AVX, 4 threads): 605 to 595 ms. With 4 threads the program keeps the 4
  cores busy (3.979 CPUs used); what it loses against one thread (2127 ms
  of CPU time, 2220 with 4) is the frequency (4.55 to 4.39 GHz with 4
  cores working).
- **D146 — Uniform branches in SPMD bodies.** An `if` of a SPMD body
  whose condition is uniform (`g == 12` of a uniform counter, D144) is a
  real branch: every active lane takes the same side, only that side is
  computed, without masks; the next state and the finished lanes meet in
  `phi`s. A value leaf that is a literal or a variable is selected at once
  (testing whether a lane reaches it costs more). mandelbrot (by 4, D147):
  AVX one thread 2211 to 2103 ms, 4 threads 595 to 563 ms; native 1839 to
  1752 ms and 524 to 502 ms.
- **D147 — The Isocron mandelbrot of the Benchmarks Game tests the escape
  every 4 iterations** (`examples/benchmarks_game/mandelbrot.isc`), as the
  fastest programs of the site do (C++ #4 every 5, Rust #4 every 4): a
  point with |z| > 2 never comes back, and in 3 more iterations |z| stays
  far from an overflow (|z| <= about 6 at its first escape), so the image
  is the same (checked: N = 1 to 4001 against the classic C, 16000 against
  every program). The simplest form, tested at each iteration, stays as
  `mandelbrot.simple.isc`. This is a change of the program, not of the
  compiler: the compiler cannot space out the tests of an arbitrary
  program. Rough timing in the Docker image (N = 16000): `ivybridge` one
  thread 2110 ms (C++ #4 2170, Rust #4 2676), 4 threads 578 (554, 626), 8
  threads 337 (334, 351); native 1749 (2151, 2428), 502 (560, 620), 297
  (335, 348). The CPU time with 4 threads is the same as C++'s (2220 ms
  against 4 x 554); the ranking with threads waits for the measurement at
  a fixed frequency (SPEC section 14).
- **D148 — Syntax 2, inspired by TypeScript (decided by the author,
  2026-11-02).** Only the front-end changes: the new parser builds the
  same AST, so the type checker, the IR, the code generator and the
  runtime are untouched. One syntax in the end: the old parser stays only
  to convert the existing programs (examples, guide, SPEC, tests), then
  goes. TypeScript where it has an equivalent; additions where it has
  none (sizes in types, `match`, `action`, typed literals).
  - Comments `//` and `/* */`; statements end with `;` (required: no
    automatic insertion).
  - `function f<T extends Num>(x: T[n], k: size): T { ... }` (pure),
    `action f(...): R { ... }` (effects), `action main(args: string[k]):
    i32 { ... }`. A body is a sequence of `const` declarations (immutable;
    `let` is refused with a message), `if (c) { ... } else { ... }`,
    `if (c) return e;`, expression statements (actions) and `return e;`:
    lowered to the `let` chains and `if` expressions of the AST (a guard
    `if (c) return a;` followed by the rest is `if c then a else rest`).
  - Expressions: `c ? a : b`; `&&`, `||`, `!`; `==` and `===` (the same),
    `!=` and `!==`; `**` the power (right associative); `^` the bitwise
    xor (`xor` goes); `&`, `|`, `<<`, `>>`; `+` also concatenates strings
    (`++` goes); lambdas `(x) => e`, `x => e`, `(x: f64) => e`,
    `(x) => { ...; return e; }`.
  - Method syntax: `e.m(a, ...)` is `m(e, a, ...)` when `e` is not a
    record with a field `m` (`xs.map(f)`, `xs.filter(p)`, `xs.reduce(op,
    e)` in the order of JavaScript, `a.concat(b)`); `e.length` is
    `len(e)`; `xs.with(i, v)` is the update `xs with [i] = v` (as
    `Array.prototype.with`).
  - Types: `i8` ... `u64`, `f32`, `f64`, `boolean`, `string`, `void`;
    arrays `T[n]`, `T[r][c]` (r rows of c, outer size first as in C),
    `T[k <= n]`; tuples `[A, B]` and their values `[a, b]` (an array
    literal is a tuple where a tuple is expected or when its elements
    have different types), destructured by `const [a, b] = p;`; records
    `{ a: A, b: B }`; function types `(x: A) => B`; generics `<T extends
    Num>`.
  - Sum types `type Shape = Circle(f64) | Square(f64);`, `match (e) {
    Circle(r) => a, Square(c) => b, _ => c }`.
  - Modules: `import * as geom from "./geom";` (then `geom.area`) and
    `import { area, Shape } from "./geom";`; `export` accepted (everything
    is public for now). C functions: `declare function cbrt(x: f64):
    f64;`. Modifiers: decorators `@allow_reassoc`, `@require_tail`, ...
  - Kept from syntax 1: typed literals (`42u8`, `1.0f32`), conversions
    `f64(x)`, `size` parameters, `print`.
  - Later: template literals, `{...r, f: v}`, top-level `const f = (x) =>
    ...`, single-quoted strings.
- **D149 — Syntax 2 in place: what differs from the plan of D148
  (2026-11-02).** Every program of the repository (examples, prelude,
  guide, SPEC, tests: 380 programs) was converted by `iscc convert`, each
  checked to give the same AST as before; `parse` reads syntax 2 only.
  - A value is given by `return`, as in TypeScript: an action whose result
    is not `void` ends with `return` on every path (an error otherwise),
    and an expression statement is never the value of a body, even the
    last one (syntax 1 made the last line of an action its value; the
    converted prelude had `if (e == "") { Ok(m); } else { Err(e); }`).
  - Tuples: `[a, b] as const`, or `[a, b]` where a tuple is expected (a
    declared result, an annotated `const`, an argument). An array literal
    whose elements have different types is not made a tuple: the reader
    sees the difference in the text.
  - `{ ...r, f: v }` (the update of a field) is done; named imports
    (`import { f } from "./m";`) are not yet; `export` is accepted and
    changes nothing (everything is public).
  - `bool` is refused with a message (`boolean`): one spelling.
  - Messages and the memory report write types and code in syntax 2:
    `boolean`, `void`, `f64[n]`, `f64[]` (a size unknown), `[A, B]`,
    `(T) => R`, `x.slice(a, b)`, `y.with(0, …)`, `_ => ...`, `const`,
    `function`. The operators keep their text (`&&`, `||`, `^`).
  - `print` still writes a tuple `(1, 2)` and an array `[1, 2]`: an output
    format, not program text, and the two stay distinct.
  - Precedence of JavaScript: `&`, `^`, `|` below the comparisons (syntax 1
    had them above), so `x & 1 == 0` is a type error; the converter added
    the parentheses wherever the tree needed them. `-x ** 2` is refused as
    in TypeScript; comparisons do not chain.
  - The examples were then written as a TypeScript programmer would: guards
    `if (c) return a;`, `+` of strings, methods where they read better
    (`lines(t)[0].split(",").map(trim)`). `mandelbrot.isc` computes its
    first two iterations before the test of the last group instead of
    repeating them in both branches: same image, same time (2518 ms before,
    2515 ms after, N = 16000, one core at a fixed frequency).
  - The old lexer, the old parser, the printer to syntax 2 and `iscc
    convert` are removed once every program was converted: one syntax, a
    front-end of 2001 lines instead of 3981 (`lexer.rs`: tokens, numbers,
    strings, scalar types; `parser.rs`: the parser and the lowering of
    statements).
  - A block of a `match` arm or of a lambda, `{ ...; return v; }`, gives
    `v` as the value of that arm or lambda: `match` is an expression, so its
    `return` does not leave the enclosing function (in TypeScript, a
    `return` inside a `switch` would). To be confirmed by the author.
- **D150 — Modules as in TypeScript; template literals (2026-11-02).**
  - `import * as m from "spec";` and `import { f, g as h } from "spec";`.
    A spec that starts with `./` or `../` (or `/`) is a path relative to
    the importing file, `.isc` added; a bare name is looked for in
    `$ISC_PATH` only (D139 also looked next to the file: TypeScript's
    rule is simpler to read, and the error names the file next to it when
    there is one). A module is its file (canonical path): read once
    whatever the paths that name it, so that its types are the same
    everywhere; its key is the file name (`m~2` for a second file of the
    same name), which no program can write.
  - `export` makes a function, a type (with its constructors) or a
    `declare function` visible to other files; anything else is private
    (as in TypeScript; D139 made everything public). A C function stays
    global under its C name: its symbol is global anyway.
  - `m.x` is resolved when the modules are loaded, by a walk of the names
    of the tree (a qualified name has no scope); a name imported by name
    is resolved by the type checker after the local variables and the
    module's own declarations, so that `const area = 5;` hides an imported
    `area` as in TypeScript. A name both imported and declared in a file,
    a name the module does not export (the error says whether it is
    private or missing) and an alias used twice are compile errors.
  - Template literals `` `a${e}b` ``: the lexer splits the text and the
    insertions (nested templates and braces inside an insertion
    included); the parser writes `"a" + $text(e) + "b"`, where `$text` is
    `to_string` under a name no program can declare (a program may have
    its own `to_string`). `to_string` of a string is the string itself
    (as `String(s)`); an array is a compile error ("expected a scalar
    type"), not JavaScript's `1,2`, since `print` writes `[1, 2]`.
- **D151 — The names of TypeScript (2026-11-02).** The author asked whether
  `to_string` exists in TypeScript: it does not (`String(x)`,
  `x.toString()`). The standard library takes TypeScript's conventions:
  camelCase for every name of several words (`readF64`, `writeBytes`,
  `parseI64`, `stringOf`, `readText`, `parseCsvF64`, `printBytes`, the
  `try` actions `tryReadText`..., the fallible forms `...OrError`, the
  decorators `@requireFused`, `@requireNoAlloc`, `@requireTail`,
  `@allowReassoc`, and `requireUnique(...)`), and TypeScript's own names
  where the meaning is the same: `String(x)` and `x.toString()`,
  `Math.sqrt`, `Math.exp`, `Math.log`, `Math.sin`, `Math.cos`,
  `Math.pow`, `Math.min`, `Math.max`, `Math.PI`, `Math.E` (`Math.f` is
  always the namespace, as an imported module's prefix is; the functions
  keep their bare names too). A TypeScript name whose meaning differs is
  not taken: `parseFloat("1,5")` is 1 in JavaScript (a prefix), so
  `parseF64`, which refuses that text, keeps its own name. The examples,
  the guide and SPEC name their own functions in camelCase too
  (`stdDev`, `movingAverage`); the file names do not change. The earlier
  entries of this file keep the names of their time.
- **D152 — Functions returned by functions; deeper patterns; sizes read
  by a lambda given to a recursive function (2026-11-02).**
  - A function that is not recursive may return a function: its call is
    inlined as a function value (the callee's parameters bound, its
    `const`s and `if` guards evaluated as values of functions). The values
    the returned function reads are computed once into variables bound
    around the expression being lowered (`Lower::hoisted`), so that
    `map(x, adder(sum(x)))` computes `sum(x)` once, before the loop (checked
    in the IR by a test). A `size` parameter of such a function is checked
    (negative: a panic). A recursive function that returns a function is
    refused: it is not inlined, and a function is not a run-time value.
  - A function chosen by `match` may read variables bound at any depth of
    its pattern (D132 allowed one level). A part read through a node of a
    recursive type below the first level exists only when the arm matches
    (`Cons(a, Cons(b, _))` on a list of one element): it is read under the
    test of its arm, else it is the zero of its type (before, both the
    interpreter and the compiled program read a missing node).
  - A lambda given to a recursive function may read sizes of its context:
    like the values it captures (D131), they are parameters of the
    specialization (`i64`), and its key exists. A lambda that reads a
    function of its context still has no key (after v1).
- **D153 — Functions in sum types, by defunctionalization (2026-11-02).** A
  field of a constructor may be a function (`Apply((x: f64) => f64)`,
  `Option<(x: f64) => f64>`, recursive types; a structure of functions in
  one field is refused: one field per function). The value holds a `u32`
  code: the index of its function among the functions the program stores
  in that field (type, type arguments, variant, field); a call is the
  branch of an array of functions (D114) on the code. The set is known only
  once the whole program is lowered, so the lowering is repeated until a
  pass stores no new function (a pass uses the set of the previous one;
  the codes of a pass are those of the previous pass, new functions after
  them; at most 16 passes). A function chosen at run time is stored as a
  choice of codes. Such a function cannot read a variable or a size of its
  context (a compile error: closures in data are after v1). A value that
  holds a function is neither printed (the check of printable values now
  looks into the payloads of sum types: it printed the code before) nor
  compared (class Eq).
- **D154 — Arrays of arrays of strings; regular rows; `fill` of owned
  values (2026-11-02).** The rows of an array of arrays may hold strings
  (an internal class `ROW`: a scalar or a string); the block owns them:
  freed and copied one by one. A row is written into the block: a view
  (a variable, a slice, a row) copies its strings, a new array (any other
  expression: `split`, `filter`, ...) is moved and only its block freed.
  An array of arrays is regular: each row is checked against the length of
  the first, a difference is a panic `size mismatch: <row> != <first>` at
  the function of the `map`, in the interpreter too (it built ragged
  arrays the type did not describe; the compiled program refused such
  maps). Two defects found on the way: `fill(n, s)` of a string stored the
  same string in every element (freed n times: Valgrind), now each
  element is a copy; and `fill(n, toString(i))` (a new string) failed the
  memory check (never freed), now bound, copied, then freed. Ragged arrays
  and arrays of arrays of arrays are after v1.
- **D155 — The standard library of v1, first part (2026-11-02).** Each
  function means what it means in TypeScript (the ECMAScript standard),
  with one difference everywhere: a length or an index of a string counts
  bytes of UTF-8, not units of UTF-16. The code of each function is in
  `isc-runtime-core` (no `std`, written through `core::fmt::Write`), the
  same for the interpreter and the compiled program.
  - `Math.floor`, `ceil`, `trunc`, `round` (a half toward +inf, -0 kept, as
    ECMAScript), `abs` (of an integer too: `x < 0 ? -x : x`, modulo 2^bits),
    `tan`, `asin`, `acos`, `atan`, `atan2` (musl's `libm`, D108).
  - Strings: `indexOf`, `includes`, `startsWith`, `endsWith`, `replaceAll`
    (an empty pattern: around each character), `toUpperCase`,
    `toLowerCase` (Unicode, no locale), `padStart`, `padEnd` (2 or 3
    arguments; the last copy of the filling cut before a character that
    would pass the length), `repeat`, `trimStart`, `trimEnd`, `s.slice(a,
    b)` (no clamping, no negative index, a panic inside a character).
  - `x.toFixed(d)`: the exact decimal value (Rust's formatting at 1100
    decimals is exact), rounded at `d` decimals, a half away from zero
    after the sign (ECMAScript takes the larger of two equally near
    numbers); `NaN`, `Infinity`; from 1e21 on, the text of `print` (no
    exponent anywhere in Isocron). `(99.995).toFixed(2)` is `100.00`: the
    double nearest 99.995 is above it.
  - The standard input: `readStdin()`, `readStdinText()`,
    `readStdinLines()` (`lines` of the text), read once; a later read finds
    its end. The interpreter can be given its input (`rt::STDIN`, the
    tests).
  - `random(seed, i)`: SplitMix64 of a counter, in the prelude (a function
    of its arguments: parallel, the same bits everywhere; checked against
    an independent Rust version); `randomInt(seed, i, n)`. TypeScript's
    `Math.random()` has no seed and no meaning in a pure function.
  - `sort(x)`, `sortBy(x, key)`, stable, by the order of the type
    (floats: -0 before +0, NaN last), from the primitive `argsort` (the
    runtime sorts indices with `sort_unstable_by` of `core`, ties broken by
    the index: stable without allocation); `toReversed`. TypeScript's
    `sort` sorts in place by the texts of the numbers: Isocron keeps the
    names of the goal for an order that is not TypeScript's.
  - The class `Ord` holds the strings (a type parameter `T extends Ord`
    no longer requires a scalar; its comparisons are those of the instance).
  - `iota(n)` and `fill(n, v)` of any `i64` (the author asked how to write
    a simple loop: `const n = 5; iota(n)` was refused): a size known at run
    time, checked positive or zero.
- **D156 — Parsing is pure; ECMAScript's conversions; `bigint`
  (2026-11-02).**
  - `parseI64`, `parseF64`, `tryParseI64`, `tryParseF64` are functions,
    no longer actions: parsing a text has no effect (a panic is allowed in
    a function, as an index out of bounds), and `lines(t).map(parseF64)`
    must be writable (the author asked how to convert a string to a
    number; `args.map((a) => parseI64(a))` was refused).
  - `Number(s)`, `parseFloat(s)`, `parseInt(s, radix)` as ECMAScript: a
    `f64`, NaN when there is no number, never a panic (the white space of
    ECMAScript, `Infinity`, `0x`/`0o`/`0b` for `Number`, the longest prefix
    for the two others, `parseInt` exact through a big integer then rounded
    once). `parseI64` and `parseF64` stay strict (a panic, or a `Result`):
    TypeScript's names now carry TypeScript's meaning, and the strict
    forms keep theirs (D151).
  - `bigint`, as TypeScript: literals `123n` and `0xffn`, `+ - * / % **`
    (`/` toward zero, `%` of the sign of the dividend; a division by zero
    and a negative exponent are panics), comparisons, `BigInt(x)` of an
    integer or a text, `i64(b)` modulo 2^64, `f64(b)` and `Number(b)` to the
    nearest (one rounding), its text; no mixing with the other numbers (a
    type error). A literal takes the type its context requires (SPEC section 4): `0`
    may be a `bigint`, so that the generic `sum` of the prelude takes it.
  - Its representation: in the type checker `Ty::BigInt` (the classes of
    the integers), in the IR `Type::Big`, laid out, owned, copied and freed
    as a string (its bytes: `+` or `-`, then the digits of the magnitude in
    base 128, least significant first; zero is no byte, the zero of every
    value of the IR). The bytes are canonical, so equality compares bytes;
    a distinct IR type keeps `(+)` of a `reduce` an addition, not a
    concatenation. The arithmetic is `num-bigint` (`alloc`, no `std`) in
    `isc-runtime-core`, for the interpreter and the compiled program; the
    runtime gives `alloc` a global allocator over `isc_alloc`. Each
    operation decodes and encodes its operands (linear, below the cost of
    a multiplication or a division).
- **D157 — Regular expressions (2026-11-03).** A type `RegExp`, written
  `/source/flags` where an expression starts (the lexer's rule of
  JavaScript: after an operator, an opening bracket, a comma or `return`;
  else a division) or `RegExp(source, flags)`; flags `g i m s u`. As
  TypeScript: `re.test(s)`, `s.search(re)`, `s.replace(re, to)` (every match
  with `g`, else the first), `s.replaceAll(re, to)` (`g` required: a panic
  otherwise, as ECMAScript's TypeError), and the patterns `$$ $& $` $' $1`
  to `$99` in `to`; `s.replace(text, to)` replaces the first occurrence and
  a text pattern takes the `$` patterns too (ECMAScript's GetSubstitution:
  `replaceAll` of texts goes through the same engine, the pattern escaped).
  `countMatches(s, re)` and `matches(s, re)` (the texts of the matches)
  replace `(s.match(re) || []).length`, which needs `null`. The engine is
  that of the crate `regex` (`regex-automata`, `alloc`, no `std`), in
  `isc-runtime-core`: leftmost first as ECMAScript, linear time, so a
  backreference or a look-around is refused with its reason; `\d`, `\w`,
  `\s` are those of Unicode (ECMAScript's are ASCII but `\s`). A literal is
  compiled when the program is (an invalid one is a compile error); a
  `RegExp` value is the text `flags/source`, compiled again at each use
  (a cache is later work). Not printed, not compared.
- **D158 — `Map<K, V>`, `Option`, `??` (2026-11-03).** A dictionary that is
  a value, never changed in place, as the rest of the language: `Map(entries)`
  (also `new Map(entries)`; `new` makes nothing else) keeps each key at its
  first entry with its last value, the result of TypeScript's `set` entry
  after entry; `countBy(xs, f)` counts by key (the name of lodash: TypeScript
  has no counting, and its `Map.groupBy` gives rows of different lengths).
  `m.size`, `has`, `get` (an `Option<V>`), `keys`, `values`, `entries` (arrays
  in the order of first insertion, as the iterators), `set` and `delete` (a
  new map). `type Option<T> = None | Some(T)` joins the prelude (a program
  that declares one of these names replaces it, as `Result`); `a ?? b` is
  `match (a) { Some(x) => x, None => b }`, right-associative below `||`,
  TypeScript's `??` with `None` for `undefined`. Layout: three arrays, the
  keys by first insertion, the values, and the ranks of the keys sorted
  (`argsort`), searched by bisection: built in O(n log n) with no hashing
  (the same order and bits everywhere, and parallel sorting later),
  `get` in O(log n); `set` rebuilds in O(n log n), so a map is made from
  all its entries at once (a hash table changed in place needs uniqueness
  of the map, later work). In the checker it is a sum type of the prelude
  with one constructor `$Map` that no program can write (a name of the
  prelude may start with `$`); its functions are written in Isocron over
  two primitives, `$mapOf(keys, values, order)` and `$mapParts(m)`, and the
  lowering lays it out as the tuple of the three arrays. A key has the new
  class `Key` (integers, booleans, strings, `bigint`): a float is refused at
  compile time, NaN not being equal to itself (the classes are now 16
  bits). A map is not printed, compared, stored in an array or a
  constructor, nor the value of a type parameter (it is three arrays). The
  call `x.m(...)` now names the function `m` even if a local `m` that is
  not a function exists (`const keys = ...; m.keys()`), as in TypeScript.
  Found on the way: reading a scalar part of a tuple after its parts that
  own memory were taken was refused by the memory check (D138); allowed.
- **D159 — Type aliases and constants of the top level (2026-11-03).** Two
  declarations of TypeScript that the programs of the Benchmarks Game
  wanted. `type X<T> = <type>;`, where the type starts with `{`, `[`, `(`,
  a built-in type, or a name followed by `<` or `[`, is an alias (else the
  declaration is a sum type, as before: `type A = B;` is a sum with one
  constructor). An alias is the type itself (TypeScript's structural
  typing): it is replaced by its type everywhere before the checking, its
  own names first resolved in its module (so an exported alias works
  through `import`), and a size variable in it becomes one of the
  signature that uses it; an alias that contains itself is an error
  (a recursive type is a sum). `const X = v;` at the top level is a
  function without parameters, called where `X` is read (inlined like any
  non-recursive function; a local of the same name hides it). Its type is
  written, or is that of its literal (`i64` and `f64` for TypeScript's
  `number`, `string`, `boolean`, `bigint`, or an array of these): no
  inference across declarations, which keeps the signatures independent of
  the bodies. A constant that depends on itself is refused (it would never
  end). The visitor of names now meets whole types too (`Visit::Type`).
- **D160 — The ten programs of the Benchmarks Game (2026-11-03).**
  `examples/benchmarks_game/` holds binarytrees, fannkuchredux, fasta,
  knucleotide, mandelbrot, nbody, pidigits, regexredux, revcomp and
  spectralnorm, written as a TypeScript programmer would, in the
  functional style of the language, each in parallel where the problem
  allows: fasta reads the t-th draw of its generator from the table of its
  full period (IM values), so every byte is computed independently;
  fannkuch-redux computes the permutation of each rank (4-bit digits of a
  `u64`); k-nucleotide counts k-mers coded 2 bits per base with `countBy`;
  regexredux is the TypeScript program of the reference nearly word for
  word; revcomp and fasta compute every byte of their output by its
  position. `bench/benchmarks_game/references.txt` gives the argument and
  the input of the reference output of each (`fetch.sh` extracts them);
  `check_programs.sh` compares, in the Docker image and in both modes, the
  output of each Isocron program to the reference byte for byte: all ten
  are the same. `tests/benchmarks_game.rs` checks them in `cargo test`
  against implementations written in the test (their reference outputs
  stay outside the repository), interpreted, compiled and under Valgrind,
  the three readers of FASTA on the output of our fasta. Their measurement
  and the programs of the other languages for nine of them are later work.
- **D161 — The questions of SPEC section 16 before v1, decided
  (2026-11-03).** (1) The uniqueness algorithm is written step by step in
  SPEC section 8: regions (the branches of an `if`, the bodies of
  lambdas), the order of evaluation, owned variables and aliases, the last
  use on each path, the positions that take a value, move or copy (with
  the reason in the memory report), `requireUnique`, freeing, and the
  independent check (D96). It is the algorithm of the implementation
  (`passes.rs`, `ownership`), which makes `requireUnique` portable: any
  conforming compiler gives the same verdicts. (2) The values passed to
  `extern` functions (D125) are final for v1. (3) Part 3 is final, with
  what v1 added (D150 to D159), written in the grammar of section 3 (now
  that of the parser: type aliases, constants, `??`, `new`, the literals of
  `bigint` and of regular expressions, the class `Key`) and in the types
  and the library of sections 4 and 6. (4) The points of section 16 marked
  v1 are confirmed: functions in structures (D148, D153), mutual tail
  recursion (D113), the fixed form of a parallel scan of floats (D112),
  `scatter` and its rule of collisions (D111); SPEC sections 6, 10 and 11
  say so. The questions left in section 16 are those of after v1.
- **D162 — Panics certain at compile time (2026-11-03).** Part (6) of v1
  asks for the errors found at compile time whenever possible: an
  operation whose literal operands make it panic is now a compile error
  with the message of the panic, at its position, as Rust's
  `unconditional_panic`: a division or remainder of integers by the
  literal 0 (whatever the dividend), a literal index outside an array
  literal, a negative literal exponent of an integer or a `bigint`, a
  negative literal size of `iota` or `fill`, a negative literal count of
  `repeat`, a literal number of digits of `toFixed` outside 0 to 100, and
  `replaceAll` of a `RegExp` whose flags are written without `g` (an
  invalid literal regular expression already was, D157). The check is a
  walk of the typed tree of the program's functions (not of the prelude);
  values known only at run time keep their panic. The tests of these
  panics at run time now use values read at run time.
- **D163 — No name of a machine in the repository (2026-11-03).** The
  scripts of measurement wrote the host name of the machine into every
  report (`socket.gethostname()`), and the text named the server and the
  path of the project on it. The history was rewritten (the reports
  `results/xeon*`, the machine called `xeon`); the reports now name the
  machine by `$ISC_BENCH_MACHINE`, else by its CPU (`run.machine_label`).
- **D164 — Components of a tuple accumulator owned; fields evaluated in
  the order of the program (2026-11-03).** Goal of performance (TODO,
  "Performance"): every optimization starts from a profile and a model of
  cost. n-body made 42 allocations and 36 copies of arrays of 5 numbers at
  each step (callgrind: `calls=`), where its step needs none; the memory
  report gave the reasons. (1) The accumulator of a `fold`, a tuple taken
  apart at the top of the body (`const [vx, vy, vz] = v;`) and not read
  otherwise, now gives each component its own value, as a new tuple did
  (D138, `split_tuples`): each `with` updates in place instead of copying
  ("`v` is used again"); the result of a `fold` is a new tuple too. (2) A
  record is a tuple of its fields in the order of their names, and they
  were evaluated in that order: in `{ x: b.x + DT * vx, vx: vx }` the field
  `vx` took `vx` before `x` read it, hence a copy. The fields are now
  evaluated in the order of the program (each bound to a local first when
  the orders differ), as in TypeScript. n-body, 50 000 000 steps, one core
  at a fixed frequency: 27.619 s before, 9.929 s after; 3 copies and 3
  allocations remain per step (the parameter of the recursive `steps` is
  borrowed, D102). Test: `tests/report.rs`.
- **D165 — The common case of the allocator is a leaf (2026-11-03).** The
  model of cost of an allocation served by the cache of the thread is a
  few loads and stores; callgrind counted 63 instructions per allocation
  and 43 per free in n-body: the function that held the fast path also
  held every slow one, so each call saved and restored six registers, then
  read the mode of the allocator and computed the class through the
  general path. `isc_alloc` now takes a block of at most 1 KiB from the
  cache of the thread itself (`heap::alloc_cached`), and `isc_free` gives
  one back (`heap::free_cached`); anything else, a block in an unexpected
  state included, is one tail call to the complete path (cold), which keeps
  every check. 25 instructions per allocation and 28 per free, no saved
  register. n-body, 50 000 000 steps, one core at a fixed frequency: 9.930 s
  before, 8.351 s after; binary-trees at depth 18: 2.571 s, 2.377 s (its
  cost is elsewhere: the refills of the cache, the header of 64 bytes of
  each node, D166).
- **D166 — An allocator of nodes (2026-11-03).** binary-trees spent its
  time in the allocator (perf: `heap::refill` 29 %, `alloc_bytes` and
  `isc_free` 10 % each, `heap::spill` 6 %): a node of `Tree` holds 32 bytes
  (its count, its tag, two pointers) but took a block of 64 bytes and a
  header of 64, and building or freeing millions of them emptied and filled
  the cache of the thread through the global lists, under their lock,
  which 4 threads then fought for (the program was slower on 4 threads than
  on one). The size of a node is known where the compiled code makes it
  (`Box`) and where it frees it (`isc_rec_drop`), the only two places:
  `isc_node_alloc(bytes)` and `isc_node_free(p, bytes)` now take cells of
  that size (a multiple of 16 bytes, up to 256) from a free list of the
  thread, carved from regions of 256 KiB, without a header; a cell freed
  by another thread joins the list of that thread; the regions are kept
  (their memory is at most the largest number of nodes alive at once).
  Larger nodes, WebAssembly and `ISC_ALLOC=glibc` use the general
  allocator, so that Valgrind still sees every node in the tests. The
  check of a double free by the header is lost for nodes: their frees are
  made by the compiler and checked by the verifier of the IR (D96).
  binary-trees, fixed frequency: depth 18 on one core 2.406 s before,
  1.082 s after; depth 21 on 4 threads (`machine.sh apply parallel`)
  43.338 s before, 4.326 s after.
- **D167 — `Map` and `countBy` by a table of hashes (2026-11-03).**
  k-nucleotide spent 78 % of its time sorting (perf: `isc_argsort`):
  `countBy` and `Map(entries)` sorted all their n keys to group the equal
  ones (D158), O(n log n), where the keys counted are 4 or 16 distinct
  k-mers among 125 million. The model of cost of a grouping is one pass
  over the keys with a probe of a small table in the cache. `$group(keys)`
  (a primitive of the prelude, shared by the interpreter and the runtime,
  `isc_runtime_core::group`) gives the distinct keys in the order of their
  first occurrence with their first and last indices and their counts, by
  a table of hashes with linear probing; only the m distinct keys are then
  sorted, for the search by bisection. The result depends on the equality
  of the keys only, not on the hash (integers by value, strings and
  `bigint` by their bytes, canonical). O(n + m log m) instead of
  O(n log n). k-nucleotide on 25 000 000 bases, 4 threads, fixed frequency
  (`machine.sh apply parallel`): 36.935 s before, 3.384 s after. Test:
  `tests/stdlib.rs` (`map_groups_by_hashes`).
- **D168 — `bigint` stored in bytes in the compiled programs (2026-11-03).**
  pidigits spent 78 % of its instructions converting (callgrind): each
  operation decoded its operands from digits of 7 bits (D156) into the
  words of num-bigint and encoded its result back, on numbers of thousands
  of digits, for operations (a product by a small integer) far cheaper
  than that. The digits of 7 bits were valid UTF-8 for the interpreter,
  whose `bigint` is a string, and the IR, whose literals are strings: both
  keep them (`isc_runtime_core::big`). The compiled programs store the
  magnitude in bytes, least significant first (`big256`, the same code by a
  macro, also canonical, so that equality, hashing and order are those of
  the bytes and of the values as before); a literal is converted when the
  code is generated (`big_to_256`). num-bigint's own conversions of bytes
  take them one by one (still 83 % of the instructions): the bytes go to
  and from its words by words of 32 and 64 bits instead. An experiment in
  assembly on the conversion of 7 bits (pext and pdep of BMI2, outside the
  repository) gave 1.487 s on 4000 digits; this, without assembly, 1.046 s
  (4.237 s before). pidigits 10 000 digits, one core at a fixed frequency:
  42.649 s before, 11.574 s after. The conversions are still 73 % of the
  instructions (num-bigint takes words of 32 bits only): the next step is
  the common operations done on the bytes themselves (TODO).
- **D169 — A constructor without payload of a recursive type is not
  allocated (2026-11-03).** After D166, binary-trees spent a third of its
  time allocating, freeing and dropping its nodes, and half of them are
  `Leaf`s, which hold nothing. A value of a recursive type whose
  constructor has no payload (the tuple of its tag and zeros) is now the
  odd value 2 * tag + 1 instead of a node (a node is aligned on 16 bytes):
  its tuple is read back as its tag and zeros (`node_tuple`), and the
  increment and the drop of its count, its equality and its printing skip
  it (`is_node`). The point of TODO "a constructor without payload (`Nil`)
  without an allocation" is done this way. binary-trees, fixed frequency:
  depth 18 on one core 1.077 s before, 0.679 s after; depth 21 on 4
  threads 4.323 s, 2.542 s. Test: `tests/stdlib.rs`
  (`constructors_without_payload_are_not_allocated`).
- **D170 — fannkuch-redux by chunks, without divisions (2026-11-03).** Its
  profile was the generated code alone (no allocation): the model of cost
  gave two divisions of 64 bits per level to rebuild each permutation from
  its rank (22 for N = 12, tens of cycles each), `factorial(n - 1)`
  recomputed for each permutation inside the lambda, and two passes over
  the 479 001 600 permutations (the checksum, then the maximum). The
  program (no change of the compiler) now cuts the permutations into
  chunks of 40 320 computed in parallel: the first permutation of a chunk
  and its counters come from its rank, the next ones by the step of the
  reference programs (a rotation, a counter of the factorial base
  incremented with its carry), the checksum and the maximum in one `fold`
  over a tuple of scalars, combined by one `reduce`; a flip reverses the
  first digits of the `u64` without a loop (its 16 digits reversed by
  shifts and masks, which LLVM makes a `bswap`, then shifted). N = 12 on
  4 threads, fixed frequency: 41.728 s before, 4.305 s after.
- **D171 — The common operations of `bigint` on its bytes (2026-11-03).**
  After D168 the conversions to and from num-bigint were still 73 % of the
  instructions of pidigits (num-bigint takes words of 32 bits only, its
  constructor of words of 64 bits is private). The compiled programs now
  add, subtract, multiply (by one word in one pass; schoolbook up to 48
  words; num-bigint's Karatsuba beyond), divide (by one word in 128 bits,
  else Knuth's algorithm D, The Art of Computer Programming 4.3.1),
  negate and compare their values on their bytes themselves, read by words
  of 64 bits, the whole words without a test of bounds
  (`isc_runtime_core::limbs`); `/` truncates toward zero and `%` has the
  sign of the dividend, as before; num-bigint keeps the power, the text,
  the parsing and the large products. Checked against num-bigint: every
  operation on values of every size and sign (`big_forms`), and 20 000
  divisions chosen to reach the corrections of the quotient and the step
  that adds back (`big_divisions`). pidigits 10 000 digits, one core at a
  fixed frequency: 11.574 s before, 2.503 s after (42.649 s before D168).
- **D172 — An array of at most one block is filled without a task
  (2026-11-03).** n-body made three calls of `isc_parallel_for` at each
  step, each through an outlined task (callgrind: about 110 instructions
  for the call and 80 per task, to compute 5 numbers): a `map` or an
  element-wise operation outside a task always went through the pool,
  even for one block of B elements, which one thread takes anyway. When
  its length is at most B (a test at run time), the array is now filled in
  place, as inside a task; beyond, nothing changes. The elements do not
  depend on the cut (D121). n-body, 50 000 000 steps, one core at a fixed
  frequency: 8.135 s before, 6.915 s after.
- **D173 — An array literal of scalar constants is data of the program
  (2026-11-03).** Three of the nine allocations of each step of n-body were
  tables of constants (`masses()`, the indices of the pairs): an array
  literal was built and freed at each evaluation, and so was every use of a
  constant of the top level holding one (D159), a common form of
  TypeScript programs. A literal whose elements are all scalar constants is
  now `ConstArray` in the IR, a global constant of the compiled program
  aligned as an array: borrowed from no variable (the ownership pass and
  the verifier give a copy to any consumer: `with`, the initial value of a
  `fold`), never freed. As this changes a verdict of `@requireNoAlloc` (it
  now accepts such a literal), SPEC section 8 states it. n-body, 50 000 000
  steps, one core at a fixed frequency: 6.915 s before, 6.374 s after.
  Tests: `tests/stdlib.rs` (`arrays_of_constants`), `tests/phase4.rs`.
- **D174 — Regular expressions: the bounds of the matches, ECMAScript's
  empty matches, the searches shared out (2026-11-03).** regex-redux:
  removing the 830 000 newlines of its input cost about 0.56 µs per match,
  each found by `search_captures`, which resolves the groups with a slower
  engine, while most replacements name no group. A replacement whose text
  has no `$1` to `$99` now takes the bounds of the matches from the fast
  engines (`find`), built in one buffer. The comparison of the two paths
  (a test of the runtime) showed that the iterator of the crate skips an
  empty match right after a match, where ECMAScript counts it
  (`"aaa".replace(/a*/g, "-")` is `--`): one walk of the matches by
  ECMAScript's rule now serves `replace`, `countMatches` and `matches`,
  which also fixes `countMatches` and `matches` (`"aaa".match(/a*/g)` has
  two matches). A search or a replacement inside a `map` is a loop over its
  text: the `map` is shared out among the threads (the 9 counts of
  regex-redux: 763 ms on one thread, 249 ms on 4). regex-redux on 5 000 000
  bases, 4 threads at a fixed frequency: 3.748 s before, 3.042 s after.
  Open: 44 % of its instructions are the prefilter of literals of the
  crate (Aho-Corasick), which for the 256 cases of a pattern with `i` on
  DNA finds many false candidates (the 9 counts take 592 ms without it,
  1398 ms with it), while it makes a rare word 5 to 10 times faster to
  find; choosing by the data without timing is later work.
- **D175 — Atomic counts only for the nodes a task may share (2026-11-03).**
  In a task every increment and drop of a count was atomic, since another
  thread might hold the node (SPEC section 8: atomic when the value leaves
  its thread). A node can reach another thread only through what the task
  captures or returns: the recursive types reached from the types of its
  captured variables and of its result, through the layouts of the nodes,
  are shared; a type none of whose reached types is shared is local, and
  its counts are plain (inside an atomic drop, everything stays atomic, so
  that a local node holding a shared one never drops it plainly).
  binary-trees builds and drops its trees inside the tasks: no atomic
  operation is left. binary-trees 21 on 4 threads at a fixed frequency:
  2.742 s before, 2.385 s after. Test: `tests/memory.rs`
  (`atomic_counts_only_for_shared_nodes`: none for a tree of the task,
  some for a captured one, the same results on 4 threads).
- **D176 — The free list of nodes in the compiled code (2026-11-03).**
  After D175, `isc_node_alloc` and `isc_node_free` were still about a third
  of the instructions of binary-trees, much of it the calls themselves.
  The size of a node is known where it is made and freed: the compiled
  code now takes a cell from, and gives it back to, the free list of the
  thread itself (`isc_node_thread`, a thread-local variable of the runtime,
  local-exec) when the allocator of Isocron is in use (`isc_heap_mode`, now
  exported by the runtime) and the node takes at most 256 bytes; the
  runtime keeps the rest (an empty list, larger nodes, `ISC_ALLOC=glibc`,
  WebAssembly), so that Valgrind still sees every node in the tests.
  binary-trees, fixed frequency: depth 18 on one core 0.609 s before,
  0.509 s after; depth 21 on 4 threads 2.385 s, 2.082 s.
- **D177 — The ten programs measured against the fastest in Docker
  (2026-11-03).** `programs.txt` now lists, for the nine benchmarks other
  than mandelbrot, the fastest program in C, C++, Rust and Go on the site
  of the Benchmarks Game, with the options of their command lines (their
  libraries: APR, Boost, GMP, PCRE2, khash of klib); the image has these
  libraries and the dependencies of the Rust programs (one Cargo project,
  at the versions that build with Rust 1.84.1, the one of the Benchmarks
  Game, by the MSRV-aware resolver of Cargo; `pcre2-sys` pinned to 0.2.9,
  whose next version Cargo 1.84 cannot read). Go #5 and #4 of regex-redux
  bind PCRE1, gone from Debian trixie: Go #3 is built. Every program is
  checked against the reference output of its benchmark
  (`check_programs.sh`); Go #4 of pidigits writes no last line shorter than
  10 digits and is checked at N = 10000 against Isocron. `run.sh measure`
  runs `measure.sh` in the image, in `isc-bench.slice`, on 4 physical
  cores with 4 threads, after `bench/machine.sh apply parallel`; the
  results are in `results/benchmarks_game.md`, without the name of the
  machine. They correct a comparison made earlier with the times of the
  site, measured on an older machine: on the same machine, Isocron is the
  fastest on mandelbrot only; fasta, reverse-complement and spectral-norm,
  thought at the level of the fastest, are 1.54, 7.47 and 2.61 times
  slower.
- **D178 — `filter` by blocks, in parallel (2026-11-03).** Profile of
  reverse-complement (4 threads, 254 MB of input): 0.875 CPUs used, the
  `filter`s sequential, two passes each (count, then write) over the
  254 MB of the input (the headers) and the 250 MB of the bases, 12
  instructions per byte. Model: each pass reads its input once; on 4
  cores, a quarter of the time. A `filter` of more than one block, outside
  a task, with more than one thread: the elements each block of B keeps,
  counted (a task per block), the place of each block in the result (the
  sum of the counts before it, in order, by the caller), then each block
  writes its elements from its place (a task per block). The predicate is
  pure and evaluated twice, as in the sequential form; the result is the
  sequential one for any number of threads; a node kept gets one owner
  more in its task, atomic when its type may be shared (D175).
  reverse-complement at fixed frequency, 4 threads: 0.531 s -> 0.415 s;
  one thread: 1.014 s -> 1.018 s (the same two passes). Test
  `filters_and_divisions_of_the_index_do_not_depend_on_threads`.
- **D179 — The standard input read in a buffer of its size when it is a
  file (2026-11-03).** After D178, `readStdin` alone takes 0.104 s for
  the 254 MB of reverse-complement, `cat` 0.026 s. Model: the buffer
  started at 64 KiB and doubled, so that the bytes were copied once more
  (254 MB) and 512 MB of new pages were zeroed by the kernel, against 254
  MB read once into 254 MB of new pages. When the standard input is a
  regular file (`< file`, as the Benchmarks Game runs them), `lseek` gives
  the bytes left after its offset (put back after), the size of the
  buffer, as `readFile` already did; a pipe keeps the doubling buffer. The
  loop is one function (`read_fd`) for both, with its probe for end of
  file before growing. `readStdin` 0.104 s -> 0.056 s (about the copy of
  `cat` and the zeroing of 254 MB); reverse-complement 0.414 s -> 0.365 s
  on 4 threads. Test `standard_input_from_a_file` (offsets 0, 1000 and the
  end of the file).
- **D180 — The quotient and the remainder of the index of a loop by a
  constant, carried by the loop (2026-11-03).** After D179, half of the
  time of reverse-complement is its map that puts the bases in lines of
  60: 32 instructions per byte, 12 of them for `q / 61` and `q % 61` (a
  multiplication by the inverse, its corrections, a multiplication back),
  computed again at every element though `q` moves by one. Model: from
  one index to the next, the remainder grows by one and goes back to 0 at
  the divisor, which adds one to the quotient: an addition, a comparison
  and two selections. In a loop over a `map`, one element at a time
  (`fill_range`), each `x / c` and `x % c` of its body, `x` the index (a
  parameter bound to an `iota`, never negative) and `c` a positive integer
  literal, is the slot of the loop: the quotient and the remainder of the
  first index computed once before the loop, moved on after each
  element. The same values (truncated division of a non-negative number
  by a positive one), no panic possible. reverse-complement at fixed
  frequency: 0.369 s -> 0.356 s on 4 threads, 0.975 s -> 0.930 s on one
  (28 instructions per byte left: the two bounds checks and the branch of
  the line ends). The slots are emptied with the environment when another
  function or a task is emitted. Test
  `filters_and_divisions_of_the_index_do_not_depend_on_threads` (the
  remainder by 61 carried, the results of the interpreter for 1 to 8
  threads).
- **D181 — Indexing in vector code (2026-11-04).** spectral-norm 5500
  against the floor of the machine: 40 products of a 5500 x 5500 matrix by
  a vector, 1.21e9 divisions of `f64`, 0.626 ns each per core for
  independent divisions of 4 lanes (`bench/floor`, D182): 0.189 s on 4
  cores; Isocron took 0.527 s (2.8 times the floor). Profile: 25.5e9
  instructions, 21 per element, the hot loop scalar (`vdivsd`, its
  accumulator through memory): `vec_ok` refused every lambda reading an
  array by an index (`u[j]`), so the sum ran one element at a time. An
  index of an array of the context (a variable, elements of integers or
  floats) by an index that cannot panic is now vector code: every lane
  checked at once (one comparison, one test of the mask), the first lane
  out of bounds giving the panic of the scalar loop; a vector of
  consecutive indices (the index of the loop, recorded by lane in
  `Func::contig`) is one vector load, any other a gather. So that the
  first lane out of bounds is the element at which the scalar loop
  panics, a body with an index has no other site that can panic
  (`index_vec_ok`); with several threads another block may still fail
  first, as before (SPEC section 5: which panic is not specified). The
  window cascade (D115) keeps its scalar form for such bodies. spectral-norm
  at fixed frequency: 0.527 s -> 0.195 s on 4 threads (1.03 times the
  floor), 2.125 s -> 0.778 s on one; the nine other programs unchanged
  (within 1 %). Tests `vector_index.rs` (loads, gathers, `u8` and signed
  indices, panics against the interpreter).
- **D182 — The floors of the machine (2026-11-04).** The goal of
  2026-11-03 asks that each optimization start from a model of cost on
  this machine; the work had been ranked by the ratio to the fastest
  program of another language, which hides the work a program does not
  need when the others do it too. `bench/floor/floor.c` measures, without
  Isocron, the costs of the resources (bandwidths of reading and copying,
  writing into new pages, the divider and the square root by throughput
  and latency, fused multiply-adds, the latency of an addition and of a
  load from L1, random increments in tables of 256 KiB, 2 MiB and 64 MiB,
  the nodes of a tree in an arena, a big integer times a word with `mulx`
  and `adc`); `bench/floor/counts.c` counts the work of the programs that
  their sizes do not give (3.76e9 flips of fannkuch-redux 12, 6.25e9
  iterations of mandelbrot 16000, the k-mers of k-nucleotide: only 138 127
  and 139 882 distinct keys of 12 and 18, the generator of fasta having a
  period of 139 968); `bench/floor/floors.py` multiplies them
  (`results/floors.md`). Two first models were wrong and corrected before
  use: fused multiply-adds measured through memory (2.9 lanes per cycle,
  8 in registers), and the passes of pidigits counted unfused with a
  multiplication by a word in plain C (a floor above the time of the C
  program; `mulx` and `adc`, one pass per operation: 0.669 s, the C program
  1.1 times it). Isocron over its floor: fasta 57.8, regex-redux 10.1,
  k-nucleotide 8.1, reverse-complement 5.4, n-body 4.9, pidigits 3.7,
  binary-trees 3.0, fannkuch-redux 2.9, mandelbrot 1.5, spectral-norm 1.0;
  the work goes in that order.
- **D183 — The reduction of an exact operator over fewer elements than
  lanes, in one loop (2026-11-04).** fasta was 57.8 times its floor
  (D182): 34.2e9 instructions for 254 MB, 135 per byte. Profile: each
  byte calls `count(cp, (c) => c <= r)` over 4 or 15 probabilities, and a
  `reduce` always took the fixed form of section 10: a block of B, its
  results on the stack (`stacksave`), 32 lanes set to the neutral value,
  read under a mask, then their tree, about a hundred instructions for 4
  comparisons. An exact operator (integer `+`, `*`, `min`, `max`, `min`
  and `max` of floats, `exact_op`) gives the same value in any order (the
  bits of a NaN are not observable, SPEC section 10), so that the fixed
  form is not needed for the bits: with fewer elements than lanes, one
  loop from `z`. Not beyond: in one loop over the whole range LLVM kept a
  check of bounds that it removes within a block (the k-mers of
  k-nucleotide, 7 instructions per step of the inner `fold` instead of 5,
  12 % slower when this rule also took larger arrays). fasta at fixed
  frequency: 1.135 s -> 0.391 s on 4 threads, 4.493 s -> 1.517 s on one;
  the other programs unchanged. Test
  `exact_reductions_in_one_loop_give_the_results_of_the_interpreter`
  (wrapping sums and products, `min` and `max` with NaN and -0, counts in
  the tasks of a map).
- **D184 — A size of every lane and shifted indices in vector code
  (2026-11-04).** k-nucleotide was 8.1 times its floor; half of its time
  is `occurrences`: `count(iota(n - k + 1), (i) => kmer(codes, i, k) ==
  target)`, `kmer` a `fold` over `iota(k)` reading `codes[i + j]`, one
  element at a time (a chain of a shift and an addition per base). `vec_ok`
  refused the check that `k` is not negative (`CheckNonNegative` of the
  `iota`), though `k` is the same in every lane: it is now vector code when
  its size is not varying (one scalar check, failing for every lane at
  once, so not a site of `index_vec_ok`). And a vector of consecutive
  indices plus a value of every lane (`i + j`, `j` the step of the `fold`)
  is a vector of consecutive indices (`Func::contig`): one vector load,
  not a gather. The 5 queries count their k-mers 32 lanes at a time.
  k-nucleotide at fixed frequency, 4 threads: 3.199 s -> 2.098 s, the same
  output; the other programs unchanged. Test
  `folds_over_a_size_of_every_lane_and_shifted_indices_are_vector_code`
  (no gather, the results and the panics of the interpreter).
- **D183, corrected (2026-11-04).** The one loop of D183 starts from `z`
  once, the fixed form once per lane: they agree only when `z` is the
  identity of the operator (`0` for `+`, `1` for `*`) or when the operator
  is idempotent (`min`, `max`). The full suite found it:
  `reduce(iota(3), (+), 1)` of the guide gave 4, not 35 (`z` added in each
  of the 32 lanes, docs/language.md). The rule is now `order_free` (an
  exact operator, and `z` its identity or the operator `min` or `max`);
  fasta keeps its gain (its `count` starts from 0). The test adds the
  example of the guide, a product from 2 and `min`, `max` from any `z`.
- **D185 — Branches under a mask and checks replayed in vector code;
  bytes permuted, indices downwards (2026-11-04).** fasta was still 19.5
  times its floor: its `map` of lines stayed one element at a time, since
  vector code allowed an `if` only with branches that cannot panic, and
  `base(i)` indexes arrays. Now, in the vector loops of `fill_range` and
  `reduce_block`:
  * a check that fails (an index out of bounds, a size) jumps to a replay
    of the vector (`elem_replayed`): its elements computed again one at a
    time, in order, so that the panic is the one of the scalar loop, its
    element and its site, whatever the number of sites (D181 allowed one);
  * the branches of an `if` are computed under the mask of their lanes:
    an index checked, loaded (`llvm.masked.load`) or gathered only for
    them, a check of a value of every lane (`uniform_check`) failing only
    when a lane is active, the load of a value of every lane only when it
    is in bounds; the sites of a branch must be indexings (`maskable`);
  * `len` of an array of the context, two sizes of every lane compared
    (the inlined signature of `pick`) and the `reduce` of an array of the
    context with an operator whose order is free (`count` over the
    probabilities: a scalar loop, the accumulator a vector) are vector
    code;
  * elements of fewer than 32 bits have no gather instruction on x86
    (LLVM emulates one load per lane, slower than the scalar loop:
    reverse-complement went from 0.356 s to 0.422 s): they are vector code
    only when their index is consecutive (the index of the loop, a
    parameter bound to an `iota`, plus or minus values of every lane: one
    load; downwards, `m - 1 - i`: one load and its lanes reversed), or,
    with AVX-512 VBMI and in `fill_range`, when the array is a table of at
    most 256 bytes: its 8 parts of 32 bytes in registers (masked loads,
    nothing read past its end), 4 `vpermi2b` of 64 entries and the choice
    by bits 6 and 7. The loop tests the size of each table first and runs
    all its elements one at a time when one is larger (the same values:
    the 250 MB of bases of reverse-complement).
  At fixed frequency, 4 threads: fasta 0.391 s -> 0.327 s (one thread
  1.519 s -> 1.260 s), reverse-complement 0.359 s -> 0.348 s, the
  other programs unchanged (their binaries identical, or within 1 %).
  The remaining cost of the vector code of fasta: the divisions by
  constants of its 64-bit indices, one lane at a time (x86 has no 64-bit
  vector `mulhi`). Tests `vector_index.rs`: masked branches with indices
  past the end in the lanes not taken, a size checked in a branch no lane
  takes, two sites failing in one vector (the order of the scalar loop),
  tables of 4, 256 and 300 bytes.
- **D186 — The quotient and the remainder of the index by a constant,
  carried by the vector loop (2026-11-04).** After D185 the vector code of
  fasta divides its 64-bit indices by constants (`q / 61`, `q % 61`) one
  lane at a time: x86 has no 64-bit vector `mulhi`, LLVM extracts each
  lane, multiplies it by the inverse and inserts it back, about ten
  instructions per lane. As D180 does for the scalar loop, the vector loop
  of `fill_range` computes the quotients and remainders of its first `w`
  indices once (the divisions of one vector), then each step adds `w % c`
  to the remainders and `w / c` to the quotients, and where a remainder
  reaches `c` (at most once: it was below `c`) subtracts `c` and adds one
  to its quotient: a comparison, two additions, a subtraction and two
  selections per vector. fasta at fixed frequency: 0.326 s -> 0.255 s on 4
  threads, 1.267 s -> 0.985 s on one; the other binaries are those of D185
  but reverse-complement's (its vector loop, not taken: its table of
  bases is larger than 256 bytes). Test
  `divisions_of_the_index_are_carried_by_the_vector_loop` (divisors 61, 7,
  16 and 1, from the starts of the blocks).
- **D187 — Bytes close together read through a window (2026-11-04).**
  reverse-complement was 5.2 times its floor: its `map` of lines reads
  `bases[m - 1 - (q / 61 * 60 + q % 61)]`, consecutive downwards but for
  the end of each line, in 250 MB: a gather of bytes, emulated one lane at
  a time, so that D185 ran its loop one element at a time. A vector of 16
  such indices spans at most 17 bytes. An index of an array of bytes
  computed from the index of the loop alone (no other array read in it)
  now reads, when the array has more than 256 bytes, the window of its
  lanes: their lowest and highest indices (two reductions), and when they
  are less than 64 apart, two masked loads of 32 bytes from the lowest
  (nothing past the end) and one `vpermi2b` of the indices relative to
  it; a gather otherwise. Only the arrays indexed by another array's
  values (`table[bases[...]]`) still decide whether the loop is vector
  code (at most 256 bytes, D185). reverse-complement at fixed frequency:
  0.350 s -> 0.291 s on 4 threads, 0.894 s -> 0.668 s on one; fasta and
  k-nucleotide unchanged (their outputs the same), the other binaries
  identical. Test `bytes_close_together_are_read_through_a_window` (lines
  of 60 downwards, indices 80 apart, a window at the end of the array).
- **D188 — A runtime for x86-64-v3 (2026-11-04).** regex-redux was 10.1
  times its floor. Profile (5 MB, callgrind): 43 % of the instructions in
  the automaton of Aho-Corasick, the prefilter of the 9 counts, about 33
  instructions per byte and pass, and `memchr` in SSE2. The runtime is
  built without the standard library (`no_std`, embedded in `iscc`), for
  the x86-64 baseline: `aho-corasick` and `memchr` choose their SIMD code
  by `cfg(target_feature)` when they cannot detect it at run time, so that
  Teddy (the packed search of literals) never existed in it. `build.rs`
  now also builds the runtime with `-Ctarget-cpu=x86-64-v3`, linked for
  `native`, `avx2` and `avx512` when the host has AVX2, BMI2 and FMA
  (`runtime_v3`); `sse` and `avx` (the Celeron of the POC, the i5-3330 of
  the Benchmarks Game) keep the baseline. The results do not depend on it
  (Rust does not contract floating-point operations; the same code, other
  instructions); Valgrind decodes it (the runs under it are `avx2`). The
  prefilter of the counts is now Teddy in AVX2 (`FatAVX2<4>`: the flag `i`
  of the program makes 2^8 cases of each literal). regex-redux at fixed
  frequency, 4 threads: 3.013 s -> 2.684 s; the same output; the other
  programs unchanged. Test
  `the_runtime_for_x86_64_v3_is_linked_only_for_targets_that_have_it`.
- **D189 — Global searches of regular expressions split among the threads
  (2026-11-04).** After D188 regex-redux used 1.45 CPUs: its 5
  replacements, one after the other, are dense (3.5 million matches of
  `a[NSt]|BY` and of `<[^>]*>` in 50 MB) and the engine (`meta::Regex`)
  costs 120 to 420 ns per match (its start state, its prefilter, the
  forward then the reverse lazy DFA: `^>.*\n|\n` reads each line back to
  its start), on one core. And `isc_rx_replace` wrote its result through
  `written`, which calls its function twice (to count the bytes, then to
  write them): every replacement searched twice. Now the compiled program
  (`countMatches`, `matches`, a replacement of every match without groups)
  takes the matches of the global search (`rx_bounds`): on a text of at
  least 512 KiB, with several threads and outside a task, one part per
  thread, each searched from its start up to its first match that starts
  in the next part (`bounds_from`, its own cache), then merged
  (`merge_bounds`): the search after a match depends only on where it goes
  on, so that while that place is one where a part's search went on, the
  next match is the next one of that part, and otherwise one true search,
  until such a place is met: the matches of one search, whatever the cuts.
  Inside a task (the 9 counts of regex-redux, D174), one search: its parts
  would run one after the other, each filling a new cache of the lazy DFA
  (the counts took 1.45 s instead of 0.58 s with 32 parts). The result of
  a replacement is built once, its buffer (a block of the runtime's heap,
  the global allocator) is the string (`adopt`). regex-redux at fixed
  frequency: 2.686 s -> 1.146 s on 4 threads, 4.159 s -> 3.110 s on one;
  the same output. Tests `merged_parts_find_the_matches_of_one_search`
  (isc-runtime-core: 11 patterns, random texts and cuts) and
  `global_searches_split_among_threads_find_the_matches_of_one_search`
  (1 to 4 threads against the interpreter).
- **D190 — The strings of the runtime written once (2026-11-04).** Found
  with D189: `written`, through which the runtime builds the results of
  `replace` and `replaceAll` of a text, `toUpperCase` and `toLowerCase`,
  `padStart` and `padEnd`, `repeat`, `toFixed` and the decimal text of a
  `bigint`, called its function twice: once to count the bytes, once to
  write them into a block of that size. Every one of these did its work
  twice. It now writes once into a `String` that grows (the global
  allocator of the runtime: blocks of its heap) and gives its buffer as
  the string (`adopt`, no copy). Micro-benchmark `bench/micro/strings.isc`
  (the start of the audit of the primitives against their cost, point 6),
  one thread at fixed frequency: `replaceAll` of 50 MB 341 ms -> 171 ms,
  `toUpperCase` of 50 MB 412 ms -> 119 ms, the text of a `bigint` of 9135
  digits 397 us -> 208 us, `padEnd` 24 ms -> 24 ms (a copy either way);
  the same results.
- **D188, measured on the ten programs (2026-11-04).** D188 had been
  measured on 4 programs; on all ten, against the binaries of D187 (the
  same IR): pidigits 2.496 s -> 2.224 s (the runtime's multiplications of
  big integers with BMI2 `mulx`), binary-trees 1.921 s -> 2.020 s and
  fannkuch-redux 4.301 s -> 4.416 s, the others unchanged. Explained by the
  place of the code, not by the runtime: binary-trees runs the same 43.509e9
  instructions with both runtimes, at an IPC of 2.66 instead of 2.89, its
  hot addresses moved by 16 bytes (the runtime of another size before
  them); the same object linked with the two runtimes (`ISC_RUNTIME`) gives
  2.015 s and 1.925 s, and with functions and loops aligned on 64 bytes
  (`ISC_LLVM_ARGS="-align-all-functions=6 -align-loops=64"`) 2.049 s and
  2.047 s (fannkuch-redux 4.297 s and 4.306 s). Not kept as the default:
  on the ten it moves the lottery rather than removing it (fannkuch-redux
  4.417 s -> 4.300 s, binary-trees 2.012 s -> 2.067 s, n-body 5.911 s ->
  5.992 s, the others the same). A margin of about 3 to 5 % comes from the
  place of the code when unrelated code changes size.
- **D191 — Array literals that their scope only reads, on the stack
  (2026-11-04).** n-body allocated 7 arrays and copied 3 per step. The
  first: `const m = masses()` in `advance`, an array of 5 numbers on the
  heap at every step and freed at its end. The first of the small arrays
  of a fixed size without the heap (goal, priority 1): a `let` whose value
  is an array literal of scalars (maybe after `let`s of scalars: an
  inlined function) and whose scope only reads it, borrowed (an element,
  its length, the source of a `map`, `reduce`, `fold`, `filter` or `scan`,
  a copy; never moved, freed, viewed by a slice or a window, stored, or
  passed to a function: a tail call would carry it into the loop) is an
  `alloca` of the function, never freed (`stack_array`, `stack_vars`: the
  tail calls do not free it). n-body at fixed frequency, 4 threads: 6.441 s
  -> 6.111 s; the only program whose IR changes. Test
  `literal_arrays_only_read_live_on_the_stack` (in a tail-recursive loop;
  returned, viewed, passed, updated: on the heap; the interpreter's
  results, Valgrind clean). It found that the tail call freed the array on
  the stack (fixed before this commit).
- **D192 — Divisions of 64-bit lanes by constants with the 52-bit products
  of AVX-512 IFMA (2026-11-04).** After D186 the vector code of fasta still
  divides one lane at a time `t % IM` (`t` the number of the draw, not the
  index of the loop): x86 has no 64-bit vector product high. With AVX-512
  IFMA (and VL; this Xeon has them), `vpmadd52huq` gives the high 52 bits
  of a product of 52-bit numbers: with `m = ceil(2^52 / c)`, the high
  part of `x * m` is `x / c` for `0 <= x < 2^52 / c` (`m` exceeds
  `2^52 / c` by less than 1, so that the product exceeds `x / c * 2^52`
  by less than `x`, below `2^52 / c`, less than the distance to the next
  multiple), the remainder `x - q * c`. One unsigned comparison of the
  vector chooses it, LLVM's division otherwise (a negative or large lane);
  4 lanes per instruction (the 256-bit form, which the functions of the
  host may use). Not for a power of two (LLVM shifts: spectral-norm's
  `/ 2` stays as it was). fasta at fixed frequency: 0.257 s -> 0.207 s on
  4 threads, 0.987 s -> 0.786 s on one; the only one of the ten whose IR
  changes. Test `divisions_of_64_bit_lanes_by_constants_are_exact` (the
  edges of the bound, multiples, negative and large values, 3000 random
  ones; divisors 2, 3, 7, 61, 139968, 2^40).
- **D193 — Slices of arrays of arrays, rows of constants (2026-11-04).**
  Two faults found by writing the test of D192, both older (the compiler of
  v1, 56dfbc8, has the first): (1) the compiled code could not slice an
  array of arrays (`rows.slice(a, b)`: an IR of the wrong type, refused by
  LLVM), where the interpreter could; it is now a new block (not a view:
  `slice_is_view`), each row copied, the source freed after when it is a
  producer made for the slice; (2) a `map` whose rows are arrays of
  constants (D173: data of the program, borrowed) freed them after copying
  them into the rows (an invalid free under Valgrind); they are now copied
  as a borrowed variable is. The IR of the ten programs does not change.
  Test `slices_of_arrays_of_arrays_and_rows_of_constants` (the
  interpreter's results, Valgrind clean).
- **D194 — The groups of integer keys in a small range, by a table
  (2026-11-04).** Profile of k-nucleotide (callgrind): 34 % of the
  instructions in `isc_group` (the `countBy` of the 1-mers and 2-mers, 4
  and 16 distinct keys), 45 instructions per key: its hash, the probing,
  the comparison. When the integer keys lie in a range of at most 2^16
  values or of at most as many values as keys (their smallest and largest,
  one pass that LLVM vectorizes, in `i64`: a `u64` with its top bit
  flipped keeps its order), the key minus the smallest indexes a table of
  the groups: the same three columns (first and last index, count) in the
  order of the first occurrences. A first form in `i128` kept 34
  instructions per key (its pass of the range not vectorized); in `i64`,
  19 per key over the whole of `isc_group`. k-nucleotide at fixed
  frequency: 2.108 s -> 1.591 s on 4 threads, 2.710 s -> 2.200 s on one;
  the same output. Test `dense_groups_as_hashed_ones` (isc-runtime-core:
  small and negative ranges, a range too large, one key, none).
- **D195 — `filter` in vector code, its vectors compressed (2026-11-04).**
  reverse-complement filters 254 MB twice, one element at a time (its
  headers: `filter(iota(n), (i) => data[i] == 62)`; its bases: the bytes
  but the newlines). AVX-512 compresses the lanes of a mask into
  consecutive places (`vpcompressb` with VBMI2, `vpcompressd` and `q`):
  when the source and the predicate are vector code and the elements are
  of 32 or 64 bits, or bytes with VBMI2 (32 bytes or 16 lanes per vector),
  the count adds the bits of each mask, and the write tests it: none kept,
  nothing written; all kept, the vector stored; some, compressed into
  memory (`llvm.masked.compressstore`). A first form compressed every
  vector: the headers (3 kept of 254 million) took 0.697 s on one thread
  instead of 0.625 s, the instruction into memory being slow [not isolated: D375]; and a
  parameter of a `filter` bound to an `iota` is now the index of the loop
  (D185), so that `data[i]` is a load. reverse-complement at fixed
  frequency: 0.294 s -> 0.258 s on 4 threads, 0.664 s -> 0.561 s on one;
  k-nucleotide's filter compressed too (unchanged: not where its time
  goes), the other programs unchanged. Test `filters_compress_their_vectors`
  (bytes, 32 and 64 bits, none, all and some kept, one block and several).
- **D196 — The division of big integers whose quotient is one word, their
  results built once (2026-11-04).** pidigits was 3.3 times its floor:
  21.3e9 instructions for its 9.09e8 passes over a word, 23 per word (4 to
  6 for a loop of `mulx` and `adc`). The profile: the conversions of the
  division (`words`, 13 %), the copies (`memmove`, 11 %: the result of
  each operation copied into a new string), the zeroing of each result
  (`memset`, 4 %). Its divisions, `(num * 3 + acc) / den` with a quotient
  of one digit, took Knuth's algorithm D whole: both operands converted to
  words and copied, shifted to normalize them, the result converted back.
  Now a quotient below 2^64 (`a < b * 2^64`: as many words, or one more and
  `a / 2^64 < b`) is Knuth's estimate from the top two words of `a`
  shifted as `b` would be, refined by the second word of `b` (at most 2
  too large), then checked by the sign of `a - q * b` in one pass over the
  bytes, nothing stored, `q` lowered while negative (`quotient_word`); the
  result of `isc_big_op` is its own buffer (`adopt`, D190); a result whose
  every byte is written is not zeroed first (`unwritten`). A first form
  took `a` of one word more than `b` without the second condition (the
  test of the two forms found a quotient of 2^118). pidigits at fixed
  frequency, one thread: 2.232 s -> 0.888 s (1.33 times its floor, 1.22
  times the C program with GMP); the same output; Valgrind clean (no byte
  read before written). Test `small_quotients_as_num_bigint` (3360
  divisions: random quotients of 64 bits, at 2^64, divisors whose top word
  is small, signs mixed).
- **Rejected after measurement (2026-11-04): the stack floor in
  local-exec, the free list of nodes without the test of the mode, the
  allocations `noalias`.** (1) The assembly of the recursive functions
  read the stack floor of the thread through `__tls_get_addr`
  (`isc_stack_floor_tls`, local-dynamic); in the linked executables there
  is no such call (none in binary-trees): the linker relaxes the sequence
  to local-exec. (2) Taking a node from the free list without testing the
  mode of the heap (the list is filled only by Isocron's allocator): 3
  instructions fewer per node, binary-trees 2.016 s -> 2.017 s. (3)
  `isc_alloc`, `isc_alloc_zeroed` and `isc_node_alloc` declared `noalias`
  (a new block shares no byte with another): n-body 6.130 s -> 6.135 s,
  the other programs within 1 %. None kept: no measured gain.
- **D197 — fannkuch-redux: the flips of a chunk as a `map` (2026-11-04).**
  A change of the program, not of the language (as D170). Its counters
  (n = 11, one thread): 15.4 cycles and 29 instructions per flip, 1.1
  mispredicted branch per permutation; the loop of the flips is 19
  instructions whose chain (each flip reads the first digit of the one
  before) takes 7 to 8 cycles: bound by latency, as the reference programs
  (a table of masks and `pshufb`). The flips of different permutations are
  independent: the chunk now makes its permutations one after the other
  (a `fold` writing them in place into an array) and then takes their
  flips by `map(perms, (p) => flips(p, 0))`, which the compiler runs as
  vector code (`flips`, recursive on scalars, is SPMD, D140): one
  permutation per lane, the loop until the last lane ends. fannkuch-redux
  12 at fixed frequency: 4.413 s -> 1.663 s on 4 threads (the fastest C
  2.480 s), 1.342 s -> 0.502 s on one (n = 11); the same output. The floor
  of D182 assumed the flips one after the other (1.472 s): wrong, the
  bound is the throughput, below it.
- **D198 — Tables of bytes in registers before the vector loop
  (2026-11-04).** The profile of reverse-complement's `map` of lines
  (`perf annotate`): at every vector of 16 bytes, the 8 masked loads of
  32 bytes of its table of complements, their masks computed again, then
  the permutations (D185): LLVM does not move them out of the loop, not
  knowing that the stores into the result miss the table. `fill_range`,
  which tests the size of each table before its vector loop, now loads
  their parts there, once (`table_registers`, `Func::table_parts`), and
  the permutations take them (`permute_bytes`). reverse-complement at
  fixed frequency, 4 threads: 0.263 s -> 0.253 s; fasta (tables of 4 and
  15 bytes) unchanged; the same outputs.
- **D199 — 32 lanes for the `map`s that make bytes (2026-11-04).** After
  D198 the vector loop of the lines of reverse-complement and of fasta
  makes 16 bytes per iteration (`MAP_WIDTH`), its fixed costs (the window
  of D187, the checks, the steps of D186) shared by 16 bytes, half a `ymm`
  register of results. A `map` whose elements are bytes now takes 32
  lanes. At fixed frequency, 4 threads: reverse-complement 0.254 s ->
  0.242 s, fasta 0.207 s -> 0.174 s, mandelbrot unchanged; 64 lanes were
  worse (0.436 s and 0.249 s: registers spilled, the permutations of
  bytes made for 32 lanes at most). The same outputs.
- **D200 — Regions for the trees that die whole (2026-11-04).** binary-
  trees allocated each node from the allocator of nodes (D166) and freed
  each one by a walk of its tree, though every tree it builds dies whole:
  built by a function that only builds, lent to `check`, dropped. The
  compiler now finds the *builders* (`builders`: recursive functions with
  no parameter holding a recursive value, whose result is a node made from
  scalars, constructors without payload and results of builders, nothing
  read, shared or freed) and the `let`s of their results whose body only
  lends the tree to functions returning no recursive value
  (`region_use_ok`: no node of it can outlive the `let`). Such a tree is
  made in a *region* (`isc_region_new`): a variant of each builder
  (`isc_fn_N_rg`, its region as first parameter) cuts its nodes by a bump
  in the compiled code, the runtime adding a chunk when one is full (4 KiB,
  then 64 KiB); at the end of the `let`, or before a tail call, the region
  is freed whole (`isc_region_free`), without a walk. A moved, returned or
  stored tree, or one given to a function returning a tree, stays on the
  heap. With `ISC_ALLOC=glibc` each node is a block of its own, for
  Valgrind. binary-trees 21 at fixed frequency: 4 threads 2.030 s -> 1.317
  s, 1 thread 5.060 s -> 3.603 s; the same output. Profile after (1
  thread): 36.96 G instructions for about 1.23 G calls (builds and checks),
  154 page faults: the cost is now the calls (prologue, stack test, one
  call for each leaf), not the memory.
- **D201 — Loops of tasks shared with the idle threads (2026-11-04).**
  After D200, binary-trees 21 on 4 threads used 3.08 processors
  (`perf stat`): its 9 depths (each about 33.5 M nodes) are the tasks of
  the outer `map`, 3 rounds on 4 threads, and the trees of a depth were
  reduced on one thread, for two reasons: a loop called from a task ran in
  place (`BUSY`, D102), and an exact `reduce` cuts its elements into blocks
  of B = 4096 (the fixed form), so 32 trees of depth 20 are one block even
  outside a task. Two changes. (1) The runtime: `isc_parallel_for` called
  while a job runs puts its tasks in a slot (`NestedSlot`, 64 of them, the
  first tried depending on the stack of the caller); the caller takes them
  in turn, and so do the threads that have finished their share
  (`help_until_done`), until every share is done; the caller then closes
  the slot (1) and waits until no helper counts itself in it (a helper
  counts itself before it reads the slot, sequentially consistent on both
  sides: the frame of the loop is never read once gone). A worker spins
  SPIN times without work, then sleeps by 50 us; `main` only spins. The
  workers run on stacks of 1 GiB like the body of `main` (`start_worker`),
  so that a recursion in a task reaches the same depth on any thread. (2)
  The compiler: an exact `reduce` (D183) of a `map` whose elements are
  loops (`inner_loop`) takes HEAVY_CHUNKS chunks per thread, taken in turn,
  each reduced by blocks of B in vector code (`reduce_block`), each result
  in its own 128 bytes, then reduced in order (`reduce_heavy`), in a task
  too: any grouping gives the same bits. At fixed frequency, 4 threads:
  binary-trees 1.316 s -> 0.989 s (3.683 processors used), k-nucleotide
  1.591 s -> 1.443 s (its 5 counts of k-mers, a `map` of 5 tasks on 4
  threads, now shared), the eight others unchanged; the same outputs at 1
  and 4 threads. Rejected after measurement: sharing also the `map`s of
  loops inside tasks (into arrays). fannkuch-redux 1.663 s -> 1.803 s:
  56.28 G -> 58.80 G instructions, 21.13 G -> 23.08 G cycles; the 2.52 G
  more instructions over its 11880 `map`s of 40320 permutations are about
  6.6 k per chunk of 1260, the scalar ends of its two ranges (D121 rotates
  each chunk) of SPMD code, 7 or 8 permutations each flipped one by one,
  whose branches the masked SPMD code does not mispredict (IPC 2.66 ->
  2.55). A first `reduce_heavy` with a scalar loop per chunk made
  k-nucleotide slower (1.597 s -> 2.136 s): its `count` of k-mers is
  vector code in `reduce_block`.
- **D200, D201 corrected: the runtime of WebAssembly (2026-11-04).** The
  full suite after D201 failed the tests of WebAssembly (5 of them: "no
  runtime for wasm32-wasip1"): the regions of D200 call `isc_heap`, which
  WASI has not, and the hash of the first slot of D201 multiplied a
  `usize` (32 bits there) by a 64-bit constant; `build.rs` then left an
  empty runtime, as for a target not installed. The regions are now for
  the native targets only (the compiler makes none for WebAssembly), the
  hash is computed in `u64`, and `build.rs` stops when a WASI target is
  installed but its runtime does not build (checked by adding an error for
  WASI only: "building crates/isc-runtime for wasm32-wasip1 failed (the
  target is installed)").
- **D203 — `x.length` whatever the program declares (2026-11-04).** The
  parser wrote `x.length` as the call `len(x)`, and the functions of a
  program shadow the primitives of the same name: a program declaring its
  own `len` (a test of D202 did, the length of a list) changed every
  `.length` into a call of it ("type mismatch: expected `List`, found
  `string[k]`"). The parser now writes the name `$length`, which no program
  can declare (like `$text`, D150), and the checker takes it for the
  primitive; `len(x)` still calls the program's `len` when it has one.
- **D202 — Base cases computed by the callers (2026-11-04).** After D201,
  binary-trees made about 1.23 G calls, half of them for leaves: the
  call of `bottomUp(0)` returning `Leaf`, of `check(Leaf)` returning 1,
  each with its prologue, its test of the stack and its epilogue (about
  30 instructions per call, 36.95 G in all on one thread). A recursive
  function whose body is `if (c) a else b`, `c` and one branch simple (no
  call, no allocation, no division, no index: nothing that can panic; the
  branch a scalar or a constructor without payload; `base_cases`), has
  that branch computed by its callers: `c` on the arguments, the call only
  for the other branch. The parameters of a recursive function are
  borrowed (its caller owns them): nothing changes hands. LLVM then also
  drops the test at the entry of `bottomUp` (internal, every caller tests
  it). A tail call (a jump) and the calls of SPMD code are left as they
  are. A recursion one frame less deep at a leaf: the limit of the stack
  (D102) is reached one call later, the same on every machine. binary-
  trees 21 at fixed frequency: 4 threads 0.986 s -> 0.671 s, 1 thread
  3.555 s -> 2.269 s, 36.95 G -> 26.41 G instructions; fannkuch-redux
  (its `factorial`, called once; the base case of `next` is a tuple)
  1.661 s -> 1.663 s, the eight others
  compile to the same code; the same outputs at 1 and 4 threads.
- **D204 — Lengths fixed by the types (2026-11-04).** The checker knows
  that `x: f64[5]` holds 5 elements, but the IR typed every array `f64[]`:
  n-body's pair loop compared each index with a length read at run time
  (13 comparisons out of about 75 instructions per pair), and `b.x + DT *
  vx` was compiled as for any length (a loop of 512-bit vectors, a scalar
  end, a parallel path above 4096 elements). `Type::Array` now carries
  `Fixed`, the length when the type is a constant (`Poly::as_const`),
  which takes no part in the equality or the hash of types (an `f64[5]`
  and an `f64[n]` are the same type to every pass) and is not shown in
  messages. The generated code puts it as a constant into the aggregate of
  every value of such a type (`fix_len`, an `insertvalue` that LLVM folds
  into every read of the length), on the results of expressions only: a
  binding is not fixed, and the two sizes of a `CheckSize` are computed
  without it (`no_fix`), so that a parameter whose size is tested at run
  time (a call, the entry of a recursive function) is tested on its real
  length. At fixed frequency, 4 threads: n-body 5.960 s -> 5.552 s (23
  calls of `isc_panic_bounds` in its loop -> 2), reverse-complement 0.240
  s -> 0.228 s, k-nucleotide, regex-redux and fasta unchanged, the others
  compile to the same code; the same outputs at 1 and 4 threads. Profile
  of n-body after (1 thread): 996 instructions and 365 cycles per step,
  25 % in `isc_alloc` and `isc_free`; its pair loop is still a loop, each
  pair a chain root, division, products (the hottest instruction the
  division).
- **D205 — Short loops unrolled whole (2026-11-04).** After D204, the 10
  pairs of a step of n-body were still a loop: LLVM's threshold of size
  for a full unrolling is smaller than 10 bodies of a pair, so the root
  and the division of each pair waited for those of the pair before, the
  10 being independent of each other (they read the positions only). A
  loop whose bounds are constants of the generated code and whose count
  is at most UNROLL_FULL = 16 now gets `llvm.loop.unroll.full`; the
  length that a type fixes (D204) is such a constant: a read of field 1
  (or 2, the columns) of an aggregate just fixed is the constant itself
  (`known_len`, in `ins`). At fixed frequency, 4 threads: n-body 5.529 s
  -> 4.785 s; mandelbrot, k-nucleotide, regex-redux, reverse-complement
  and fasta (whose code changes) unchanged, the others compile to the
  same code; the same outputs at 1 and 4 threads.
- **D206 — The states of loops owned (2026-11-04).** n-body's `steps(b, n)`
  is a loop (its call to itself a tail call) whose other result is `b`: its
  parameter was borrowed, so each step copied `b.vx`, `b.vy` and `b.vz`
  into the accumulator of its `fold` ("a part of `b` is kept as a value of
  its own"), and the loop then freed the whole old state. Such a parameter
  (a tuple, every call of the function to itself a tail call, every other
  result the parameter itself, read only through its fields) is now owned
  (`own_states`, `Function::consumed`): the body starts with `let b.k =
  take k b` for each field, reads `b.k` for `b[k]` and gives back the tuple
  of the `b.k` for `b`; each field is owned on its own, moved at its last
  use or freed. The generated code copies the parameter once on entry (its
  caller lends it) and never frees a slot the body takes apart. The
  borrowed form copied it once too, as its result: no copy is added. The
  memory check (`verify_fn`) knows such a parameter as owned. n-body at
  fixed frequency, 4 threads: 4.763 s -> 4.135 s; the nine others compile
  to the same code; the same outputs.
- **D207 — A `map` written into an input that dies (2026-11-04).** After
  D206, a step of n-body still made 3 new arrays, `b.x + DT * vx` and the
  two others, and freed the 3 old ones. A `map` of scalars materialized as
  the value of a `let` or as a component of a tuple, one of whose inputs is
  an owned array of the same scalars at its last use, now writes its
  results into that input (the input marked `Use::Move`: not freed, its
  memory the result); element `i` is read before it is written, the vector
  code included. Not when the body of the `map` may panic (an index, a
  size, a division of integers, a call: a replay must read elements not
  yet written), nor for a `map` that a `reduce` reads (never materialized).
  The memory check takes the input as moved into the result, the memory
  report counts no allocation. n-body at fixed frequency, 4 threads: 4.135
  s -> 3.589 s; the nine others compile to the same code; the same outputs.
- **D208 — The allocations `noalias` (2026-11-04).** Rejected before
  (see "Rejected after measurement", 2026-11-04: n-body 6.130 s -> 6.135
  s), when each step of n-body allocated and freed its arrays. After D206
  and D207 a step allocates nothing: its six arrays are made once, before
  the loop, and written in place. Its loop was then 466 scalar instructions,
  75 stores and 148 reads of memory per step: LLVM could not tell the six
  arrays apart, so each update of `vx[i]` went through memory and the 10
  pairs could not be put side by side. `isc_alloc` and `isc_alloc_zeroed`
  are now declared `noalias`, as `malloc` (a new block shares no byte with
  any other; Isocron's arrays are values, two owned arrays never share
  memory): the loop is 258 instructions, vector code of 2 pairs at a time
  (`vmulpd`, `vsqrtpd`, `vdivpd`). At fixed frequency, 4 threads: n-body
  3.571 s -> 2.093 s; the nine others unchanged (within 0.5 %); the same
  outputs at 1 and 4 threads.
- **Rejected after measurement (2026-11-04): the standard input mapped
  (`mmap`) instead of read.** A regular file of at least 1 MiB on the
  standard input, read from its start, mapped privately with
  `MAP_POPULATE` (a page before it for the header, a class of its own
  unmapped by `free`): slower. At fixed frequency, 4 threads:
  reverse-complement 0.235 s -> 0.252 s, k-nucleotide 1.435 s -> 1.456 s,
  regex-redux 1.154 s -> 1.165 s, the same outputs. The cause, measured on
  a program that reads the 254 MB of fasta25m.txt and prints its length
  (best of 5 runs, each with the file opened again): read 57.75 ms, 293
  page faults (huge pages), 16376 dTLB misses; mapped 76.03 ms, 72 faults,
  28679 misses; with one pass over the bytes, 75.65 ms against 96.20 ms.
  Filling the entries of about 62000 pages of 4 KiB costs more than
  clearing huge pages and copying into them, and the passes over 4 KiB
  pages miss the TLB more often. (A first measurement with `perf stat -r
  5` was wrong: its runs shared one standard input, the four last ones
  read nothing; it gave 12.7 ms for reading and is not kept.) The floor
  of reverse-complement (0.067 s, of which 0.056 s for reading) agrees
  with the 55.5 ms measured here.
- **Rejected after measurement (2026-11-04): the kept elements of a
  `filter` compressed into a register.** In `write_kept`, a vector of at
  most 512 bits keeping some of its elements compressed into a register
  (`llvm.x86.avx512.mask.compress`) and stored by a mask of its first
  lanes, instead of `llvm.masked.compressstore` (the compression into
  memory, slow on Intel's cores [not shown: D375]). At fixed frequency, 4 threads:
  reverse-complement 0.231 s -> 0.230 s, k-nucleotide 1.442 s -> 1.443 s,
  the same outputs: no gain. The vectors that keep some of their elements
  are not what the filter of reverse-complement pays (60 ms of its 0.23 s,
  measured by phases); an estimate of what it pays, to be measured: two
  passes over its 247 MB (counting, then writing) and the 247 MB it
  writes into new pages, which the system first clears.
- **D212 — The generated code starts on a page (2026-11-04).** A change
  of the runtime that leaves its hot paths alone moved binary-trees: the
  linker puts the cold functions of the runtime (`.text.unlikely`) before
  the generated code, whose place, and the alignment of its loops, then
  depend on their size (848 bytes more with D211: its hot addresses
  0x2b26 -> 0x2e76, the same 26.47e9 instructions, 7.80e9 -> 8.23e9
  cycles, 5 % slower; the same effect as D188). The module now starts with
  an empty function aligned on 4096 bytes (`isc_code_start`, not internal,
  so kept), the generated functions after it in the same section: their
  place modulo 4096 no longer depends on the code before them. A draw
  made once: at fixed frequency, 4 threads, against D208, binary-trees
  0.672 s -> 0.706 s (n = 19, one thread: 0.90 -> 4.06 million misses of
  the cache of decoded instructions, `frontend_retired.dsb_miss`; D213
  takes it back), the nine others unchanged (n-body 2.025 s -> 2.029 s,
  regex-redux 1.158 s -> 1.148 s, the others within 1 %).
- **D211 — A cached large block grown instead of new memory
  (2026-11-04).** reverse-complement spent about 38 % of its cycles in the
  kernel clearing new pages (`clear_page_erms`, `folio_zero_user`): 365
  huge pages for its arrays (bases 50, 74, 123 MB and lines 54, 83, 131 MB
  for its three records), each larger than any cached block, so that the
  cache unmapped the blocks of the record before and mapped new ones. When
  no cached block can serve a request, the largest smaller one now grows
  to it (`grow_cached`: `mremap` to a place of the same offset modulo 2
  MiB, its huge pages moved whole, after one advice on the whole mapping:
  `mremap` moves a single area, the parts that `shape_large` advised
  differently were refused with EFAULT); its written pages stay, only the
  rest is new. reverse-complement: 365 -> 306 new huge pages. At fixed
  frequency, 4 threads, with D212 (the place of the code no longer
  moving): reverse-complement 0.230 s -> 0.210 s, regex-redux 1.148 s ->
  1.137 s, the eight others unchanged (binary-trees 0.706 s -> 0.705 s);
  the same outputs. A request for zeros on such a block is cleared (its
  contents DIRTY), checked by a test.
- **D213 — The recursive functions aligned on 64 bytes (2026-11-04).**
  With the code on a page (D212), binary-trees lost 5 % to the misses of
  the cache of decoded instructions (n = 19, one thread: 2.84 million
  `frontend_retired.dsb_miss`, 74.3 million µops decoded by the legacy
  path, `idq.mite_uops`): its small recursive functions, called at each
  node, crossed lines of the cache. Each recursive function now starts on
  64 bytes (`align 64`; D188 aligned every function and every loop, which
  moved the gains and losses around): 1.20 million misses, 59.3 million
  µops, 1.663e9 -> 1.520e9 cycles. At fixed frequency, 4 threads:
  binary-trees 0.700 s -> 0.667 s, fannkuch-redux 1.662 s -> 1.676 s
  (+0.8 %), n-body 2.106 s -> 2.052 s (its spread between series is about
  2 %), the seven others unchanged; the same outputs.
- **D214 — A `map` by rows of the constant that divides its index
  (2026-11-04).** The formatting of reverse-complement (`map(iota(length),
  (q) => q % 61 == 60 || ... ? 10u8 : base(q / 61 * 60 + q % 61))`) cost
  2.83 cycles and 5.30 instructions per byte on one thread, the same from
  the caches as from memory (inputs of 2.5, 25 and 254 MB): bound by its
  instructions, the indices of 32 bytes in 64-bit lanes, their quotients
  and remainders carried (D186), the bounds of a window and its
  permutation (D187). A `map` over an index `q` whose body divides it by
  one constant `c` of at least the width of its vectors, and is neither a
  loop nor a call (`row_split`), now runs by rows of `c` (`fill_rows`): in
  a row `q / c` is the row, the same for the lanes, and `q % c` the place
  in it, consecutive lanes (given to the body through `pre`; the scalar
  replay of a failed check computes them itself), so that an index made of
  them is consecutive too: one load where the window was. A row by vectors
  from its start, then one more ending at its end over the one before (an
  element computed twice is the same value); a row shorter than a vector
  one element at a time. The formatting: 1.28 cycles and 3.23 instructions
  per byte. At fixed frequency, 4 threads: reverse-complement 0.214 s ->
  0.182 s; the nine others compile to the same code (fasta's lines call
  `pick`: a first version without that rule took it 0.174 s -> 0.178 s,
  each row computing 3 of its 61 elements twice); the same outputs.
- **Rejected after measurement (2026-11-04): the product of a big integer
  by a word in assembly.** pidigits spends 54 % of its cycles in
  `limbs::mul_word`, whose loop (LLVM) reloaded its pointers and the
  multiplier at each word (`mulx` writes into `rdx`, the register of the
  multiplier). Written in assembly for the CPUs with BMI2 (groups of 4
  words: 4 `mulx`, then `add` and 4 `adc` in one chain): the same output,
  21 % fewer instructions for the program (8.68e9 -> 6.83e9), but 2.6 %
  more cycles (2.83e9 -> 2.90e9; at fixed frequency, one thread, 0.888 s
  -> 0.901 s). A micro-benchmark of the loop alone over 520 words (the
  size pidigits reaches), the operands aligned or one byte off (the
  magnitudes start after the sign byte): assembly 0.786 and 0.792 ns per
  word, Rust 0.848 and 0.855 ns. The alignment changes nothing, and the
  loop is at the floor of the machine [contradicted: D375] (0.739 ns per word, `mulx` and
  `adc`, `results/machine_floor.txt`): pidigits loses its time elsewhere
  than in this loop, to be measured (`big256::op`, 38 % of its cycles,
  and the work around the loops).
- **Rejected after measurement (2026-11-04): an array recomputed instead
  of stored.** k-nucleotide makes the codes of its 125 million bases with
  `map(bytes(sequence), code)`: 1 GB of `i64` written into new pages
  (`clear_page_erms`, 24.7 % of its cycles with 4 threads, which use 2.25
  processors) and read by 7 passes. A pass (`rematerialize`): a `let` of a
  `map` whose body is a few operations on scalars that cannot panic, its
  elements wider than those of its source, and every use an element or
  the length, each element computed again where it is read; the same
  outputs. At fixed frequency: 4 threads 1.438 s -> 1.390 s, but one
  thread 2.169 s -> 2.640 s. The rule does not bound the reads of an
  element: a window of a k-mer reads each base k times, about 5.4e9
  computations of `code` for the 5 queries; with one thread they cost more
  than the memory saved. Kept in TODO instead: an array stored in the
  narrowest type its values fit (here a byte for 0 to 3), the same type
  for the program.
- **D217 — Consecutive indices compared and checked from their ends
  (2026-11-04).** After D214 the loop of a line of reverse-complement (249
  instructions for 61 bytes) compared vectors of 32 consecutive 64-bit
  indices with a value of every lane (`q % 61 == 60`, `q == length - 1`:
  16 `vpcmpneqq` and 18 `kunpck` to put their 8-lane masks together) and
  checked their bounds lane by lane (8 `vpcmpnltuq`). A vector of
  consecutive indices (D181) compared with a value of every lane, `==` or
  `!=`, is now a mask of at most one bit, the lane found from the first
  index (a subtraction, a shift); its bounds are checked on its first and
  last lanes, every lane only when they fail (the first lane out, for the
  message). At fixed frequency, 4 threads: reverse-complement 0.184 s ->
  0.174 s, k-nucleotide 1.433 s -> 1.382 s, fasta, fannkuch-redux and
  spectral-norm unchanged (their code changes), the others the same code;
  the same outputs.
- **D218 — An array stored in a narrower type (2026-11-04).** The codes
  of k-nucleotide (`map(bytes(sequence), code)`, values 0 to 3) were 1 GB
  of `i64` written into new pages and read by 7 passes; recomputing them
  where they are read was slower with one thread (rejected above). A `let`
  of a `map` whose values all fit a narrower integer type (`int_range`:
  the leaves of an `if`, `x & c` with `c >= 0`, a byte or a boolean
  converted) and that is read only by element and length is now stored in
  that type (`u8`, `u16` or `i8`, when narrower), its body converted to
  it, each element converted back where it is read (`narrow_arrays`): the
  same values for the program, here 125 MB of bytes. At fixed frequency:
  k-nucleotide, 4 threads 1.387 s -> 1.178 s, one thread 1.965 s -> 1.778
  s; the nine others compile to the same code; the same outputs.
- **D219 — A search of a large text chooses its engine on a sample
  (2026-11-04).** In regex-redux the prefilter of the literals of each
  pattern (Teddy, `aho-corasick`) took 72 % of the cycles of the 9 counts
  on one thread: its literals (`agggtaaa`, `tttaccct`...) are made of the 4
  letters of the text, candidates everywhere, each checked again by the
  automaton. Without any prefilter the counts took half the time (2.21 s
  -> 1.12 s on one thread), but the replacements, whose literals are rare
  (`<[^>]*>`, `tHa[Nt]`), lost what that saved (the program 1.140 s ->
  1.093 s). A global search of a text of 4 MiB or more (`rx_for`: the
  counts, the matches, the replacements) now compiles its pattern with and
  without the prefilter, runs each twice over the first 64 KiB of the text
  (to fill its cache and bring the sample in, then timed by the counter of
  cycles), and searches the text with the faster one. The two find the
  same matches: the choice changes the time only. At fixed frequency:
  regex-redux, 4 threads 1.144 s -> 0.855 s, one thread 3.116 s -> 1.953
  s; the same outputs. Tried before it and not kept: counting by searches
  forwards only (no gain: for these patterns the engine finds an inner
  literal and searches backwards from it anyway, `find_rev` still 4 % of
  the cycles).
- **D220 — The cached large blocks smaller than a request stay cached
  (2026-11-04).** reverse-complement makes two large arrays per record
  (its bases without newlines, then its lines), records of 50, 75 and 125
  MB. A request that found no cached block of its size grew the largest
  smaller one (D211) and unmapped all the other smaller ones, on the idea
  that an array that grows makes them useless: the second array of each
  record then took new memory, which the system clears (strace: 669 MB of
  new pages for the large blocks, 270 of them the input). The smaller
  blocks now stay cached: the next request grows the largest of them in
  turn, and `trim_large` still bounds the cache to max(64 MiB, the bytes
  in use) and unmaps the stale blocks. New pages 669 -> 537 MB, mremap 2
  -> 4. The cost of new pages on xeon (a C program, 254 MiB written by
  stores of 64 bytes): huge pages, 4 threads, 20.6 ms fresh against 12.8
  ms on pages already written, one thread 38.7 against 18.0 ms; pages of 4
  KiB, 4 threads, 22.2 ms fresh. At fixed frequency, best of 7:
  reverse-complement 0.175 s -> 0.157 s on 4 threads, 0.259 s -> 0.245 s
  on one; the nine others unchanged (n-body 2.026 / 2.089 s, then 2.066 /
  2.113 and 2.099 / 2.050 s alternated: noise, the same generated code).
  Test `every_array_of_a_phase_grows_from_the_cache` (three arrays per
  phase, one of zeros, while a large array stays in use: 4 grown blocks
  against 2 before).
- **D221 — A large file read by every thread (2026-11-04).** The floor
  of reverse-complement (67 ms) was beaten by the Rust program (66 ms): a
  floor a program beats is wrong. Its term for the input, 56 ms, was the
  time of our own `readStdin` (one `read` of 254 MB into new pages), not
  the cost of the machine. Measured on xeon (a C program, 254 MB of a file
  in the page cache): one thread 57.1 ms into new pages, 30.4 ms into pages
  already written; `pread` by parts, 2 threads 32.6 / 18.9 ms, 4 threads
  27.5 / 15.7 ms. The copies out of the page cache, and the clearing of
  the new pages, are made by the thread that reads: one core for one
  `read`. `readStdin` and `readBytes` of a file of 8 MiB or more
  (READ_PARALLEL_MIN) now read it with one task per thread of the pool,
  each its part with `pread` from the offset of the descriptor; the
  offset is then set after the bytes read, and the usual loop reads what
  is left (a file that grew) or what a part did not get (a file that got
  shorter: the bytes without a gap are kept). A pipe, one thread, a busy
  pool, WebAssembly: one `read` as before. At fixed frequency, 4 threads,
  best of 7: readStdin of 254 MB 0.056 s -> 0.029 s, reverse-complement
  0.155 s -> 0.128 s, k-nucleotide 1.179 s -> 1.148 s, regex-redux 0.858
  s -> 0.846 s; one thread unchanged; the same outputs. Test
  `large_files_read_by_several_threads` (offsets 0, 1, in the middle, at
  the end; 1 and 4 threads; a pipe; `readBytes`).
- **D222 — The floors redone from what each result requires
  (2026-11-04).** The floor of reverse-complement (D182) was beaten by
  the Rust program (D221): it took the time of our own reading of the
  input. The others had the same fault, or another hypothesis on the way
  to do the work: binary-trees took a C program that recurses, pidigits
  the passes of our divisions and a multiplication by a word written in C
  (0.74 ns a word: 0.33 ns in assembly), spectral-norm only divisions,
  mandelbrot 7 operations per iteration, regex-redux an automaton walked
  byte by byte, fannkuch-redux and fasta chains that the work does not
  have. Each floor is now what the result requires, under the rules of
  the Benchmarks Game (its algorithm), at the cost of the cheapest way the
  machine has to do it, measured in C by `bench/floor/floor.c` (new: a
  file read by every thread into new pages, into pages written, by blocks
  in the caches; an array changed in place; the flips of fannkuch-redux,
  4 permutations per vector, 8 vectors interleaved, and its next
  permutation; a big integer times a word in assembly; counts in tables
  of 4096; trees made by a loop and visited with an explicit stack; the
  generator of fasta in 4 vectors of lanes, 32 numbers ahead; two literals
  found in 64 bytes by the masks of the 4 bases in general registers,
  checked against a count byte by byte). Three primitives were first
  limited by my own code and were rewritten before use: fasta 1.945 ->
  1.029 ns per number (IUB), 1.641 -> 0.557 (homo sapiens), one chain of
  lanes made 4; the literals 0.157 -> 0.074 ns per byte, the masks out of
  the one port of mask operations; the flips 0.766 -> 0.692 ns, 8 vectors
  instead of 4. `floors.py` flags a floor below a measured time; none is.
  Floors (Isocron at D219 / floor, the fastest other / floor):
  binary-trees 0.396 s (1.7 / 2.1), fannkuch-redux 0.883 s (1.9 / 2.8),
  fasta 0.037 s (4.7 / 20.6), k-nucleotide 0.268 s (4.4 / 4.4), mandelbrot
  0.260 s (2.5 / 3.0), n-body 1.253 s (1.6 / 1.6), pidigits 0.194 s (4.6 /
  3.5), regex-redux 0.030 s (28.8 / 25.6), reverse-complement 0.017 s
  (10.2 / 3.9), spectral-norm 0.107 s (1.8 / 1.9). Some are still above
  the machine (see `results/floors.md`: the flips, the next permutation,
  the smallest tables of k-mers, the trees that fit the L3 cache).
- **D223 — Valgrind with 2 threads in every suite (2026-11-04).** The
  complete suite (`cargo test`, then with `ISC_ALLOC=glibc`) took 13.3
  minutes; three suites took 100, 50 and 48 s each (bitwise, text,
  result). Under valgrind the threads run one at a time, and each idle
  worker of the pool spins (SPIN = 65536 pauses, about 3 ms on the
  machine) before it sleeps: the program of the test of `concat`, 0.00 s
  alone, took 47.4 s under valgrind with the 16 threads of the machine,
  0.31 s with one. Six of the nine helpers already gave valgrind
  `ISC_THREADS=2` (the pool and its worker still checked); the three
  others now do too: bitwise 100.8 s -> 12.0 s, result 47.8 s -> 3.9 s,
  text 49.9 s -> 5.0 s.
  The two passes also run at once (`tools/suite.py`: one build, then the
  39 test binaries twice, 6 at a time, the longest first, each allocator
  its own directory of temporary files through `ISC_TEST_TMP`): 3.0
  minutes instead of 13.3, the same 78 runs and the doc tests.
- **D224 — Two simple shapes of patterns searched by masks
  (2026-11-04).** regex-redux was 28.8 times its floor (D222). By step, 4
  threads: reading and cleaning 71 ms, the 9 counts 287 ms, the
  replacements 14, 13, 152, 192 and 124 ms; the three dear ones have
  millions of matches, each a search of the automaton from its start. Two
  shapes are now recognized in the syntax tree of a pattern
  (`sem::rx::simple`): an alternation of sequences of classes of ASCII
  bytes, all of one length L from 1 to 64 (literals, classes, groups,
  fixed repetitions, at most 32 sequences: the 9 counts, `tHa[Nt]`,
  `aND|caN|Ha[DS]|WaS`, `a[NSt]|BY`); and such a head, then `[^c]*` or
  `[^c]+`, then the ASCII byte c (`<[^>]*>`, `\|[^|][^|]*\|`). In 64
  bytes: a comparison per distinct byte (AVX2 in the runtime for
  x86-64-v3, SSE2 in the other), the masks of the classes, then the starts
  of each sequence as the and of the masks of its classes shifted by their
  places across two blocks; every start in a mask taken in order, without
  overlap; for the second shape the end is the first c after the head
  (`memchr`). A class with a member outside ASCII (`k` or `s` ignoring
  case: the Kelvin sign, the long s) keeps the automaton: ASCII bytes are
  whole characters in UTF-8, so the matches of bytes are those of
  characters. The parts in parallel and their merge (D189) take any
  engine (`merge_bounds_with`). Elsewhere than x86-64 (WebAssembly),
  the automaton as before. The interpreter keeps the automaton: the tests
  compare the two. At fixed frequency: regex-redux, 4 threads 0.857 s ->
  0.479 s, one thread 1.958 s -> 0.786 s; the runtime without AVX2
  1.039 s -> 0.519 s (a first version without SIMD there took 1.829 s);
  the same outputs. Tests: `shapes_of_patterns` and
  `masks_find_the_matches_of_the_automaton` (isc-runtime-core),
  `simple_shapes_found_by_masks` (strings).
- **D225 — An array of bytes made only to be written, by blocks
  (2026-11-04).** In reverse-complement each record makes its lines (a
  `map` of about 250 MB in all) only to give them to `printBytes`: written
  into pages, new or taken again from the cache of large blocks (D220),
  then read back by `write`. The code generator now recognizes `let x =
  map(...) in printBytes(x)` (bytes, made by `materialize`: not a stream,
  not rows, its elements without a loop of their own, no input moved into
  it) and makes the array by blocks of 1 MiB into one buffer: each block in
  parallel as `materialize` makes the whole array (tasks of BLOCK
  elements, the same `fill_range`, the element `i` of the block from `lo`
  stored at `buffer - lo + i`, so the code of the elements does not
  change), then written. A quarter of a MiB per thread: the bytes stay in
  the caches, never go to memory nor to new pages. The same bytes in the
  same order. A first version lost the last block, shorter than the others
  (`for_range` takes whole steps only): the loop now counts the blocks.
  At fixed frequency, best of 7, 4 threads: reverse-complement 0.128 s ->
  0.107 s (one thread 0.246 -> 0.228 s), fasta 0.173 s -> 0.168 s,
  mandelbrot unchanged (0.638 s); the same outputs. Test
  `bytes_written_by_blocks` (lengths around the blocks, rows of 61,
  between `print`s, 1 and 4 threads, against bytes computed in the test,
  valgrind).
- **D226 — Consecutive indices checked from their ends under a mask
  (2026-11-04).** After D225 the lines of reverse-complement took about 20
  ms on 4 threads, the bound the computation (about one cycle a byte):
  `bases[m - 1 - i]` is read in the branch of an `if` (not a newline), and
  under a mask the check of D217 (the first and the last of consecutive
  indices) was not tried: every lane compared, 4 comparisons of 8 indices
  of 64 bits per vector of 32 bytes. Under a mask it holds as well: every
  lane in bounds, the active ones are; when it fails (lanes left out past
  the end) the test of the active lanes as before, and its panic. At
  fixed frequency, best of 7: reverse-complement 0.106 s -> 0.104 s on 4
  threads, 0.229 s -> 0.218 s on one; fannkuch-redux 1.677 s -> 1.661 s
  (its IR changes); the other eight unchanged (k-nucleotide and n-body:
  the same IR, their differences of a series to the next noise). The same
  outputs. Test `consecutive_indices_under_a_mask`.
- **D227 — A floor keeps the bits of the computation (2026-11-04).**
  Two floors of D222 assumed other bits than the programs compute.
  mandelbrot counted 4 operations per iteration, with fused
  multiply-adds (`fma(zr, zr, cr - zi zi)`): a fused multiply-add rounds
  once where the program rounds twice, the escapes near the border of the
  set may change, and the image with them (the C++ program gpp-4 already
  gives another image than the reference at N = 200). n-body put the mass
  into the division (`dt m / d^3`) where the program computes `dx * m[j] *
  mag` after `mag = dt / d^3`. Isocron keeps the bits of the computation
  as written, so does its floor: mandelbrot 7 operations per iteration (3
  products, 4 sums, `2.0 * zr` exact) and one addition for the test every
  8 iterations, 0.260 s -> 0.436 s (Isocron 1.5 times it, gpp-4 1.8);
  n-body the chain as written, 11 latencies of an addition, a root and a
  division, 1.253 s -> 1.316 s (Isocron 1.6, gcc-9 1.5), the value of
  D182. The rule written in `results/floors.md`.
- **D228 — The length of an array known before running is a constant
  (2026-11-04).** fasta was 4.7 times its floor, all of it computation:
  about 15 instructions per byte of output. Its first sequence,
  `alu[i % alu.length]` over the bytes of a constant string, was a scalar
  loop with a hardware division (`div`) per byte, about 9 cycles a byte:
  `alu.length` was a value of the run, so neither the remainder carried by
  the loop (D180) nor the vector code applied. A pass of the IR
  (`constant_lengths`, after `producer_lengths`) now replaces `len(v)` by
  a constant when `v` is bound to an array whose length is known before
  running: the bytes of a constant string, a literal or constant array,
  `iota`, `fill` or `zeros` of a constant length, a `map` of one, through
  `let`s. Of the ten programs only fasta changes. At fixed frequency, best
  of 7: fasta 0.167 s -> 0.142 s on 4 threads, 0.650 s -> 0.546 s on one;
  the same output. Test `lengths_known_before_running` (the interpreter's
  results, no `len` of them left in the IR).
- **D229 — Consecutive indices through a remainder by a constant
  (2026-11-04).** In fasta `values[t % IM]`, with `t` consecutive in the
  lanes, was a gather (`vpgatherqq`, 4 of 8 lanes per 32 bytes): the
  remainder hid that its lanes are consecutive but where they cross a
  multiple of IM. In vector code `a[x % c]`, `x` consecutive (D217) of
  type i64 and c a constant of at least the lanes: the first remainder
  `x0 % c` computed alone; when `x0 >= 0` and `x0 % c + w - 1 < c`, the
  lanes are `x0 % c + j` and the index is read by one load (its bounds
  from its ends, D217); otherwise the general code, its gather and its
  checks. The same values. Of the ten programs only fasta changes. At
  fixed frequency, best of 7: fasta 0.142 s -> 0.135 s on 4 threads,
  0.547 s -> 0.519 s on one. Test
  `consecutive_indices_through_a_remainder` (crossings in a vector, a
  negative `x`, c below the lanes, an index past the end).
- **D230 — A function of an element of a small table, computed once per
  element of the table (2026-11-04).** After D229 each random byte of fasta
  still cost a division of floats (`f64(values[t % IM]) / f64(IM)`,
  `vdivpd`, 2 cycles a lane) and a count over the cumulative
  probabilities, for 2e8 bytes, though `values` has only the 139968 values
  of one period of the generator. A pass of the IR (`table_functions`,
  after `constant_lengths`) looks in each `map` for a read `t[i]` of an
  array bound outside it, at an index that varies, and the highest node
  above it that depends on nothing else of the `map`: `h(t[i])`. When `h`
  costs more than a read (a loop, a division of floats, a root) and cannot
  panic whatever the element (`total`: no call, no slice; a read of an
  array of known length at an index whose bounds, from `int_bounds`, are
  inside it: `codes[min(count(...), n - 1)]`, a count being a sum of 0
  and 1 over an array of known length), the `map` becomes `if length >= 2
  * len(t) then (let H = map(t, h) in the map with H[i]) else the map`:
  `h` once per element of `t`, then a read per element. The same
  operations on the same values: the same bits; `H` has the length of `t`,
  so `H[i]` panics where `t[i]` did. The `n` of `iota(n)` bound once, the
  test of a size around the input looked through. Of the ten programs only
  fasta changes. At fixed frequency, best of 7: fasta 0.135 s -> 0.047 s
  on 4 threads, 0.520 s -> 0.085 s on one; the same output (also for N =
  1, 7, 1000). Its floor (D222: 2e8 numbers of the generator) is to be
  redone: the program needs one period and copies. Test
  `functions_of_table_elements_computed_once`.
- **D231 — The messages of `iscc` in colors (2026-11-04).** Asked for:
the output of a compilation, with or without errors, in colors. As rustc
and cargo: the place (`file:line:col`) in bold, `error` in red, the notes
in cyan, then the line of the source with a caret under the column (in
blue the margin and its number); after a build, `compiled file -> output
in N ms` in green. Only when the error output is a terminal and
`NO_COLOR` is not set (https://no-color.org), or with `--color always`;
`--color never` and pipes give the text of before, unchanged, which the
tests and scripts read. The messages of the compiled programs (their
panics) do not change: they are part of the behavior the tests check.
Test `messages_in_colors_on_demand` (tests/cli.rs).
- **D232 — The completion of `iscc` in the shell (2026-11-04).** Asked
for: completion when typing `iscc ...`. `iscc completions bash|zsh|fish`
prints the script of the shell, kept in the driver (no file to install
apart): the commands (`check`, `ir`, `run`, `build`, `completions`), the
options of each, the values of `--isa`, `--target`, `--color`, `--lanes`,
any file after `-o`, `--emit-llvm`, `--emit-asm`, `--link`, else the
`.isc` files and the directories. zsh takes the script of bash through
`bashcompinit`. Test `completion_of_the_shell` (the script of bash run in
bash on 7 cases).
- **D233 — The floor of fasta from one period of its generator
  (2026-11-04).** After D230 fasta took 0.047 s, below nothing but already
  0.8 of its floor of D222 (2e8 numbers of the generator, 0.037 s): that
  floor assumed every number computed, though the generator has a period
  of 139968 values. The floor now: one period and the symbol of each value
  for the two tables (3 x 139968 numbers at the cost of `fasta_number_iub`,
  1.029 ns), then the 254 166 745 bytes copied from tables in lines of 60
  and a newline, into blocks that stay in the caches (a new primitive,
  `table_rows`: 0.0347 ns a byte per core on 4 threads): 0.002 s. The
  measured programs: Isocron 0.049 s in Docker (20 times its floor), Rust
  0.755 s (326 times).
- **D234 — The bytes written by blocks through the choice of a table
  (2026-11-04).** After D230, half of the processor time of fasta on 4
  threads was the clearing of new pages by the kernel (`clear_page_erms`),
  in the tasks that fill the lines of the two random sequences: D230 put
  their `map` inside `if length >= 2 * len(t) then (let H = ... in map)
  else map`, under the `let`s of the length and its test
  (`check_non_negative`), and D225 recognized only `let x = map(...) in
  printBytes(x)`: the lines (about 200 MB) were made whole again, into new
  pages. The code generator now looks through the choice (one branch of
  the shape is enough: the other is made whole, written, freed), the
  `let`s (bound, then freed as they free) and the test of a length
  (`print_stream_any`). Of the ten programs only fasta changes. At fixed
  frequency, best of 7: fasta 0.048 s -> 0.017 s on 4 threads, 0.085 s ->
  0.061 s on one; the same output (N = 1, 7, 1000, 25e6). Test
  `bytes_written_by_blocks_through_a_table`.
- **D235 — The Markdown made for GitHub (2026-11-04).** Asked for: the
floor in the tables of the benchmarks, colors in every Markdown file, a
better look on GitHub. GitHub keeps no color of HTML or CSS (it removes
`style`), but renders the colors of math (`$\color{green}{...}$`, also in
tables), emoji, its alerts (`> [!NOTE]`, `[!TIP]`, `[!IMPORTANT]`,
`[!WARNING]`, `[!CAUTION]`), Mermaid charts and `<details>`. `bench/mdstyle.py`
holds the look: the readable names of the programs, Isocron over the
fastest in green below 0.95, orange to 1.05, red beyond, with a mark (🟢
🟡 🔴), a time over its floor in green up to 1.5, orange up to 3, red
beyond, a bar chart, the marks of the verdicts of SPEC section 14 (🟢
*beats*, 🟡 *equals*, 🔴 *slower*). `report.py` gives the floor of each
program (`results/floors.tsv`, now written by `floors.py`) and Isocron
over it, a chart of Isocron over the fastest, the plain programs and what
the others use in `<details>`, and writes the summary of the results into
`README.md`, between `<!-- results:begin -->` and `<!-- results:end -->`.
`floors.py` colors the gaps and charts them. `README.md` is laid out again
(the summary of the results first, alerts, `<details>` for the long
options); `results/README.md` is the index of the results;
`docs/language.md` has alerts and the commands of D231 and D232. The
reports of the campaigns of the proof of concept keep their numbers as
they were: only the marks of their verdicts are added (made again from
their JSON, they would change: their scripts changed since), and their
scripts add them from now on. `DECISIONS.md`, `TODO.md` and `SPEC.md`
keep their text.
- **D236 — A replacement written by every thread (2026-11-04).** After
D224 the matches of a replacement were found in parallel (D189), but its
result was built by one thread (`replace_bounds`), a piece of text and the
replacement per match: three replacements of regex-redux have millions of
matches. When the replacement has no `$` (each one the same bytes) and
there are at least 16384 matches, the matches are cut into one part per
thread; the size of each part of the result is known before (its text
between the matches, then `to` per match), and each thread writes its
part at its place (`replace_parallel`). The bytes of `replace_bounds`.
At fixed frequency, 4 threads, best of 5: regex-redux 0.475 s -> 0.336 s;
by step: the cleaning 66 ms, the 9 counts about 91 ms, the 5 replacements
about 184 ms. Test `replacements_written_in_parallel` (the whole texts of
the interpreter, 1, 4 and 16 threads, with `$&` the path before).
- **D237 — Alternations of simple shapes (2026-11-04).** The cleaning of
regex-redux, `/^>.*\n|\n/mg` replaced by nothing, went through the
automaton (66 ms with the reading on 4 threads): two alternatives of
other kinds, the first at the start of a line. The engine of masks (D224)
now takes an alternation of alternatives each fixed or delimited, of any
length from 1 to 64, at the start of a line (`^` with `m`) or anywhere: a
mask of starts per alternative (the starts of lines from the mask of `\n`
shifted by one, the byte before the block for its first place), and at a
place the first alternative that matches there is the match (the order of
a backtracking engine: `a|ab` matches `a`); a delimited alternative with no
stop byte left is dead for the rest of the search. Its order is the order
of the cross products of `expand`, the one of the automaton. At fixed
frequency: the cleaning 0.066 s -> 0.030 s; regex-redux 0.336 s -> 0.322 s
on 4 threads, 0.791 s -> 0.714 s on one; the same outputs, with the
runtime without AVX2 too. Tests: the shapes and the matches of the
automaton (isc-runtime-core) on such alternations,
`alternations_of_shapes_found_by_masks` (strings).
- **D238 — The masks of the engine of shapes, tighter (2026-11-04).** On
one thread the engine of shapes (`Simple::bounds_from`) took 64 % of
regex-redux: about 5.8 instructions and 0.56 ns a byte for the 9 counts
(the primitive of the floor: 0.074 ns). Its loop checked the bounds of
each index of class, and compared each letter twice when the pattern
ignores case. A lower case letter whose upper case is in the same classes
is now compared once, with the bytes `| 0x20` (only these two bytes give
it); the indices of classes and of alternatives, below their bounds by
construction, are masked, which drops the checks. At fixed frequency,
alternated in one series: regex-redux 0.324 s -> 0.305 s on 4 threads,
0.716 s -> 0.650 s on one; the same outputs, with the runtime without AVX2
too. Tried and not kept: each class computed in registers from its bytes,
then its mask once (no write per byte): 0.312 s and 0.677 s, worse, the
bytes of overlapping classes compared again for each class. The engine is
still 7 times the cost of the primitive: its loop of starts, the vectors
of the bounds of dense matches (millions of pairs written, merged, read
again), compares of 32 bytes where AVX-512 gives a mask of 64 at once.
- **D239 — The product of a big integer by a word in assembly
  (2026-11-04).** pidigits: `limbs::mul_word` 53 % of the time, `big256::op`
  38 % (its additions, already one chain of `adc`), 8.7e9 instructions at
  3 a cycle. The loop of `mul_word` (Rust, `u128`) used `mulx` but chained
  an addition and an `adc` per word: the carry waited two cycles a word.
  An earlier try in assembly (before D222) was dropped because the loop
  was then at the floor of the time, 0.74 ns a word, which was the cost of
  a loop written in C; the primitive of D222 in assembly costs 0.33 ns.
  Where `mulx` is (the runtime for x86-64-v3), the words by 4 are now that
  loop: the products of 4 words, the high word of each added to the low
  word of the next in one chain of `adc` through the flags; the rest of
  the words and a partial last word in Rust, from its carry. The runtime
  without BMI2 and WebAssembly keep the loop of Rust. pidigits at fixed
  frequency: 0.888 s -> 0.697 s; the same digits for N = 1, 27, 1000,
  10000, with both runtimes. Test `big_integers_times_a_word` (1 to 41
  words, all ones, signs, small and large words).
- **D240 — A quotient of one word certain from the top words
  (2026-11-04).** The division whose quotient is one word (D196) estimated
  it from the three top words of the dividend and the two of the divisor
  (Knuth's step D3: the estimate is the quotient or one more), then always
  checked it by a pass over all the words (`a - q b >= 0`). pidigits
  divides twice per digit tested. The lower words of the divisor add less
  than one unit of its second word: when `rhat * 2^64 + u0 >= qhat * (v2 +
  1)` (the three top words of the dividend exceed `qhat` times the two top
  words of the divisor plus one), `qhat * b <= a`, and since the estimate
  is never below the quotient, it is the quotient: no pass. Otherwise
  (rarely: the dividend just below a multiple of the divisor) the pass as
  before. pidigits at fixed frequency, alternated: 0.696 s -> 0.503 s; the
  same digits for N = 1, 27, 1000, 10000. Test
  `quotients_at_the_edge_of_the_estimate` (more than 3000 dividends at
  `q b`, `q b + 1`, just below `(q + 1) b`, at `(q + 1) b`; divisors whose
  low words are all ones or zero), against num-bigint.
- **D241 — A runtime for x86-64-v4 (2026-11-04).** The generated code
uses AVX-512 on this machine, the runtime only AVX2 (x86-64-v3, D188):
the engine of shapes (D224) needed two comparisons of 32 bytes, two
`vpmovmskb`, a shift and an or for the mask of 64 bytes that one
`vpcmpeqb` into a mask register gives. `build.rs` now builds a third
runtime for x86-64-v4, which `iscc` links for `native` and `avx512` on a
host with AVX-512 F, BW, CD, DQ and VL (`--isa avx2`, asked by the tests
under valgrind, which has no AVX-512, keeps the one for v3); the engine
has a version of its comparisons for it. First measure: pidigits 0.506 s
-> 0.746 s: with AVX-512 LLVM turned the loop of the additions of big
integers (Rust, a carry per word) into vector code that carries through
masks, much slower than one chain of `adc`. The additions and
subtractions of words are now in assembly on every x86-64 (by 4 words,
one chain of `adc` or `sbb` through the flags), whatever LLVM chooses. At
fixed frequency, 4 threads but pidigits: regex-redux 0.305 s -> 0.294 s,
pidigits 0.503 s -> 0.490 s (the runtime for v3 too: 0.490 s; the one
without BMI2 0.822 s), k-nucleotide 1.153 s -> 1.141 s, the seven others
unchanged (n-body, alternated: 2.057, 2.056, 2.018 s against 2.067,
2.029, 2.020 s); the same outputs. The test of the shapes also runs with
the runtime for v3 (`--isa avx2`).
- **D242 — k-nucleotide by the rules: every k-mer counted (2026-11-04).**
The rules of the Benchmarks Game ask, for the lengths 3, 4, 6, 12 and 18,
to count all the k-mers of the sequence in a table, then to read the
given ones in it; the programs in C, C++, Rust and Go do so. The Isocron
program (D160) counted only the 5 given k-mers, one scan each (`count(...,
kmer == target)`): less work than the others, and its result of the
publications (0.98 of the C++ program at D240) was not a comparison of
the same work. It now counts every k-mer of each length with `countBy`
and reads the query with `get`; the same output. At fixed frequency:
4 threads 1.141 s -> 3.061 s, one thread 1.757 s -> 4.867 s, 2.6 times
the C++ program. The publications from here on measure this program.
- **D243 — Groups of integer keys with the key in its slot, in parallel
(2026-11-04).** Profile of k-nucleotide by the rules (D242): `isc_group`
53 % of the cycles, `clear_page_erms` 20 %. `group` keeps the index of a
group in its slot and compares the key of its first occurrence in the
array of keys: a random read of memory per key once the array is large
(1 GB of `i64`), then the arrays of the last index and the count.
`WordGroups` keeps the key, its last index and its count in its slot (16
bytes, one line per probe and update; indices and counts below 2^32,
`group` beyond); the slot of the key 16 places ahead is prefetched.
From 2^20 keys on, one table per part of consecutive indices (in a task
too: the parts shared with the idle threads, D201), the tables merged in
the order of the parts: the groups in the order of their first
occurrence, the columns of `group` whatever the number of threads.
k-nucleotide at fixed frequency: 4 threads 3.057 s -> 2.563 s, one
thread 4.869 s -> 4.341 s; the same output.
- **D244 — `countBy` groups its keys as they are made; an `iota` read in
place is never made (2026-11-04).** After D243, each `countBy` of
k-nucleotide still wrote its 125 million keys (1 GB, new pages: the
`clear_page_erms` of the profile) before grouping them, and its input
`iota(n)` was bound to a variable, read twice, so made whole (1 GB more)
and read by the `map` as an array: the index of each lane a load, the
windows of `codes` a gather. Three changes:
  - `countBy` (prelude) no longer reads its keys twice: `$group(map(xs,
    f))`, the key of each group made again from its first element
    (`f(xs[g[i]])`, at most one more `f` per distinct key).
  - `$group` of a `map` of integer keys (codegen `group_stream`): the
    parts of `isc_group_parts` (one per thread from 2^20 keys, at least
    2^18 keys each) in parallel, each making its keys by blocks of 1024 in
    a buffer on its stack (`fill_range`, the vector code of the `map`) and
    adding each block to its own table (`isc_groups_add`); the tables
    merged in their order (`isc_groups_end`). From 2^31 keys on, the keys
    made whole and `isc_group`. From 2^16 keys on, the keys below 2^16 are
    counted in a table indexed by the key, which grows to the largest of
    them (what `group_dense` did for an array of keys).
  - `iota_vars` (pass, after the first fusion): `let v = iota(n)` whose
    every use reads it in place (an input of a loop, its length, an
    element, the source of a slice, inside lambdas too) becomes `let len
    = n` with each use `iota(len)`: a producer computed where it is read.
    Of the ten programs, only k-nucleotide has one (6).
k-nucleotide at fixed frequency: 4 threads 2.563 s -> 1.659 s, one thread
4.341 s -> 3.706 s; the same output for 1, 2 and 4 threads; the other
nine programs unchanged (no `countBy`, no `iota` rewritten). The tables
of the two large frames (12 and 18: about 140 000 keys each, 8 MiB per
part at half load) still take 4 times the memory of one table on 4
threads (32 MiB for a 16 MiB L3: 15.7 ns per key on 4 threads, 6.6 on
one); a load of 3/4 halves it (k=12 alone: 0.981 s -> 0.856 s on 4
threads) but costs one thread 0.437 s in branches mispredicted (25 to 64
million): left at 1/2 here, the tables to be split by key next. Test
`keys_grouped_as_they_are_made`.
- **D245 — The keys of a block hashed once, the table in registers
(2026-11-04).** Counters of k=12 alone (`countBy` of 125 million keys
into 138 127 groups) after D244, one thread: 21 cycles and 71
instructions per key over reading the input; on 4 threads, 50 cycles.
`isc_groups_add` hashed each key twice (its prefetch, then its probe)
and read the table, its mask and the flag of small keys from memory at
every key (the pushes of a new group might change them). `add_block`
hashes the keys of a chunk of 1024 first, then adds each one while the
slot of the key 16 places later is prefetched, the table and its mask
in registers between two growths; keys of 64 bits are added where they
are, narrower ones widened by chunks of 256. k=12 alone, one thread:
4.82e9 -> 4.31e9 cycles, 12.99e9 -> 10.52e9 instructions. k-nucleotide
at fixed frequency: 4 threads 1.666 s -> 1.601 s, one thread 3.700 s ->
3.538 s; the same output. Two other ways, measured and not kept:
  - the tables split by key once a part finds more than 2^15 distinct
    keys in the first eighth of its range (every part making every key
    and keeping those of its class): the misses of the L3 halved on 4
    threads (11.1e6 -> 6.0e6) but the instructions doubled (13.4e9 ->
    29.0e9), the cycles the same (9.41e9 -> 9.22e9); k-nucleotide 1.654 s
    -> 2.126 s (its local arrays zeroed at every block cost one thread
    0.29 s too).
  - a load of 3/4 instead of 1/2 (4 MiB per part instead of 8): 4
    threads 1.624 s -> 1.590 s, one thread 3.538 s -> 4.534 s, branches
    mispredicted 25e6 -> 65e6 (k=12: the probes of more than one slot,
    1.18 -> 1.55 per key, measured by `probe.c` of the session on the
    keys themselves).
  On 4 threads the excess over one thread (8.79e9 against 4.31e9 cycles
  for k=12) is mostly the misses of the L3 of the 4 tables of 8 MiB
  (15.3e6, the memory): one table shared by the parts, or slots that fit
  more keys per line, are next.
- **D246 — A target for Windows, tested on a real Windows (2026-11-04).**
  `iscc build --target x86_64-windows` (also `x86_64-pc-windows-gnu`,
  `windows`) writes a console executable for Windows, the code generated
  for the same instruction set as native code (`--isa`), LLVM's triple
  `x86_64-pc-windows-gnu`. Of the two ABIs of Windows, GNU (MinGW-w64) and
  MSVC, the code of the program is the same (LLVM, the convention of
  Win64); MinGW-w64 is free and builds from Linux (Debian packages), MSVC
  needs the SDK of Microsoft: MinGW-w64 first, MSVC when `iscc` runs on
  Windows itself.
  - The runtime (`crates/isc-runtime/src/windows.rs`) keeps the functions
    of Linux it calls, with the same names and meanings, over the Win32
    API: files, standard streams and `pread` by handles (`CreateFileW` of
    the UTF-8 path, `ReadFile` with an offset, `WriteFile`: bytes as they
    are, no text mode), the futex of the pool by `WaitOnAddress` and
    `WakeByAddressAll`, threads by `CreateThread` (the stack of 1 GiB of
    D102 reserved), the CPUs by the affinity of the process, the clock by
    `QueryPerformanceCounter`, the stack floor by
    `GetCurrentThreadStackLimits`, the end of a thread by a fiber local
    slot (`FlsAlloc`). The memory of the heap: `VirtualAlloc` reserved and
    committed (Windows does not commit on first touch), a part given back
    decommitted and the address space released once nothing of it is
    committed, MADV_DONTNEED as decommit and commit (zeros, as on Linux),
    `mremap` as a copy (Windows cannot move pages [only true of `VirtualAlloc`: D375]), the advice on huge
    pages ignored. The thread caches of the heap and of the nodes are in
    the section `.tls$`, read through `gs:[0x58]`, `_tls_index` and
    `@SECREL32` (the generated code reads `isc_node_thread` the way LLVM
    reads any thread-local variable of COFF). Errors: the last Win32 error
    mapped to its POSIX code, with the text of glibc, so that the messages
    are those of Linux. The C library of MinGW (`msvcrt`) gives only
    `strlen`, `getenv`, `exit`, `abort`, `_aligned_malloc`, `memcpy`.
  - Linking: `rust-lld` as the linker of MinGW (`-m i386pep`), `crt2.o`,
    `crtbegin.o`, `crtend.o`, `libgcc` and the import libraries of
    MinGW-w64 (`$ISC_MINGW_DIR`, else `/usr`); a stack of 8 MiB for the
    main thread, as on Linux. The program imports `KERNEL32.dll`,
    `msvcrt.dll` and `api-ms-win-core-synch-l1-2-0.dll`, nothing else.
  - Tests (`crates/iscc/tests/windows.rs`): each program built for Linux
    and for Windows and run on both, on 1 and 4 threads, with the same
    standard output, error and exit code: printing, threads, floats,
    UTF-8, maps, sorting, regular expressions, big integers, a recursion
    a million deep, files (one of 9 MiB: the reads in parallel), the
    standard input, a missing file, a panic, a stack overflow, and the ten
    programs of the Benchmarks Game. The Windows machine is a real
    Windows 11 Enterprise (evaluation) in a virtual machine (QEMU/KVM in
    the container `dockurr/windows`, 4 of the CPUs 8 to 15, its disk
    outside the repository, SSH and its screen on the loopback only),
    reached by `tools/windows/run.py` (`$ISC_WINDOWS_RUNNER`, its key
    `$ISC_WINDOWS_KEY`); without it, the tests are skipped with a message.
    Wine was considered and not kept: it checks the format, the ABI and
    the logic, not Windows itself. A program still running after
    `$ISC_WINDOWS_TIMEOUT` seconds (300) is stopped there (code 124).
  - On the Windows machine (build 26100, 4 virtual CPUs) both tests pass
    (17.57 s): every output, error and exit code the same as on Linux, on
    1 and 4 threads. A first program of the test, `1 + down(n + 1)`,
    never overflowed on either system: LLVM makes that recursion a loop
    (an accumulator), which is allowed (the program never ends either
    way); the test recurses twice (`down(n + 1) + down(n + 2)`).
  - Not yet: the runtimes for x86-64-v3 and v4 (AVX2 and AVX-512 searches,
    D188, D241) for Windows, the console in UTF-8 (`SetConsoleOutputCP`),
    `iscc` itself on Windows, the MSVC ABI, timings on Windows.
- **D247 — iscc on Windows (2026-11-04).** `iscc.exe` (target
  `x86_64-pc-windows-gnu`), built on Linux by `tools/windows/build_iscc.sh`
  (the gcc of MinGW-w64 as its linker; `LLVM-C.lib`, the import library of
  the official LLVM 21 for Windows): on Windows, native code is code for
  Windows (`Options::windows` is `cfg!(windows)`, the default name of a
  program `<file>.exe`).
  - The runtimes of native code are those of the target of iscc (build.rs:
    `--target $TARGET`), so that iscc.exe embeds the runtime of Windows.
  - The files of MinGW-w64 its linker needs (`crt2.o`, `crtbegin.o`,
    `crtend.o`, `libmingw32`, `libmingwex`, `libmsvcrt`, `libmoldname`,
    `libkernel32`, `libsynchronization`, `libgcc`, `libgcc_eh`: 8.48 MB)
    are embedded in iscc when it is built (from `$ISC_MINGW_DIR`), on Linux
    too: building for Windows then needs no MinGW either. The linker:
    `$ISC_LLD`, else the `rust-lld` of Rust, else `ld.lld` of LLVM (the
    `PATH`, or `C:\Program Files\LLVM\bin`). An iscc.exe needs only LLVM
    21 for Windows: `LLVM-C.dll` (it exports the 35 functions of the C API
    the back-end calls) and `ld.lld.exe`.
  - The interpreter on Windows: the C functions of `declare function` are
    called by a few lines of assembly for Win64 (the position of an
    argument gives its register; each of the first four goes in its
    integer and its vector register, as for a variadic call; the others on
    the stack after the 32 bytes of shadow space), looked up in the
    program, `ucrtbase`, `msvcrt`, `kernel32` and the DLLs of `--link`
    (`LoadLibraryW`, `GetProcAddress`). The errors of files: the Win32 code
    mapped to the POSIX one with the text of glibc (`isc_runtime_core::
    oserr`, shared with the runtime of Windows), the messages of Linux.
  - Test `iscc_on_windows` (`$ISC_WINDOWS_ISCC`: the iscc.exe to test, with
    LLVM 21 in `$ISC_WINDOWS_LLVM_BIN` of the Windows machine): `iscc
    build` then the program, and `iscc run`, on the Windows machine of
    D246, against the Linux program: printing and threads, files, the
    standard input and a missing file, a panic; C functions of Windows,
    `MultiByteToWideChar` with six arguments (two on the stack), in both.
    Passes. The LLVM of the machine: 21.1.8, the version of the host.
- **D248 — An iscc that needs nothing else, on Linux and on Windows
  (2026-11-04).** Before: the shared library of LLVM 21 (on Linux, 15
  shared libraries for the iscc built here, `libLLVM.so.21.1` and those it
  needs; `LLVM-C.dll` on Windows), a
  linker (`cc` and the development files of glibc, `rust-lld` or
  `ld.lld`), the WASI C library of the Rust targets, and a glibc of at
  least 2.39 for the iscc built here.
  - LLVM 21.1.8 and LLD built from their sources as static libraries
    (`tools/release/build_llvm.sh`): the targets x86 and WebAssembly, LLD
    for ELF, COFF, MinGW and Wasm, no compression, XML, line editing,
    terminfo or RTTI; for Linux in `manylinux_2_28` (AlmaLinux 8, glibc
    2.28: 117 libraries), for Windows cross-compiled by MinGW-w64 in
    Debian 13 (119). Debian 11 was the first choice, but its repositories
    are archived.
  - `isc-codegen` with `ISC_LLVM_STATIC`: the 103 libraries of `llvm-config
    --libnames all` (an archive gives only the members used), LLD's drivers
    and `libstdc++` statically; `src/lld_shim.cpp` calls `lld::lldMain`
    with the four drivers; `isc_codegen::lld::link` runs it in the process,
    one link at a time (LLD keeps state between links; the tests link from
    several threads). Without `ISC_LLVM_STATIC`, the shared LLVM and the
    linkers of the system as before.
  - iscc embeds what LLD links with: for Linux, the start files of glibc
    of the build (`Scrt1.o`, `crti.o`, `crtn.o`, and `libc_nonshared.a`,
    whose `__libc_csu_init` and `__libc_csu_fini` the `Scrt1.o` of glibc
    2.28 calls; from glibc 2.34 on, `__libc_start_main` ignores them [contradicted for the init: D375] and
    runs the constructors itself), a program then linked with the
    `libc.so.6` and `libm.so.6` of the machine (no development files: the
    program uses the glibc of the system, as with `cc`); the MinGW-w64
    files of D247; the WASI C library of both WebAssembly targets. The
    precompiled `alloc` of Rust calls `_Unwind_Resume` in its cleanups,
    never run with `panic = "abort"`: the runtime of Linux defines it (an
    abort), `libgcc_s` is not needed.
  - The same code: the assembly that our LLVM gives for knucleotide,
    nbody, mandelbrot and regexredux is the one of Debian's LLVM 21.1.8,
    byte for byte.
  - The iscc for Linux: 125 004 856 bytes (its symbols removed), it needs
    a glibc of at least 2.28 and `libgcc_s`. `tools/release/selftest_linux.sh`:
    in Debian 12 slim, Ubuntu 20.04 (glibc 2.31) and AlmaLinux 8 (glibc
    2.28), without compiler, linker, development files or network, it
    builds the ten programs of the Benchmarks Game, whose outputs are
    those of the programs built here, and builds for Windows and
    WebAssembly. iscc.exe: 109 575 168 bytes, it imports only DLLs of
    Windows (`KERNEL32`, `ADVAPI32`, `SHELL32`, `ole32`, `USERENV`,
    `WS2_32`, `ntdll`, `bcryptprimitives`, `msvcrt`, the synchronization
    API set); `tools/release/selftest_windows.py`: on the Windows machine
    of D246, LLVM made absent, the ten programs with the outputs of Linux,
    `iscc run`, WebAssembly.
  - Most of the size: the embedded runtimes (six, 5.2 to 7.8 MB each),
    then LLVM and LLD. Compressing the runtimes is left for later.
- **D249 — The first release, 0.1.0 (2026-11-04).** The license of the
  workspace (MIT, `Cargo.toml`) now has its file, `LICENSE`, its holder
  "The Isocron authors" (those of the history). `tools/release/package.sh`
  makes the two archives of a release, `isocron-VERSION-x86_64-linux.tar.gz`
  and `isocron-VERSION-x86_64-windows.zip` (the compiler of D248, `LICENSE`,
  `THIRD_PARTY_LICENSES.md` and the texts of the licenses of what the
  compilers contain: LLVM and LLD, the Rust standard library and the crates,
  MinGW-w64, the GCC runtime, the start files of glibc, wasi-libc;
  `README.md`, `CHANGELOG.md`, `docs/language.md`, the examples) and their
  `SHA256SUMS`. The measurements of the Benchmarks Game published again
  for it, at commit `5d0d2b0` (D248): the table of the README dated from
  D240, whose k-nucleotide (1.148 s) was the program that did less work
  than the rules ask (D242). Now: Isocron the fastest on 7 of the 10
  programs, k-nucleotide 1.603 s against 1.182 s for C++ (1.36), n-body
  1.04 times C, reverse-complement 1.58 times Rust; binary-trees 0.678 s
  -> 0.669 s, pidigits 0.503 s -> 0.490 s, regex-redux 0.305 s -> 0.295 s
  (D241), the others within their noise. The C++ program mandelbrot gpp-4
  still gives another image than the reference at N = 200 (D227), the
  same as Isocron at N = 16000.
- **D250 — The programs for Windows keep only the functions they use
  (2026-11-04).** A hello for Windows was 1 517 568 bytes, the same hello
  for Linux 38 512: its code (`.text`, 1.04 MB) held the engine of regular
  expressions, the big integers and most of the runtime, which the linker
  removes on Linux (`--gc-sections`). Rust puts the code of a crate in one
  section on the targets `*-windows-gnu` (the object of `isc_runtime`: 1
  section `.text`, against 1193 for Linux): the linker keeps all of it or
  nothing. The runtimes for Windows are now built with
  `-Zfunction-sections=yes` (an unstable flag, allowed on the stable
  compiler for this build only by `RUSTC_BOOTSTRAP=1`): the hello 54 272
  bytes, regex-redux 1 522 176 -> 1 226 240 (it uses the engine). The
  precompiled `core` and `alloc` of Rust keep their sections as built.
  - A failure of `programs_on_windows` (its standard error) in the first
    series after a long pause, not in the three next: `run.py` shares one
    SSH session (ControlMaster); with none open, the `run.py` started at
    once by the tests race to open it, and the losers print `ControlSocket
    ... already exists, disabling multiplexing` on their standard error,
    taken for the program's. Reproduced: 6 runs at once without a session,
    5 such lines with the old `run.py`, none now: the commands around the
    program (`mkdir`, `scp`) keep their messages, shown only on failure,
    and a session whose connection died is closed first.
- **D251 — Release 0.1.1 (2026-11-04).** The programs for Windows of D250,
  and `iscc --version` (also `-V`, `version`): `iscc 0.1.1 (x86_64-linux)`
  or `(x86_64-windows)`, the version of the workspace. The programs for
  Linux and WebAssembly are those of 0.1.0 (the flags changed only for the
  runtimes of Windows): the measurements of D249 stand. Test
  `version_of_the_compiler`.
- **D252 — The lines of a text in parallel, the small blocks refilled by
  batches (2026-11-04).** k-nucleotide spent 473 ms before its first
  count: reading 30 ms, `readStdinLines` 323 ms more (4.2 million lines,
  one new string each, on one thread, in two passes), the headers 54, the
  join 38, the codes 19 (4 threads).
  - `isc_lines` of a text of 1 MiB or more by parts in parallel: each part
    counts its `\n` (memchr), a line starts at the start of the text and
    after each `\n` (the part of the `\n` owns the line after it), the
    counts give the index of the first line of each part, each part makes
    its strings; the same array for any number of threads. Alone, slower
    (355 -> 605 ms): `heap::refill` 83 % of the cycles. The blocks of a
    refill were carved one by one, each under the lock of the chunks
    (one for all classes), at most 16 per refill.
  - The allocator: a thread keeps up to 256 blocks of a class (32 before;
    still at most 128 KiB), half of them taken per refill; the new blocks
    of a refill carved under one lock of the chunks (`carve_many`; the
    blocks of 16 KiB or more keep the placement of `carve`). The lines of
    k-nucleotide: 354 -> 190 ms. What is left of `refill` (39 %) is the
    first write of the headers of new blocks: a line of 60 bytes takes a
    block of 128 with its header of 64 bytes.
  - At fixed frequency, 4 threads, the ten programs built before and after
    (one series): k-nucleotide 1.607 s -> 1.453 s, regex-redux 0.303 s ->
    0.291 s, the eight others unchanged (within 2 ms). Test
    `lines_of_a_large_text_made_in_parallel` (lines of 0 to 300 bytes,
    empty, `\r\n`, runs of `\n` across the cuts of the parts, with and
    without a final `\n`; 1, 3 and 4 threads).
- **D253 — The large tables of `countBy` scattered by rounds; the keys of a
  block added in two passes (2026-11-04).** k-nucleotide after D252: 68 %
  of its cycles in `add_block`. Cycles and instructions per key over the
  reading (4 threads): small keys (k = 1) 7.7 and 36, large ones (k = 12)
  38.9 and 110.
  - The block in two passes: the small keys counted at once (no hash, no
    prefetch: they were hashed and prefetched as the others), the others
    then hashed and probed; the buffers of a block allocated once per
    table (zeroed at each call before: 24 bytes a key). The groups are no
    longer made in the order of their first index (the small keys of a
    chunk before its hashed ones): `columns` sorts them by it when they are
    not (a first version without it gave other orders: test
    `keys_grouped_as_they_are_made`).
  - Several tables at once (the parts of a `$group` on several threads):
    once a part's table has more than 2^15 hashed groups (beyond the L2),
    its next keys that fit with their index in 64 bits are scattered by the
    high 8 bits of their hash into 256 lists of `key << bits | index`, in
    pieces of 1024 entries reused from round to round; every 2^20 entries
    (8 MB a part) each list is counted at once into the small table of its
    list (a few thousand entries into a table in the L1). At the end, list
    by list in parallel, part after part (its groups of its table, then of
    its list), then every group by its first index. A key too large to fit
    stays in the table. One part alone does not scatter (its table stays in
    the L3: one thread 3.457 s -> 4.426 s when it did).
  - Measured and not kept, with a prototype in C on the keys of
    k-nucleotide (k = 12, 4 threads, the keys computed by a scalar loop):
    a table per thread 577 ms; every key scattered then counted 287 ms (in
    iscc: 1 GB of new pages per frame, `clear_page_erms` 17 % of the
    cycles); the tables split by key 802 ms; small buffers poured into
    tables shared under locks 1.537 s for k-nucleotide in iscc (the lines
    of the tables carried from core to core); poured into private tables
    677 ms; slots of 8 bytes compared 8 at once (AVX2) 391 ms.
  - k-nucleotide at fixed frequency: 4 threads 1.454 s -> 1.115 s (under
    the 1.182 s of C++ in Docker at D249), one thread 3.478 s -> 3.156 s;
    the same output for 1, 2, 3, 4 and 7 threads. Per key (4 threads): k =
    1 3.5 cycles, 21 instructions; k = 12 28.8 cycles, 87 instructions.
    Test `keys_scattered_by_their_hash` (3 million keys, about 200 000
    large ones, small and negative ones; 1 and 4 threads; the order of the
    first occurrences and the counts).
- **D254 — The loops that fill an array give each thread a contiguous range
  of their blocks (2026-11-04).** reverse-complement after D253, 4 threads:
  104.3 ms; by phase (the program stopped after each): the read 29.6 ms,
  the headers 12.9 ms, the filter of the bases 42.8 ms, the lines and the
  output 20 ms. The filter, by its cost model (two reads of 254 MB at 47.3
  GB/s, 250 MB of new pages at 18.0 GB/s): 25 ms; one thread 54 ms, so 4
  threads barely helped; 2788 page faults against 1761 with one thread;
  `clear_page_erms` 30 % of the cycles. The static share gave task k to
  thread k mod T: with blocks of 4096 bytes, the 4 threads wrote the same
  huge page of the new array at once, its first write faulting it while
  the others waited for the 2 MiB to be cleared.
  - `isc_parallel_for_fill`: thread k takes the tasks `n k / T .. n (k +
    1) / T` (`static_share`, in isc-runtime-core, with its test). Used by
    the blocks of a `map` (not those whose elements are loops: dynamic,
    D145), the writing pass of a `filter`, and the chunks of a printed
    `map` (`print_stream`). The other loops (reductions, the counting pass
    of a `filter`, scans) keep every T-th task: reading arrays already
    there, the threads stay close in memory (D65).
  - Every static loop by ranges was measured too: reverse-complement 89.4
    ms, but the large `sum`, `dot` and `std_dev` 0.6, 1.3 and 1.1 % slower
    with 4 threads, 0.9, 2.2 and 2.5 % with 8 (DRAM pages, as D65 found;
    `dot` with 8 threads would fall behind C in `results/xeon.md`). By
    ranges only when the new array had no page written yet (a flag of the
    allocator): 103.2 ms, the bases of a record taking the block of the
    shorter record before (not new, its end new): not kept.
  - Measured (fixed frequency, best of 11, 4 threads): reverse-complement
    103.5 -> 89.9 ms (stopped after its filter 85.3 -> 74.3 ms, the
    faults 2706 -> 1780),
    k-nucleotide 1121.2 -> 1107.4 ms; the 8 others within their noise
    (regex-redux 293.0 -> 294.7 ms: its loops are in the runtime, the same
    in both). The kernels, large arrays, 4 threads: `sum` +0.2 %, `dot`
    -0.1 %, `moving_average` -0.5 % and -1.1 % (B), `filter_sum` +0.7 %,
    `prefix_sum` -0.3 %. Test `arrays_filled_by_ranges_do_not_depend_on_threads`
    (maps and filters of 12 to 54 MB, new and from the cache, a printed
    reversed map; 1 to 8 threads; the calls in the IR).
  - A run of the kernels gave baselines up to 1.8 times slower (the small
    `histogram` 42856 ns against 23993 ns) for both versions alike: a
    service scanning media on the sibling hardware threads of the cores
    measured (`machine.sh apply parallel` confines every other process to
    them). Remeasured once it had stopped: 24061 ns.
- **D255 — The writing pass of a filter skips the blocks that keep nothing
  (2026-11-04).** The headers of reverse-complement, `filter(iota(n), (i)
  => data[i] == 62u8)` over 254 MB, took 12.4 ms with 4 threads for 3
  elements: the counting pass and the writing pass each read every block
  (two reads at 47.3 GB/s: 10.8 ms by the model). A block keeps the
  elements between its place and the place of the next block (the number
  of elements kept after the last one): when they are equal, its writing
  task returns at once.
  - Measured (fixed frequency, 4 threads): the headers of
    reverse-complement 12.4 -> 6.8 ms (one read: 5.4 ms by the model); the
    program 90.9 -> 85.5 ms (2.06e8 instructions fewer); k-nucleotide, its
    headers among 4.2 million lines, 1115.3 -> 1103.6 ms (7.0e8 fewer). The
    8 others the same within their noise; the `filter_sum` kernel +0.1 %
    (large, 4 and 8 threads), -2.9 % (small). Test
    `sparse_filters_skip_the_empty_blocks` (elements kept at the edges of
    the blocks and in the last one, none, nearly all; 1 to 8 threads).
- **D256 — The functions and loops of the runtime aligned on 64 bytes
  (2026-11-04).** The mapping of the standard input (D257) made regex-redux
  slower, 293.6 -> 306.5 ms; with its input given by a pipe, where nothing
  is mapped, it was still slower: 318.76 -> 323.29 ms, best of 15. The new
  module had moved the code of the runtime after it, its hot loops (the
  engine of regular expressions) at other places of the 32-byte windows of
  the decoders (`perf`: the same functions at other addresses, their share
  of the cycles changed). The runtime is now built with
  `-align-all-functions=6 -align-loops=64` (LLVM): a function and its loops
  keep their place in those windows whatever code comes before them.
  - Measured (fixed frequency, 4 threads, best of 15): regex-redux by a
    pipe, D255 and D255 with D257 aligned: 318.74 and 317.15 ms; from its
    file: 0.294 and 0.289 s. The functions alone aligned: 321.67 and 321.05
    ms, slower than D255 unaligned (317.76 ms): the loops too.
  - The 10 programs, D255 aligned against D255 (best of 7): within their
    noise but k-nucleotide, 1097.5 -> 1108.4 ms; remeasured (best of 15,
    twice): 1.093 and 1.096 -> 1.098 and 1.098 s, 0.3 % (its loops placed
    otherwise; not measured further). The kernels of `results/xeon.md`
    (their loops are compiled code, not the runtime's): -0.6 % to +0.4 %
    at 4 and 8 threads but `histogram_skewed`, large, 4 threads, +2.1 %.
  - Size (padding): a hello for Linux 51432 -> 55528 bytes, for Windows
    54272 -> 57344; regex-redux 1485144 -> 1567064, k-nucleotide 272808 ->
    293288.
  - The noise of these measurements: `machine.sh apply parallel` confines
    every other process to the sibling threads of the cores measured, the
    commands run meanwhile (this session's own) included; the same binary
    of the large `histogram` gave 879741 ns in one run and 1595674 ns in a
    run during which commands ran (a kernel bound by its ports, the most
    sensitive to a busy sibling). The versions alternate within a run, so
    their comparison holds; no command now runs during a measurement.
- **D257 — A large regular file as the standard input is mapped, held by
  a read lease (2026-11-04).** reverse-complement after D255: reading its
  input of 254 MB into new pages took 29.4 ms with 4 threads (the copy out
  of the page cache, and the clearing of the pages it fills: 9.4 GB/s,
  `file_read_new`), a third of the program. A prototype in C on the same
  file, 4 threads: read in parallel 28.9 ms (and 0.6 ms to free); mapped,
  its pages touched by every thread, 2.4 ms (and 4.7 ms to unmap: the
  pages of the page cache, one by one); `MAP_POPULATE` 8.0 ms; writable and
  populated (a copy of each page) 65.1 ms.
  - `readStdin` of a regular file, opened read-only, of 8 MiB or more from
    an offset that is a multiple of a page (Linux, x86-64): the file mapped
    read-only, after a page that holds the header the allocator reads at
    free (a class no block has), its pages touched by the threads of the
    pool. The array must keep the bytes the file had, as `read` gives
    them: a read lease (`F_SETLEASE`), which the kernel gives only when no
    one has the file open for writing; a process that opens it for writing
    or truncates it waits while the handler of `SIGIO` copies the bytes
    into memory of the process and puts it at the place of the mapping
    (`mremap`, one call), then gives the lease back. Made private page by
    page (`MADV_POPULATE_WRITE`), the bytes were lost to a truncation: the
    kernel unmaps every page past the new end, the private ones included
    (a SIGBUS at 12 MB, the new length, in the first test). The mapping is
    read-only so that no write is lost during the copy: an array changed in
    place (D207) faults, the handler of `SIGSEGV` makes the same copy, and
    the write is done again; any other fault gets the handler from before.
    No lease (another owner, a file open for writing), or a handler of
    `SIGIO` or `SIGSEGV` already there: the file is read (`docs/language.md`).
  - The allocator bounds its cache of large blocks and its idle bytes by
    the bytes in use; the mapped array counts with them (`held_outside`):
    without it, regex-redux (its input of 50.8 MB) faulted 1820 -> 3696
    pages, 294.1 -> 313.1 ms, freed blocks given back sooner.
  - Measured (fixed frequency, 4 threads, best of 7, against D256): the read
    of reverse-complement 29.4 -> 8.3 ms; reverse-complement 84.1 -> 64.0
    ms, k-nucleotide 1095.4 -> 1076.6 ms, regex-redux 292.0 -> 290.1 ms;
    the 7 others the same within their noise. Tests
    `a_mapped_input_keeps_its_bytes_when_the_file_is_written` (24 MB,
    mapped (`/proc/<pid>/maps`), truncated and written while the program
    runs: the same sum, the writer waits less than 10 s; a file already
    open for writing: not mapped), `a_mapped_input_freed_early_gives_the_file_back`,
    `a_mapped_input_changed_in_place` (the copy on a write, 1 and 4
    threads); the tests of D221 read files of 20 MB mapped.
  - In a run of the complete suite the first test once found its input
    read, not mapped (the same test 5 times alone and 3 times beside the
    other allocator: mapped each time). The kernel refuses the lease while
    any process has the file open for writing, and a child of another test
    of the same process holds a copy of every descriptor between its
    `clone` and its `exec` (the most likely cause, not shown). The program
    then reads the file, as it should: the test checks the bytes on each of
    up to 3 tries, and that one try was mapped.
- **D258 — The mapped input given back by every thread (2026-11-04).**
  The timeline of reverse-complement after D257 (`perf`, 4 threads, by 2
  ms): its last 5 ms were the exit, the main thread in the kernel and the
  others idle. The array of the mapped input lives to the end: the exit
  unmaps its 254 MB of pages of the file one by one on one thread (`strace`:
  no `munmap` of it before). In C on the same file: `munmap` 3.58 ms;
  `MADV_DONTNEED` of a part by each of 4 threads 1.04 ms, then `munmap`
  0.07 ms.
  - When the program ends (`isc_run_big_stack`, after its main function),
    and when the array is freed while the pool is idle, its pages are given
    back by every thread of the pool, each its part, then unmapped; by one
    thread when the pool is busy.
  - Measured (best of 15, twice): reverse-complement 0.065 -> 0.061 s,
    k-nucleotide 1.080 -> 1.075 s, regex-redux 0.289 s both; one thread
    0.176 s both. The same output. The tests of D221 (files of 20 MB on 4
    threads, released at the end) and
    `a_mapped_input_freed_early_gives_the_file_back` (freed during the
    program) cover both ways.
- **D259 — Not kept: the ranges of a filling loop taken in turn
  (2026-11-04).** In the timeline of reverse-complement (D258), three of
  the four threads waited about 2 ms at the end of the writing passes of
  its second and third records: their arrays take the block of the record
  before (D211: a cached block grown), whose start is already written and
  whose end faults new pages, so equal ranges do not cost the same. Four
  ranges per thread taken in turn (`isc_parallel_for_fill`): measured
  (best of 7, 4 threads) reverse-complement 61.1 -> 61.5 ms, the 9 others
  the same within their noise, the kernels that fill an array -1.3 % to
  +0.2 %. No gain: not kept. The wait is not explained yet (TODO).
  - In Docker (`build/spot.sh`, not the protocol: the Isocron programs of
    D258, the others of the publication of D249, best of 9): reverse-
    complement 63.4 and 62.6 ms against Rust's 65.6 and 65.3 ms;
    k-nucleotide 1073.6 ms against C++'s 1177.4 ms.
- **D260 — The Benchmarks Game measured again at `d1e3586`: the fastest
  on 9 of 10 (2026-11-04).** The protocol of D249 (`image.sh`, `run.sh
  build`, `check`, `measure 4 3 native` after `machine.sh apply
  parallel`), the Isocron programs built by the compiler of D258 (D259
  changed only DECISIONS.md). Against D249: k-nucleotide 1.603 -> 1.074 s
  (C++ 1.174 s), reverse-complement 0.104 -> 0.062 s (Rust 0.068 s), both
  now ahead; n-body 2.021 -> 2.040 s, still 1.05 times C (1.945 s, Rust
  1.946 s); binary-trees 0.669 -> 0.667 s, fannkuch-redux 1.663 -> 1.661
  s, mandelbrot 0.639 s, pidigits 0.490 s, regex-redux 0.295 -> 0.289 s,
  spectral-norm 0.195 -> 0.199 s (Rust 0.205 s).
  - fasta 0.018 -> 0.020 s is the noise of the lowest of 3 runs, not a
    regression: the programs of 0.1.0 (the compiler of D249) and of now,
    run in turn 25 times in the same container (`build/spotfasta.sh`),
    both 17.7 ms at best, 18.2 and 18.0 ms at the median, each with runs
    of 19 to 34 ms.
  - The floors are the same; reverse-complement's (the file read twice
    by blocks at 30.2 GB/s, 0.017 s) still holds with the mapped input of
    D257, which maps the pages of the file instead of copying them.
- **D261 — A `fold` split: the roots and divisions that do not depend on
  the accumulator computed first, two elements at a time; the variance of
  n-body found (2026-11-04).**
  - The profile of n-body: one step is a loop of 258 instructions, 6 roots
    and 6 divisions of the divider (LLVM pairs 8 of the 10 pairs, 2 stay
    scalar), 39 accesses to the stack. Measured on this machine (a program
    of 8 independent chains, then one): a root holds the divider 4.5
    cycles, a division 4, the same for 1 or 2 lanes, twice for 4 lanes;
    latencies 13 and 13.5 cycles. A model of the loop (each instruction
    when its operands are ready, the divider one unit): 87.5 cycles of
    latency alone, 107.0 with the divider, whose queue is on the chain of a
    step (a root waits for it before its 13 cycles); measured 126.4.
  - The `fold` of `split_plan` (an `iota` of an even constant of at most
    16, `let`s at the head of its body that read neither the accumulator
    nor what depends on it, a root or a division of floats among them):
    those `let`s computed before the loop into slots on the stack, then the
    loop of the accumulator, which reads them. The `let`s that use the
    divider, or a value that does, for two elements at a time in vectors of
    2 lanes, their parts that do not use the divider computed per element
    and put into a vector just before it (`pack_leaves`: one shuffle in,
    one out); the others per element. The same operations on the same
    values: the same bits. A check of an index that fails before the loop
    jumps to the `fold` as written (`split_fail`), whose panic is then the
    one of the program, in its order; constant indices remove that path.
  - A first form, every such `let` in vectors (the elements of arrays
    gathered into vectors): 5 roots and 5 divisions of 2 lanes, but 263
    instructions with permutations across lanes of 256 bits (LLVM merged
    the vectors): 126.4 -> 129.9 cycles a step, not kept.
  - Measured (the sibling thread offline, 5 runs in turn): n-body 1.979 ->
    1.861 s, 126.4 -> 118.8 cycles a step, the divider active 76.8 -> 59.4
    cycles a step, a loop of 239 instructions, 5 roots and 5 divisions of 2
    lanes, 30 accesses to the stack. The same output; the 9 other programs
    and the 9 kernels the same binaries. In Docker (`build/spotnb.sh`, 7
    runs in turn, the conditions of the publication): Isocron 1.892 s at
    best (1.905 s median) against gcc-9 1.945 s (1.952 s) and rust-9
    1.947 s (1.948 s).
  - The model with the latencies of these chains of 1.0 (13 and 13.5
    cycles) did not predict the gain (107.0 -> 108.5 cycles). Measured again
    on values that do not converge (D263: 18 cycles a root, 14 a division,
    for 2.6 to 517 and 0.037; 13 only for 1.0), it does: 110.5 -> 101.5
    cycles (latency alone 93.0 -> 98.0, the two shuffles); the rest of the
    step, about 16 cycles in both, is what it leaves out, the ports and the
    window of the scheduler (the issue of instructions stalls 75 cycles of
    128; TODO: a model with them).
  - The variance of n-body (TODO of D233): the same binary took 128.0 to
    138.8 cycles a step from one series to the next, sometimes changing
    within a run, with the sibling of its core (CPU 15) idle. Not the
    interrupts (about 550 ticks and 10 of the network a run, the same in
    both), not the physical pages of the stack (moved at each run, the
    same cycles), not the disambiguation of loads (SSBD by `prctl`: the
    same), not the programs run before on the core. With the sibling awake
    (a loop of `pause`): 165.8 cycles in every run (the core shared); with
    the sibling offline: 126.3 to 126.4 in every run. The core of one
    thread has states of different speed while its sibling sleeps and
    wakes; without the sibling, one. The measurements of this decision
    are made with the sibling offline.
- **D262 — The Benchmarks Game measured again at `c77e61b`: the fastest
  on 10 of 10 (2026-11-04).** The protocol of D249 and D260, the Isocron
  programs built by the compiler of D261, the Windows VM stopped. Against
  D260: n-body 2.040 -> 1.897 s, now ahead of C (1.945 s) and Rust (1.946
  s); k-nucleotide 1.070 s (C++ 1.176 s), reverse-complement 0.061 s (Rust
  0.067 s), spectral-norm 0.195 s (Rust 0.205 s), binary-trees 0.669 s,
  fannkuch-redux 1.663 s, fasta 0.018 s, mandelbrot 0.639 s, pidigits
  0.490 s, regex-redux 0.289 s, each the same within its noise. Before
  it: the full suite green (82 runs), the tests of the Windows target on
  the VM (3, the ten programs among them).
- **D263 — The time above each floor attributed (2026-11-04).** Each
  program's gap to its floor given to the components that make it,
  measured at `c77e61b` with the sizes of the publication on 4 threads
  (`perf stat`, `perf record`, LBR call graphs), against the terms of the
  floor that do the same work: the table "Where the time above each floor
  goes" of `results/floors.md`, and the order of the work after it. Among them:
  - mandelbrot: 7.18e9 operations of 512 bits, 5.75e10 of lanes against the
    4.45e10 the floor counts (the lanes of a group of 64 pixels iterate
    until its last escapes), the port of the vectors busy 94.2 %;
  - spectral-norm: the divider active 97.4 % of the cycles, every element
    divided (the floor reads 48 % of them from memory);
  - fannkuch-redux: 5.62 cycles and 14.9 instructions a flip, the ports 0
    and 5 at 91.5 % and 93.5 %;
  - binary-trees: 3.97 ns and 43.1 instructions a node, the cost of the
    recursive primitive, not of its loop;
  - regex-redux: the copies are those of the substitutions into new pages
    and the vector of the bounds of matches grown by copies (LBR: 11.3 %
    and 3.1 % of the samples).
  - The divider measured on fixed values (`divcal2`): a root 4.5 cycles
    held and 18 of latency, a division 4 and 14, for 2.618 to 517 and
    0.0371; for 1.0, 13 and 13. The floor of n-body (latencies measured by
    a chain `x + 1.0`, about 2.6) holds.
  - The first measure of the TODO of the runtime given to LLVM: the time in
    the runtime is in functions that loop over a block or a number
    (`add_block` 34 % and `count_round` 26 % of k-nucleotide, `bounds_from`
    45 % of regex-redux, `mul_word` 60 % of pidigits), whose calls cost
    little; binary-trees spends its time in the generated code. Inlining
    them would save little; kept as a TODO, behind the causes of the table.
  - A first run after `machine.sh apply` reads its input from the disk (the
    page cache dropped): k-nucleotide 2.150 s, regex-redux 0.514 s; the
    profiles run each program once before (as `measure.sh` keeps the
    lowest of 3).
- **D264 — Release 0.2.0 (2026-11-04).** The programs of the Benchmarks
  Game the fastest on 10 of 10 (D262), the time above each floor attributed
  (D263). The version of the workspace and of the runtime 0.2.0; the
  compilers for Linux and Windows built in the images of D248
  (`build_iscc.sh`), the archives by `package.sh`.
- **D265 — The runtime inlined into the program: measured, not done
  (2026-11-04).** Asked for: test not compiling the runtime apart, so that
  the generated code inlines its fast paths. What it can save is bounded by
  the calls of the generated code into the runtime (callgrind, `--isa
  avx2`, `ISC_ALLOC=glibc`, 4 threads, small sizes; pidigits also at its
  size, 10000 digits): pidigits 1.138e6 calls for 5.172e9 instructions
  (0.22 a thousand), k-nucleotide 0.11 a thousand at its small size (the
  frees and strings of its groups, a number that does not grow with the
  input), binary-trees about 65000 calls of its regions for 1.84e9
  instructions, the 7 others under 0.001 a thousand. At 20 instructions of
  call, prologue and return each, at most about 0.4 % (pidigits), less for
  the others: the time of the runtime is in functions called rarely that
  loop over a block or a number (`isc_big_op` 4.70e9 instructions for
  341000 calls). The other gain of a runtime compiled with the program, the
  instructions of its CPU, is already there: `iscc` links a runtime built
  for x86-64-v4 (AVX-512) or v3 when the CPU has them. Linking the runtime
  as bitcode would also need the LLVM of rustc (23.1.1) read by that of
  `iscc` (21.1.8), which reads only older bitcode. Not done; to do again if
  a program calls the runtime per element.
- **D266 — A release published by one command (2026-11-04).** Asked for:
  `tools/release/publish.sh VERSION DIR [--dry-run]` pushes main, tags the
  commit "Release VERSION:" (an annotated tag `vVERSION`, "Isocron
  VERSION", as for 0.1.0 and 0.1.1), pushes the tag and makes the release
  on GitHub with `gh`: the two archives of `package.sh` and `SHA256SUMS`,
  the text of `DIR/notes.md`. It checks first, and stops on the first that
  fails: main checked out with nothing to commit, that commit with the
  version in `Cargo.toml` and a section in `CHANGELOG.md`, the sums of the
  archives, `gh` logged in, main ahead of `origin/main`, no tag of that
  version on another commit, no release of it on GitHub. `--dry-run`: the
  checks and the commands it would run.
- **D267 — An extension of VS Code (2026-11-04).** Asked for: the tools
  for VS Code. `tools/vscode`, without npm or a build step (plain
  JavaScript on the API of VS Code 1.75): the language `isocron` (`.isc`),
  a TextMate grammar (keywords, the types, the literals with their
  suffixes, strings, templates and their `${}`, comments, the functions of
  the standard library and of `Math`) also injected into the ` ```isc `
  blocks of Markdown; the configuration of the language (comments, pairs,
  indentation); snippets; the errors of `iscc check FILE --color never`
  (`PATH:LINE:COLUMN: error: MESSAGE`, a path of Windows too) as
  diagnostics when a file is opened or saved, in the file they are in; the
  commands that check, run (`iscc run`, the arguments asked), build and
  run, and show the IR; the outline and the definitions (the file and the
  modules it imports); the problem matcher `$iscc` for tasks. The compiler:
  `isocron.iscc` (a name in the PATH, a path, `${workspaceFolder}`), run
  where the files are (`extensionKind` workspace: with Remote - SSH, on the
  server). The errors come from the file on disk: checked when saved, not
  while typed (a module is read next to its importer, so a copy of the
  buffer elsewhere would not resolve its imports). `package.py` writes the
  `.vsix` vsce would (`[Content_Types].xml`, `extension.vsixmanifest`,
  `extension/`), dates fixed. Tests (`test/run.sh`, in a container of
  Node.js): the functions of `lib.js`, and every Isocron file of the
  repository and every ` ```isc ` block of `docs/language.md` tokenized by
  vscode-textmate with nothing left open, the scopes of chosen tokens.
  `.vscode/` of the repository: the `iscc` of `target/release`,
  rust-analyzer on both workspaces (the runtime is its own) in a target of
  its own, the builds and inputs out of the watchers and the searches, the
  tasks that build, test and check.
  - Tried in a real VS Code (`test/windows.py`): VS Code 1.95.3 portable
    on the Windows 11 VM, the `.vsix` installed by its command line
    (`isocron.isocron@0.2.0` in its list), the 10 steps of
    `test/integration/index.js` run inside it on the files of the package
    with the `iscc.exe` of 0.2.0 (named through `${workspaceFolder}`): the
    language, the error of `err.isc` at 2:16, none in a correct file, an
    error fixed and saved gone, the outline, the definitions (a module
    imported, the file), the IR, `ok.exe` built and printing 45: all
    passed.
- **D268 — The README laid out again; the logo and the icons of the
  project (2026-11-04).** Asked for: a main page less austere. The README
  opens on the logo (`<picture>`: `docs/media/isocron-logo.svg` on a light
  page, `isocron-logo-dark.svg` on a dark one), a line, three badges and
  links; a first program with its real outputs (the harmonic series, 1e9
  terms: 628 ms on one core, 80 ms on eight, 21.30048150234794 both times);
  then four sections (fast: the block `report.py` writes; exact; parallel
  without threads; checked before it runs, with a real error of sizes at
  compile time and its run-time form), the floors, how to start (the
  archives of 0.2.0, a first program, where to read on), the documents of
  the project, and in `<details>` the build from source, the tests, the
  benchmarks and the layout. Its programs are fenced as `ts`: GitHub knows
  no `isc` and would show them without colors.
  - The chart of the README is an image, `docs/media/benchmarks.svg`,
    written by `report.py` beside the table from the same rows
    (`mdstyle.ratio_svg`): one row a program, the value in its bar, the
    colors of the marks of the table, a dashed line at 1. A Mermaid chart
    is drawn by each viewer: its ten names under vertical bars overlapped,
    and an option of orientation is not taken by all. The Mermaid charts
    of `results/benchmarks_game.md` and `results/floors.md` are
    horizontal (a row a name), their titles shortened (one was cut at
    700 px).
  - The logo, the icon, the marks in one color and the PNGs of 16 to 512
    px in `docs/media`, as delivered (two archives) but for their names, in
    English, and for the C2PA manifests the SVG files carried (credentials
    naming the tool that made them; this repository keeps no such
    reference): the drawings are unchanged.
  - The extension of VS Code: its icon (`icon.png`) and the icons of the
    `.isc` files for light and dark themes (`icons/`), packed by
    `package.py` (the types PNG and SVG, the icon among the assets of the
    manifest); tried again in VS Code on Windows, the 10 steps passed.
- **D269 — The language server `isc-lsp`: the hover, the errors as the
  text is typed, the definitions, the outline, for every editor
  (2026-11-04).** Asked for: help on hover over the code, in the
  extension of VS Code. A first version added `iscc hover FILE LINE
  COLUMN` to the compiler, its JSON read by the extension only; not kept
  (never pushed): a command of the compiler for the editors, and in
  practice for VS Code alone. The standard way, the same for every editor,
  is a language server, a program of its own (as rust-analyzer, gopls).
  - `crates/isc-lsp`, the binary `isc-lsp`: the Language Server Protocol
    on stdio (its JSON in `json.rs`: the workspace has no dependency for
    it). It depends on `isc-syntax` and `isc-types` only: no code
    generation, no runtime (`isc-lsp.exe` about 2 MB, `iscc.exe` about
    114 MB). It reads a program as `iscc check` does: the loading of the
    modules moved, unchanged, from `iscc` into `isc_types::modules`, so
    that both load them the same way; `for_each_child` of the checker
    public. The compiler `iscc` has no new command.
  - What it gives, on the text as the editor has it (saved or not): the
    error of the checker at its place (in an imported module: on that
    module, cleared when it is gone); on hover, from the typed tree, the
    type of a local or a parameter (of a function or of a lambda) where it
    is declared or read, of a size, of a field (at its `.`), the signature
    of a function where it is declared or called (at its `(`) or passed
    as a value, with the comments `//` above its declaration (also in a
    module), else the documentation of a function of the standard library
    from `docs/language.md` built into the server (each row of a table
    whose first cell writes a call, each section titled by a function: 69
    names), with a link to its section; while the program does not check,
    a declaration of the file by its name. The definition of a local or a
    function (also in a module); the outline (functions, actions, types).
    Positions in UTF-16 as the protocol counts; a panic of the checker on
    a text being typed gives no answer, not a dead server.
  - The editors: the extension of VS Code starts the `isc-lsp` beside the
    compiler it uses (setting `isocron.languageServer`), for the hover
    only, its client in `extension.js` (no npm); its other features stay
    as they were, with `iscc`, also without the server. Neovim 0.10:
    `tools/neovim/isocron.lua`. The archives of a release carry `isc-lsp`
    beside `iscc` (`build_iscc.sh`, `package.sh`).
  - Tests: `crates/isc-lsp/tests/lsp.rs` drives the server as an editor
    does (15 places of a program and a module, the documentation, the
    definitions, the outline, an error typed and fixed, a constant typed
    and not saved, an error in a module, the end of the protocol);
    `tools/neovim/test/run.sh`: 8 steps in Neovim 0.10.4 (Docker);
    `tools/vscode/test/run.sh`: 42 tests; in VS Code on Windows (an
    `iscc.exe` and an `isc-lsp.exe` of these sources), 14 steps, the
    hovers among them.
- **D270 — Release 0.2.1: `isc-lsp` beside `iscc` (2026-11-04).** Asked
  for: the executables of the language server (D269) to download. A
  release of its own, not files added to that of 0.2.0: the files of a
  release are built from its tag (`v0.2.0` has no `isc-lsp`) and its
  `SHA256SUMS` covers them all. 0.2.1 (a version compatible with 0.2.0,
  as Cargo counts: a tool added, the compiler and the programs it makes
  unchanged since 0.2.0). The archives as for 0.2.0 (`build_iscc.sh`, now
  with `isc-lsp`; `package.sh`, now copying it beside `iscc`), from the
  sources of the commit of the release; the extension of VS Code at 0.2.1
  too.
  - The compiler unchanged, checked: the ten programs of the Benchmarks
    Game (11 files) built by `iscc` 0.2.0 and 0.2.1 give the same LLVM IR,
    to the byte; their executables differ (same sizes, 3931 bytes of
    `.text` for nbody) by the number of the version alone: the sources of
    0.2.1 built under the number 0.2.0 give the 22 files of 0.2.0, to the
    byte. Rust writes in the names of the symbols of the runtime a hash of
    its crate that counts the version (`Cs8quoeD7URSa_` /
    `CsdlsRN8XvvpF_`), and the linker places the functions by them.
  - Tried: the archive for Linux on Debian 12 slim without network
    (`iscc --version`, nbody built and run, `isc-lsp` answering), its
    `isc-lsp` in Neovim (8 steps); the archive for Windows in VS Code (the
    extension 0.2.1, 14 steps) and by `selftest_windows.py` (the ten
    programs, the same outputs; `iscc run`, WebAssembly).
- **D271 — The extension of VS Code in the releases (2026-11-04).** Asked
  for: the `.vsix` with the release. `isocron-VERSION.vsix`, a file of the
  release beside the two archives (one for both systems, installed as it
  is downloaded), in `SHA256SUMS`: made by `package.sh` with
  `tools/vscode/package.py` (refused when `tools/vscode/package.json` is
  at another version), published by `publish.sh`. For 0.2.1, added to the
  release already published: made from the sources of the tag `v0.2.1`
  (the same bytes as the `.vsix` tried on Windows), `SHA256SUMS` made
  again with it and the notes saying it; the four files downloaded again,
  their sums right.
- **D272 — The whole rows of a `map` by rows at constant places
  (2026-11-04).** Part 1 of the goal of 2026-11-04 (each of the ten
  programs under 3 times its floor). fasta was 7.8 times its floor: 702
  million instructions for its 254 MB, about 2.1 a byte of the lines where
  the floor copies them from tables. The profile of the loop of the lines
  (D214): each row of 61 computed its bounds in the range (`smax`, `smin`,
  its length, a test of its width) and ran a loop of vectors from them,
  about 50 instructions a row before its two vectors. Only the first and
  the last row of a range can be cut; the rows wholly inside it now run by
  vectors at the constant places 0, w, ..., c - w, their bounds 0 and c,
  so that what depends on the place in the row folds (`q % 61 == 60`, a
  constant mask); the two cut rows as before. At fixed frequency, best of
  9, 4 threads on the host: fasta 18.07 ms -> 14.12 ms (702 -> 469 million
  instructions, 66.4 -> 50.8 ms of CPU), reverse-complement 60.7 ms ->
  59.2 ms. The same outputs. Test `whole_rows_by_vectors_at_constant_places`
  (rows as wide as the vectors, one lane wider, twice as wide and more,
  lengths a multiple of the row or not, an element depending on one place
  of the row around each place of the vectors, 64-bit elements; 1, 3 and
  4 threads).
- **D273 — A table read at a remainder copied with its period continued
  (2026-11-04).** After D272 the line of ALU of fasta (`alu[i % 287]`,
  `i` consecutive in the 32 lanes) still crossed the end of its table in
  about 31 of 287 vectors, each then the general code of D229 (a gather of
  32 lanes and their checks). Before a vector loop, each table read at
  `x % c` in its body (an array of scalars bound outside it, c a literal of
  at least the lanes and at most 4096: the copy is on the stack, once per
  loop) is copied with its first `w - 1` elements after its `c` when it
  has at least `c` elements; every vector of consecutive `x >= 0` then
  reads its lanes by one load from the remainder of its first lane. A
  table shorter than c, or a negative `x`, takes the code of before (its
  panic). At fixed frequency, best of 9, 4 threads: fasta 14.17 ms ->
  11.21 ms (469 -> 407 million instructions, 51.7 -> 39.0 ms of CPU); the
  same output. Test `tables_read_at_a_remainder_are_copied_with_their_period`
  (bytes in lines as fasta, `f64`, booleans, c = 4096 and 4097, under a
  mask, a table shorter than c, a negative `x`).
- **D274 — A whole row of 33 to 64 bytes in one vector (2026-11-04).**
  After D273 a line of fasta (61 bytes) still took two vectors of 32
  lanes (at its places 0 and 29), each with its own remainder by 139968
  (two products), its checks and its masks: about 35 instructions a
  vector. On a target that loads and stores bytes under a mask (AVX-512
  BW and VL), a `map` of bytes by rows of 33 to 64 (without tables of
  bytes permuted in registers, D198) now makes each whole row in one
  vector of 64 lanes, the lanes after the row masked (their checks and
  gathers skipped as in a branch of an `if`; the replay of a failed check
  computes the elements of the row only, the last row may end the array)
  and stored under the mask; the tables of D273 are then copied for 64
  lanes. At fixed frequency, best of 9, 4 threads: fasta 11.19 ms ->
  8.16 ms (407 -> 230 million instructions, 40.9 -> 26.9 ms of CPU, about
  45 instructions a line); reverse-complement, whose lines read a table
  permuted in registers, unchanged (58.1 and 57.6 ms). The same outputs.
  Test `rows_narrower_than_the_vector_are_masked` (a table exactly as long
  as the row: the masked lanes would read past it; rows of 33, 40, 61 and
  64; the panic of the last place of a row past its table; 1 and 4
  threads).
  - Measured on the way: reverse-complement took 74.8 ms with both
    binaries on a copy of its input made earlier, 58 ms on a new copy of
    the same bytes: 63631 page faults against 5454 (the input is mapped,
    D257, and the pages of that copy in the page cache were small ones).
    The inputs of a measurement are made again before it.
- **D275 — The masks of a simple shape: one per byte in registers, the
  first six places without a branch (2026-11-04).** regex-redux was 9.7
  times its floor; 43 % of its CPU in the engine of masks (D224), about
  0.66 ns a byte over the 9 counts where the floor scans for two literals
  at 0.074 ns. Its profile: the masks of the classes made by an `or` into
  an array in memory for each class of each distinct byte (a chain of
  stores and loads each block), and each alternative left at its first
  empty mask, a branch the text decides (mispredicted about once a block
  in random bases). Now (1) the fixed alternatives over at most 8
  distinct bytes (no stop byte, no start of line: the 9 patterns and the
  first 3 replacements) have a plan `Fixed`: one mask per distinct byte,
  each class the `or` of the masks of its bytes; (2) every alternative
  takes its first six places without a branch, each shift a constant,
  then goes on while its mask is not empty (a test then predicted: after
  six places of classes of a few bytes nearly no candidate is left).
  Measured and left: every place of every alternative without a branch
  (shifts of 128 bits by a variable), 293.6 -> 459.6 ms. In alternation
  (the binaries run in turn, best of 9 each: a busy process on the sibling
  threads of the cores of the measurements moves every time), 4 threads:
  regex-redux 293.6 ms -> 279.4 ms (1 alone), 291.8 ms -> 260.4 ms (1 and
  2). The same output. Test `fixed_alternatives_find_the_matches_of_the_automaton`
  (the 9 patterns and others over random bases with upper cases and other
  bytes, from and until anywhere; also built for AVX2 and AVX-512).
- **D276 — The matches of the parts of a search merged by runs
  (2026-11-04).** After D275 the timeline of regex-redux (4 threads)
  showed the main thread alone 15 to 20 ms after each replacement
  searched in parts (D189), the three others waiting: the merge of the
  parts (`merge_bounds_with`) pushed every match one by one into a vector
  grown by copies, and found where a part goes on after a match that
  crosses into it by a walk of its matches. The vector now has the room of
  all the matches of the parts; the matches of a part before the start of
  the next one are copied at once (they follow each other in it), only the
  one that crosses the next start handled alone; the place where a part
  goes on is found by halving (its matches are in order). The same
  matches. In alternation, best of 9, 4 threads: regex-redux 260.4 ms ->
  208.2 ms. Test `merged_parts_are_one_search` (parts cut anywhere, inside
  a match and at its end, patterns of many matches, of overlapping
  candidates, of empty matches, delimited ones).
- **D277 — The searches by masks inside a task cut into parts too
  (2026-11-04).** The 9 counts of regex-redux are the tasks of a parallel
  `map`; inside a task a search ran whole on its thread (the parts of the
  automaton would each fill a cache of its lazy DFA again), so with 4
  threads the last count ran alone, the three others waiting (about 20 ms
  of the timeline). The masks of a simple shape have no cache: inside a
  task their search is now cut into parts as outside, the parts shared
  with the threads that have finished their tasks (D201); with every
  thread busy the task runs its parts itself (only the merge more, D276).
  In alternation, best of 9, 4 threads, the machine loaded (load average
  16): regex-redux 207.9 ms -> 202.6 ms; the timeline shows the four
  threads busy to the end of the counts. The same output. Test
  `searches_inside_tasks_are_shared_with_idle_threads` (9 patterns in the
  tasks of a `map`, 1 to 4 threads).
- **D278 — The classes of a block looked up by one permutation
  (2026-11-04).** After D275 the masks of the classes still took most of
  the engine of masks: LLVM had made of the `or` of the masks of the
  bytes of each class a vector loop with blends and horizontal `or`s
  through the stack. On a processor with AVX-512 VBMI (seen by `cpuid` and
  `xgetbv` when the pattern is made: the runtime for x86-64-v4 does not
  assume it), a plan of at most 8 classes has a table of 128 bytes, the
  classes of each ASCII byte (a bit each, upper cases of folded letters
  included); a block of 64 bytes gives the classes of its bytes by one
  `vpermi2b` (the bytes of 128 and more in none), then one test per class.
  An exact lookup: the two lookups by nibbles of Teddy give a superset of
  a class like `[cgt]` with its upper cases (`s`, `w`). In alternation,
  best of 9, 4 threads, the machine loaded: regex-redux 202.4 ms -> 194.4
  ms. The same output. Test `fixed_alternatives_find_the_matches_of_the_automaton`
  (the table chosen where the processor has it; built for x86-64-v4).
- **D279 — A replacement of a large text written in parallel whatever its
  number of matches, cut by bytes (2026-11-04).** `replace_parallel`
  (D236) wrote in parallel only from 16384 matches on, its parts cut by
  matches: a replacement of a text of 50 MB with 6220 matches (the first
  of regex-redux) was written by one thread, and parts cut by matches can
  hold very different numbers of bytes. A text of 1 MiB and more is now
  cut into one part of about the same bytes per thread, a cut inside a
  match moved to its end; each part, its text and its matches, written at
  its place (its size known before). The bytes of `replace_bounds`. In
  alternation, best of 9, 4 threads: regex-redux 195.4 ms -> 193.4 ms.
  Test `replacements_of_large_texts_cut_by_bytes` (few matches, a match
  longer than a part across the cuts, empty matches, `$&`; 1 to 16
  threads).
- **D280 — The short copies of a replacement without a call (2026-11-04).**
  The last three replacements of regex-redux have 3522378, 3544465 and
  1690880 matches in about 50 MB, about 14 bytes apart: the parts of a
  replacement written in parallel (D236) called the copy of the system
  twice a match (the text before it, then its replacement of 1 to 3
  bytes), its call and its dispatch on the size costing more than the
  bytes. Below 32 bytes the copy is now two moves that overlap (of 16, 8
  or 4 bytes, or three bytes), inline; from 32 bytes on, the copy of the
  system. The same bytes. In alternation, best of 9, 4 threads: regex-redux
  192.5 ms -> 182.7 ms. Test `short_copies_of_a_replacement` (every length
  of text from 0 to 64 between the matches, replacements of 0 to 33 bytes,
  in a text written in parallel; 1 and 4 threads).
- **D281 — The matches of a search in parts kept as runs of the parts
  (2026-11-04).** After D276 the merge of the parts of a search still made
  one vector of all the matches: for the 3.5 million matches of a
  replacement of regex-redux, 56 MB copied by the main thread alone (the
  timeline: about 4 ms each, the three other threads waiting), then read
  again by the replacement. The merge now gives runs (`merge_runs_with`):
  the matches of a part from where the global search goes on in it to the
  one that crosses into the next part, and the matches found alone at a
  cut; a search keeps its parts and their runs (`Found`). A count adds the
  lengths of the runs; the list of matches is made from them; a replacement
  written in parallel (D279) finds its cuts in the runs by halving and
  writes the matches of each part from them; the other replacements make
  the vector as before. The same matches and texts. In alternation, best
  of 9, 4 threads: regex-redux 184.8 ms -> 167.9 ms. Tests: the merge
  (`merged_parts_are_one_search`), and the searches split among threads,
  the replacements written in parallel by matches and by bytes, the short
  copies (`strings.rs`, 1 to 16 threads).
- **D282 — The stop of a delimited alternative found in the masks of the
  blocks (2026-11-04).** The replacements `<[^>]*>` and `\|[^|][^|]*\|`
  of regex-redux (3544465 and 1690880 matches) called `memchr` for the
  stop byte of each match, a few bytes after its head. The stop byte of a
  delimited alternative now has a class of its own, its mask made with the
  others; its first place after the head is found in the masks of the
  block and the next one (the head ends before the end of the next), the
  text after them searched only when it is not there. The same matches.
  The phases of regex-redux (best of 3, 4 threads): `<[^>]*>` 31.5 ms ->
  29.6 ms, `\|[^|][^|]*\|` 15.6 ms -> 14.1 ms; in alternation, best of
  9: 167.9 ms -> 165.1 ms. Test `stops_found_in_the_masks_of_the_blocks`
  (stops at every distance from 0 to 300 bytes of their heads, too close
  for `mid`, missing at the end).
- **D283 — Room for the matches of a search reserved from their density
  (2026-11-04).** The vector of the matches of a part of a search grew by
  doublings, each a copy of all its matches: for the 3.5 million matches
  of a replacement of regex-redux, 56 MB copied about 20 times (the
  allocator of the runtime has no growing in place). A search whose vector
  is full (from 1024 matches) now reserves at once the room of the matches
  of the rest of its range at the density seen so far, and an eighth more.
  The same matches. The third replacement of regex-redux alone (best of 3,
  4 threads): 24.2 ms -> 20.4 ms; in alternation, best of 9: 164.8 ms ->
  153.8 ms. Tests: the searches of `rx::simple` and `strings.rs`.
- **D284 — The sizes of the parts of a replacement counted by every
  thread (2026-11-04).** Before writing a replacement in parallel (D279,
  D281), the caller alone walked its matches to count the bytes of each
  part of the result: 3.5 million matches in the third replacement of
  regex-redux, the three other threads waiting. Each thread now counts the
  bytes of its part (a first parallel loop), the places of the parts are
  their sums, then each writes its part (the second). The same texts. The
  third replacement of regex-redux alone (best of 3, 4 threads): 20.4 ms
  -> 19.3 ms; in alternation, best of 9: 153.8 ms -> 150.6 ms. Tests: the
  replacements written in parallel (`strings.rs`, 1 to 16 threads).
- **D285 — The pages of the mapped standard input given by the kernel in
  one call per thread (2026-11-04).** A large file given as the standard
  input is mapped (D257), its pages touched by every thread, one read per
  page (`touch_pages`): where the page cache holds the file in pages of 4
  KiB (a tmpfs, here after the memory of the machine was short [contradicted: D375]), each page
  is a fault, 63629 for the 254 MB of reverse-complement, 29 % of its CPU
  in their handling. Each thread now asks the kernel for the pages of its
  part in one call (`madvise(MADV_POPULATE_READ)`, Linux 5.14), touching
  them one by one only where the system refuses it. Input in a tmpfs, in
  alternation, best of 9, 4 threads: reverse-complement 75.1 ms -> 65.3
  ms, 63629 -> 1552 faults; input on ext4 (large pages of the page cache,
  5459 faults): 58.6 ms either way. The same output. Measured on the way:
  the inputs of the measurements are now on the disk (`target`), as the
  inputs of the publication are, not in the tmpfs, where their pages
  change with the memory of the machine [a hypothesis: D375]. Tests: the standard input mapped
  (D257, `stdin.rs`).
- **D286 — The threads of reverse-complement waiting at the end of its
  writing passes: explained, nothing to gain (2026-11-04).** TODO.md held
  them as not explained (about 2 ms each, D259). The timeline (4 threads,
  input on the disk) shows the thread or two that end the passes of the
  filters of records 2 and 3 clearing pages half of their time while the
  others wait. The array of each filter is a cached large block grown
  (D211, D220: 50, then 74, then 123 MB): its first pages are those of the
  block before, written; its end is new memory that the system clears at
  its first write; the threads write contiguous ranges (D254), so the
  thread of the last range gets every new page. Measured: the new memory
  of a large block (from 8 MiB) given by every thread before it is written
  (`madvise(MADV_POPULATE_WRITE)` by shares of huge pages): the waiting is
  gone from the timeline, the clearing now a phase of its own on the four
  threads; in alternation, best of 9 (5 for k-nucleotide), 4 threads:
  reverse-complement 59.2 -> 59.5 ms, regex-redux 147.3 -> 148.2 ms,
  k-nucleotide 1077.3 -> 1067.0 ms. The clearing is moved, not saved (the
  new pages of 123 MB are what that array costs): not kept.
- **D287 — No C called by the runtime: the system of Linux called directly
  (2026-11-04; the decision of the author, its priority before the rest of
  the goal of the day).** The author: « pas d'unsafe et pas d'appels C »,
  first, since it touches every program. Not all of it is possible in
  Rust: a system call, raw memory, a pointer from the generated code need
  `unsafe` there; none at all means a runtime that is not in Rust (in
  Isocron, with primitives checked by the compiler: the way to `iscc`
  built by `iscc`). The plan, in this order: (B) Linux without the C
  library: the system called directly; then the start of the programs,
  their threads and their thread-local storage, the functions of memory
  LLVM calls, static programs without glibc; the second run of the tests
  (`ISC_ALLOC=glibc`) and the leaks seen by Valgrind replaced by a mode of
  checks of the allocator of the runtime; each step measured on the ten
  programs (the start of a program without the dynamic loader is cheaper);
  (A) the `unsafe` left confined to the module of the system, the
  allocator and the SIMD, each block justified (`SAFETY`), checked at
  compile time; then the design of a runtime written in Isocron.
  - B1, this commit: `linux.rs`, the functions of the C library the
    runtime called (`open`, `read`, `write`, `pread`, `close`, `lseek`,
    `fcntl`, `mmap`, `munmap`, `mremap`, `madvise`, `mprotect`,
    `nanosleep`, `sched_getaffinity`, `clock_gettime`, the futex,
    `rt_sigaction` with its restorer, `tgkill` for `raise` and `abort`,
    `exit_group`), with their names and meanings, by the instruction
    `syscall` (one function, `syscall6`, the only one that enters the
    kernel). An error is the negative code of the kernel (no `errno`
    without a C library): `fail(r)` gives the callers its code; the texts
    are those of glibc (`oserr`, as under Windows). The environment is read
    from the stack the kernel made (`argv[argc + 1 ..]`), noted by
    `isc_args`. Left in glibc for the next steps: the threads (pthread),
    the start, the thread-local storage, the functions of memory, and its
    allocator for `ISC_ALLOC=glibc`. The same outputs; the tests of the
    standard input mapped (its lease, SIGIO and SIGSEGV), the texts, the
    files, `Result`, the threads, the heap.
- **D288 — The functions of memory of the runtime: the crate `isc-mem`,
  safe code on slices (2026-11-04).** Without a C library (D287), the
  functions LLVM and `core` call (`memcpy`, `memmove`, `memset`, `memcmp`,
  `bcmp`, `strlen`) are the runtime's (under Linux; a program linked with
  glibc takes them too: the definitions of an executable come first). The
  author's demand: the code as clean and safe as can be, and no loss.
  - The way of glibc on this processor, read in its code
    (`__memmove_evex_unaligned_erms`, `__memcmp_evex_movbe`,
    `__memset_evex_unaligned_erms`, its thresholds: `rep movsb` from 0x840
    bytes, `rep stosb` above 0x800, past the caches from half the
    last-level cache, here 0x800000): from both ends without a loop up to 8
    vectors, the loop of 4 vectors to aligned places, `rep movsb` aligned
    on 64 bytes, 4 pages at a time past the caches (their 16 vectors read
    before any is written: otherwise a load waits for the store of the
    same 12 low bits of address; the stores on lines of 64 bytes: a line
    half written goes to memory in parts when 3 others come between, 1.23
    ms against 0.70 for 16 MiB before), `memcmp` by masked loads up to 32
    bytes, its loop of 4 vectors by 2 ternary operations (LLVM made the
    form of AVX2 4 comparisons into masks, on one port of this processor).
    The threshold past the caches read by CPUID (half the L3), not fixed.
  - Safety: the pointers LLVM gives become slices once, at the entry of
    each symbol (`symbols.rs`) and of each long path (`raw`), by the
    contract of the function; all the rest is safe code on slices; the
    few things Rust cannot say safely are in small functions with a safe
    interface: the instructions of the vectors (`vec`, the set enabled by
    `cfg`), `rep movsb` and `rep stosb`, the stores past the caches (their
    alignment checked), the masked loads, `strlen`. 61 lines of code inside
    `unsafe` blocks, against 494 in the first version (raw pointers
    everywhere); every block with its `SAFETY`, `deny(unsafe_op_in_unsafe_fn,
    clippy::undocumented_unsafe_blocks)`, Clippy clean for the 3 variants.
    The accesses of the short paths by `get` (no panic possible there: a
    function that may call one aligns the stack, 2 instructions more for
    every length); a place outside, a fault of the algorithm, stops the
    tests (built with the assertions of debugging), and in the runtime
    reads zeros or writes nothing, never outside.
  - Two traps of LLVM: a loop that moves, fills or measures bytes becomes
    a call of `memcpy`, `memset` or `strlen`, that is of itself: the index
    of such loops goes through `opaque` (an empty `asm!`); `#![no_builtins]`
    would forbid it, but takes the crate out of the optimisation at the
    link of the runtime, whose `core` then lacks `panic_bounds_check`
    (a link error). Unoptimised, a copy of a fixed size is a call of
    `memcpy`: the crate is built optimised in the profile of development
    too, and its tests run with its own symbols (the whole test process
    copies with them: a recursion would overflow the stack, seen once).
  - The short lengths straight on, as glibc (32 to 64 bytes without a
    taken branch, the long paths reached by a jump to a function of the C
    convention: with a call, `memcpy` saved a register for every length;
    LLVM had seen that the long path returns `d`, hence `opaque` on it).
    Measured and not kept: `vzeroupper` (0.0 cycle: 7.14 and 7.06 for 32
    bytes), `vptest` against `vptestmb` (98.47 and 98.36 cycles for 2 KiB).
  - Measures (Xeon E-2388G, cycles per call, glibc against ours): copy 7
    bytes 8.09 / 7.24, 32 bytes 6.08 / 6.61, 192 bytes 20.07 / 19.80, 1 KiB
    46.40 / 46.53, 2 KiB 81.22 / 80.66; fill 1 KiB 29.69 / 25.66, 2 KiB
    45.73 / 42.65; past the caches, 16 MiB copied 698186 / 681454 ns, 256
    MiB 15418082 / 15418126 ns. Still slower (TODO.md): 2 bytes copied or
    filled (7.08 / 9.08, 7.25 / 10.07: the chain of branches, which LLVM
    lays out with taken branches), `memcmp` of 384 to 2048 bytes (22.39 /
    24.49, 88.81 / 96.53; its loop at parity, 89.54 / 89.49 on the same
    places: the cycles are in its fixed part, still to be located).
  - The programs, A (glibc's functions) against B (ours), the median of
    the differences of alternated rounds and its interval at 95 % by
    bootstrap: k-nucleotide -0.71 % [-1.26, -0.10], fasta -0.23 % [-0.79,
    +1.21], reverse-complement +0.35 % [-0.38, +0.80], regex-redux +0.02 %
    [-0.22, +0.10], pidigits +0.05 % [-0.02, +0.11], fannkuch-redux -0.04 %
    [-0.12, +0.21]; the 4 others spend nothing in these functions. The
    ceiling of this step was their share of the programs (4.24 % of
    regex-redux at most); its gain is the end of glibc for them.
  - Tests: `isc-mem` against the standard library for every length to
    600 and larger ones, every alignment, the moves that overlap at every
    distance to 40 and more, the paths past the caches called themselves,
    for x86-64, v3 and v4 (`tools/suite.py`); `memory_functions.rs`: a
    program copying and comparing strings of every length, for each
    instruction set, the result of the interpreter, and none of these
    functions imported.
- **D289 — Static programs without a C library under Linux (2026-11-04;
  the steps B2 and B3 of D287).** A program that declares no C function
  (and is built without `--link`, `ISC_CC`, `ISC_ALLOC=glibc`, or the
  option `glibc` of `BuildOptions`) is a static position-independent
  executable (`-static -pie --no-dynamic-linker`: its addresses random, as
  before), its object and the runtime only, its entry `isc_start`
  (`process.rs`).
  - `isc_start`, in assembly: the relocations of the program
    (`R_X86_64_RELATIVE` of `DT_RELA`, the only kind of a static PIE) to its
    load address (`__ehdr_start`), before any code that reads an address
    from memory; then `start`: the program headers (`PT_TLS` noted,
    `PT_GNU_RELRO` made read-only, as the loader does), the thread-local
    storage of the main thread (a static buffer of 4 KiB, a mapping if it
    does not fit; the block below the thread pointer, as the linker counts
    it for x86-64, the control block above it, its first word pointing to
    itself: `%fs:0`, the only word of it the code reads), the bounds of its
    stack (from above `AT_EXECFN` down by `RLIMIT_STACK`), `main`,
    `exit_group`.
  - The threads by `clone`: one mapping each (a guard page, the stack
    reserved, not committed, `MAP_STACK`, the thread-local storage and the
    record of the thread at its top); the new thread starts in the block of
    assembly of `spawn`, which it never leaves, calls `thread_main` (the
    bounds of its stack, its function, its cache of the heap given back),
    then `exit`; `join` waits on the word the kernel clears and wakes at
    the end of the thread (`CLONE_CHILD_CLEARTID`, a futex that is not
    private), then unmaps. No key of pthread: the threads of the runtime
    give their cache back themselves.
  - glibc kept for the programs that declare C functions (their code needs
    it: its start, its threads, its allocator for `ISC_ALLOC=glibc`):
    `isc_os`, the table of these functions, is defined at the link
    (`--defsym=isc_os=isc_os_own` or `isc_os_glibc`), and the other table,
    with the C functions it calls, is dropped by `--gc-sections` (an
    undefined symbol of a dropped section is no error for LLD, tried first).
  - Valgrind (the tests of SPEC section 15, no leak) follows only the
    blocks of glibc's allocator: those tests build their programs with
    glibc (`compile_glibc`); the second run of the suite (`ISC_ALLOC=glibc`
    in the environment of the builds) links all its programs with glibc:
    both kinds of programs tested. In a static program `ISC_ALLOC=glibc`
    does nothing. An `iscc` without LLD inside (a build of development)
    links the static programs with the `rust-lld` of the Rust toolchain.
  - The first chunk of the heap in pages of 4 KiB (`MADV_NOHUGEPAGE`): this
    machine gives huge pages to any mapping (`transparent_hugepage`
    `[always]`), and the first touch of the chunk cleared a page of 2 MiB
    at every start: 100 huge pages for 100 starts of the empty program, 0
    after. Tried first and wrong: the chunk without `MADV_HUGEPAGE` only
    (0.19 ms of processor as before: the kernel gives the huge page
    anyway). The later chunks keep their huge pages (D79).
  - The empty program (`print(1)`, `perf stat -r 500`, the creation of the
    process by `perf` counted): with glibc 0.684 ms, 0.37 ms of processor,
    68 page faults (200482 cycles in the program, 947357 in the kernel);
    static 0.463 ms, 0.19 ms, 15 faults (17002 cycles in the program, 4637
    instructions, 577362 in the kernel); with the first chunk in pages of 4
    KiB 0.327 ms, 0.05 ms, 18 faults (143503 cycles in the kernel); an
    executable that does nothing but end: 0.296 ms. The binaries are
    smaller (fasta 209184 bytes against 231744).
  - Safety as `isc-mem` (D288): the stack of the kernel, the headers of the
    program, the image of the thread-local storage, the buffer of the main
    thread and the mapping of each thread become slices once; the rest is
    safe code on them; left in `unsafe` blocks: the system calls, the block
    of `clone` (the new thread starts inside it), the thread pointer, the
    record of a thread. 87 lines of code inside `unsafe` blocks in
    `process.rs`, against 156 in its first version.
  - The ten programs, linked with glibc (8c30abd) against static, the
    median of the differences of alternated rounds and its interval at 95
    % by bootstrap: fasta -5.81 % [-6.19, -5.07], reverse-complement -1.16
    % [-1.46, -0.71], binary-trees -0.54 % [-1.16, -0.11], spectral-norm
    -0.22 % [-0.34, -0.17], mandelbrot -0.12 % [-0.14, -0.04]; no
    measurable difference for regex-redux +0.16 % [-0.09, +0.42],
    k-nucleotide -1.09 % [-1.58, +0.41], pidigits -0.03 % [-0.18, +0.08],
    fannkuch-redux +0.10 % [-0.10, +0.38]. n-body +0.06 % [+0.06, +0.07] in
    that run, not found again in the next (-0.04 % [-0.93, +0.24], 15
    rounds, the machine noisier: intervals of about 1.5 %); its code is the
    same: to be measured again on a quiet machine (TODO.md).
- **D290 — The parallel loops of the runtime, safe (2026-11-04).** The
  tasks of the pool take a pointer to their context, which each task of
  the runtime turned back into its context by hand and wrote its results
  through (the most of the code outside the checks of the compiler in
  `lib.rs`). `par.rs`: `run(n, order, closure)` and `parts_mut(Parts,
  order, closure)`, which gives each task its own `&mut` part of a slice
  cut at increasing bounds (checked), as rayon; the only code that turns
  the context into the closure and the bounds into slices is there. The
  closures are `Copy` (references and numbers): the copy of a context of
  at most 64 bytes in the job line (D201) is a copy of the closure. First
  use: `lines` (`isc_lines`), its parts counted and filled through
  `parts_mut`, its array of strings made zero (a new large block is zero
  already, D79), `str_array` safe; `IscStr` is `Send` (it owns its bytes or
  points to constant ones). The lines of `lib.rs` outside the checks: 1378,
  then 1310. reverse-complement +0.10 % [-0.53, +0.53], fasta -0.15 %
  [-1.07, +0.77] (41 rounds each): no difference.
  - The question of the author (a runtime written in Isocron now): its
    pure parts can be (`lines`, `split`, `replace`, sorting, parsing), as
    modules shipped with the compiler, the interpreter then running the
    same code; its core cannot yet: the allocator, the pool, the start and
    the system calls need primitives the language does not have (regions
    of memory, state shared between threads, atomics, typed system calls),
    to be designed in SPEC first. So the effort in Rust goes to what stays
    in Rust (the allocator, the pool, the system), not to the functions
    that will move to Isocron.
- **D291 — The design of the runtime written in Isocron (SPEC section 17,
  2026-11-04).** The author asked why the runtime is not written in
  Isocron now. Its pure parts can be; its core needs primitives the
  language lacks. The design: modules shipped with the compiler, the only
  ones that may import `isocron:sys`; a closed list of primitives (typed
  system calls on arrays, regions of raw memory as unique values, the two
  primitives that turn a block of a region into the memory of an array and
  back, shared memory with atomic accesses only and the futex, threads and
  their local region, the start in assembly), each with its contract, in
  the compiler and in the interpreter (the same results everywhere); the
  rest of the runtime checked as any program. The order of the work, and
  three questions added to section 16: linear values (uniqueness as a
  guarantee, not an optimisation), the engine of regular expressions and
  the big integers (rewritten or kept in Rust), the access to explicit
  vector instructions from the modules of the runtime.
- **D292 — An affine recurrence modulo a constant computed in parts from
  powers of its step (2026-11-04).** Part 1 of the goal of 2026-11-04
  (each of the ten programs under 3 times its floor): fasta was 7196 µs,
  3.11 times its floor of 2.313 ms. With N = 1 it still took 1420 µs: in a
  profile of 200 runs, `main` had 17.7 % of the samples, 99.9 % of them in
  the loop of `generator()` (`fold(iota(IM - 1), fill(IM, 42), (v, t) =>
  v.with(t + 1, (v[t] * IA + IC) % IM))`), and the workers waiting for it
  57 %. The model of a step: one chain of `imul` by a (3 cycles), `add`,
  the multiplication of the remainder by its constant (4), its shifts (2),
  `imul` by m (3) and `sub`, about 14 cycles a value, 14.50 measured (one
  thread), while the three other threads wait; the floor computes the
  period in lanes, 1.029 ns a value with its symbol. A power of an affine
  step modulo m is again one, f^s(y) = (A y + C) % m, its (A, C) found by
  squarings. Rule R (SPEC section 6, `docs/language.md`): a `fold(iota(n),
  init, (v, t) => v.with(t + 1, (v[t] * a + c) % m))` (the product and the
  sum in either order, either of them possibly absent), with integer
  literals a >= 0, c >= 0, m >= 2 such that (m - 1) a + c and m (m - 1)
  fit in an `i64`, computes v[1] as written; when v[1] is in 0 .. m and v
  has the n + 1 elements written, v[s] = f^(s - 1)(v[1]) for s in 2 .. n + 1
  in one part of consecutive indices per thread (at least 8192 values a
  part, on the pool; in a task, on its thread), each part as 8 chains
  advanced together, each from its power of f, so that their steps
  overlap. From 0 <= y < m no step and no power wraps: the same integers as
  the loop. Otherwise (fewer than 64 steps, an update out of bounds and its
  panic, v[1] outside 0 .. m) the loop as written. At fixed frequency: the
  generator alone 14.50 -> 2.99 cycles a value on one thread (the model:
  three multiplications a step on one port, 3 cycles), 0.633 -> 0.023 ms on
  4 threads; alternated rounds, medians, 95 % intervals by bootstrap:
  fasta N = 1 1420 -> 810 µs (-44.45 % [-46.72 %, -40.44 %], 61 rounds),
  fasta N = 25000000 7196 -> 6615 µs (-7.39 % [-8.41 %, -6.89 %], 61
  rounds), 2.86 times its floor. The LLVM IR of the nine other programs is
  unchanged. The same outputs. Test `affine_recurrences_match_the_interpreter`
  (each form of the step, a multiplier above m, the largest m, a first
  value far above m, the paths of the loop chosen at compilation and when
  running, the panic of an update out of bounds, in the tasks of a `map`;
  1 to 8 threads).
- **D293 — A `filter` read only by arrays written by blocks, made by
  windows (2026-11-04).** Part 1 of the goal of 2026-11-04:
  reverse-complement took 57764 µs, 3.43 times its floor of 16.832 ms
  (the file read twice by blocks in the caches). In a profile of 20 runs
  the bases of each record (`filter` of the newlines, up to 125 MB) were
  written by 24.9 % of the samples and counted by 11.5 %, the lines that
  read them back reversed took 23.1 %, the clearing of new pages 8.3 %, the
  count of the headers 11.6 %, the pool waiting 11.2 %: the bases go to
  new pages and to memory and come back from it, where the floor never
  stores them. Now `let x = filter(src, p) in body`, when `body` reads `x`
  only by its length and, in the function of a `map` over an `iota`
  written by blocks (D225), by indices whose bounds over a range of the
  index of the `map` follow from interval arithmetic (sums, differences,
  products and quotients by literals, remainders by positive literals,
  negations, choices, conversions; an integer of fewer than 64 bits by the
  range of its type; the variables bound outside the `map` as they are):
  the elements that `p` keeps are counted by blocks of 1024 in parallel and
  the place of each block in the result (the sum of the counts before it)
  is kept with the length; each task of the output (4096 elements) computes
  the bounds of the indices it reads over its range widened by 64 at each
  end (the lanes of its vectors), finds the blocks that hold them (by
  halves in the places) and filters them again into a buffer on its stack
  (32 KiB, the heap beyond), read at the same indices: the array is never
  made whole, its parts stay in the caches. Each read keeps its check
  against the length (the same panic). Before the blocks, the bounds over
  the whole `map` are computed with checks of overflow: with an overflow,
  or a window of the first task wider than 4 x 4096, the array is made
  whole for that output (each block written from its place, in parallel).
  At fixed frequency, alternated rounds, median and 95 % interval by
  bootstrap: reverse-complement 57764 -> 41443 µs (-28.33 % [-28.69 %,
  -28.06 %], 31 rounds), 2.46 times its floor; medians of 15 runs: 229.65
  -> 164.47 ms of CPU, 1442 -> 63 page faults. What remains (profile): the
  lines with their windows 52.8 % of the samples, the count of the headers
  16.1 %, the count of the bases 14.2 %, the pool waiting 6.6 %. The LLVM
  IR of the nine other programs is unchanged. The same outputs. Test
  `filters_read_by_windows` (reversed lines, a stride and an offset, an
  index of 8 bits, a window too wide, elements of 8 bytes on the heap, an
  index past the end and nothing kept with their panics, sizes around the
  blocks, 1 to 8 threads, the interpreter, valgrind).
- **D294 — The small blocks carved from spans of huge pages, one per
  thread (2026-11-04).** Part 1 of the goal of 2026-11-04: k-nucleotide
  took 1083 ms, 4.04 times its floor. Variants of the program, each with
  one of its counts (medians of 7 runs): the reading, the lines, the join
  and the codes alone 262.0 ms; with the 1-mers 297.6, the 2-mers 301.6,
  the 3-mers 305.4, the 4-mers 305.9, the 6-mers 311.2, the 12-mers 556.9,
  the 18-mers 571.9 ms (the floor reads the file in 27 ms). The codes are
  not an array of `i64` read again by each count (codes of 8 bits gave the
  same times: 297.8 against 297.7 ms, 560.8 against 557.9 ms): they are
  fused with their reads. In the profile of the reading alone, `refill`
  (the small blocks of the 4.2 million lines) had 33.97 % of the samples,
  83 % of them in the wait for the lock of the chunks (`pause` 32.13 %,
  `lock cmpxchg` 50.75 %), the clearing of new pages 11.36 %: `carve_many`
  wrote the headers of its blocks under the lock, and with them the first
  writes into the new huge pages, cleared by the kernel 2 MiB at a time,
  while the other threads waited. The headers written after the lock
  (only the range reserved under it) made it slower: 262219 -> 281571 µs
  (+7.25 % [+6.81 %, +7.74 %]), the wait gone (`refill` 6.42 %) but the
  clearing 36.55 % of the samples for the same 402 huge pages a run. A
  program of 4 threads writing 512 MiB of new memory in huge pages shows
  why: taking the next 16 KiB of one pointer each (the threads in the
  same huge pages at once) 89.1 ms and 350 ms of CPU, each its own 128
  MiB 39.3 ms and 150 ms, the same 256 huge pages. Now each thread carves
  its small blocks of less than 16 KiB from a span of its own, taken under
  the lock from the chunks and ending at the end of a huge page, so that
  each huge page is written first by one thread; the first chunk of the
  process, in pages of 4 KiB (D289), is still carved by all the threads
  one block after the other under the lock (a page touched by two threads
  at once is cleared twice at most 4 KiB), then an atomic flag sends every
  later carving to the spans without the lock. Two steps were measured on
  the way: the spans alone (each one from the start of its huge page)
  made binary-trees 3.58 % slower ([+3.32 %, +4.67 %]): the same
  instructions (26.43 against 26.47 billion), as many misses of the L1
  cache, twice the clears of the pipeline for the order of memory (1.10
  and 1.30 million against 2.71 and 2.39), all in the making of the
  trees, with 4 threads only (on one thread the same cycles) and no line
  shared between the cores (6 and 9 loads hitting a line modified by
  another core in `perf c2c`): the blocks the threads carve in the same
  order were at the same places of their huge pages. Spans starting at
  colors (8, by steps of 8 KiB and 192 bytes) took the clears back to 1.68
  and 1.42 million, binary-trees +0.50 % [-0.43 %, +1.29 %] then +1.22 %
  [+0.09 %, +2.21 %]; with the first chunk shared as before, none left.
  Which way through the caches the same places make these clears is not
  shown (TODO). At fixed frequency, alternated rounds, against D293: the
  reading of k-nucleotide alone 262472 -> 228813 µs (-12.77 % [-13.39 %,
  -12.34 %], 21 rounds; 695.36 -> 563.27 ms of CPU, medians of 15);
  k-nucleotide 1085220 -> 1048271 µs (-3.40 % [-4.30 %, -2.71 %]);
  binary-trees -0.13 % [-0.36 %, -0.01 %]; the eight others between
  -0.14 % and +0.03 %, every interval across 0. Test
  `lines_of_a_large_text_made_in_parallel` (also 40 MiB of lines, many
  spans and chunks for each thread, and 8 threads).
- **D295 — The rounds of scatter of a grouping bounded together, larger
  for few parts (2026-11-04).** Part 1 of the goal of 2026-11-04:
  k-nucleotide after D294, 1048 ms, 3.9 times its floor. The variant of
  the program with the 12-mers alone costs about 87 instructions and 27
  cycles a key over the reading alone (`add_block` 41 instructions,
  `count_round` 30, the keys 13.7). The traffic of memory measured by the
  counters of the memory controllers: 6754 MiB read and 4295 MiB written
  for the reading and the 12-mers against 3549 and 2694 MiB for the
  reading alone, about 38 bytes a key: the lists of a round (2^20 entries
  of 8 bytes a part, 32 MiB for 4 parts) do not stay in the L3 cache with
  the tables of the lists (24 bytes a slot, 3.3 MB a part). Measured and
  not kept: the table of the next list prefetched while one is counted
  (527.2 -> 524.8 ms for the 12-mers, 539.6 -> 536.5 for the 18-mers);
  the slots and their mask held in locals in `count_round` (525.6 -> 522.3,
  537.4 -> 534.2); slots of 16 bytes, the key and the count in one word
  (531.7 against 524.4 by rounds of 2^20, and still 752.9 by 2^17); one
  table a part without scatter, grown at 60 % (612.5 and 615.2 ms); in a
  separate program, the keys of each thread's quarter of the positions
  into its own table (1019.8 ms) or each thread owning a quarter of the
  keys, the keys exchanged by rounds between barriers (610.1 to 643.6 ms),
  both slower than the scatter of the runtime with its keys made (about
  295 ms a pass). The rounds measured (the 12-mers, the 18-mers, ms): 2^17
  825.7 and 824.6, 2^18 667.7 and 668.6, 2^19 572.5 and 587.4, 2^20 523.0
  and 538.1, 2^21 504.9 and 518.8, 2^22 495.2 and 508.7, 2^23 521.0 and
  537.1: each round reads again the table of every list of its part, and
  rounds of 64 MiB a part cost more in new pages than they save. The round
  of a part is now `2^24 / parts` entries (128 MiB for all the parts
  together, between 2^16 and 2^22 entries): 2^22 for 4 parts, 2^20 as
  before for 16. At fixed frequency, alternated rounds, against D294:
  k-nucleotide 1050076 -> 991264 µs (-5.74 % [-6.62 %, -4.95 %], 11
  rounds), the nine others between -0.31 % and +0.42 %, every interval
  across 0. Its peak of resident memory 1098740 -> 1297400 KiB. Test
  `keys_scattered_by_their_hash` (also 40 million keys on 16 threads, 16
  parts whose rounds end twice in each).
- **D296 — The blocks of small keys counted in one loop (2026-11-04).**
  Part 1 of the goal of 2026-11-04: k-nucleotide after D295, 991 ms. Its
  profile: `add_block` 41.02 % of the cycles, `count_round` 21.94 %, the
  keys (`isc_task_29`) 15.73 %; in `add_block`, the counting of the small
  keys (by their value, the k-mers of fewer than 9 bases) about 31 % of
  its samples, behind a test of each key (small or not) and of the length
  of the table of the small keys. Now a block of keys whose largest is
  small (found first, the table grown once to it) is counted in a loop of
  its own, without those tests; the other blocks as before. At fixed
  frequency, alternated rounds, median and 95 % interval by bootstrap:
  k-nucleotide 989988 -> 967269 µs (-2.34 % [-3.80 %, -1.51 %], 11
  rounds). The same outputs. Test `small_keys_counted_in_one_loop` (keys
  that grow along the indices, then blocks with large keys among them; 1
  and 4 threads).
- **D297 — The reads of the index of a `fold` in vector code checked
  before its loop (2026-11-04).** Part 1 of the goal of 2026-11-04:
  k-nucleotide after D296. The loop of its keys (`fold(iota(k), 0, (c, j)
  => c * 4 + codes[i + j])` in vector code, 16 keys a step) took 16
  instructions a step, 7 of them the check of the first and the last
  index of its vector (D217), again at every `j`. Now, in a `fold` over an
  `iota` in vector code (not under a mask), the reads `a[p + j]`, `a[j +
  p]` and `a[j]` of its index `j` (`a` and `p` bound before the `fold`, an
  index of 64 bits) are checked in every lane at `j = 0` and `j = n - 1`
  before the loop: `p + j` grows with `j`, so that every index of the
  loop is between two in bounds, and the loop reads them without a check
  (9 instructions a step); when one of them is not, the loop as written,
  with its checks and its panic. Measured and not kept: the pass `irce` of
  LLVM after `default<O3>`, k-nucleotide -1.33 % [-1.89 %, -1.27 %] (7
  rounds) while the check of this loop was still there, a gain not
  explained. At fixed frequency, alternated rounds, median and 95 %
  interval by bootstrap: k-nucleotide 976336 -> 954270 µs (-2.51 % [-3.73
  %, -1.37 %], 11 rounds); the LLVM IR of the nine other programs
  unchanged. With D296, against D295: k-nucleotide 987547 -> 955004 µs
  (-3.26 % [-4.10 %, -1.90 %]), the nine others between -0.26 % and +0.56
  %, every interval across 0. Test `fold_reads_checked_before_their_loop`
  (the results of the interpreter, an empty `fold`, an index past the end
  at the last `j` only and before the start at the first `j` only, with
  their panics; 1 and 4 threads).
- **D298 — The lines of a text made in parallel in one block, freed with
  the last of them (2026-11-04).** Part 1 of the goal of 2026-11-04:
  k-nucleotide after D297, 954 ms. Its reading alone (the lines of the
  input, the headers, the join, the codes) took 228 ms where the floor
  reads the file in 27 ms: each of its 4.2 million lines a small block of
  its own, 128 bytes for a line of 60 (64 of header), carved, cleared by
  the kernel, then freed one by one (D294 profile: the clearing of pages
  19.5 % of its cycles, `refill` 8.9 %, `isc_free` 6.4 %, `spill` 2.5 %).
  Now `lines` (on several threads, from 1 MiB of text) makes its strings
  in one block: before each line the address of the block (8 bytes),
  then its bytes, to the next multiple of 16, 80 bytes for a line of 60;
  each line starts 8 bytes past a multiple of 16, where no block of the
  heap (64), of glibc (16) or of the system starts, so that `isc_free`
  knows it before reading any header; the first 8 bytes of the block
  count the lines alive, the block freed with the last of them (an atomic
  count). The place of each part in the block is bounded in the first
  pass (its bytes, those of its last line past its end, 23 for each line);
  the strings and the parts of the block are given to their tasks as
  slices (`par::parts_mut2`, safe). A line kept after the others keeps
  the whole block (at most the size of the text) until it is freed. At
  fixed frequency, alternated rounds, median and 95 % interval by
  bootstrap: the reading alone 228273 -> 185735 µs (-18.61 % [-19.13 %,
  -18.06 %], 21 rounds); k-nucleotide 953830 -> 904084 µs (-4.96 % [-5.71
  %, -4.22 %], 11 rounds), the nine others between -0.24 % and +0.14 %,
  every interval across 0; the peak of resident memory of k-nucleotide
  1301496 -> 1115492 KiB. The same outputs. Test
  `lines_kept_in_one_block_freed_with_the_last` (lines kept in another
  array and sorted after the array of all the lines is gone, empty and
  `\r\n` lines; 1, 3, 4 and 8 threads; valgrind with glibc's allocator).
- **D299 — The standard output written by blocks into files (2026-11-04).**
  The first step of the audit of every form of the language against the
  machine (asked by the author: the basic functions, `print` among them):
  each `print` ended with a write of the system (`isc_print_newline`
  flushed the buffer), 1000 writes for 1000 lines; a million lines of one
  number into `/dev/null` took 185.5 ms, of a short string 169.8 ms, where
  the bytes cost a few nanoseconds a line. Now the standard output is
  written at each newline only when it is read as it comes (a terminal, a
  pipe, a socket: a program that prints a line and then computes still
  shows it), by blocks of 64 KiB into a regular file or a character device
  that is not a terminal (`fstat`, then `ioctl(TCGETS)` under Linux;
  `GetFileType`, then `GetConsoleMode` under Windows; at each newline
  under WASI), as C's standard output does [only into a file: D375]; all of it is written at the
  end of `main`, before a panic, before the standard input is read and
  before a C function is called (whose own buffer stays C's). At fixed
  frequency: a million lines of a number into `/dev/null` 185.5 -> 22.2
  ms, of a string 169.8 -> 6.5 ms, one line of a million numbers 25.6 ->
  25.2 ms (the formatting of the numbers, next); 1000 lines into a pipe:
  still 1000 writes. The ten programs, alternated rounds: pidigits -0.08 %
  [-0.13 %, -0.04 %], the others between -0.36 % and +0.34 %, every
  interval across 0 but n-body's (+0.02 % [+0.01 %, +0.68 %] in 11 rounds,
  -0.16 % [-0.76 %, +0.73 %] in 31). Test
  `output_by_blocks_into_files_and_by_lines_into_pipes` (more than a buffer
  of lines and `printBytes` into a file, a pipe and `/dev/null`, and a
  panic after them: the bytes of the interpreter).
- **D300 — The decimal text of integers written directly (2026-11-04).**
  The audit of every form, after D299: a line of `print` of a number
  still took 22 ns, its formatting through `core::fmt` (`write!` of `{}`,
  for `print`, `toString` and the templates). Now the digits are written
  two at a time from a table of the pairs "00" to "99", from the end of a
  buffer of 20 bytes, the sign before the digits of the absolute value
  (`i64::MIN` included): the text of the interpreter. Measured since this
  decision on the cores 4 to 7 alone (the cores of measurement of
  `bench/machine.sh`, without their siblings): a session of a desktop
  started on the cores 0 to 3, which the other processes keep during a
  measurement, made the earlier runs on the cores 0 to 7 vary by up to 10
  %. At fixed frequency, alternated rounds, medians and 95 % intervals:
  a million lines of a number into `/dev/null` 22195 -> 12975 µs (-41.73 %
  [-41.98 %, -41.35 %], 21 rounds), one line of a million numbers 25436 ->
  16116 µs (-36.32 % [-36.71 %, -35.92 %]), `toString` of a million
  numbers in a `map` 6776 -> 3540 µs (-47.92 % [-49.46 %, -46.94 %]); the
  ten programs between -0.13 % and +0.19 %, every interval across 0. Test
  `integers_written_as_the_interpreter` (every count of digits around the
  powers of ten, both signs, the extremes of `i64` and `u64`, integers of
  8 and 32 bits, `print`, `toString`, a template).
- **D301 — String literals read where they are; searches of short
  needles (2026-11-04).** The first group of the audit of every form
  (`bench/primitives/`, `results/primitives.md`: each form in Isocron and
  the same work in C the cheapest way known, on one CPU at fixed
  frequency): of 20 functions of strings on lines of 60 bytes, `startsWith`
  took 18.60 ns against 1.17 in C, `endsWith` 18.31 against 0.94,
  `indexOf` of a byte 56.87 against 3.79, `includes` 47.04 against 12.65.
  Two causes. Each evaluation of a string literal made a new string
  (`isc_str_new`) freed after its use: `l.startsWith("Z")` in a loop
  allocated and freed "Z" at each line. And the searches went through the
  general search of `str` (`find`, `contains`), prepared at each call. Now
  a string literal is borrowed data of the program, read where it is, as
  an array of constants (D173): the ownership pass and its checker take it
  as borrowed, a consumer that keeps it gets a copy; a direct tail call
  that passes one keeps it in its loop without owning it (the first
  version made the call of pidigits with `""` a recursion: +7.17 %
  [+6.80 %, +7.22 %]). `startsWith` and `endsWith` against a literal of at
  most 16 bytes are compared in the generated code (its length, then one
  or two integers); a search of a byte is `memchr`, of 2 to 16 bytes their
  first byte by `memchr` then the others, longer `memmem`. On one CPU,
  medians of 5 runs of 200 rounds, before and after: `startsWith` of 1
  byte 18.60 -> 1.90 ns, of 8 bytes 18.10 -> 1.88, `endsWith` 18.31 ->
  1.99, `indexOf` 56.87 -> 7.66, `includes` 47.04 -> 25.94, a
  concatenation with a literal 17.32 -> 9.83, a template 45.85 -> 38.20.
  The ten programs, alternated rounds: k-nucleotide -1.12 % [-1.44 %, -1.01
  %]; regex-redux +0.26 % [+0.11 %, +0.61 %] (31 rounds), +0.20 % [+0.06 %,
  +0.39 %] (61), with the same instructions (49.494 to 49.507 billion for
  10 runs of each) and cycles within their variation from a run to the
  next (16.84 to 16.98 billion for both): not a cost of the change;
  binary-trees +0.59 % in 11 rounds, +0.07 % [-0.18 %, +0.33 %] in 31; the
  others between -0.56 % and +0.26 %, every interval across 0. Tests
  `searches_of_short_needles` (literals of 0 to 17 bytes, texts shorter
  and longer, characters of several bytes, needles of 1, 2 to 16 and more
  bytes, overlapping candidates) and `literals_given_to_tail_calls` (a
  million calls, valgrind).
- **D302 — Case, padding and replacement written once (2026-11-04).** The
  audit of the strings after D301: `replaceAll` of one byte took 825.43 ns
  for a line of 60 bytes against 48.82 in C, `toUpperCase` 186.64 against
  10.48, `padStart` 61.64 against 1.99. Their strings were written through
  `fmt` (`write_char` at each character, into a `String` that grows), and
  `replace`/`replaceAll` of a text went through the engine of patterns,
  the text escaped into a pattern compiled at each call. Now an ASCII text
  is mapped byte by byte into a string of its length; a fill of one byte
  pads into a string of the final length; a replacement without `$`
  patterns of a non-empty text finds its places by `find_bytes` (D301) and
  writes the string of its final length once. The other cases as before
  (Unicode, longer fills, `$` patterns, an empty text: the code of the
  interpreter). On one CPU, medians of 5 runs of 200 rounds: `replaceAll`
  825.43 -> 55.10 ns, `toUpperCase` 186.64 -> 18.29, `padStart` (after a
  `slice`) 61.64 -> 20.57. The ten programs, alternated rounds: between
  -0.82 % and +1.09 %, every interval across 0. Test
  `direct_writes_of_case_padding_and_replacement` (ASCII and Unicode,
  fills of one byte and longer ones, replacements without and with `$`,
  of an empty text, at the ends, overlapping; valgrind).
- **D303 — Parts of strings read where they are (2026-11-04).** In the
  audit, `l.slice(5, 25).length` took 10.11 ns for a line of 60 bytes and
  `bytes(l)[7]` 8.39 ns: each made a new string (an allocation, a copy,
  then its free) only to read it. A slice of an array is a view of its
  source (rule F4); a slice of a string, or its bytes, is now one too when
  the string is read where it is (a variable or a component of one, as
  every operand of an operation on strings after `bind_temporaries`): the
  ownership pass and its checker take it as borrowed from its source, the
  generated code checks `0 <= a <= b <= n` and that each bound starts a
  character or ends the string, as the runtime does (a failed check calls
  the runtime, which stops the program with its message, at the same
  place), then reads the string at `p + a`; `bytes(s)` is the same pointer
  and length. A consumer that keeps one gets a copy, as for every borrowed
  value. The floors of these two forms copied too; the cheapest known is
  the check, then the read: rewritten. On one CPU, medians of 5 runs of
  200 rounds: `slice` 10.11 -> 2.17 ns (its floor with the checks of
  characters 1.32), `bytes` 8.39 -> 1.96 (floor 0.86), `padStart` of a
  slice 20.57 -> 12.24. The ten programs, alternated rounds: k-nucleotide
  -1.50 % [-2.29 %, -0.54 %] (`bytes(sequence)` copied the whole
  sequence); the others between -1.12 % and +0.10 %, every interval across
  0. Test `parts_of_strings_read_where_they_are` (variables, fields,
  elements, parameters, slices of slices, literals; kept in arrays, fields,
  results, concatenations, tail calls, the accumulator of a fold, a map in
  parallel; bounds out of the string and inside characters, against the
  interpreter; valgrind).
- **D304 — Reductions of integers in one accumulator (2026-11-04).** After
  D303, `sum(map(ls, (l) => i64(bytes(l)[7])))` took 1.96 ns an element
  against 0.86 in C: the elements of a map of strings are not taken by the
  vector loops, and the scalar loop of the fixed form (section 10) kept its
  V accumulators in memory, `acc[i mod V]`, a load and a store for each
  element. For `+`, `*`, `min` and `max` of integers (modular: associative
  and commutative, the argument of rule P) from their neutral value, every
  order of the elements gives the value of the fixed form; that loop has
  now one accumulator, a register. From another value (V copies of it in
  each block) the fixed form as before. With one accumulator LLVM
  vectorized the `count` of a `startsWith` by gathers, 4.09 ns an element
  against 1.95: the loop is not vectorized by LLVM (its elements are behind
  pointers). On one CPU, medians of 5 runs of 200 rounds, before and after:
  `bytes` 1.96 -> 1.63 ns, `slice` 2.17 -> 1.98, `equal` 5.66 -> 4.94,
  `less` 5.57 -> 4.90, `startsWith` 1.97 -> 1.75, `endsWith` 2.07 -> 1.78,
  `indexOf` 7.65 -> 6.82. Of the ten programs, only binary-trees is
  another binary: +0.13 % [-0.13 %, +0.31 %], the same output. Test
  `integer_reductions_of_strings_in_one_accumulator` (`+`, `*`, `min`,
  `max`, `count`; wrapping sums and products; 8 to 64 bits, signed and
  unsigned; 1 to 300000 elements; neutral and other initial values; 1, 3
  and 8 threads; against the interpreter).
- **D305 — Lines and parts of a text in one block (2026-11-05).** In the
  audit, `lines` of a text took 47.21 ns a line of 60 bytes against 8.51
  in C, `split` of a line of 6 parts of 5 bytes 170.23 ns against 32.09.
  Each part was a block of the heap of its own (an allocation, then its
  free), the text was split twice (to count, then to make) by `str::split`
  (a character: 29 ns a line; a text: the general search, prepared at each
  call), and an array of strings was freed by a call for each. Now the
  parts of `lines` and `split` are strings of one block, as the lines of a
  large text (D298), whatever its size: one allocation, each part freed
  with the last; a separator of one byte (and the `\n` of the lines) is
  found 32 bytes at a time, compared into the bits of a mask (written so
  that LLVM makes it one vector comparison, no unsafe code), the last
  bytes of the text read again in its last 32 (copied into 32 bytes on the
  stack, they were read before their stores were done: 37 % of a `split`),
  a longer one by `find_bytes`; the parts are found in a generic function
  compiled with their writer, a short one copied by 16 bytes. The block
  holds its size: an array of strings is freed by the runtime
  (`isc_free_strs`), the strings of a block one after the other counted
  down at once, by their address, without reading them (an atomic
  operation, and a line of cache, for each before). Tried and not kept:
  the end of the text as one more place of the mask, for one call of the
  writer (inlined, but more instructions: `split` 75.26 -> 83.75 ns);
  `memchr_iter` (`split` 120.67 ns, a call for each part). On one CPU,
  medians of 5 runs of 200 rounds: `lines` 47.21 -> 15.34 ns, `split`
  170.23 -> 75.87. What remains of `split` (2.4 times its floor), by
  instructions: the two passes and the writes 61 %, the free of the
  strings 20 %, `memcpy` of the last part of each line 9 %, the
  allocations about 6 %; the floor allocates nothing and frees nothing.
  The ten programs, alternated rounds: k-nucleotide -2.78 % [-3.56 %,
  -2.07 %] (the 4166671 lines of its input freed by blocks, not one by one); the
  others between -1.66 % and +0.08 %, every interval across 0, the same
  outputs. Test `lines_and_parts_in_one_block` (empty texts, lines and
  parts, `\r\n`, separators of one and several bytes at both ends and one
  after the other, characters of several bytes; parts kept after their
  array, sorted, filtered, two blocks in one array, strings of a block
  among others; a text of 3 MiB; 1, 3 and 8 threads; valgrind).
- **D306 — The fixed alternatives searched by lanes (2026-11-05).** Of
  the 150.21 ms of regex-redux, the 9 counts took 41.1 ms: one count, on
  one thread, 0.29 ns a byte, where the floor's scan for its two literals
  takes 0.074 (`bench/floor/floor.c`, `literal_pair_scan`). The fixed plan
  (D275) took 288 instructions a block of 64 bytes (225 million for a
  count of the 50000000 bytes of the cleaned text, 4.8 a cycle): a test per class and its store, then
  for each place of each alternative its class read, its length tested,
  two masks loaded. Now, with the table of classes (D278) on a processor
  with GFNI and alternatives of at most 16 places, the masks of the classes
  of a block are the lanes of one vector, in five instructions: the
  classes of each byte looked up, the bytes of each lane of 8 reversed,
  their bits transposed by `gf2p8affineqb`, byte c of each lane into lane
  c; each alternative takes its places 8 at once (a permutation of the
  lanes by its classes, for this block and the next, the shifts of their
  places, an `and` of the lanes); the matches, their choice and order as
  before. The intrinsics are safe in their functions of `target_feature`;
  the loads of 64 bytes and the call, after the test of the processor, are
  the only unsafe code, as for D278. On one thread, a count: 0.29 -> 0.20
  ns a byte, 111 instructions a block. Tried and not kept: the general
  plan compiled for each length of 2 to 12 places, 0.71 ns a byte: fewer
  instructions (4150 against 4498 million for 20 counts), 2.8 times the
  cycles, stalled at the front of the processor (uops not delivered 126.5
  -> 2014.1 million, switches of the cache of decoded uops to the decoders
  0.34 -> 48.8 million cycles; the code of 11 cases in each of the 8
  variants no longer fits in that cache). regex-redux -13.31 % [-13.63 %,
  -13.02 %] (the counts 41.1 -> 26.8 ms); the others between -1.83 % and
  +0.12 %, the intervals across 0 except fasta at -1.26 % [-1.84 %,
  -0.09 %]: its binary the same before and after, measured again over 121
  rounds -0.03 % [-0.49 %, +0.44 %] (the interval of 61 rounds is not a
  proof alone). The tests of `isc-runtime-core` now also run for
  x86-64-v4 in the suite, the only build where its searches of AVX-512 are
  compiled (D278 was tested only through compiled programs against the
  interpreter). Test `lanes_find_the_matches_of_the_masks` (alternatives of
  1 to 16 places, texts of 0 to 4099 bytes, bytes outside ASCII, every
  start and end).
- **D307 — The Benchmarks Game measured again at `d49c22a` (2026-11-05).**
  The README still showed the publication of D262 (`c77e61b`), before D264
  to D306. The protocol of D249 and D260 again (the image with the iscc of
  the host, every program built and checked: 102 outputs the same as their
  references; `bench/machine.sh apply parallel`, 4 threads, the lowest of
  3 runs), the Windows VM stopped. Isocron, against D262: fasta 18 -> 11
  ms, k-nucleotide 1070 -> 863 ms, regex-redux 289 -> 129 ms,
  reverse-complement 61 -> 42 ms; the fastest of 10 of the 10 programs;
  over their floors fasta 4.8, regex-redux 4.3, k-nucleotide 3.2,
  reverse-complement 2.5 (under 3 now). The six others are slower in this
  run (binary-trees 669 -> 698 ms, spectral-norm 195 -> 199, n-body 1897
  -> 1919, pidigits 490 -> 496, fannkuch-redux 1663 -> 1677, mandelbrot
  639 -> 640), and so are the programs of the other languages, the same
  binaries as in D262 (Rust binary-trees 868 -> 877 ms, C++ k-nucleotide
  1176 -> 1210, Rust spectral-norm 205 -> 212, C fannkuch-redux 2467 ->
  2475): the state of the machine during the run. The six built by the
  compiler of `c77e61b` and by that of `d49c22a`, alternated rounds on the
  cores of the measurements: binary-trees -0.65 % [-1.76 %, -0.14 %],
  spectral-norm +0.43 % [-0.65 %, +0.58 %], n-body +0.05 % [-0.27 %,
  +0.90 %], pidigits -0.14 % [-0.42 %, +0.19 %], fannkuch-redux -0.05 %
  [-0.12 %, -0.00 %], mandelbrot +0.01 % [-0.09 %, +0.22 %]: none slower.
  `results/benchmarks_game.md`, the summary of the README and its chart
  (`report.py`), the table of `results/floors.md` (`floors.py`) made
  again; its attribution of the gaps (D263) stays that of `c77e61b`, said
  so.
- **D308 — The text between the matches of a replacement moved by 32
  bytes (2026-11-05).** A replacement of regex-redux (`a[NSt]|BY` by
  `<2>`: 3522378 matches, 53516158 bytes written) took about 22 ms on 4
  threads: its search 9.4 to 9.7 ms, its sizes 1.3 ms, its writing 10.9 to
  11.1 ms (timed in a build of its phases, not kept), 12.4 ns of CPU a
  match to write. Of the 21.0 million branches mispredicted by 8 such
  replacements, 83.92 % were in its writing: `copy_short` chose among four
  moves by the length of the text between two matches, a length the text
  decides (about one mispredicted every two matches). Now that text, up to
  32 bytes, is one move of 32 where the text and the part of the result of
  the thread have the room, the bytes past it written for nothing (the next
  writes cover them, never past the part, which another thread writes).
  8 replacements: 19.0 -> 14.0 ms each, 21.0 -> 4.6 million branches
  mispredicted; the result the same bytes (the five replacements of
  regex-redux, against the compiler of `c77e61b`). The ten programs,
  alternated rounds: regex-redux -9.65 % [-10.36 %, -9.23 %] (its
  replacements of `a[NSt]|BY` 24.9 -> 19.5 ms, `<[^>]*>` 24.1 -> 18.9 ms,
  `\|[^|][^|]*\|` 12.9 -> 10.8 ms); the others between -0.22 % and +0.71
  %, every interval across 0, the same outputs. Test
  `replacements_written_by_moves_of_32_bytes` (gaps of 0 to 44 bytes,
  matches at both ends, replacements longer, shorter and empty, a text of
  2.9 MB; 1, 3 and 8 threads, as the interpreter).
- **D309 — A part of a search stopped at the next part (2026-11-05).** In
  regex-redux the searches of few matches took longer than those of
  millions: `tHa[Nt]` (6220 matches) 6.3 to 6.6 ms, `aND|caN|Ha[DS]|WaS`
  (15867) 10.1 ms, on 4 threads, where one thread searches the text at 0.165 and
  0.265 ns a byte. Timed part by part (a build of its parts, not kept),
  they ended at 2125, 6389, 4204 and 2157 us for `tHa[Nt]`, 3353, 10081,
  6602 and 3337 for the other. Each part searched up to the first match
  that starts at the next part or after (the merge of D276 resumes there):
  where no match comes after, as `H` in the last sequence (lower case
  only), to the end of the text: the part 1 from 12.5 MB to the end, 37.5 MB (its
  time three times that of the last part).
  Now a search by masks (by lanes, by the fixed plan, by the general one)
  stops at the start of the next part when no match crosses it (its end of
  matches at or before it): from there its search and that of the next part
  are the same, both from that start with no match before; and the merge,
  at the end of the matches of a part that stopped there (or that met no
  more), goes on with the next part from its start. A match across the
  start: as before. A search from `at` up to `at` (`find_at`, the first
  match) does not stop. The automaton's searches unchanged. regex-redux
  -9.76 % [-10.31 %, -9.41 %] (`tHa[Nt]` 11.6 -> 7.5 ms, `aND|...` 15.3 ->
  8.7, the counts 26.6 -> 24.0); the binaries of the nine others are the
  same bytes (k-nucleotide +0.41 % [+0.08 %, +0.60 %], binary-trees +0.46
  % [+0.06 %, +1.52 %], reverse-complement +0.03 %: the same program twice,
  the intervals of 11 and 61 rounds are no proof alone, as in D306); the
  same output on 1, 3 and 8 threads. Test
  `parts_stopped_at_the_next_are_one_search` (texts whose last half has no
  match, and the reverse; parts cut inside matches and at their ends;
  fixed and delimited patterns, by lanes and by masks; merged against one
  search); the tests of `until` take the new rule (`up_to`).
- **D311 — The fixed alternatives searched by groups of 8 blocks
  (2026-11-05).** After D309 the 9 counts of regex-redux took 24.0 ms of
  its 104.7: a count 0.20 ns a byte on one thread, 111 instructions a block
  (D306), of which the `and` of the 8 lanes of each alternative (7
  instructions each, a chain of their latencies). Now the masks of the
  classes of 8 blocks of 64 bytes (5 instructions each, D306) are
  transposed, 8 by 8 lanes of 64 bits: lane i of vector c the mask of class
  c in block i; each place of an alternative is the vector of its class
  shifted by its place, with the next block's (the lanes moved by one),
  anded lane by lane: no reduction; a group without a start of match is
  left at once, the starts of the others taken block by block as before
  (their choice, their order, the stop of D309). The classes and shifts of
  each place prepared once for a search. A prototype of the method, alone,
  0.063 ns a byte (the 1502 matches of a naive count); in the engine a
  count 0.20 -> 0.097 ns a byte, 74 instructions a block before the places
  prepared once (0.106). The floor's scan written the same way
  (`literal_lanes_scan` in `bench/floor/floor.c`: the pattern known, its
  shifts constants) takes 0.0636 ns a byte against 0.0747 for
  `literal_pair_scan`; `floors.py` takes the cheaper, and the floor of
  regex-redux does not move (its scan is bound by the reading of the text
  from memory, `max` of the two). Only that constant added to
  `results/machine_floor.txt`: the run of `bench/floor/run.sh` that measured
  it moved 12 others by 2 to 8.9 % (the state of the machine:
  `tree_loop_depth20` +8.9 %, `random_increment_256KiB` -8.8 %), the floors
  of the other programs kept. Tried and not kept: the 8 loads of a group in
  the text without a test, the same time (0.096 to 0.101 ns a byte against
  0.097 to 0.101). regex-redux -14.53 % [-15.02 %, -12.82 %] (104.7 ->
  89.6 ms: the counts 24.0 -> 12.9 ms); k-nucleotide and pidigits, other
  binaries (the runtime they link), +0.33 % [-0.20 %, +0.63 %] and -0.05 %
  [-0.13 %, +0.17 %]; the seven others the same binaries; the same output
  on 1, 3 and 8 threads. The search by lanes of D306, single blocks, gone.
  Tested by the tests of D306 and D309 (`lanes_find_the_matches_of_the_masks`
  against the fixed plan, `parts_stopped_at_the_next_are_one_search`,
  against the automaton), in the build for x86-64-v4 of the suite.
- **D312 — The 17 cycles of a step of n-body explained by measurement
  (2026-11-05).** D261 left 17 of the 118.8 cycles of a step outside its
  model of latencies (98.0 cycles, 101.5 with the divider), given to the
  ports and the window of the scheduler without a measure. Measured now,
  one thread on its core, its sibling idle, 50 million steps: 119.62
  cycles and 243.00 instructions a step; the ports of the vector units 0
  and 1 receive 78.75 and 79.47 uops a step, the port 5 48.84, the port 6
  1.00, the loads 28.00, the stores 14.00 and 14.00; no uop executes in
  11.43 cycles a step. The loop of the step (its 243 instructions, from
  the assembly of the compiler) simulated by `llvm-mca` (LLVM 21.1.8, the
  model of Rocket Lake, the same for Ice Lake: latencies, ports, divider,
  window of the scheduler), built from the tree of LLVM of the releases
  (the target `llvm-mca` alone): 116.04 cycles a step, 97.0 % of the
  measure; its analysis of bottlenecks: resources in 59.43 % of the cycles
  (port 0 43.07 %, port 1 39.62 %, divider 18.08 %, port 5 13.80 %),
  registers in 74.07 %. So the chain of latencies (101.5) waits about
  14.5 cycles a step for the ports 0 and 1, busy two cycles of three with
  the other operations of the step, and the divider; the last 3.6 cycles
  (3.0 %) are what the simulation leaves out (the front of the processor,
  the 39 accesses to the stack [30 after the split of D261: D375]). n-body is 1.5 times its floor.
- **D313 — The parts of a search by masks and of a replacement, several
  per thread, each thread its range first (2026-11-05).** Profiled alone
  (`a[NSt]|BY` on the 50 MB of regex-redux, 40 calls, 4 threads): the four
  threads have the same time, but the caller spends 14.05 % of all the
  samples in `parallel_for`, waiting, and does about a quarter of the
  search and of the writing of each other thread (`bounds_groups` 2.18 %
  against 9.81 to 9.87 %, the writing 1.78 % against 8.84 to 9.23 %); the
  same wait in `<[^>]*>` (13.19 %) and `\|[^|][^|]*\|` (13.94 %). The parts
  were one per thread, of the same bytes, and their cost follows their
  matches: the first quarter of the text (its first sequence, in capitals,
  without `a` nor `B`) has 210332 of the 3522378 matches, the three others
  1051544 to 1141369; the caller's part is the first.
  - 8 parts per thread taken in turn (`PARTS_PER_THREAD`, for the search by
    masks, the sizes and the writing; the automaton keeps one part per
    thread, its cache): the waiting gone (1.91 % for the caller), the three
    replacements repeated 4.3 to 6.5 % faster, but regex-redux +1.91 %
    [+1.17 %, +2.67 %] (31 rounds): its results are new pages, and threads
    writing neighbouring parts of 1.5 MB fault the same huge pages together
    (D254): 165 to 170 million cycles of the kernel -> 221 to 223 million,
    the cycles of the program 970 to 984 million -> 940 to 948 million.
  - Kept: each thread takes the parts of its own contiguous range first,
    from its first, then those left in the ranges of the others from their
    ends (`RANGES`, `take_range`: the first and the end of each range in
    one word of 64 bits, taken by one exchange, no task taken twice; a job
    `dynamic` and `contiguous` at once; more than 64 threads or 2^32 tasks:
    only dynamic). The threads write their own pages first and meet only
    at the ends of their ranges. Repeated, one run of 10 calls each:
    `a[NSt]|BY` 13.38 -> 12.37 ms, `<[^>]*>` 17.08 -> 15.74 ms,
    `\|[^|][^|]*\|` 11.26 -> 10.24 ms. regex-redux -1.59 % [-2.27 %,
    -0.61 %] (31 rounds), then -1.70 % [-2.66 %, -0.76 %] (31 rounds, 90.56
    -> 88.96 ms) with the ten programs, all of them other binaries (the
    runtime they link): fasta -1.10 % [-1.94 %, -0.56 %], reverse-complement
    -0.20 % [-2.91 %, +1.55 %], k-nucleotide +0.02 % [-0.47 %, +0.31 %],
    pidigits -0.15 % [-0.23 %, +0.00 %], fannkuch-redux +0.01 % [-0.11 %,
    +0.11 %], binary-trees -0.08 % [-1.13 %, +0.09 %], mandelbrot -0.01 %
    [-0.02 %, +0.05 %], n-body +0.01 % [-0.01 %, +0.03 %], spectral-norm
    +0.01 % [+0.00 %, +0.01 %]. The kernel still spends 20 to 26 million
    cycles more (164 to 168 million -> 185 to 194 million): by `perf -g`,
    the zeroing of new pages under `heap::alloc_medium` (2.52 % of the
    samples, none before), the 32 lists of matches of a search now medium
    blocks of about 2 MB where there were 4 of 14 MB; the next step, the
    sizes of the parts taken from the search (no list read again), is
    where they are measured again.
  - Tested by `uneven_parts_finished_by_the_other_threads`
    (`crates/iscc/tests/strings.rs`): a text of more than 1 MiB whose first
    half has no match, a count and replacements longer, shorter and empty,
    on 1, 2, 3, 5, 8 and 65 threads (more than the 64 ranges), as the
    interpreter.
- **D314 — The sizes of a replacement in parallel from its search; its
  result written by pieces (2026-11-05).** After D313 a replacement still
  read its list of matches twice after the search had written it (16 bytes
  a match, 56 MB for the 3.5 million of `a[NSt]|BY`): once to count the
  bytes of each part of the result (`sizes`, 7.4 % of the samples of
  `a[NSt]|BY` alone, 5.9 % and 4.1 % for the two next), once to write it;
  and its parts were cut by bytes again, apart from the parts of the
  search (`first_at`, `before`). Now the task that searches a part sums the
  bytes of its matches right after finding them, where they are in its
  caches (`Found::removed`); the result is written by pieces: each run of
  the merged matches (D281: the matches of one part, except one or none at
  each end where two searches meet, `Found::removed_of`) with the text
  before it, and the text without match longer than a part (the first
  sequence of regex-redux, without any `a`) cut into copies of a part,
  each piece at a place known before, written in the ranges of D313. No
  `sizes` pass, no second cutting. Repeated, two runs of 10 calls each:
  `a[NSt]|BY` 12.51 and 12.45 -> 12.42 and 12.22 ms, `<[^>]*>` 15.83 and
  15.73 -> 15.48 and 15.63 ms, `\|[^|][^|]*\|` 10.30 and 10.18 -> 10.21
  and 10.24 ms. regex-redux -1.28 % [-1.66 %, -0.28 %] (31 rounds, 88.71 ->
  87.99 ms); the nine other programs are the same binaries. Tested by the
  tests of D308 and D313, the latter with a long text without match at the
  end (`uneven_parts_finished_by_the_other_threads`).
- **D315 — The text of a replacement given to it at its last use, written
  where it is when each match is replaced by as many bytes (2026-11-05).**
  The second replacement of regex-redux, `aND|caN|Ha[DS]|WaS` by `<3>`,
  replaces 15867 matches of 3 bytes by 3 bytes in a text of 50 MB that dies
  with it, and copied the 50 MB into a new string (7.1 ms of the program
  before D313). Now:
  - The ownership pass moves the text of a replacement by a regular
    expression into it when that is its last use, owned and without alias
    (`Use::Move`, the conditions of D207 for a `map` that writes into its
    input); the memory verifier accepts that operand moved (the replacement
    owns it), the others stay read (D134); the code generator passes
    `given` to `isc_rx_replace`.
  - The runtime, given the text: when every match has the bytes of the
    replacement (each one checked, not their sum), writes the replacement
    over each match (`replace_over`, each run of the search by a task from
    65536 matches on) and returns the text itself; otherwise builds the
    result as before and frees the text. Never in the standard input mapped
    from its file, which is read only (D257: `writable`); a literal or a
    view is never given (not owned).
  - regex-redux -3.81 % [-4.39 %, -3.58 %] (31 rounds, 88.29 -> 84.51 ms):
    cycles of the program 914 million -> 872 to 890 million, of the kernel
    190 million -> 177 to 179 million (7 runs, twice). The four last
    replacements of regex-redux get their texts (`(move)` in its IR); the
    three others free them. The nine other programs are the same binaries.
  - Tested by `texts_given_to_their_replacement` (`strings.rs`: a new text,
    one made in a lambda, no match, a shorter then a longer replacement in a
    row, a text used after its replacement that keeps its bytes; short and
    long texts on 1, 3 and 8 threads as the interpreter; Valgrind) and
    `a_mapped_input_given_to_a_replacement_is_not_written` (`stdin.rs`).
- **D316 — The last 2 MiB of a region of medium blocks without huge pages
  (2026-11-05).** A timeline of fasta (perf at 50 kHz, by thread and by
  0.1 ms): before its first line and between its first and second
  sequence, its main thread works alone 0.55 to 0.85 ms each time, the
  three others waiting; in both, mostly `clear_page_erms` (132 and 122
  samples in 10 runs: about 0.25 ms each), under `heap::alloc_medium`.
  The system gives huge pages to every anonymous mapping (`always`): 4
  huge pages a run (`thp_fault_alloc`), 2 MiB each cleared by the kernel
  at its first touch. One of them held only the sentinel that `new_region`
  writes in the last page of each region of 256 MiB, at the first medium
  block of the program: 2 MiB cleared for 64 bytes. That last huge page of
  a region is now `MADV_NOHUGEPAGE` (the sentinel a page of 4 KiB; a block
  that reaches the end of a region gets pages of 4 KiB for its last 2
  MiB): 3 huge pages a run. Leaving it out of `MADV_HUGEPAGE` alone did
  not change them (the setting `always`): 4 again, and its -1.08 % was not
  from them (not kept as a measure of it). fasta -4.27 % [-5.03 %,
  -3.74 %] (61 rounds, 6374 -> 6102 µs), with N = 1 -33.15 % [-35.82 %,
  -31.72 %] (765 -> 517 µs: the cost at the start of every program with a
  medium block); then with the ten programs, all other binaries: fasta
  -4.57 % [-5.12 %, -3.81 %], reverse-complement -0.91 % [-1.74 %,
  -0.58 %], regex-redux -0.15 % [-1.23 %, +0.26 %], k-nucleotide +0.39 %
  [-0.15 %, +0.98 %], pidigits +0.07 % [-0.13 %, +0.19 %], fannkuch-redux
  -0.05 % [-0.12 %, +0.02 %], binary-trees -0.03 % [-0.63 %, +0.31 %],
  mandelbrot -0.05 % [-0.07 %, -0.02 %], n-body -0.00 % [-0.02 %,
  +0.02 %], spectral-norm -0.00 % [-0.01 %, +0.25 %]. The three huge pages
  left are faulted, by `perf -e page-faults` (39 faults a run), in
  `alloc_medium` (3) and `refill` (11, with pages of 4 KiB), where the
  blocks of the program begin: to be attributed to their blocks before
  any change. Tested by `medium_blocks_to_the_end_of_a_region`
  (`memory.rs`: 17 blocks of 16 MiB, past the end of a region, written,
  read, freed, then 34 blocks of 8 MiB; both allocators, 1 and 4
  threads).
- **D317 — `countBy` by tables of counts only, each key with its count in
  one word, looked for by lines of 8 slots (2026-11-05).** Timed count by
  count (`clock()` between them, 4 threads): k-nucleotide spends 34 to 43
  ms on each of its counts of 1 to 6 bases (the floor 20 ms) and 257 to 266
  ms on each of 12 and 18 bases (the floor 69 ms: random increments in a
  table of 2 MiB per thread); 3 counts of 18 bases alone: `count_round`
  31.88 % and `add_block` 30.11 % of the samples, the keys made 15 %.
  `countBy` used `$group`, whose three columns (first index, last index,
  count) need a slot of 16 bytes a key (a table of 4 MiB per thread for
  140000 keys, beyond the L2 cache: D253 scattered the keys into lists
  counted again), but reads only the first indices and the counts (the
  last index is for `Map(entries)`, whose last value wins).
  - `$groupCounts` (a primitive of the prelude, `Prim::GroupCounts`,
    `StrOp::GroupCounts`): the columns of `$group`, the last indices zeros,
    in the interpreter as in the compiled code; `countBy` calls it. The
    runtime (`isc_groups_new(n, parts, counts)`, the flag 64 of the kind of
    `isc_group`) counts it in `CountTable`: `key << bits | count` in one
    word (`bits` those of n, so that no count reaches them), the small keys
    by their value, a key too large for its word (a negative one) in a
    `WordGroups` of its own, the groups in the order of their first
    occurrence, the tables of the parts merged in their order.
  - Probed slot by slot (linear probing, the hash's low bits), it was
    slower: the counts of 12 and 18 bases 365 to 386 ms; 208 million
    mispredicted branches against 27 million (3 counts of 125 million
    keys): the end of the probe, one or two slots, about one key in two
    (a simulation of the table with the keys of the input: 1.54 and 1.58
    slots a key, load 0.53). Now the slots are lines of 8 in one line of
    the cache, the 8 compared with the key at once (in vectors), one
    branch on the result (found, an empty slot, the next line): 7.0
    million mispredictions; the counts of 12 and 18 bases 226 and 231 ms.
  - Then one pass a chunk of 1024 keys: the line of the key 16 places
    later prefetched with its hash computed there, where an array of
    (key, hash, index) of 24 bytes a key was written then read again; a
    small key among them counted by its value (about 4 a chunk with 12
    bases, which made every chunk take the path key by key otherwise):
    the instructions of 3 counts of 18 bases 35.1 -> 24.7 billion; the
    counts of 12 and 18 bases 172 and 179 ms.
  - k-nucleotide -19.33 % [-20.20 %, -18.61 %] (11 rounds, 848.98 ->
    687.93 ms), the same output on 1, 3, 4 and 8 threads; regex-redux and
    pidigits other binaries (the runtime they link): -0.11 % [-0.61 %,
    +0.47 %] and -0.13 % [-0.15 %, -0.00 %]; the seven others the same
    binaries.
  - Tested by `counts_by_lines_of_slots` (`stdlib.rs`: about 200000
    distinct keys, the table grown from 1024 lines, small keys among them,
    negative ones, in parallel parts and in one part without the table of
    small keys (the key 0 hashed), as the interpreter on 1, 3 and 8
    threads; Valgrind) and the tests of `countBy` before.
- **D318 — The whole rows of a `map` take an equality with the index and
  the remainders of their tables out of their loop (2026-11-05).** After
  D316 a line of fasta (61 bytes, one vector of 64 lanes, D274) took about
  40 instructions, 12.8 cycles in its third sequence where the floor's
  copy from tables takes about 6.8: `q == length - 1` (true in the last
  line only) made into a mask at every line (`shlx`, `cmov`, four
  operations of masks, one mask reloaded from the stack), and the
  remainder by 139968 of the first lane of the table of its symbols
  computed again at every line (a product by the inverse, shifts, a
  product, the tests of the sign and of the room), where it grows by 60.
  Now `fill_rows` (`row_hoist`), for its whole rows in one vector each:
  - an equality `q == K` of the index with a value of before the loop
    (`row_lin`: literals, variables bound before, sums, products by
    literals; nothing that can fail) is false outside the row `K / c`: the
    rows before and after it run with the equality false, that row alone
    with it;
  - a table read at `x % c` (c at least the lanes) whose index is linear
    in the row and the place (`row_lin`, through the `let`s of the body:
    `x = step * row + place + K`) gets the remainder of its first lane
    carried from a row to the next (`step % c` added, `c` taken off where
    it reaches it), computed once for the first row; `index_vec` reads it
    there instead of dividing. Every lane is at least 0 when the first
    lane of the first row is, the step is at least 0 and below 2^31 and
    the rows are below 2^31 (no product wraps): otherwise the general code
    (and its panic), as before.
  - fasta -13.82 % [-14.69 %, -13.18 %] (61 rounds, 6246 -> 5390 µs), 226
    -> 189 million instructions; only the equality at first (the table of
    139968 is read where it is, not copied with its period, D273, so its
    remainder was not carried): -9.30 %. The nine other programs are the
    same binaries.
  - Tested by `rows_take_their_equalities_and_remainders_out_of_their_loop`
    (`vector_index.rs`: `q == n - 1` and `q == k` for k inside, before,
    after the rows and at their ends; tables read at remainders with steps
    60, 7 and 1000 for c = 997, the table of 139968; a first lane below 0
    and a table shorter than c, their panics; 1, 3 and 4 threads, as the
    interpreter).
- **D319 — The Benchmarks Game measured again at c6e020e: each of the
  ten programs below three times its floor; the lowest of 12 runs
  (2026-11-05).** The image `isocron-bg` made again with the `iscc` of
  c6e020e (D318), every program built in it and its output checked (the 22
  of Isocron the same as the references; the C++ program mandelbrot gpp-4
  another image than the reference at N = 200, D227, the same as Isocron
  at N = 16000), then `run.sh measure 4 12 native` after `bench/machine.sh
  apply parallel` (the other processes on 8 to 15). The lowest of 12 runs
  now, for every language, where it was 3: D307 saw runs of fasta alone at
  11.9, 16.4 and 20.9 ms among runs of 7.3 to 7.9 ms (the noise of the
  machine), and `measure.sh` counts whole milliseconds, the start of
  `timeout` included (1.43 to 1.48 ms); with 3 runs fasta was published at
  11 ms. A pass of the 50 programs takes about 90 s: 12 of them about 18
  minutes. `run.sh` and `measure.sh` default to 12, the protocol, the
  READMEs and the report say so.
  - Isocron the fastest on 10 of 10 (`results/benchmarks_game.md`, the
    README, `docs/media/benchmarks.svg`), and each program below three
    times its floor (`results/floors.md`, the floors themselves unchanged):
    regex-redux 83 ms (2.79 times its floor; 9.7 when the goal was set),
    fasta 6 ms (2.59; 7.8), k-nucleotide 680 ms (2.54; 4.0), pidigits 491
    ms (2.53), reverse-complement 41 ms (2.44; 3.6), fannkuch-redux 1663
    ms (1.88), spectral-norm 195 ms (1.82), binary-trees 670 ms (1.70),
    mandelbrot 638 ms (1.46), n-body 1897 ms (1.44). The fastest others:
    Rust 851 ms (binary-trees), C 2467 ms (fannkuch-redux), Rust 755 ms
    (fasta, regex-redux), C++ 1176 ms (k-nucleotide), C++ 785 ms
    (mandelbrot), C 1947 ms (n-body), C 683 ms (pidigits), Rust 65 ms
    (reverse-complement), Rust 207 ms (spectral-norm).
  - `results/floors.md`: its table and chart made again, its section on
    the order of the work written again (the four programs that were above
    three times their floor and the decisions that took them below, the
    gaps that are left); its attribution of the gaps stays that of
    c77e61b (D263), marked so.
- **D320 — Release 0.2.2 (2026-11-05).** The end of the first part of the
  goal of 2026-11-05: each of the ten programs below three times its floor
  in the publication of D319, the fastest on 10 of 10. A version
  compatible with 0.2.1 (as Cargo counts): the language unchanged, the
  compiler and the runtime faster (D272 to D318), the programs without C
  under Linux (D287 to D289). The archives as for 0.2.1 (`build_iscc.sh`
  for Linux and Windows in the images of the releases, from the sources of
  the commit of the release; `package.sh`; the extension of VS Code at
  0.2.2), tried before they are published: the archive for Linux on Debian
  12 slim without network (`selftest_linux.sh`), the one for Windows on the
  machine of Windows (`selftest_windows.py`: the ten programs, the same
  outputs; `iscc run`; WebAssembly), the machine stopped afterwards.
  `CHANGELOG.md`, the links of the README and of `tools/vscode/README.md`
  at 0.2.2.
- **D321 — The tasks of a parallel fill of at least 16 KiB, apart from B
  (2026-11-05).** Measured for the question of B (SPEC section 16): the
  compiler built with B = 1024, 4096 and 16384, a `sum` and a `dot` of n
  floats (the lowest of 3 runs): on one thread B changes nothing that
  shows (57.3 to 60.7 ns at n = 1000, 13809 to 13856 ns at 10^5, 3.40 to
  3.44 ms at 10^7); on 4 threads at n = 10^5, B = 1024 takes 6845 and
  13407 ns (98 tasks) against 1776 and 6689 at 4096, 16384 2054 and 4019;
  at 10^7 the three within 2.5 %. B fixes the bits of a reduction of more
  than B floats (the `dot` at 10^5: 38655.506687999994 with 4096,
  38655.506688 with the two others). Six of the ten programs are other
  binaries with another B, the same outputs: with 16384 fasta -16.43 %
  [-16.94 %, -15.80 %] and reverse-complement -8.91 % [-9.27 %, -8.58 %],
  the four others within 0.83 %; with 1024 fasta +54.69 % and
  reverse-complement +37.41 %. Their gain is not their reductions: B is
  also the grain of the tasks that fill an array (`materialize`) and the
  blocks of the output (`print_stream`), whose elements do not depend on
  the cut (D121): a task of B bytes for the lines of fasta, 62000 tasks
  of 4 KiB for its 254 MB, each with its two rows of 61 cut at its ends
  (the general code, D272).
  - So B stays 4096 for the reductions (and their results), and a task of
    a fill writes at least 16 KiB (`FILL_BYTES`, `fill_block`: B elements
    for elements of 4 bytes and more, as before; 16384 bytes; 8192
    elements of 2 bytes), but where it reads a `filter` by windows (D293:
    their margins are for tasks of B elements). fasta -17.64 % [-18.62 %,
    -16.49 %] (61 rounds, 5441 -> 4439 µs): 188.6 -> 143.8 million
    instructions (about 960 a task less, 46500 tasks less), 108000 ->
    40000 mispredicted branches, 61.5 -> 50.0 million cycles;
    k-nucleotide -0.15 % [-0.80 %, +0.08 %]; the eight others the same
    binaries (reverse-complement too: its gain with B = 16384 comes from
    another use of B, to be found).
  - Tested by `fills_by_tasks_of_16_kib_do_not_depend_on_threads`
    (`parallel.rs`: arrays of bytes, `u16` and `f64` of 16383 to 1048577
    elements, rows of 61 written to the output, a sum of floats; 1, 3 and
    4 threads, as the interpreter).
- **D322 — The standard library: modules shipped inside `iscc`, over a
  closed list of primitives (2026-11-05; SPEC section 18).** The second part
  of the goal of the author: programs of computation that read their
  environment and folders, date their results, exchange JSON, hash and
  speak HTTP. Choices:
  - The modules (`sys`, `path`, `time`, `json`, `sha256`, `net`, `http`,
    `https`) are files of Isocron included in the binary of `iscc`
    (`include_str!`), not files installed next to it: an `iscc` alone is
    complete (D248), and a module cannot be missing or of another version
    than its compiler. Their diagnostics name them `std:<name>`.
  - A bare spec (`"http"`) names a standard module first, then
    `$ISC_PATH` (D150 looked in `$ISC_PATH` only). A module of the program
    with the same name is imported by its path (`"./http"`): no file of the
    disk can change what `import { get } from "http"` means, so the same
    program compiles the same everywhere.
  - The standard modules are checked like the program's (types, sizes,
    uniqueness, the verifier of the IR): no exception, no unchecked code in
    Isocron. The trusted base stays the primitives and the runtime.
  - The primitives are actions named `sys*`, callable from the standard
    modules only: a program that names one is refused with the module that
    offers it. This keeps the list closed (section 17) and lets a primitive
    change its form without breaking programs.
- **D323 — The errors of the system as values: `IoError` (2026-11-05).**
  Every action that can fail for a reason outside the program (a missing
  file, a refused connection) returns `Result<T, IoError>`; a panic stays
  for the errors of programming (D20). `IoError` is a sum of the kinds a
  program acts on (`NotFound`, `PermissionDenied`, `AlreadyExists`,
  `NotEmpty`, `Refused`, `TimedOut`) and `Other(code, message)`; each
  carries the object of the error (a path, an address). The kinds are
  mapped from `errno` (Linux, WASI) and `GetLastError` (Windows) in the
  module, so that a program written for one system handles the errors of
  the other. The primitives return `[value, code]` tuples as the
  `_or_error` primitives of D138: the code generator builds no sum value;
  the module builds the `Result`. The `try_*` actions of D138 stay (their
  error a string).
- **D324 — A function the target lacks refused at compile time
  (2026-11-05).** Each primitive lists the targets that have it (WASI
  preview 1: no sockets, no TLS, files in the preopened folders only). A
  program built for a target is refused when a primitive it can reach
  (the call graph after specialization: an unused function does not count)
  is missing there; the message names the function of the program on the
  path, the primitive and the target. A check at run time would let a
  program build for a target where it cannot run (the remark of the
  author: problems found at compile time, as in Rust).
- **D325 — JSON decoded into a type of the program, by a decoder written
  for each type at specialization (2026-11-05).** `decode<T>(text)` and
  `encode<T>(x)`: the compiler writes, for each `T` used, a decoder and an
  encoder in Isocron (as it specializes generic functions), which read the
  text once without building a `Json` tree. The fields are checked at
  compile time: a `T` that JSON cannot carry (a function, a `Map`, a sum
  other than `Option`) is a compile error naming the field; a field missing
  from the text, of another type, or an `i64` not exact is an error of the
  decoding naming the path of the field (`$.points[3].x`); a field of the
  text unknown to `T` is skipped (as most decoders of JSON do, so that a
  service may add fields). The call takes its type arguments as in
  TypeScript (`decode<Point>(text)`): `f<` is read as type arguments when
  what follows reads as types closed by `>` and followed by `(`, else as a
  comparison; a type inferred from the expected result would make the
  meaning of a call depend on where its value goes.
- **D326 — Connections and listeners as unique values (2026-11-05).**
  `Conn` and `Listener` are unique (section 8): moved, never copied; `close`
  consumes them, and a use after `close` is a compile error from the
  uniqueness checked by the verifier of the IR, not a failure at run time.
  The names are resolved in Isocron under Linux (`/etc/hosts`, then the
  servers of `/etc/resolv.conf` by UDP, A and AAAA), by `GetAddrInfoW`
  under Windows (its resolver is a function of the system there, as every
  call of the system of Windows); the tests ask a DNS server written in the
  test, on the local machine.
- **D327 — `serve`: the handler a pure function applied by `map` to the
  batch of ready requests (2026-11-05).** `serve(port, handler)` takes a
  `function (Request) => Response`. The loop of the server, in Isocron
  over `sysPoll`, accepts, reads the ready bytes, and applies the handler
  to the batch of complete requests by the `map` of the language, in
  parallel on the pool, then writes the responses: the handler can call no
  action and shares nothing in writing, so no data race is possible by
  construction, and the parallelism is the one of every `map` (checked by
  the verifier of the IR, the same results on any number of threads). A
  task per connection with its own stack would need a scheduler of tasks
  in the runtime and would let effects in. One request at a time per
  connection (no pipelining), connections kept alive, bodies by
  `Content-Length` up to 16 MiB by default, a malformed request answered
  `400` without calling the handler.
- **D328 — HTTPS by the TLS of the system, confined (2026-11-05).** The
  goal of the author (2026-11-05) names it: SChannel under Windows (the
  certificates of the store of the system), OpenSSL under Linux loaded at
  run time by `dlopen` of `libssl.so.3`, only by the programs that import
  `https`; refused for WASI. This settles the open point of TODO (the step
  HTTPS decided again, D290): these are the only calls of C code the
  standard library makes, all in one module of the runtime (`tls.rs`),
  each `unsafe` justified; a program that imports `https` is built with
  the C library of the system under Linux (D289: the programs that declare
  C functions), every other program stays static and without C. A TLS
  written in Isocron was the alternative: the cryptography of a new
  implementation, unaudited, is a larger risk than the library of the
  system. The client checks the certificate and the name of the host; the
  option `ca` (certificates in PEM) replaces the store of the system for a
  private server, as the tests do on the local machine; `serveTls` is the
  server of the same form as `serve`.
- **D329 — The questions of SPEC section 16 settled (2026-11-05).** The
  goal of 2026-11-05 asks for them before the standard library; each with
  its measure or its reason (SPEC section 16, now without open question):
  - B = 4096, fixed: it fixes the bits of a reduction of more than B
    floats; another B changes nothing that shows on one thread, and none
    is better everywhere on 4 (D321, which took the grain of the fills
    apart from it).
  - The reproducible mode stays the default: it equals or beats the best
    strict variant of every language (`results/xeon-summary.md`); against
    the reassociated variants of C and Zig, 5.69 against 5.24 µs on the
    small `std_dev`, nothing on the sizes read from memory; `allowReassoc`
    stays out of v1.
  - `@requireVectorized`: out of v1 (SPEC section 1); when it comes,
    checked for each target built, the error naming the target.
  - Modular integer overflow kept: none of the gaps of the ten programs to
    their floors comes from it (the attribution of D263).
  - Total inlining kept without a threshold: the ten programs are 68 to
    296 KB, but regex-redux (1.6 MB: the engine of regular expressions).
  - The threshold of the stack: 32 KiB (D86).
  - The transcendental functions: the precision of musl's (`libm`, D108),
    the same bits on every target and in the interpreter.
  - The layout: columns always in v1; an automatic choice later.
  - The FMA of Rust, Go, Zig and Fortran: settled in the POC (D34 and
    after).
  - The name Isocron kept: published under it since 0.1.0.
  - Linear values: the uniqueness of section 8 is a guarantee for the
    types declared unique (`Conn`, `Listener`, `Region`, `Shared`), checked
    by the verifier of the IR (D326).
  - The engine of regular expressions and the big integers stay in Rust
    in v1, each replaced by Isocron only when as fast (section 17).
  - Explicit vector instructions in the modules: out of v1 (SHA-256 on
    `u32`); later a family of bounded primitives.
  - Named time zones and the local time of the system: out of v1 (UTC and
    fixed offsets).
- **D330 — The module `sys` under Linux and in the interpreter (2026-11-05;
  part 2 of the goal, its first step).** The standard modules are files of
  Isocron included in `iscc` (`isc_types::modules::STD_MODULES`,
  `crates/isc-types/src/std/`), imported by a bare spec before
  `$ISC_PATH`, their key `std:<name>` (their diagnostics say `std:sys:96:27`;
  a file `sys.isc` of `$ISC_PATH` does not hide them, one next to the
  program is imported by its path). The primitives of the system are one
  family, `Prim::Sys(SysOp)` and `ActionOp::Sys(SysOp)`, from one table
  (`SysOp`: its name, its arguments and its tuple): the type checker
  refuses one outside a standard module, naming the module to import; the
  code generator passes the strings and arrays by their parts and the
  booleans as `i32`, the runtime writing the tuple at a pointer
  (`isc_sys_<name>`); the interpreter does each one with the standard
  library of Rust (`isc_interp::rt::sys`). Their codes of errors are the
  same in both: 1 not found, 2 permission denied, 3 already exists, 4 not
  empty, 5 refused, 6 timed out, the error of the system negated
  otherwise, -1 for a path holding a zero byte.
  - Eleven primitives: `sysGetenv`, `sysEnviron` (in the order of their
    bytes), `sysStat` (kind, size, time of the last change in
    nanoseconds), `sysReadDir` (without `.` and `..`, in the order of
    their bytes), `sysMkdir`, `sysRemoveFile`, `sysRemoveDir`, `sysRename`,
    `sysCwd`, `sysReadFile`, `sysWriteFile` (made or emptied, or
    appended). Under Linux the system called directly (`newfstatat`,
    `getdents64`, `mkdirat`, `unlinkat`, `renameat`, `getcwd`, `openat`,
    `read`, `write`: `linux.rs`, `system.rs`), the environment that of the
    stack of the kernel (D287).
  - The module `sys` in Isocron: `IoError` and `message`, `env`, `envAll`,
    `cwd`, `exists`, `isFile`, `isDir`, `fileSize`, `modified`, `listDir`,
    `makeDir`, `makeDirs`, `remove`, `removeAll`, `rename`, `readFile`,
    `readText` (UTF-8 checked by a `fold`: otherwise an error, not a
    panic), `writeFile`, `appendFile`, `writeText`, `appendText`. Its loops
    with effects are recursions (a lambda cannot act).
  - Tested by `crates/iscc/tests/std_sys.rs`: every function in a new
    folder, compiled and interpreted, the same outputs; a primitive
    refused in a program; a `sys.isc` of `$ISC_PATH` not hiding the
    standard one. To do: Windows (`kernel32`) and WASI (wasi-libc), the
    module of paths, the example of the guide, the microbenchmark and its
    floor.
- **D331 — The module `sys` under Windows (2026-11-05).** The eleven
  primitives of D330 by the functions of `kernel32` (`system.rs`): the
  paths in UTF-16 (`MultiByteToWideChar`, -1 for a path that holds a zero
  byte or is not UTF-8), `GetFileAttributesExW` (a folder by its
  attribute, the time of the last write from 100 ns since 1601 to ns since
  1970), `FindFirstFileW` and `FindNextFileW` on `path\*` (without `.` and
  `..`), `CreateDirectoryW`, `DeleteFileW`, `RemoveDirectoryW`,
  `MoveFileExW` with `MOVEFILE_REPLACE_EXISTING` (as the standard library
  of Rust, so as `rename` under Linux), `GetCurrentDirectoryW`,
  `GetEnvironmentStringsW` (without the entries of the drives, `=C:=...`),
  `CreateFileW`, `ReadFile` and `WriteFile` (appended at the end of the file
  with `OPEN_ALWAYS`). The codes of the last error as the standard library
  of Rust names their kinds (2 and 3 not found, 5 permission denied, 80 and
  183 already exists, 145 not empty, 10061 refused, 1460 and 10060 timed
  out). Tested in the machine of Windows by `sys_on_windows`
  (`windows.rs`: the program of `std_sys.rs` run there and under Linux, the
  same outputs on 1 and 4 threads); the three other tests of Windows pass
  with the new runtime.
- **D332 — The module `sys` under WASI (2026-11-05).** The eleven
  primitives by the functions of wasi-libc (`stat`, `opendir`, `readdir`,
  `mkdir`, `unlink`, `rmdir`, `rename`, `getcwd`, `open`, `read`, `write`,
  `environ`), in the folders the host opens: WASI has them all, so `sys`
  is not refused there (D324 refuses only what a target lacks). The layout
  of `struct stat` of wasi-libc for wasm32 measured with `offsetof` (a
  program in C built by `zig cc -target wasm32-wasi` and run by
  `wasmtime`): 144 bytes, the mode at 24, the size at 48, the time of the
  last change at 88 (seconds, 64 bits) and 96 (nanoseconds, 32 bits); the
  name of a `dirent` at 9; `nlink_t` is 32 bits there [contradicted: D375], so the layout of
  Linux would have been wrong. The codes of WASI for the kinds: 44 not
  found, 2 and 63 permission denied, 20 already exists, 55 not empty, 14
  refused, 73 timed out (`wasi/api.h`). Tested by `sys_under_wasi`
  (`std_sys.rs`: the program of D330 under `wasmtime`, the same outputs as
  under Linux).
- **D333 — The module `path`, the guide, the microbenchmark of `sys` and
  its floor (2026-11-05).** `path`: pure functions on the text of a path
  (`join`, `dirname`, `basename`, `extension`, `normalize`, `isAbsolute`),
  in Isocron, reading nothing of the system: the separator `/` on every
  target (Windows takes it too), so that a result depends on the text
  alone (SPEC section 18 said `\` too under Windows: a function of the
  text could not tell the target), `C:/` and `C:\` roots everywhere; the
  last `/` found by a `fold` (no `lastIndexOf`), `normalize` a recursion
  with a string for its parts kept. `docs/language.md`, "The standard
  library": the functions of `sys` and `path` and an example run by the
  interpreter, compiled and as WebAssembly. `bench/primitives/sys.isc` and
  `sys.c` (the same work in C: `stat`, `getenv`, `open`, `read`, `write`,
  `getdents64` and a sort, the paths by hand), the same checksums;
  `run.py` on one CPU at fixed frequency, the median of 5 runs of 200
  rounds (`results/primitives.md`): `normalize` 335.56 ns against 49.95
  (6.7 times), `basename` 25.51 against 4.82 (5.3), `env` 61.64 against
  19.02 (3.2), `readFile` of 4 KiB 4316.62 against 1929.61 (2.2),
  `listDir` of 65 names 11000.35 against 9011.29 (1.2), `writeFile` of 4
  KiB 3317.43 against 3232.49 (1.0), `exists` 666.28 against 691.52
  (1.0). Tested by `paths_as_text` (`std_sys.rs`) and the guide.
- **D334 — The module `time` (2026-11-05).** One primitive, `sysNow` (the
  clock of the system in nanoseconds since 1970: `clock_gettime` with
  `CLOCK_REALTIME` under Linux, `GetSystemTimePreciseAsFileTime` under
  Windows, `_CLOCK_REALTIME` of wasi-libc, `SystemTime` in the
  interpreter); the rest in Isocron (`crates/isc-types/src/std/time.isc`):
  instants as `i64` nanoseconds (1677 to 2262: the year 1 is out of
  range), the records `Date`, `Time`, `DateTime` (an offset in minutes),
  the proleptic Gregorian calendar by the algorithms of the civil days of
  Howard Hinnant (`daysOf`, `dateOf`, with divisions rounded down),
  `weekday` (ISO 8601, 1 Monday), `addDays`, `addMonths` (the day kept or
  the last of the month), `daysBetween`, `isLeapYear`, `daysInMonth`,
  `toDateTime`, `toInstant`, `formatIso` and `formatDateTime` (RFC 3339, a
  fraction without its zeros at the end), `format` (YYYY MM DD HH mm ss
  SSS Z), `parseIso` (RFC 3339 and the forms of ISO 8601 it holds; a day,
  an hour, an offset that are not refused as a `Result`). The values
  checked against the `datetime` of Python (the days of 1970-01-01,
  2000-02-29, 2026-11-05 and 0001-01-01 and their weekdays, 9805 days
  between 2000-01-01 and 2026-11-05). Tested by `time_dates_and_instants`
  (`std_sys.rs`: the interpreter, compiled and WASI, the same outputs) and
  `time_on_windows` (in the machine of Windows); the guide has "Dates and
  times". `bench/primitives/time.isc` and `time.c` (the same work in C:
  `clock_gettime`, the same algorithms, the text by hand, the same checks
  of a date read), the same checksums, one CPU, median of 5 runs of 200
  rounds: `now` 150.78 ns against 17.59 (8.6: the runtime calls the
  kernel, where glibc reads the clock in the vDSO), `formatIso` 380.90
  against 45.81 (8.3), `parseIso` 37.43 against 23.31 (1.6), `dateOf` and
  `daysOf` 19.15 against 17.30 (1.1), `toDateTime` 3.70 against 3.37
  (1.1).
- **D335 — A payload holds an array of its own size; the module `json`
  read and written (2026-11-05).** The language: a size named in the
  payload of a constructor (`type Tree = Leaf(i64) | Branch(Tree[k])`)
  is of each value, as the size of an array returned by a function: any
  at a construction (a size of inference), a new existential one where a
  pattern takes the payload apart (`VariantDef::sizes`, `payload_sized`);
  the code is the same (an array is a pointer and a length). An array of
  a recursive type asks nothing of the payloads of its nodes (D130: the
  nodes are pointers in it), so its nodes may hold arrays; its equality
  still asks theirs. Tested by `payloads_hold_arrays_of_their_own_size`
  (`stdlib.rs`: an array payload built and taken apart, a tree whose
  nodes hold arrays of nodes, kept in an array, compared; the
  interpreter, compiled, Valgrind).
  - `json` (`crates/isc-types/src/std/json.isc`), in Isocron without a
    loop: the text checked as UTF-8 by a `fold`; a `scan` of the states
    of the bytes (outside a string, inside, after a backslash), a `scan` of
    the depth of the brackets outside strings; an opening and its closing
    have the same level and follow each other in the order (level, place),
    so one `argsort` pairs them all; the commas of a container are a run
    of the commas in that order, found by halving; the end of each string
    by halving in the quotes that end strings. Then a descent: an array
    or object filled element by element in arrays made first (`fill`),
    updated where they are (`with`), as an array of sums holding a `Json`
    or of tuples holding one is not stored (D138, D154). `Json` is `Null |
    Bool | Num | Str | Arr(Json[k]) | Obj(string[m], Json[m])` (the keys
    and values apart, for the same reason); numbers by the grammar of RFC
    8259 then `parseF64`; escapes, `\u` and surrogate pairs; at most 512
    levels; errors with their line and column. `stringify` without spaces,
    numbers by `toString` (their shortest form), NaN and the infinities as
    `null` (as JavaScript). Tested by `json_read_and_written` (`stdlib.rs`:
    the interpreter, compiled, Valgrind) and the guide ("JSON: `json`").
    To come: `decode<T>` and `encode<T>` (D325), the microbenchmark and
    its floor.
- **D336 — `json.decode<T>` and `json.encode`: the functions of each type
  written by the compiler; what they asked of the language (2026-11-05).**
  - The type arguments of a call, `f<T>(x)` as in TypeScript (D325):
    `ExprKind::TypeApp`, read when what follows `<` reads as types closed by
    `>` and followed by `(`, else a comparison; their number checked, each
    unified with the parameter it gives. A size that a type argument names
    and the function has not is any size (a size of inference), or, for
    `json.decode`, the size of the value read (existential, as a result).
  - A size bound by an array in a record or a tuple of a parameter
    (`SymOrigin::ParamPath`: `r: { xs: f64[k] }`, `k` the length of
    `r.xs`), or by the rows of an array of a parameter (`t: f64[r][c]`,
    `{ xs: f64[k] }[n]`): these are known to the types only, not values of
    the function (the block keeps its columns, but the interpreter has no
    row of an empty array), so the use of one as a value is refused at
    compile time. The arrays of a record or a tuple are tested at the call
    as the array itself when only the run time knows their sizes; on entry
    of a function that is not inlined, only for the parameters that a call
    could not prove (`unproved_params`): n-body's `steps(b: Bodies, n)`
    keeps its code (its calls prove the sizes 5 of its tuple).
  - Three defects found on the way, older than this work, each general:
    a row of a producer of rows was computed alone, its memory owned by no
    one (`t[0]` of a `map` building rows, refused by the verifier of
    memory: the producer is now evaluated, the row a view of it); an array
    of records holding arrays did not free their arrays (`holds_owned`
    counted no array in a tuple: Valgrind found 3 blocks lost for
    `map(iota(3), (i) => ({ id: i, xs: map(...) }))`, now none, the views
    stored in such records copied); `toString` of a float wrote into 64
    bytes and cut the rest (5e-324 has 326: `"0."` compiled, the whole
    number interpreted), now 352 bytes for an `f64`.
  - Inlining with a limit (SPEC section 7): a function is called rather
    than inlined when its nodes, the functions it inlines counted, exceed
    4000, or when it has at least 100 and all its copies more than 2000
    (decided again until nothing changes; not `main`, not a function that
    takes or returns a function). The decoders made it necessary: the
    program of the test of decoding (twelve types) took 50 s to compile;
    21 s once each element was checked once (a fault was computed twice,
    the code doubling at each level), 17.7 s with limits of 300 and 20000,
    2.93 s with 100 and 2000. The ten programs and the microbenchmarks:
    no function concerned (the largest product of size and copies is
    328), their LLVM IR byte for byte the same as before.
  - The decoders (`codec.rs`): the form of `T` (booleans, integers of
    every width checked in their range, `f64`, `f32`, strings, `Option`,
    arrays fixed or not, tuples, records); for each form `codecCheck`,
    `codecBuild`, `codecEncode` (and `codecShape` when it holds arrays)
    written in Isocron, added to the module `json` and checked as it is.
    A first check of the program finds the calls of the placeholders
    `decode<T>` and `encode<T>` (their parameter of any class: they are
    never specialized); the program is checked again with the functions of
    each type, and each call is given those of its type when its types are
    resolved (`zonk_expr`). Two passes over the index of the text: the
    check, whose fault carries its place and its path on the way back
    (`$.points[0].y`), then the building, which cannot fail; an array of
    values holding arrays checks that their lengths agree (the rows of an
    array of arrays have one length) before it is built. A member unknown
    to `T` is checked, then left; the last of two members of one name
    counts. A type that JSON cannot carry is refused with its place in `T`
    (`a function at .f`); so is a type parameter (the type must be known
    where `decode` is called: v1).
  - `parse` written the same way, in two passes: the descent of D335
    filled each array by a recursion whose array parameter was copied at
    each step (8000 elements: 0.223 s; now 0.004 s; a million 0.295 s, its
    `stringify` with it); the strings without recursion (each escape found
    by the index, decoded by `map`, joined: the recursion of D335 copied
    its text at each escape and searched each backslash to the end); the
    brackets checked level by level (a closing of the other kind is found
    at its place, not at the opening it shifts); a missing comma reported
    at the word after the value. The numbers are written as JavaScript
    writes them (an exponent from 1e21 and below 1e-6, -0 as 0).
  - Tested by `sizes_bound_by_arrays_in_records_tuples_and_rows`,
    `type_arguments_of_calls`, `texts_of_long_floats`,
    `json_decoded_into_types_and_encoded` (four programs: integers at the
    ends of their ranges, rows of other lengths, fixed sizes, tuples,
    escapes, faults with their paths), `json_types_refused_at_compile_time`
    (all in `stdlib.rs`: the interpreter, compiled with 1 and 4 threads,
    Valgrind), `json_on_windows` (`windows.rs`: four of these programs in
    the Windows VM, the same outputs with 1 and 4 threads) and the guide.
    The tests of JSON took 564 s before the limit of inlining, 52 s after.
- **D337 — JSON read as a cursor; what the microbenchmark of `json` asked
  of the language and the runtime (2026-11-05).** The microbenchmark
  (`bench/primitives/json.isc` and its floor `json.c`: a document of 1000
  records of about 44 bytes, `{"id", "x", "name", "ok"}`, read by `parse`
  and `decode<Rec[k]>`, written by `stringify` and `encode`; the floor reads
  and writes it as C does it the cheapest way known, one pass, a tape for
  `parse`, structures for `decode`, the names in an arena) gave, per record,
  `decode` 3248.54 ns against 86.48 (37.6 times), `parse` 3500.36 against
  168.12 (20.8), `stringify` 621.27 against 390.17 (1.6), `encode` 258.05
  against 217.98 (1.2). By profile and count of instructions (callgrind),
  then each change measured on the fixed machine:
  - The decoders as a cursor: a value read from its place, a check
    returning the place after it or its fault, then the building; the
    members of a record read one after the other by a tail recursion (a
    loop) carrying its fields, their names compared byte by byte where
    they are (an escape in a key: read first). `decode` 3248.54 -> 1429.85.
  - The index without sorting: the sort of the brackets and of the commas
    by (level, place) (18 % of the time) left out; the closing of a
    container is the first byte where the depth falls below its level,
    the commas of an array those of its level in its segment (a `filter`),
    the kind of a closing checked by the reader that visits each container
    (the brackets are no longer checked level by level: a fault is found
    where the reader meets it, `$.extra: a bracket closed by the other
    kind`); `parse` checked by the same cursor. `decode` 707.82, `parse`
    1460.97.
  - The empty strings own no memory (the runtime, general): `new_str` of no
    byte points to a static block that `isc_free` leaves; the faults `""`
    returned by every check were an allocation and a free each (21.6
    allocations per record counted by callgrind). The parts of `decode`:
    684 -> 603 ns.
  - The index in one `scan`: `4 * depth + state` after each byte (the
    states and the depths were two scans and two maps); the short
    searches (the end of a string, a name compared, a backslash) as loops
    that stop, not `count`, whose reduction of fixed order costs its set
    up for two to five bytes. `decode` 538.85, `parse` 1335.97, then 1091.19
    with the building of `parse` as a cursor too. The readers by segments
    of D336 removed.
  - Found on the way, general: a conditional or a `match` whose branches
    are arrays of other sizes was refused (`c ? a : b`, `a: i64[n]`, `b:
    i64[m]`); it now has a new size fixed at run time, as the result of a
    call (`join`, through tuples and records; each branch given only the
    form expected, not its sizes); the lowering gave a conditional the
    type of its first branch, so that `.length` of `pick(false, [1, 2, 3],
    [4])` was 3 compiled (now its own type, 1). `fill(n, row)` built a
    plain array in the code generator (an LLVM error: `{ ptr, i64 }` for an
    array of arrays): it is now `map(iota(n), (_) => row)`. A function of
    the prelude that a program declares again (`count`) replaced it inside
    the modules of the standard library too (the benchmark declares its
    `count`, and `json` counted with it): the modules keep the prelude's,
    under `std:prelude.` (the prelude itself still follows the program, as
    the guide says).
  - Measured at the end (`results/primitives.md`): `decode` 543.62 against
    86.70 (6.3), `parse` 1092.39 against 168.41 (6.5), `stringify` 602.60
    against 390.60 (1.5), `encode` 267.63 against 217.75 (1.2). The gap of
    `decode`, by its parts (`parts2.isc`, the fixed machine, per record):
    the index 48.1 ns and the checks of UTF-8 and of the depth 5.1 (the
    floor has none: it reads once, sequentially); a decoding of `{ id: i64
    }[k]` on the same text 454.5 ns, of the whole record 526-527: about 58
    ns for each member and each pass, against about 22 for a member of the
    floor, and two passes (the check, then the building) where the floor
    has one; a member unknown to the type costs as much as a known one (it
    is checked). The rest of a member: the calls of the loops that are not
    inlined (a prologue of six saved registers and the test of the stack),
    their results as tuples in memory, the string of a name (an allocation,
    as the arena of the floor). An array of 1000 integers: 78.9 to 80.1
    ns an element. Open (TODO): one pass for the values that hold no rows (the
    building returning its fault), the accumulator of a tail recursion
    updated where it is, the calls of short loops.
  - Tested by `branches_of_arrays_of_two_sizes`,
    `a_program_declaring_a_function_of_the_prelude_leaves_the_modules_alone`
    (`stdlib.rs`: the interpreter, compiled, Valgrind), the programs of
    JSON of D336 (their faults now with the paths of the cursor) and the
    guide.
- **D338 — The module `sha256` (2026-11-05).** FIPS 180-4 in Isocron on
  `u32` (`crates/isc-types/src/std/sha256.isc`), as SPEC section 18 says:
  the message padded as it is read (its bytes, 0x80, zeros, its length in
  bits, big-endian: `paddedByte`, a word read whole when its 4 bytes are of
  the message), never copied; each block of 64 bytes compressed by a
  `fold` of its 64 rounds whose state is a tuple of 24 words, the 8 of the
  hash and the window of 16 words of the schedule (the next word made at
  each round: `W[t+16] = s1(W[t+14]) + W[t+9] + s0(W[t+1]) + W[t]`); the
  blocks by a `fold`. `digest(x): u8[32]`, `digestText(s)`, `hex(x)`.
  Tested by `sha256_of_the_vectors_of_the_nist` (`stdlib.rs`: the vectors
  of the NIST, the empty message, `abc`, the messages of 448 and 896 bits,
  a million `a`; the lengths 55, 56, 63, 64, 65, 119, 120 and 128 at the
  bounds of the blocks and a text of UTF-8, each as `sha256sum` gives it;
  the interpreter, compiled, Valgrind), `json_on_windows` (`windows.rs`, the
  same program in the Windows VM) and the guide.
  - The microbenchmark (`bench/primitives/sha256.isc`, its floor
    `sha256.c`: the same loop in C, the schedule in an array of 64 words):
    `digest.4k` 25839.88 ns against 17402.70 (1.5 times), `digest.55`
    425.70 against 282.90 (1.5), `hex.32` 21.05 against 33.02 (0.6)
    (`results/primitives.md`). The
    gap, by the code of the loops: a round of the floor is 39 instructions
    without access to the stack (its schedule a loop of its own, which gcc
    makes with vector instructions), a round of the module 85, of which 32
    accesses to the stack: the 24 words of the state of the `fold` do not
    fit the 16 registers, and the schedule is made at the 64 rounds (16 of
    them for nothing). Measured and left:
    the schedule in an array updated where it is by a `fold` (38402.66 ns)
    and the last 16 rounds by a second `fold` on an array of the words
    left (34795.61): their arrays (allocated at each block, read with their
    bounds checked) cost more than the moves they save.
  - The instructions SHA of the processor (SHA-NI, the lower bound,
    `digest.4k.shani` in the floor): 4177.48 to 4239.05 ns, 4.1 times the
    loop in C; out of v1 (SPEC section 16, D329: no explicit vector
    instructions in the modules).
- **D339 — The module `net`: connections of TCP, linear values
  (2026-11-05).** The design of D326 kept, but its check moved:
  - `Conn` and `Listener` are resources of the system (`Ty::Handle`, the
    names `sysConn` and `sysListener` of the module, predefined types for
    the parser so that `type Conn = sysConn;` is an alias), an integer in
    the IR (`Type::Handle`, the socket's number). They are **linear**,
    checked on the typed tree (`check_linear`): each local that holds one
    is closed, or given away (in a value, a result, to a function that
    takes it), exactly once on every path; a use after it, a connection
    never closed, closed on one branch only, left by `_`, captured by a
    lambda, made and lost are errors of compilation with their place.
    Whether a function takes each parameter is found from its body (closed
    on all its paths: taken; never: borrowed), again until nothing changes
    for the recursive ones; the errors only from the last pass. D326 put
    the check in the verifier of the IR, by the uniqueness of section 8:
    tried first (a value owned and never copied, `close` consuming it), it
    needs to take a resource out of the value of a `match` (`Ok([s,
    peer])`: the IR projects it, a borrow, and only a copy would give it to
    `close`); the linear check of the typed tree needs no move out of the
    IR, gives clearer messages, and asks that every connection be closed.
    Neither compared nor printed nor in an array, nor a type parameter of
    a function (the constraint of `Ty::Handle`).
  - The primitives (`SysOp`): `sysConnect`, `sysListen` (`SO_REUSEADDR`),
    `sysAccept`, `sysRecv`, `sysSend` (all the bytes; `MSG_NOSIGNAL`: a
    peer gone is an error, not `SIGPIPE`), `sysClose` and
    `sysCloseListener` (taking their argument), `sysSetTimeout`,
    `sysLocalPort`, `sysUdpQuery` (one datagram sent, the first answer of
    that address within a delay), `sysPlatform`, `sysResolve`. In the
    runtime (`net.rs`): Linux by its system calls, numeric addresses read
    by `core::net` (no C library, D287), a timeout (`EAGAIN` past
    `SO_RCVTIMEO`) the code 6; Windows by Winsock (`ws2_32`, now linked
    with every program for Windows, `GetAddrInfoW` for `sysResolve`); the
    interpreter by `std::net`, the same codes.
  - The names resolved in Isocron under Linux (`net.isc`): a numeric
    address, `localhost`, `/etc/hosts`, then the servers of
    `/etc/resolv.conf` on port 53, a query A then AAAA made and read by the
    module (labels, pointers of names, the text of IPv6 by RFC 5952);
    `lookup(host, server, port)` asks one server.
  - D324 made: `system_paths`, on the typed program, finds from `main` the
    primitives of the system that a program reaches and a path of calls to
    each; `iscc build` for WebAssembly refuses one that WASI lacks
    (`SysOp::on_wasi`: no sockets), naming the path (`main` -> `ask` ->
    `net.connect`) and the place of the call in the program. The test of
    the guide checks that refusal for the examples that reach one.
  - The language, on the way (general): `join` and `loosen` go through the
    arguments of sum types (`Result<string[a], E>` and `Result<string[b],
    E>` in two arms: a size fixed at run time, D337).
  - Tested by `tcp_and_dns_on_the_local_machine` (a listener on a port the
    system chooses, a connection by `localhost`, bytes both ways, 70000 of
    them read to the end of the stream, a read past its delay, a connection
    refused, the records A and AAAA asked to a server of DNS written in the
    test and a name it lacks; compiled with 1 and 4 threads, interpreted,
    Valgrind), `connections_are_closed_exactly_once` (seven refusals and a
    function that only reads its connection), `net_refused_for_wasi`
    (`std_net.rs`), `net_on_windows` (`windows.rs`, the Windows VM) and the
    guide.
  - The microbenchmark (`bench/primitives/net.isc`, its floor `net.c`: the
    same calls of the system in C; `results/primitives.md`): a round trip
    of 64 bytes 6167.09 ns against 5869.86 (1.1), a connection made
    24175.46 against 23192.41 (1.0), 64 KiB sent and received 11204.86
    against 12105.38 (0.9): the time of the kernel.

- **D340 — The client of HTTP/1.1; `concat` made into memory by copies
  (2026-11-05).**
  - The module `http` (`std/http.isc`, in Isocron over `net`): `get(url)`,
    `request(method, url, headers, body)`, `exchange(c, ...)` on an open
    connection (what `https` will reuse), `parseUrl(url)`, `header(r,
    name)` (its case ignored, the last one), `text(r)` (`?` for a byte that
    is not UTF-8), `message(e)`. A request has a connection of its own
    (`Connection: close`), `Host`, `User-Agent: isocron`, and
    `Content-Length` when it has a body or its method expects one (`POST`,
    `PUT`, `PATCH`); a name of a header that is not a token, a value with
    CR, LF or a zero byte, a method or a target that cannot be sent, are
    refused before sending (`Malformed`). The body of the response is read
    by `Content-Length`, by `chunked` (extensions and trailers ignored), or
    to the end of the stream; none for `HEAD`, `1xx`, `204` and `304`. A
    head is at most 64 KiB, a body 64 MiB (`TooLarge`). Redirections are
    given as they are.
  - `https://` gives `BadUrl` with the text "the module https sends it"
    until the module `https` exists: the client of `http` never opens TLS
    itself (SPEC section 18).
  - `Response` is `{ status, headers: [string, string][responseHeaders],
    body: u8[responseBytes] }`: the sizes of an alias are variables of the
    signatures that use it (D159), so its names are none that a signature
    of the module uses (`k` was caught by `exchange`'s headers).
  - The bytes read are a list (`type Chunks = Done | More(u8[k],
    Chunks)`), joined at the end two by two in a loop (`pairs`): each byte
    copied log2(n) times, n nodes made (the halves of the first version
    rebuilt the list at each level: n log2(n) nodes). The rest of a chunk
    not whole is read by pieces, each byte copied once, never with each
    read (a chunk of 1 MiB in reads of 16 KiB copied its start 64 times);
    each read takes at least 64 KiB, what follows the chunk kept.
  - A primitive more, the smallest that the tests needed:
    `sysShutdown(conn)`, `closeWrite(c)` in `net` (`shutdown(fd,
    SHUT_WR)`, `SD_SEND`): the sending side closed, the connection kept.
    The test of the client writes its responses from the server side of
    the same program; under Windows, a socket closed with bytes not read
    (the request) resets the connection, and the client read error 10053
    (`WSAECONNABORTED`) where Linux gave the response.
  - The microbenchmark (`bench/primitives/http.isc`, its floor `http.c`:
    the same exchanges in C, one buffer of 1 MiB, the head parsed by
    `strstr`, the chunks copied by `memcpy`). The first measure:
    `exchange.small` 7708.32 ns against 5657.14 (1.4), `exchange.chunked64`
    (64 chunks of 1 KiB) 1078937.24 against 20475.60 (52.7). By the
    profile, in order:
    - The size of each chunk looked for its end (`;` or space) up to the
      end of the buffer, not of its line: quadratic. Bounded by the line
      (`firstBefore`): 64 chunks 1079733.18 ns -> 179318.7 (a variant of
      the benchmark, 50 rounds).
    - The end of the head found by a `filter` of the whole buffer: a loop
      that stops there and starts where the last search ended; the readers
      of the body make the response (no `match` to rebuild it): 179318.7 ->
      156224.44, a body of 64 KiB by its length 73825.16 -> 48888.86.
    - `concat(a, b)` of arrays is lowered to `map(iota(n), (i) -> if i < l
      then a[i] else b[i - l])` (D136), which fuses with its consumers;
      made into memory it was a loop with a test and masked reads per
      element: 19290.12 ns for two arrays of 32 KiB of bytes, against
      1631.2 for `malloc` and two `memcpy` in C (11.8 times). The code
      generator now recognizes this form when it is made into memory
      (`concat_parts`: `a`, `b`, `l` and `n` simple and free of `i`) and,
      for scalars or tuples of scalars (`plain_columns`), makes a block and
      copies each side, column by column (`concat_copy`): 1480.69 ns, and
      16384 doubles 3627.64 -> 2795.58. When the lengths do not fit the
      form (`0 <= l <= len(a)`, `0 <= n - l <= len(b)`, false only for a
      `map` written so by hand), the `map` runs as before and its checks
      panic (tested). A general rule of the compiler: the ten programs do
      not make a `concat`, their IR is unchanged byte for byte.
      `exchange.chunked64` 155218.52 -> 48358.91 (7.7 -> 2.4 times its
      floor).
    - The join two by two in a loop (above): 256 chunks 75927.84 ->
      63018.05; the reads of at least 64 KiB: 4000 calls of `read` for 1000
      rounds -> 3000 (the floor: 2001, its reads of up to 1 MiB).
  - At the end: `exchange.small` 7531.66 ns against 5624.50 (1.3),
    `exchange.chunked64` 49754.47 against 20027.70 (2.5). The gap of the
    chunks measured: the same benchmark with a `flatten` that copies
    nothing (the newest node: a wrong checksum, the same reads) takes
    27194.65 and 27540.88 ns against 49354.71 and 49142.15 (3000 rounds
    each), its user cycles 99504384 against 282918589 and its kernel
    cycles 234627513 against 263954614: the join, six copies of 64 KiB in
    blocks of 2 to 64 KiB made and freed, is 22 µs of the 29. It would take
    one copy if `concat` grew in place an array that it consumes (its
    block unique, at its last use): open in TODO, a rule of the language
    and of the runtime, not of this module. The other 7.3 µs (each chunk
    copied into its node, the head as strings, the second read) are open
    with it.
  - Tests: `std_http.rs` (responses of every form written by the program
    itself, compiled on 1 and 4 threads and interpreted; `get` and
    `request` to a server written in the test: 3 MiB by length, 1000
    chunks, chunks of every size sent in pieces cut anywhere, one of
    300000 bytes, a head of 40 headers, a redirection, a POST with a
    header of UTF-8, a connection refused, a header refused; Valgrind);
    `http_on_windows` in the real VM (the same outputs as Linux);
    `bitwise.rs` (`concat` made by copies of every kind of element, empty
    sides, slices, 4 threads, Valgrind; the `map` of the same form with
    other lengths, the same panic as the interpreter); the example of
    `docs/language.md`, refused for WASI.

- **D341 — Arrays of their own size, `T[]` (2026-11-05).** The server of
  HTTP (SPEC section 18) gives its handler `(Request) => Response`, and a
  batch of requests to `map`: records whose bodies have different sizes in
  one array. Until now a record could not: the sizes of its fields are
  variables of the signatures that use it (D159), the same for every
  element of an array (`[{ tags: [1] }, { tags: [1, 2] }]` refused, and
  `decode<Tagged[n]>` of such a text an error at run time), and a size
  that appears only in a function type of a parameter bound nowhere. Only
  the payloads of sum types had sizes of their own (D335); a server with
  sums and accessors (`http.path(r)`) instead of records was the other
  way, against the records of SPEC and of the client's `Response`.
  - `T[]`: an array whose size the type does not name, each value its own
    (`Size::Own`), in a record, a tuple, a payload or a function type.
    Read (a field, a part of a tuple taken apart, a variable bound to it,
    the result of a call of a function value), it is opened: a new
    existential size, as the result of `filter`. A parameter `x: T[]` is
    `x: T[n]` with a size of its own named `x.length`; a result `T[]` an
    existential size; `const x: T[] = ...` any size.
  - The unification goes one way: an array of any size goes where `Own`
    is expected (a size forgotten), an `Own` never into a size that is not
    `Own`, not even a variable of inference: bound to `Own`, a size of a
    signature would accept any array afterwards, and two arrays of
    different sizes would pass as one (a `map` of both, a bound check
    removed). The parameters of function types are unified the other way
    (what the type expected passes is what the function found takes): a
    function that takes `u8[4]` is not a `(u8[]) => ...` (tested). `loosen`
    and `join` keep `Own`; a literal array whose element type expected
    holds `Own` gives it to its elements (each its own sizes).
  - A size of a callee bound through an array of its own size
    (`both(r: { body: u8[n] }, x: u8[n])` called with a `Request`) is a new
    existential size of that call: the call is tested at run time, as for
    any size known only then (`size mismatch: 1 != 3`, the same in the
    interpreter).
  - The rows of an array of arrays share their size: `T[4][]` is refused
    at compile time. The parts of a record or a tuple that is an element
    of an array may now be arrays of strings or of tuples
    (`headers: [string, string][]`), each a block of its own (D336).
  - JSON: `Shape::Own`, an array whose length is never compared with the
    other elements' (the rows of an array still are): `decode<Tagged[]>`
    of records with arrays of 1, 3 and 0 elements, and `encode`.
  - The IR: `Fixed::ANY`, the length at run time, as every array of a
    size not fixed; nothing else changes there (the ten programs do not
    use `T[]`).
  - Tests: `stdlib.rs` (`arrays_of_their_own_size`: a batch of requests
    given by `map` to a function value, `T[]` as parameter, result and in a
    tuple, equality, records of any sizes kept as `Response[]`, JSON; the
    refusals; the size tested at run time; compiled on 1 and 4 threads,
    interpreted, Valgrind), the example of `docs/language.md`.

- **D342 — The server of HTTP: `serve(port, handler)` over a `Server` of
  `net` (2026-11-05).**
  - A `Conn` is linear and never in an array (D339): the program cannot
    hold the connections of a server. The runtime holds them, behind a
    linear handle `Server` (`Ty::Handle(Handle::Server)`), each named by a
    number given in order and never twice (a stale number reaches no other
    connection). Five primitives, the least that a loop of the program
    needs: `sysServerOpen(l)` (takes the listener), `sysServerWait(s, max,
    ms) => [ids, lens, bytes, code]` (the connections waiting accepted,
    then at most `max` bytes of each connection that has some, 0 at its
    end, within `ms`), `sysServerSend(s, id, bytes)` (all of them, at most
    30 s without progress: `TimedOut`), `sysServerDrop(s, id)`,
    `sysServerClose(s)` (takes the server). Linux: `epoll`,
    level-triggered, non-blocking sockets, `TCP_NODELAY`; Windows:
    `WSAPoll` over the listener and the connections; the interpreter: the
    sockets of `std::net`, non-blocking, tried every millisecond. A wait
    starts again while only connections were accepted (their bytes come
    with the next wait), within its delay: the first version gave an empty
    wait there, where the interpreter waited for the bytes (a test told).
    Refused for WASI (D324).
  - `net`: `Server`, `Arrival = { id, bytes: u8[] }`, `serverOf(l)`,
    `wait(s, max, ms): Result<Arrival[], IoError>`, `sendTo(s, id, x)`,
    `closeConn(s, id)`, `closeServer(s)`.
  - `http`: `serve(port, handler)` (every address, no end), `serveOn(l,
    handler, limit)` (until `limit` requests answered, 0: no end; a test
    and a benchmark need its end), `type Request = { method, path, query,
    headers, body }`, the `Response` of the client (both of arrays of
    their own size, D341), `textResponse`, `httpDate`. The handler is a
    `function (Request) => Response` (D327). The loop takes the requests
    complete at the start of each connection's bytes up to the first that
    has a body, closes the connection or is a fault: the requests of a
    pipeline, answered at once and in order. D327 said one request at a
    time per connection: that made a round of the loop (a wait, the maps,
    the sends) per request and copied the rest of the connection's bytes
    into a new record each time (a record's arrays are its own, D336),
    quadratic in a pipeline: measured below. The ends of the heads are
    found in one pass (`filter` of CR LF CR LF), each head's frame (its
    lines, its two spaces, the length of its body, whether the connection
    stays) read by bytes in parallel, then one `map` over all the requests
    of all the connections makes each request, calls the handler and
    writes its response; the sends follow in order. Faults answered
    without the handler: 400 (the line, a header without a correct name or
    folded, no `Host` in HTTP/1.1), 501 (`chunked`), 413 (over 16 MiB), 431
    (a head over 64 KiB), the connection closed after. Each response has
    `Date` (one per batch) and `Content-Length`; a response of the handler
    that cannot be sent as it is (its status, a header not a token or with
    CR or LF) becomes 500.
  - Found on the way, general rules of the compiler (each with its test):
    - The type parameters that an argument fixes are bound before the
      lambdas after it are checked (`bind_type_args`): `count(rows, (r) =>
      r.id == 1)` of records was refused ("the type of this value must be
      known to use its field"), `filter` was not (a primitive).
    - An array in a part of an element of an array holds any element an
      array may hold (`constrain_part`, as `is_part_ty` of D341):
      `headers: [string, string][]` in records of an array was refused.
    - The linear check of a recursive function that takes its parameter on
      every path (a loop that closes its server at its ends): tried as
      taking it when the first assumption (it borrows) fails, kept when
      that holds.
    - **A slice of an array of tuples that own memory copied their
      pointers** (`memcpy` of the columns): two owners of each string or
      array, freed twice (the allocator's check stopped the program:
      "invalid free"; Valgrind: a read after free). Each owned element of
      the slice is now a copy of its own (`copy_value`), as the copy of an
      array does.
    - `r.length` of a record with a field `length` reads the field (it
      was the length of an array or a string, refused for a record).
    - A construction (`Ok({ body: [] })`) takes the type arguments
      expected when they hold arrays of their own size, and the other
      branch of a conditional the form expected then (not the first
      branch's, of fixed sizes).
    - `a + b + c` of strings, a template literal: one concatenation of all
      the parts (`isc_str_concat_n`, one block), not a block per `+`; the
      interpreter the same (it ran the IR's `Concat` of two parts only,
      and lost the others: a test told). `strings.template` 38.79 ns ->
      36.22.
  - The microbenchmark (`bench/primitives/http.isc`, `serve.pipelined`;
    its floor in `http.c`: the same 8 connections of 200 requests written
    first, a loop of `epoll` in C that finds each request by its end,
    checks its line and headers, reads `Content-Length` and `Connection`,
    writes the response by `snprintf` with the date of the wake and sends
    it; only the server timed). The first version: 2687.75 ns a request
    against 939.61 (2.9). By the profile: the string chains in one block
    2608.78 (a harness of the server alone, 40 rounds); the requests of a
    pipeline at once, read by bytes 1571.60 (the user cycles of 40 rounds
    382247522 -> 167300759, the kernel's 241951381 -> 195445952, the
    floor's 191964767); the head of the response without `filter` and
    `toLowerCase` 1449.67. The full run: 1444.40 against 934.81 (1.5).
    What stays, measured: the user cycles 142410957 against 39570581 for
    64000 requests (about 1600 cycles a request more), the kernel's the
    floor's (196026525 against 191134951). Counted by callgrind on one
    round of 1600 requests, each request makes 10.1 blocks (`isc_alloc`),
    4.1 copies of texts (`isc_str_new`: the literals of `textResponse`
    and of the status kept in the response), 4.0 texts of bytes checked as
    UTF-8 (`isc_str_from_bytes`: the method, the target, the name and
    value of the header; the head was already checked), 2.0 chains and
    1.0 `join`; the floor makes none. Open in TODO: literals kept without
    a copy, a text of bytes already checked, the arrays a request is made
    of in fewer blocks.
  - `strings.bytes`, `slice`, `startsWith`, `endsWith` measured 6 to 7 %
    slower in the full run (1.577 -> 1.692 ns for `bytes`), their floors
    not (0.856 -> 0.891, 1.298 -> 1.296): their loops are the same IR
    byte for byte (only `main` and the template of the benchmark changed,
    the strings it prepares by `+` now one block each); the binaries of
    the last commit and of this one run in turn: `startsWith.1` 1.858,
    1.822, 1.843 ns against 1.671, 1.938, 1.923, the same range: a
    placement of the run, not of the code.
  - The ten programs: the chains of strings change the IR of seven of
    them, only where they write their output (fasta, nbody, revcomp and
    spectralnorm: the same IR but the declaration of `isc_str_concat_n`).
    Before and after (the compiler of the last commit and this one, the
    best of 5 runs in turn, the sizes of the protocol): binarytrees 807.4
    ms -> 789.2, fannkuchredux 1486.6 -> 1478.4, knucleotide 800.5 ->
    792.6, mandelbrot 625.4 -> 627.9, pidigits 493.6 -> 492.7, regexredux
    94.0 -> 96.2, then 15 runs in turn: 91.1 and 90.8 against 90.9 and
    90.0; the same outputs.
  - Tests: `std_http.rs` (`server_on_requests_of_every_form`: the program
    writes its clients' requests, pipelined, a body, HEAD, HTTP/1.0, 400,
    501, 404, no `Host`, then serves them, compiled on 1 and 4 threads and
    interpreted, Valgrind; `server_against_clients_of_the_test`: the
    server in a program of its own against 8 threads of 25 requests kept
    alive, 1 MiB sent back, a body sent slowly in pieces, 2000000 bytes,
    two requests at once, 413, a client that leaves in the middle,
    compiled, interpreted and under Valgrind); `windows.rs` (the first
    program in the real VM, `WSAPoll`); `bitwise.rs` and `stdlib.rs` (the
    rules above); the examples of `docs/language.md`.

- **D343 — HTTPS by the library of TLS of the system, laid on the
  connections (2026-11-05).** The design of D328 kept; under Linux first
  (Windows, SChannel, in the next step).
  - TLS is laid on a connection, not a new kind of value: `net.startTls(c,
    host, ca)` makes `c` a client of TLS and its reads and writes (and
    `closeWrite`, `close`) go through its session from then on;
    `net.serverOfTls(l, cert, key)` makes a `Server` whose connections
    each make their handshake inside `wait` (non-blocking, a step at each
    of their bytes) before their bytes are given decrypted, `sendTo`
    encrypting. So the client and the server of `http` (D340, D342) serve
    as they are (`exchange` on the connection, `serveServer` on the
    server), with the same linear rules (D339). Two primitives:
    `sysTlsStart(conn, host, ca) => [text, code]` and
    `sysServerOpenTls(listener, cert, key) => [server, text, code]` (the
    listener taken). A fault of TLS is `Tls(text)` of `sys.IoError` (codes
    20 and 21 of the primitives), its text the library's (the certificate,
    the name, the handshake) or ours (no library, a PEM text without a
    certificate).
  - The one code of TLS is `isc_runtime_core::tls`, shared by the runtime
    and the interpreter (the same results, as for the floats and the
    regular expressions): OpenSSL 3 loaded at its first use by `dlopen` of
    `libssl.so.3` (its `libcrypto` with it), its functions called through
    the pointers of `dlsym`, each `unsafe` an argument of its manual. The
    runtime has `dlopen` only in its table of glibc (`isc_os_glibc`, D289):
    `iscc` links with glibc a program that reaches a primitive of TLS
    (`with_glibc`), every other one stays static and without C; OpenSSL is
    never among the libraries of a program (tested by `readelf -d`). The
    interpreter declares `dlopen` and `dlsym` of the glibc it is linked
    with. The sessions of the runtime are by socket (a list behind a lock;
    `recv`, `send`, `shutdown`, `close` look there only when it is not
    empty); the interpreter's too, ended when the connection closes or its
    value goes, so that a socket reusing the number never meets an old
    session. `SIGPIPE` is ignored at the first use of TLS: OpenSSL writes
    without `MSG_NOSIGNAL`.
  - The client: TLS 1.2 at least, the certificate checked against the
    authorities of the system (`SSL_CTX_set_default_verify_paths`) or of
    the PEM text `ca`, the name by `SSL_set1_host` (an address checked as
    one), SNI for a name; an end of the stream without `close_notify` is an
    end (`SSL_OP_IGNORE_UNEXPECTED_EOF`: HTTP tells a short body by its
    length). The server: the certificate, the rest of its chain and the
    key of the PEM texts checked to go together, `SSL_set_accept_state`,
    the bytes already decrypted in a session read without waiting (the
    socket has nothing left for `epoll` to tell).
  - `https`: `get`, `request`, `getWith(url, ca)`, `requestWith(ca, ...)`
    (`http://` sent by `http`), `serveTls(port, cert, key, handler)`,
    `serveTlsOn(l, cert, key, handler, limit)`; refused for WASI.
  - The microbenchmark (`bench/primitives/https.isc`; its floor
    `https.c`: OpenSSL called directly, its functions declared there, the
    machine having the library and not its headers; `run.py` makes an
    authority and a certificate for `localhost` and starts `openssl
    s_server -WWW` for a group that declares `// bench-server: tls`): a
    request on a connection of its own, its body of 13 bytes and of 64
    KiB. The first measure: `get.small` 1648637.35 ns against 1217073.34
    (1.4), `get.64k` 1482847.70 against 1234608.61 (1.2). The client made
    an `SSL_CTX` and read its authorities for each request; OpenSSL 3
    takes a part of a millisecond for it: the contexts are now kept by the
    authorities they trust (at most 8, for the life of the program):
    1436143.30 and 1253703.00 (1.2, 1.0). With 200 requests of each kind
    (the runs of 20 were the noise of `s_server`): 1236193.255 ns against
    1279124.841 and 1273130.585 against 1244920.627, the client's user
    cycles 960329945 against 927754529 (3.5 % more: the module, its
    blocks), its kernel's 142539271 against 133901041; the rest is the
    cryptography of the handshake, the same library. Now 100 requests a
    line.
  - Windows: SChannel in the same module (`schannel`, linked: `secur32`,
    `crypt32`, `ncrypt`, embedded for the linker, imported only by a
    program that calls them). The client: `InitializeSecurityContextW`
    over the socket, TLS 1.2; the certificate checked by SChannel against
    the store of the system and the name, or, for a `ca`, after the
    handshake by a chain whose only root is that authority
    (`CertCreateCertificateChainEngine` with `hExclusiveRoot`,
    `CertVerifyCertificateChainPolicy` for SSL and the name); the records
    by `EncryptMessage` and `DecryptMessage`, the end by
    `ApplyControlToken`. Tested in the real VM against two `openssl
    s_server` of this machine (the VM reaches it at the bridge of Docker,
    172.17.0.1): the same outputs as Linux.
  - The server of SChannel (`AcceptSecurityContext`, a step at each of the
    bytes of a connection). Its credentials first by a key imported by
    `NCryptImportKey` (PKCS #8, a container of its own named on the
    certificate by `CERT_KEY_PROV_INFO_PROP_ID`): `AcquireCredentialsHandleW`
    refused them, `0x8009030d`, and the log of the system said why (event
    36870, "a fatal error occurred when attempting to access the TLS server
    credential private key ... the internal error state is 10001"):
    SChannel uses the key in the process of the system, `lsass`. Measured
    one by one: this process opens the key, reaches it from the
    certificate (`CryptAcquireCertificatePrivateKey`) and signs with it
    (`NCryptSignHash`); RSA and EC alike; as a key of the machine; the
    certificate in a store; the protocols by default; `SCH_CREDENTIALS`;
    the key given by its handle (`CERT_NCRYPT_KEY_HANDLE_PROP_ID`, or the
    property of transfer 99: `0x8009030e`, no key for SChannel); the server
    run by a task of the account of the system: refused each time. A PFX
    imported by `PFXImportCertStore` gave a certificate that SChannel took,
    its key in the same provider with the same description: the server
    now makes in memory a PKCS #12 of the PEM texts (the bag of the key of
    PKCS #8 and the bag of the certificate tied by one `localKeyID`,
    neither encrypted nor signed: no password, no MAC) and imports it
    (`PKCS12_PREFER_CNG_KSP`); the container of its key is removed when
    the server closes (counted: the same files of keys before and after a
    run). A program killed leaves it in the store of keys of its user.
  - The antivirus of the VM (Defender) put the compiled server in
    quarantine as `Trojan:Win32/Wacatac.B!ml` in some runs (a guess of its
    learning, on a program that is not signed and opens sockets and keys),
    and once the client of the test (the failure of
    `https_client_on_windows` in the first run of the file, none in the
    runs after); an exclusion of the folder of the tests was refused to
    this work (it weakens the machine), and none was needed: the tests
    passed in the runs that followed, compiled and interpreted (the
    interpreter on Windows uses the same SChannel, `no_dl`). A program
    built by `iscc` may meet the same guess on another machine: a
    signature of the executables would be the answer, outside the
    compiler.
  - Tests: `std_https.rs` (certificates made by `openssl req` at the test:
    an authority, a server for `localhost`, another authority; the client
    against `openssl s_server`: a small body, 3000000 bytes, another
    authority, a name the certificate has not, a `ca` that is not one, a
    connection refused, the authorities of the system; the server against
    `curl` (two requests on a connection, a body, `Connection: close`, 4
    at once, 2000000 bytes) and the client of the module (a POST of 1 MiB
    back); compiled, interpreted and under Valgrind; a certificate that is
    not one refused at the start; the program linked with glibc and not
    with OpenSSL, a program without TLS static; WASI refused); `windows.rs`
    (`https_client_on_windows`: the client compiled for Windows against two
    `openssl s_server` of this machine; `https_server_on_windows`: a pair of
    a server and a client in the VM, compiled and interpreted by
    `iscc.exe`, the same outputs as the pair on Linux); the example of
    `docs/language.md`.
- **D344 — The Benchmarks Game measured again at 2ef9e9f, with the
  library of D322 to D343 (2026-11-05).** Before the release 0.3.0: the
  image `isocron-bg` made again with the `iscc` of 2ef9e9f, the 50
  programs built in it in both modes and their outputs checked (102 checks
  the same as the references, the 22 of Isocron among them), then
  `run.sh measure 4 12 native` after `bench/machine.sh apply parallel`.
  Isocron the fastest on 10 of 10, each program below three times its
  floor: regex-redux 82 ms (2.76), k-nucleotide 684 ms (2.55), pidigits
  491 ms (2.53), reverse-complement 40 ms (2.38), fasta 5 ms (2.16: D321,
  after the publication of D319), fannkuch-redux 1663 ms (1.88),
  spectral-norm 196 ms (1.83), binary-trees 677 ms (1.72), mandelbrot 638
  ms (1.46), n-body 1904 ms (1.45).
  - Three of them a little above D319: binary-trees 670 -> 677 ms,
    k-nucleotide 680 -> 684, n-body 1897 -> 1904. Their code: the IR of
    n-body the same but one declaration (`isc_str_concat_n`); in
    binary-trees and k-nucleotide only the functions that build the lines
    of the output (`isc_main_body`, the tasks of the lines: `concat` in one
    block, D342), the trees and the counts the same. Measured A/B on the
    host at fixed frequency in the slice of the measurements (4 CPUs, 4
    threads, 12 alternate rounds), the programs built by the `iscc` of the
    archive 0.2.2 (the code of c6e020e) against those of 2ef9e9f, the same
    outputs: binary-trees median 680.6 against 681.0 ms (+0.05 %),
    n-body 1906.9 against 1908.2 (+0.07 %), k-nucleotide 707.9 against
    700.3 (-1.07 %). The differences of the publication are those of one
    series of runs to another (the binary of 0.2.2 itself at 674.5 ms at
    best there, published at 670), not of the code.
  - `results/benchmarks_game.md`, the README, `docs/media/benchmarks.svg`
    and `results/floors.md` (its table and chart, its section on the
    order of the work) made again.
- **D345 — Release 0.3.0 (2026-11-05).** The end of the second part of
  the goal of 2026-11-05: the standard library (D322 to D343), designed
  in SPEC section 18, each module written in Isocron over the primitives
  of the runtime, the same results interpreted and compiled, tested on
  Linux and in the VM of Windows, refused at compile time for WASI where
  the target lacks it, each with an example tested in `docs/language.md`,
  a microbenchmark and its floor (`results/primitives.md`). A version
  0.3.0 (as Cargo counts, the language grew: `T[]`, type arguments of a
  call, imports of the modules of the library). The archives as for 0.2.2
  (`build_iscc.sh` for Linux and Windows in the images of the releases,
  from the sources of the commit of the release; `package.sh`; the
  extension of VS Code at 0.3.0), tried before they are published
  (`selftest_linux.sh`, `selftest_windows.py`, the machine of Windows
  stopped afterwards). `CHANGELOG.md`, the links of the README and of
  `tools/vscode/README.md` at 0.3.0; the README gains a section on the
  library, its example checked by `iscc check`.
- **D346 — Equality with a short literal in place; rule L, the last
  index of a `fold` from the end; `getenv` without a block of the heap
  (2026-11-05).** The gaps of `path` and `sys` that had numbers but no
  measured cause (D333). Profiled at fixed frequency on one CPU (`perf
  stat`, `perf record`, a program of each function alone, 4096 paths of
  `bench/primitives/sys.isc`, 200 rounds): `basename` 87.4 cycles and 271.5
  instructions a call, 29 % of them in `isc_str_cmp` and `memcmp` for its
  one `t == "/"`, 63 % in its two `fold`s (the last byte that is not `/`,
  the last `/`: each over the whole path, by 4 lanes of indices of 64 bits
  and their reduction, the loop vectorizer of LLVM); `normalize` 1120.8
  cycles, 26 % in the comparisons with `""`, `"."` and `".."`.
  - `==` and `!=` against a literal of at most 16 bytes, on either side,
    compiled in place (`literal_affix` of D301 with `exact`: the length,
    then the bytes as one integer or two that overlap); longer literals
    and two variables by the runtime. `basename` 87.4 -> 60.7 cycles,
    `normalize` 1120.8 -> 796.0.
  - Rule L (SPEC): `fold(iota(n), init, (a, i) => c ? i : a)`, `c` not
    reading `a` and unable to stop the program (`safe_cond`: no division
    or remainder, conversions only to wider integers, its reads `x[i + p]`
    checked at `i = 0` and `i = n - 1` before the loop as D297 does), is
    the last `i` where `c` holds: `c` from `n - 1` down, the first that
    holds; a read out of bounds or `n = 0` by the loop as written (its
    panic, at its place). In `path` (2), `json` (the line of an error),
    `time` (`fraction`). `basename` 60.7 -> 34.8 cycles.
  - The ten programs: their IR the same, byte for byte (neither form in
    them).
  - `results/primitives.md` (a whole run): `basename` 25.64 -> 9.07 ns
    (5.3 -> 1.9 times its floor), `normalize` 335.43 -> 228.61 (6.7 ->
    4.5). What is left in `normalize` (its profile): `split` (33 %),
    `out + "/" + s` for each part (`concat` 17 %, `memcpy` 11 %: the text
    kept so far copied again at each part), the frees (14 %): the
    algorithm of the module, open in TODO.
  - `env` 58.67 -> 60.55 ns in the first run after these changes, and
    66.69 (0.3.0) against 75.28 (now) in an A/B of the two binaries, its
    code the same (the IR of its loop and the instructions of
    `isc_sys_getenv` identical; only their places moved): +14.9 % with 200
    rounds, +2.7 % with 2000, -1.5 % with 20000. With 200 calls its window was 13 µs, its first calls cold. So
    `env` and its floor now make 100 calls a round: its floor 17.72 ->
    11.34 ns, warm. Its profile then (169.8 cycles and 664 instructions a
    call): `isc_sys_getenv` 54.6 % (a `Vec` for the name ended by a zero,
    made and freed each call; the 18 variables of the environment scanned,
    `HOME` the ninth; the value copied), `from_utf8_lossy` 16 % (by its
    iterator of chunks even for ASCII), the copy that `?? ""` makes of the
    payload of the `Option` (`isc_str_new`, its `free`: 18 % with the
    rest). The name on the stack below 256 bytes, the value copied once
    when it is UTF-8 (`from_utf8`, then `new_str`), `from_utf8_lossy` only
    for a value that is not: 169.8 -> 150.4 cycles; `env` 47.71 ns against
    11.34 (4.2: glibc returns a pointer into the environment, a string of
    Isocron is its own copy; the copy of `??`, D347).
  - The group `strings`, its IR the same as in 0.3.0, 5 to 7 % below the
    previous run: A/B of the binaries of 0.3.0 and of now, alternately,
    within 2.1 %; the difference is that of one session of measurement to
    another.
  - Tested by `equality_with_short_literals` (`strings.rs`: every length
    around 0, 8 and 16, either side, `!=`, a literal of 17 bytes, the IR
    without a call) and `last_index_of_a_fold_from_the_end` (`codegen.rs`:
    found, not found, `n = 0`, two reads and `&&`, a widening conversion,
    the panic of a read out of bounds as the interpreter's).
- **D348 — The text of an integer padded in one block (2026-11-05).**
  `time.formatIso` 7.0 times its floor without a measured cause (D334).
  Profiled alone at fixed frequency: 979.3 cycles and 3962.5 instructions
  a call, against a floor of 45.8 ns; each `pad(x, n)` of the module
  (`toString(x).padStart(n, "0")`) makes the text of `x`, then the padded
  text; `isc_pad` took its slow path (the writer of `fmt`, its buffer grown:
  `reserve` and `finish_grow` 13 %) whenever the text had already `n`
  bytes (`pad(2026, 4)`, a minute of two digits); the template copied the
  eight parts (`concat_n` and `memcpy` 28 %), the frees 12.5 %.
  - `isc_pad`: a text of `n` bytes or more, or an empty fill, copied once
    at its size (`padStart` gives it as it is): 979.3 -> 873.1 cycles.
  - `toString(x)` of an integer bound by a `let` whose only use, outside
    the functions of `map` and the others, is the text of a `padStart` or
    `padEnd` (`sole_pad_text`): `x` evaluated at its place, no string
    made; with a fill of one byte, `isc_pad_int` writes the text of `x`
    and the fill in one block of its size; with another fill, the text
    made, padded, freed. 873.1 -> 652.7 cycles.
  - `results/primitives.md` (a whole run): `formatIso` 314.00 -> 216.46 ns
    (6.9 -> 4.7 times its floor). What is left (its profile): the eight
    parts of the template each a block made, copied into the result and
    freed (`isc_pad_int` 21 %, `memcpy` 18 %, `concat_n` 14 %, `free` 10
    %), where the floor writes the 20 bytes in one buffer: the parts of a
    template written in its block, open in TODO. `padStart` of the group
    `strings` (a text of 4 bytes padded to 12 by `-`, not an integer)
    11.60 -> 12.08 ns, within the drift between sessions of D346.
  - The ten programs: their IR the same (neither form in them).
  - Tested by `integers_padded_in_one_block` (`strings.rs`: negative
    numbers padded as JavaScript does, `0000-7`; unsigned; a text long
    enough; `padEnd`; a fill of two bytes; a text read twice; templates;
    seven calls of `isc_pad_int` in the IR).
- **D349 — The examples of the README run; a port listened to twice
  refused everywhere (2026-11-05).** The example of `serve` in the README
  (D345) had been checked by `iscc check` only: run under Windows by the
  author, it printed nothing (`serve` answers without end) and its branch
  `Ok(_) => print("done")` could never run. Now it says where it listens,
  says why it cannot serve (`sys.message`), and returns 0 or 1.
  - Run under Linux and in the VM of Windows, compiled and interpreted:
    `hello world` for `/?world`; a second copy on the same port should
    say why it cannot serve. It did not, in three ways: under Linux
    `cannot serve: error 98 of the system` (`EADDRINUSE` not one of the
    kinds), in the interpreter the same, and under Windows compiled the
    second copy served too: the runtime set `SO_REUSEADDR`, which under
    Windows lets another socket bind a port already listened to. Now a
    port listened to is `AlreadyExists` everywhere (`EADDRINUSE` under
    Linux, `AddrInUse` in the interpreter, `SO_EXCLUSIVEADDRUSE` instead
    of `SO_REUSEADDR` under Windows: `WSAEADDRINUSE`, as Windows already
    named it); Linux keeps `SO_REUSEADDR` (there it only lets a server
    start again at once on a port in `TIME_WAIT`).
  - In the VM after it: the second copy `cannot serve: already exists:
    0.0.0.0:8080`, exit code 1, compiled and interpreted. The line
    `listening` of a compiled server killed by `taskkill /f` is not in the
    file its output went to: a file is written by blocks (D299); in a
    console each line goes out at once (`GetConsoleMode`).
  - Tested by `every_example_of_the_readme_works` (`docs.rs`: each block
    of the README run, the commands of the block `console` after it and
    their outputs, a function alone type-checked, the server started on a
    free port, asked, a second copy refused, then stopped, interpreted and
    compiled), `a_port_listened_to_twice` (`std_net.rs`) and
    `a_port_listened_to_twice_on_windows` (`windows.rs`, compiled and by
    `iscc.exe`). The file `windows.rs` in the VM: 10 of 10 (a first run
    9 of 10: `win_https_client.exe` put in quarantine by Defender, events
    1116 and 1117 at the time of the test; the next run without it).
- **D350 — Release 0.3.1 (2026-11-06).** Asked by the author after the
  example of the server of the README failed under Windows: a correction
  of 0.3.0, compatible with it (as Cargo counts), from `main` (D346, D348,
  D349). The archives as for 0.3.0 (`build_iscc.sh` for Linux and Windows
  in the images of the releases, from the sources of the commit of the
  release; `package.sh`; the extension at 0.3.1), tried before they are
  published: `selftest_linux.sh`, `selftest_windows.py`, and the example
  of the server of the README run by the `iscc` of each archive under
  Linux and in the VM of Windows (compiled and interpreted: `hello world`,
  a second copy refused with 1), the VM stopped afterwards. The signature
  of the executables by Authenticode, asked about: it needs a certificate
  of an authority that Windows trusts, an identity checked, its key on a
  device or a service of signature: the author's to get; the tools can
  sign the archives once there is one.
- **D351 — Rule A: the accumulator of a tail recursion owned; the left
  text of a concatenation grown in place (2026-11-06).** `path.normalize`
  4.5 times its floor (D346: `out + "/" + s`, the text kept so far copied
  again at each part), and a string built piece by piece quadratic: a
  recursion `build(i + 1, n, acc + x)` of 100000 pieces 122.2 ms. Its IR:
  `acc` borrowed (a parameter is lent by the caller, D102), so `acc + x`
  copies `acc`, the result copies it again (`copy acc`), and the caller
  frees each piece after the call.
  - Rule A (SPEC): a parameter that holds memory, not a tuple (those are
    the states of D206), of a function whose calls to itself are all
    tail calls and which gives it back as one of its results, is owned:
    `consumed`, copied once on entry, moved at its last use, given back
    without a copy, freed by a `Free` at the end of the branches that do
    not move it (`consumes`, D105). Kept only when the memory check of the
    function holds (`verify_fn`, which now asks that an owned parameter be
    moved on every path); otherwise the body as before, the parameter
    borrowed. In the loop of the tail calls (`tail_jump`) an owned
    parameter given again at its place stays in its slot (`kept`, not in
    `free_first`: it was freed before the jump, then read: the panic
    `invalid free` of the first try), and a literal given to it is
    copied (the loop owns its slot; a literal of D301 is not a block).
  - The left text of `a + b` (two parts, or a chain of D342) at its last
    use, owned and without alias, is given to it (`Use::Move`, as the text
    of a replacement of D315; the memory check counts it as taken):
    `isc_str_append`, `isc_str_append_n` write the rest after it in its
    block when the block has room (`heap::usable`: the size of its class,
    or of a medium block), else in a new block of at least twice its
    length, the old one freed. With glibc's allocator or under WASI no
    room is counted (a new block each time, still of twice the length).
  - `normFrom` of `path` written so that its text holds its root `/` or
    the parts `..` that nothing removed (they only come while no name is
    kept), and is itself the result: rule A applies.
  - Measured A/B against the binaries of the `iscc` of 0.3.1, one CPU at
    fixed frequency: the build of 100000 pieces 122.2 -> 3.7 ms (-97.0
    %); `normalize` 240.7 -> 213.3 ns (-11.4 %), its profile now `split`
    (38 %), the appends (19 %), `normFrom` (12 %). The ten programs: the
    IR of seven the same but for the declarations; binary-trees,
    k-nucleotide and pidigits other (accumulators of their recursions),
    the same outputs, 4 threads, 10 alternate runs: medians +0.02 %, +0.58
    %, +0.27 %, minimums 683.5 -> 677.0, 688.2 -> 688.0, 491.8 -> 490.6 ms.
  - Tested by `strings_grown_in_place` (`strings.rs`: interpreter,
    compiled, Valgrind with the allocator of glibc, the IR calling
    `isc_str_append`) and `normalize_as_before` (`std_sys.rs`: the old
    algorithm and the new one on the 7812 paths of up to five parts of
    `""`, `.`, `..`, `a`, `bc`, absolute and relative, interpreted and
    compiled on 1 and 4 threads). The negative test of the memory check that
    removes the copy of a parameter returned (`verify.rs`) took `doubled(x,
    k)`, now of rule A (no copy left to remove): its recursion made not a
    tail call, so that the parameter stays borrowed and its copy is
    there.
- **D352 — Actions as values (2026-11-06).** Asked by the author: a
  handler of `serve` that prints, reads a file or calls another service
  was refused (a `function` has no effect, D327), and "journaliser, c'est
  réducteur, ça ne concernerait que le print"; the way of the pure
  languages compared (Haskell: the handler of Warp an `IO` action, its
  state in `IORef`, `MVar`, `STM`; Elm, Roc: effects as values, the state
  threaded; Koka, Unison: effects in the types; Erlang: one process a
  request, no shared memory), the author chose "la voie la plus
  naturelle": the separation of `function` and `action` already in the
  language, as Haskell's of pure code and `IO`.
  - The language had no type of action: `f: action (i64) => i64` refused
    by the parser; a function type (`Ty::Fn`) carries no effect; every
    higher-order parameter is pure. So not only `http`: the servers of
    `net`, a walk with effects over an array (no `forEach`), the wrappers
    of actions (retry, timing, a folder made then removed).
  - The rules (SPEC section 12, "Actions comme valeurs"): a type `action
    (T...) => U`; a `function` accepted where it is expected (a pure
    function is an action without effect), not the reverse; its values the
    names of actions and functions, and the lambdas written in an action
    (one that calls an action has an action type); its call an effect,
    only in an action; no parameter of action type in a `function`;
    compiled as the higher-order functions are (specialized for each
    action given, a choice at run time by labels): no closure at run time;
    called where it is written, in the order of the action; `map` and the
    others stay pure.
  - The library: `forEach(x, f)` in the prelude; `serveAction`,
    `serveActionOn`, `serveTlsAction` in `http` and `https`, the requests
    answered one at a time in the order they are ready; `serve` keeps its
    pure handler and its parallel `map`. Actions run in parallel on the
    pool only after the primitives of the runtime are checked from several
    threads (the buffer of `print`, files, sockets, TLS): open in TODO.
  - Done (2026-11-06). The type `action (T) => U`: a third field of the
    function types (`Ty::Fn(ps, r, act)`, `TypeKind::Fn`), parsed after the
    word `action`; unified so that an action found needs an action
    expected; `can_act` true in an action whose enclosing lambdas are all
    actions (`act_lambdas`); a lambda expected as an action where one can
    act is one; the name of an action a value only in an action; the call
    of an action value an effect; no parameter of an action type in a
    `function`. The block of a lambda in an action lowered as an action
    (the checker refusing its statements when the lambda is pure). Actions
    may have type parameters (the parser refused them; nothing else did).
    The interpreter runs the IR, where the actions given are already
    specialized: unchanged. The ten programs: their IR the same.
  - The message of a statement where no effect is allowed names the action
    called and the reason: in a pure lambda of an action, in a `function`
    (with the way to effects, `http.serveAction`).
  - The prelude: `forEach`, and `mapAction` (the results of an action on
    each element, in order: `U[]`, an array of its own size, since no value
    of `U` fills an empty one; written in place by `.with` into an owned
    accumulator, rule A: 20000 elements 20 ms, 80000 14 ms, linear).
  - `serveActionLoop` of `http`: the requests ready found (pure), each
    answered by the handler in its turn by `respondFrom` (a recursion that
    takes `handler` as a parameter: a lambda that reads `handler` given to
    the recursive `mapAction` is refused, "a recursive function given a
    function that reads a function of its context", a limit of the POC, in
    TODO), the bytes made (pure); the end shared with `serveLoop`
    (`afterAnswers`). A first form built the responses by `concat(acc,
    [out])`: quadratic, 19747.8 ns a request; then 2656.8.
  - `bench/primitives/http.isc` gains `serveAction.pipelined` (the same
    1600 requests answered by an action), its floor the same loop of
    `epoll` in C measured again: one CPU at fixed frequency, 1764.63 ns
    against 937.09 (1.9), `serve.pipelined` 1474.56 (1.6). The run also
    gave `normalize` 229.85 -> 208.28 ns (D351) and `template` -9.4 %; four
    forms of `strings` 5 to 9 % above the previous run, their IR the same:
    A/B against the binary of 0.3.1, alternately, -0.3 to -2.6 %
    (`template` -7.6 %): a drift between sessions.
  - Tested by `actions.rs` (the uses allowed, interpreted and compiled on 1
    and 4 threads, an action chosen at run time; the six uses refused and
    their messages), `server_with_a_handler_that_is_an_action`
    (`std_http.rs`: the requests of one connection printed in their order,
    a file read, a pure function where an action is expected, `forEach`),
    `net_on_windows` (the same program in the VM, compiled), the example
    of `docs/language.md`.
- **D353 — The server of HTTP against nginx and lighttpd (2026-11-06).**
  Asked by the author ("as-tu fait des benchs d'un serveur http en C vs le
  nôtre ?"): until then only `serve.pipelined`, a microbenchmark against a
  loop of `epoll` written for the same work (1.5). Now `bench/http_servers/`:
  an image `isocron-http` built from that of the Benchmarks Game (nginx,
  lighttpd, wrk; h2o is not in Debian trixie, lighttpd taken instead),
  without network, the server on 4 CPUs and wrk on 4 others, `hello,
  world!` to every request, 1 to 1024 connections kept alive, 10 s each,
  at fixed frequency (`results/http_servers.md`). One connection: 67459
  requests a second against 71457 for nginx (94 %); 16 connections 197647
  against 628909; 1024 connections 67277 against 739676 (nginx) and
  174111 (lighttpd, one process). The author's fear confirmed: the server
  holds with few connections and falls with many.
  - The cause measured. The profile of the server under 1024 connections
    (`perf record -p` of the process in the container, its symbols by
    `--symfs`): `worker` of the pool 54.7 %, `heap::refill` 22.9 %,
    `isc_free` 4.9 %, `spill` 1.65 %, the work a few percent; one thread
    (`ISC_THREADS=1`): 206647 at 16 connections, 83632 at 1024 (4 threads:
    198985, 66942): the threads cost about 20 % on such small work, and
    the cost of a request grows with the connections even without them.
    The model: each turn of `serveLoop` computes `readyOf` of every
    pending connection and filters them all (`closing`, `left`), and
    `arrive` matches each arrival to its connection by a linear search: a
    turn costs in proportion to the connections, a request to the
    connections over the requests of the turn. lighttpd, one process, keeps
    its level (188598 at 16, 174111 at 1024).
  - The way: the pending connections in a table by their number (the
    runtime gives numbers that come back after a close), only those that
    received bytes read, the arrivals written into the table in place; and
    the `map` of the handler in parallel only when the turn holds enough
    requests. Open in TODO.
- **D354 — The server with many connections: a table by their numbers;
  the state of a loop owned (2026-11-06).** The cause of D353 removed.
  - The pending connections (`ps`) kept sorted by their numbers: each
    arrival found by halving (`placeOf`) and its bytes added in place
    (`fold` with `.with`); the new ones put at their places (a connection is
    numbered when it is accepted, its first bytes may come after those of
    one accepted later: the table sorted again when new ones come, a first
    form that appended them broke the order and a search found -1, a panic
    under the clients of `server_against_clients_of_the_test`); the bytes
    of a connection that ended in the same turn not read; each turn reads
    only the connections that received bytes or gave requests the turn
    before (`changed`: after a response, what is left of a buffer may hold
    a request the pipeline stopped before); the table built again only when
    a connection closes.
  - So that the table changes in place, rule A of D351 extended: a
    parameter that holds memory, not a tuple, of a function whose calls to
    itself are all tail calls, is owned also when one of those calls gives
    it another value (the state of a loop, `serveLoop(s, next, ...)`), its
    exits freeing it (`consumes`); kept only when the memory check holds.
    The ten programs: the IR of pidigits only other, the same output, 10
    alternate runs on 4 threads, medians -0.05 %. The parameters that hold nodes of
    a recursive type left out (of rule A too): their nodes keep their own
    owners, counted, not atomic inside a task (D175); owned, a list given
    to tasks broke the memory of glibc (`unaligned tcache chunk detected`,
    `recursive_types_in_parallel_tasks` under `ISC_ALLOC=glibc`, suite 160)
    and `drop_n` no longer counted its new owner (`verify.rs`); the IR of
    the ten programs the same with or without them.
  - wrk in Docker, fixed frequency (`results/http_servers.md`): at 1024
    connections 67277 -> 158735 requests a second (2.36 times), 256
    connections 151520 -> 202600; lighttpd (one process, as the loop of
    Isocron) 177463 and 194181: Isocron 89 % and 104 % of it; the threads
    no longer cost (1 thread 153792, 4 threads 156128 at 1024). nginx
    still 3.5 to 4.5 times above with more connections: its 4 processes,
    each with its loop and its connections; a loop of events for each
    core, open in TODO. `serve.pipelined` (8 connections) 1474.56 ->
    1480.00 ns, the same.
  - Tests: the HTTP and HTTPS test files (their server against clients of
    every form at once, interpreted, compiled, Valgrind); in the VM 9 of
    10: `https_client_on_windows` stopped by Defender (its executable put
    in quarantine, events 1116 and 1117 at 01:16:59 and 01:17:04; the same
    in a second run, its file the same), the server of D354 run there by
    `net_on_windows`.
- **D355 — The ceiling of the machine for a server of HTTP (2026-11-06).**
  The author: "je ne veux pas 'rejoindre nginx', je veux que ton objectif
  ce soit le plancher, comme d'hab ... enfin là du coup le plafond ...
  pour peu qu'il existe". For a throughput the reference is the ceiling:
  the requests a second of the cheapest server known for the same work on
  the same cores, nginx and lighttpd only landmarks.
  - `bench/http_servers/ceiling.c`: a loop of `epoll` for each thread and
    its own listener (`SO_REUSEPORT`), a request found by its end (CR LF CR
    LF), the 130 bytes of the response of Isocron (its date made again each
    second) copied, one `recv` and one `send` for the requests a read
    brings; nothing else. `measure.sh` gains it (1 and 4 threads) and the
    cores each server keeps busy (its CPU time over the time of the run,
    every thread and process).
  - At fixed frequency, 10 s a point (`results/http_servers.md`): the
    ceiling of one loop 79736, 275472, 289710, 264138 requests a second for
    1, 16, 256, 1024 connections, its core full (0.99 to 1.01); the ceiling
    of 4 cores 82216, 737056, 791806, 767472, with 3.05 to 3.22 cores busy:
    there the client (wrk on 4 cores) saturates first, so this ceiling is a
    bound below. Isocron `serve` 67962, 190973, 203414, 151777: 85, 69, 70,
    57 % of the ceiling of one loop, 83, 26, 26, 20 % of that of 4 cores,
    with 3.95 to 4.04 cores busy (its pool waiting for work while one loop
    works). nginx 81 to 89 % of the bound of 4 cores, lighttpd 63 to 67 %
    of the ceiling of one loop.
  - So the way to the ceiling, in TODO: a loop of events on each core
    (actions in parallel on the pool, D352); the cost of a request of the
    loop down to that of the ceiling (5.2 µs against 3.6 at 16 connections,
    the inverses of 190973 and 275472); the pool idle when it has nothing
    to do (4 cores busy for the work of one).
- **D356 — Release 0.3.2 (2026-11-06).** Actions as values (D352), the
  server with many connections (D354) and its ceiling (D355), rule A
  (D351, D354): a version compatible with 0.3.1 (as Cargo counts: the
  language only grew, `action (T) => U` and actions with type parameters).
  The Benchmarks Game measured again at 7370399 (the image made again with
  its `iscc`, the 102 checks the same, `run.sh measure 4 12 native` at fixed
  frequency): the fastest on 10 of 10, each program below three times its
  floor (regex-redux 83 ms, 2.79; k-nucleotide 684, 2.55; pidigits 491,
  2.53; reverse-complement 41, 2.44; fasta 5, 2.16), every time within 3 ms
  of D344 (their IR the same, but pidigits: D354, the same time).
  `results/benchmarks_game.md`, the README, `results/floors.md` made again.
  The archives as for 0.3.1, tried before they are published (the
  self-tests of Linux and Windows, the examples of the README run by the
  `iscc` of each archive).
- **D357 — Each parallel loop chooses the pool or the calling thread by
  its measured runs; the threads of the pool sleep sooner (2026-11-06).**
  The server of D355 kept 4 cores busy for the work of one loop. Cause
  measured: a `map` whose element is a loop of its own (`inner_loop`: a
  `filter`, a `fold`, a call) was always given to the pool, whatever its
  size; each turn of the server woke the pool for a few microseconds, and
  its threads spun 65536 pauses (about a millisecond) after each job,
  never sleeping. Giving a job to the pool and waiting for it costs about
  1 µs: a `map` of 16 elements of 10 steps 96 ns on one thread, 1088 ns on
  four; 16 elements of 1000 steps 2247 against 1264.
  - Every call to the pool (the 20 places of the code generator:
    `par_for`) goes through `isc_parallel_for_site`, with four words of its
    own for its place in the program: the cycles of its runs on the pool
    and alone (averages, seven eighths the one before: one run slowed by
    the system does not turn it over), its runs, whether its last run was
    alone (`rdtsc`, about twenty cycles). A loop whose runs on the pool take
    less than 20 µs is tried alone after 256 runs (a try costs up to four
    runs: at most 1.6 % of them), then the faster mode taken, alone unless
    the pool is faster by an eighth (the other cores kept free), the other
    tried again by two runs every 256; a run of the pool counted only after
    a run of the pool (after runs alone the threads sleep, and the first
    run pays for waking them: 12209 cycles for 16 elements of 10 steps,
    which kept a loop alone where the pool was twice as fast). The tasks
    write each its own results, whatever the thread (D121): the same
    results either way.
  - The threads of the pool spin 1024 pauses after a job, then sleep.
  - Tries kept or not, measured: tried after the second run, and up to 20
    µs, fasta 3.3 % slower in median (its loops of 2 to 20 µs run a few
    times: the tries were never gained back); without tries its time came
    back exactly (median 5.14 against 5.14 ms); hence the 256 runs. The
    last run alone as the measure (no average) kept a loop alone after one
    slow run of the pool (16 elements of 1000 steps 1819 ns).
  - Measured: the small `map`s of four threads 666 -> 70 ns (4 elements of
    10 steps) and 1088 -> 159 ns (16 of 10), the others as before (1304,
    3199, 7013 ns for the larger ones); the server (`isocron_only.sh`,
    fixed frequency, 10 s) 197367, 207104 and 156049 requests a second at
    16, 256 and 1024 connections (190973, 203414, 151777 before), its cores
    4.04, 3.99, 3.95 -> 2.24, 2.18, 1.91; eight programs of the Benchmarks
    Game, the same outputs, 8 alternate runs against the binaries before:
    medians within -0.99 and +0.91 %, minimums within -4.07 and +0.14 %.
  - Tests: `parallel.rs` reads the calls of the IR by their mode (`i32 0`
    every `threads`-th task, `1` chunks in turn, `2` ranges).
- **D358 — Small arrays of a fixed size without the heap: closed by
  measurement (2026-11-06).** The open point of TODO (D164 to D191: the
  arrays of n-body). The memory reports of the ten programs (`iscc build
  --report`, at 99e284e): no array of a fixed size is made at each
  iteration of a loop. Those of a fixed size (their bytes in the report)
  are made once, at the start: fasta 2, k-nucleotide 2, n-body 4 (40 bytes
  each, lines 20 to 27, and its 3 copies of lines 34 to 36 once), regex-redux
  2, reverse-complement 1. The arrays made at each iteration have a size
  known at run time: fannkuch-redux 2 (the parts of a chunk, lines 81 and
  87), k-nucleotide 5 (the bytes and the keys of a frame, lines 24 to 49),
  binary-trees 1 (its nodes: the work of the program). n-body's loop at
  line 56 makes none. Nothing to gain there.
- **D359 — The parts of a template written in its block (2026-11-06).**
  `time.formatIso` 4.7 times its floor, `template` of `strings` 3.1 (D348:
  each part a block made, copied into the result, freed). Profiled alone at
  fixed frequency (`formatIso` in a loop, `perf stat`): 652.0 cycles and
  2511.3 instructions a call.
  - The text of an integer (`toString(x)`, the `${x}` of a template) bound
    by a `let` whose only use, outside the functions of `map` and the
    others, is a part of a concatenation (`sole_concat_part`; the left
    part given to it too, D351): no string made, `x` described to the
    concatenation (`int_texts`). A padded text (`pad(x, n)` of `time`
    inlined: `let`s then a `padStart`/`padEnd` of a fill of one byte,
    `tail_pad`) is described with its padding the same way.
  - Such a concatenation (`concat_parts_built`) gives `isc_str_build` a
    table of three words a part: a text (its pointer, its length), an
    integer (its value, its padded width, signed, `padStart` or `padEnd`,
    its fill). The runtime counts the length of each integer without
    writing it (`digit_count`: its bits and a table of the powers of 10),
    makes the one block, writes each integer where it goes (`write_digits`,
    two digits at a time) and the texts and fills by moves (`copy_short`,
    `fill_short`), without the calls of `memcpy` and `memset`. A left text
    given (an accumulator, D351) is continued in its block when it has
    room, as `isc_str_append` does: a string built piece by piece stays
    linear (3 million pieces of `acc + toString(i) + ","` in 0.097 s).
  - The first loop of `isc_str_build` (the total length) was made by LLVM
    of `vpgatherqq` over the table, 3 to 13 parts: 721.4 cycles a
    `formatIso` (the gathers about half of the samples of the function);
    the lengths kept at the rank of each integer (a rank that grows only
    at an integer, a loop LLVM does not vectorize): 510.5 cycles.
  - `fraction` of `time` made `pad(nano, 9)` twice, its bytes, a `fold`, a
    slice and a concatenation; now the zeros at its end counted on the
    number (`nano` is `v` times 10 to the `z`) and `"." + pad(v, 9 - z)`
    written in one block: 510.5 -> 425.3 cycles; the moves for the texts
    and fills: 390.0 cycles and 1781.8 instructions (652.0 and 2511.3).
    The new `fraction` gives the text of the old one for 3 million values
    and the round ones (compiled and interpreted).
  - `results/primitives.md` (a whole run): `formatIso` 215.06 -> 129.29 ns
    (4.7 -> 2.8 times its floor), `template` 32.79 -> 25.59 ns (3.1 ->
    2.4). What is left of `formatIso` (its profile): `isc_str_build` 68 %,
    about 15 cycles a part (three words written by the compiled code, read
    back in two passes, a branch on each kind), where the floor writes
    each part in place: the parts written by the compiled code itself
    (the length summed and the literals stored where they are), open in
    TODO.
  - `concat` of `strings` (`l + "!"`, one block made and freed a call)
    10.00 ns against 9.38 at D357, +7.1 % in an A/B of the two binaries,
    while a loop of the same `concat` alone costs the same with both
    compilers (133.86 instructions, 31.7 to 32.1 cycles). The counter
    `ld_blocks_partial.address_alias` (4K aliasing) of the binary of the
    group 11.3 -> 15.4 M; gdb in the loop of `concat` (the same addresses
    as without randomization, where the times are the same): the header of
    its block at 0x100 in its page, the head of its class in the cache of
    the thread at 0x100 in its own (the TLS segment of the same 0x510
    bytes in both binaries: the place the loader gives the block of the
    main thread moved, and the block by the history of the heap; at D357
    0x180 and 0xf80). Each write of one followed by a
    read of the other waited for the write. `free` now reads the head of
    the list before it writes the header of the block (no instruction
    more): 10.40 -> 10.00 ns, a third of the gap. The other pair (`alloc`
    writes the head, `free` then reads the header) is excluded only by a
    line of 64 bytes for each class, the head in its upper half (no header
    can share its place): tried, 5 instructions and 2.5 cycles more for
    each `alloc` and `free` (the address of the slot computed), for a
    coincidence of about one position of a block in 64, in 2 classes of 8:
    not kept. The gap is that coincidence of this binary.
  - The other forms of the run outside 3 % against D357 (`startsWith.1`
    +5.7 %, `endsWith.2` +4.4, `bytes` +4.2, `parse` of `json` +4.0,
    `stringify` +3.1): in an A/B of the binaries of D357 and D359,
    -4.2, -3.4, -1.9, +0.8 and +0.2 %: the drift between sessions (D346).
  - The ten programs (their IR: the lines of their output only, in six;
    an A/B at fixed frequency on 4 threads, the lowest and the median of 8
    runs each, the same outputs): binary-trees +0.02 / -0.05 %,
    fannkuch-redux -0.00 / +0.13, fasta -0.46 / -0.71, k-nucleotide -0.19
    / -0.22, mandelbrot -0.03 / -0.06, n-body -0.34 / +0.59, pidigits
    +1.22 / +0.74 (16 runs: +0.10 / +0.31), regex-redux +1.85 / +3.31 (16
    runs: -1.95 / +0.89), reverse-complement -0.44 / -3.73, spectral-norm
    -0.16 / -0.22.
  - Tested by `integers_written_in_the_block_of_their_template`
    (`strings.rs`, `data/strings_template_parts.isc`: signed, unsigned and
    padded integers, `i64::MIN + 1`, 18 parts, accumulators of texts and of
    padded texts, a template in a `map`, fills and texts of 0 to 39 bytes,
    `formatIso`; the text of the interpreter, Valgrind; no text of an
    integer made in a concatenation, the left text of an accumulator
    given); `integers_padded_in_one_block` reads `isc_str_build` for its
    template.
- **D360 — The clocks read by the vDSO (2026-11-06).** `time.now()` 8.5
  times its floor (149.72 ns against 17.66): the runtime, without a C
  library (D287), read `CLOCK_REALTIME` by the system call, where the
  floor (`clock_gettime` of glibc) reads it by the vDSO, the code the
  kernel maps into every process so that its clocks are read without
  entering it (vdso(7)).
  - `linux::clock_gettime` (`time.now()`, `clock()`) calls the
    `__vdso_clock_gettime` of the process, found once: the auxiliary
    vector after the environment noted by `isc_args` (`AT_SYSINFO_EHDR`,
    the ELF image of the vDSO), its first `PT_LOAD` (where it was linked)
    and its `PT_DYNAMIC` (its symbols, counted by `DT_HASH` or
    `DT_GNU_HASH`), as the runtime of Go does without a C library. It is
    the kernel's interface for its clocks, not a library: the program
    still links no C library. Without a vDSO (`vdso=0`, Valgrind) or
    without that function, the system call as before.
  - `results/primitives.md` (a whole run): `now` 149.72 -> 20.70 ns (8.5
    -> 1.2 times its floor); in an A/B of the group `time` at fixed
    frequency 149.30 -> 20.82, the other forms -0.0 to +1.0 %. `concat` of
    `strings` 10.00 -> 9.38 ns: its code moved again, the coincidence of
    addresses of D359 gone. `env` of `sys` +3.7 % in the run: in an A/B
    of the binaries of D359 and D360, +2.5 % then -1.0 %, their nine
    times each overlapping (45.4 to 59.3 ns, 46.7 to 49.9): noise.
  - The ten programs read no clock: eight binaries the same bytes as at
    D359; pidigits and regex-redux differ by the places of the code of the
    runtime (its tables of exceptions numbered 510 -> 512); an A/B of 16
    runs on 4 threads, the lowest and the median: pidigits +0.00 / -0.04
    %, regex-redux +0.91 / -1.66.
  - Tested by `clocks_read_through_the_vdso` (`std_sys.rs`,
    `data/std_clock.isc`): a million readings never back, the second of
    the system; `strace` sees no `clock_gettime` of the program; under
    Valgrind (no vDSO) the system call gives the same.
- **D361 — Literals kept as strings without a copy (2026-11-06).** A
  literal a consumer keeps (returned, stored in a record, an array or a
  tuple, the start of an accumulator) was copied into a block of the heap
  (`copy` of a `string "..."` in the IR: `isc_str_new`), freed with its
  owner: 4.14 copies a request of the server (the headers of
  `textResponse` among them, D342), 22 calls a record of `decode` (15 of
  them the empty `why` of the tuples of its results), the `"Z"` of
  `formatIso`.
  - Such a copy is now the literal itself, its bytes in the read-only data
    of the program 4 bytes past a multiple of 16 (`owned_literal`: a
    pad of 4 bytes, align 16), a place where no block starts: the blocks of
    the heap are aligned on 64 bytes, the lines of `lines` 8 bytes past a
    multiple of 16 (D298), those of `malloc` and `HeapAlloc` on 16. The
    runtime never frees it (`is_literal` in `isc_free`, after the fast path
    and before the cold one), never writes it (`writable`, the replacement
    in place of D315) and gives it no room (`room`: only a block of the
    heap, aligned on 64, has room; a text grown from it gets a new block,
    D351).
  - Counted with gdb (2 rounds less 1): the server 4.14 -> 2.00 copies a
    request; `decode` 22 -> 7 calls of `isc_str_new` a record.
  - The first version left the empty literal out: `decode` +2.7 % in an
    A/B (534.2 to 538.1 ns against 549.1 to 554.0), the same calls of the
    runtime in both binaries, the instructions moved between functions:
    LLVM inlined otherwise once the paths of errors were shorter (three
    tasks of `json` now functions of 801 bytes each, `isc_bench_1` +1263
    bytes, `isc_bench_2` +550). With the empty literal kept too, `decode`
    -1.8 % in the same A/B.
  - An A/B of the binaries of D360 and D361 at fixed frequency (medians of
    9 runs): `formatIso` -5.5 %, `decode` -1.8, `stringify` -1.5, `parse`
    -0.9, `serve.pipelined` -1.2, `serveAction.pipelined` +1.0 then +0.5
    (overlapping: 1779.7 to 1852.5 ns against 1806.9 to 1843.6), `parseIso`
    +1.8 then +0.9 (overlapping: 37.4 to 39.3 against 38.2 to 39.4).
    `results/primitives.md` (a whole run): `formatIso` 129.47 -> 122.41 ns
    (2.8 -> 2.7), `basename` -3.0 %, `decode` -1.5 %, no other form
    beyond 2.5 %.
  - The ten programs (every binary changed: `isc_free`; k-nucleotide keeps
    10 literals, regex-redux 18), an A/B on 4 threads of 8 runs each:
    within -0.30 and +0.68 % for the lowest except regex-redux +1.85;
    again with 16 runs, k-nucleotide -0.48 / -0.23 %, n-body +0.51 /
    +0.02, regex-redux -0.14 / +1.11, reverse-complement +0.20 / +0.70.
  - Tested by `literals_kept_without_a_copy` (`strings.rs`,
    `data/strings_literals_kept.isc`: literals returned, in records and
    tuples of arrays, grown as accumulators, replaced where they would be
    written, sorted, made by the threads of a `map`, the empty literal;
    the text of the interpreter with 1 and 4 threads, Valgrind; three
    copies left in the IR, none of a literal).
- **D362 — A `filter` whose predicate can panic in its lanes is vector
  code (2026-11-06).** The server 1.6 times its floor: 2363.6 user cycles
  and 7596 instructions a request against 613.6 and 1602 for the floor in
  C (the kernel 3050.4 cycles against 2969.5: the gap is in the program);
  a quarter of its instructions in `headEnds` of `http` (`filter(iota(n),
  (i) => buf[i] == 13u8 && buf[i + 1] == 10u8 && ...)`), 20.2 instructions
  and 4.2 cycles a byte alone: scalar. `filter` was vector code (D195) only
  for a predicate that cannot panic or has one site: `a && b` is an `if`
  whose branch `b` reads an array, accepted in vector code only when its
  failed checks are replayed one element at a time (D185), which the
  vector loops of `filter` did not do.
  - `kept_mask`: the mask of the elements a `filter` keeps, in vector code;
    for a predicate that needs it (`lam_ok` refuses it, `lam_ok_replay`
    accepts it), a failed check jumps to the replay of the vector: the
    predicate of each of its elements computed again in order, so that the
    panic is that of the scalar loop (its element, its site) and the check
    of a lane the scalar code would not make (a guard false) panics not.
    `count_kept` and `write_kept` (the two passes, every `filter`, in
    order and by blocks) read it.
  - `headEnds` alone: 4.2 -> 1.2 cycles and 20.2 -> 4.3 instructions a
    byte. The server: 2363.6 -> 2116.8 user cycles and 7596.2 -> 6469.3
    instructions a request; an A/B of the group `http` (medians of 9 runs)
    `serve.pipelined` 1486.8 -> 1408.6 ns (-5.3 %), `serveAction.pipelined`
    -4.4 %, `exchange.small` +0.7, `exchange.chunked64` -0.4.
  - `results/primitives.md` (a whole run): `decode` 526.05 -> 473.31 ns
    (6.0 -> 5.5 times its floor), `parse` 1120.94 -> 1024.22 (6.7 -> 6.1;
    16 functions of the group changed: the filters of `json` guarded by
    `&&`), `serve.pipelined` 1486.44 -> 1405.74 (1.6 -> 1.5),
    `serveAction.pipelined` -4.4 %. `startsWith.1`, `startsWith.8`,
    `endsWith.2`, `slice`, `bytes` of `strings` -3.6 to -4.8 %: the IR of
    the group the same (0 functions of 46 changed), the drift between runs.
  - The ten programs: their IR the same, their binaries the same bytes as
    at D361.
  - Tested by `filters_with_guards_replay_their_masks` (`vector_index.rs`,
    `data/vector_filter_guards.isc`: the ends of heads of requests, a guard
    `i + 1 < n &&` that keeps the last lanes from reading past the end, a
    real fault behind `&&` at the end of 7200 and of 120001 bytes, one
    block and several, 1 and 4 threads, against the interpreter; the
    filters compressed in the IR).
- **D363 — The parts of a template written by the compiled code
  (2026-11-06).** After D359, `isc_str_build` took 68 % of `formatIso`
  (about 15 cycles a part: three words a part written by the compiled
  code, read back in two passes, a branch on each kind) and 8 % of the
  user cycles of the server (the head of a response, 10 parts).
  - The compiled code computes the length of each part (a literal: a
    constant; a text: its length; an integer: its sign and its digits from
    its bits, `ctlz` and a table of the powers of 10 `@.pow10`, its width
    `smax(len, n)` when padded), takes one block (`isc_alloc`, or
    `isc_str_reserve` for a left text given, D351: its block when it has
    room, else one of at least twice its length) and writes each part
    where it goes: a literal (a variable bound to a literal, `lit_vars`) by
    a `memcpy` of a constant length (moves of constants once inlined), a
    text by `isc_put_text` (`copy_short`), an integer by `isc_put_int`
    (its digits two at a time, its sign, its fill). `isc_str_build` and
    `digit_count` are gone.
  - `formatIso` alone: 390.0 -> 241.8 cycles and 1781.8 -> 958.8
    instructions a call. An A/B at fixed frequency (medians of 9 runs):
    `formatIso` 122.39 -> 82.28 ns (-32.8 %), `template` 25.71 -> 14.80
    (-42.5 %), `serve.pipelined` -3.4 %, `serveAction.pipelined` -2.1 %,
    `parseIso` +0.2, `concat` +0.1, `padStart` +1.1, `exchange.small`
    +0.7. 3 million pieces of `acc + toString(i) + ","`: 0.067 s.
  - `results/primitives.md` (a whole run): `template` 25.67 -> 14.80 ns
    (2.4 -> 1.4 times its floor), `formatIso` 122.37 -> 81.90 (2.7 -> 1.8),
    `serve.pipelined` 1405.74 -> 1363.77 (-3.0 %). `bytes`, `endsWith.2`,
    `startsWith.8`, `slice`, `startsWith.1` of `strings` +6.3 to +8.1 %
    (an A/B: +3.9 to +5.4 %), the forms of D362 at -3.6 to -4.8 % with the
    same IR: their functions the same machine code at the same addresses
    in both binaries (`isc_bench_1` at 0x7b0a0, `isc_alloc`, `isc_free`,
    `isc_parallel_for_site` too), `main` moved and smaller. The same
    binary with the stack moved by 0 to 3584 bytes (an environment
    variable, without randomization): `bytes` of D362 1.537 to 1.730 ns,
    of D363 1.598 to 1.720 (D363 the faster at +1024 bytes): the place of
    the stack, not their code.
  - The ten programs: fasta, n-body, reverse-complement, spectral-norm the
    same bytes; the six others (the lines of their output) an A/B on 4
    threads of 8 runs: within -0.25 and +0.60 % for the lowest except
    regex-redux +2.78 (median -0.45); again with 16 runs binary-trees
    +0.09, k-nucleotide -0.19, regex-redux +0.08.
  - Tested by `integers_written_in_the_block_of_their_template` (no table
    given to the runtime, the 18 integers of a template written by
    `isc_put_int`, its 17 literals of one byte stored as constants, the
    accumulator by `isc_str_reserve`) and the other tests of templates of
    `strings.rs`, against the interpreter, 1 and 4 threads, Valgrind.
- **D364 — The test of the HTTPS client waits for its server
  (2026-11-06).** Suite 169 (at 3a6c2aa, run alone after a forced reboot
  of the machine, its caches cold, a load average of 11.58): `std_https`
  failed with Isocron's allocator, every connection of the interpreted
  client refused (`refused localhost:34657`, 7 times), while the same
  binary alone passed 6 times of 6 (3 with each allocator). `s_server` of
  the test waited at most 5 s for `openssl s_server` to open its port,
  then went on without it; its port, taken by `free_port` and given back,
  may also be taken by another test before the server binds it. Now it
  waits up to 30 s, and a server that stopped or did not open its port is
  started again on another port, five times at most, the test panicking
  with that message otherwise. The code of the compiler and the runtime
  unchanged.
- **D365 — The test of the HTTPS pair on Windows waits for the port of
  its server (2026-11-06).** `https_server_on_windows` failed twice at
  3a6c2aa (the second time with nothing else running on the machine): the
  first request of the client refused (`refused: localhost:47970`), the
  others answered, the server waiting for its third request (no `done`).
  Its script started the server and waited a fixed 4 s (`ping -n 5`).
  Timed in the VM with the same binaries (the opening of the port polled
  every 100 ms): 1.97 then 0.53 s by `Start-Process`; by the launch of the
  test (`start /b cmd /c`), 1.14 s on the first run after a start of the
  VM, 0.02 s after; once, on the first run after a start of the VM, the
  port still closed after 30 s and the server failing with
  `0x8009030d of SChannel` (SEC_E_UNKNOWN_CREDENTIALS), the next run of the
  same files in the same folder answering everything; not seen again in
  the 5 runs after it (one after a new start of the VM, the service
  KeyIso stopped at the start and running after). The script now waits
  for the port to open (`wait.ps1`, at most 60 s), then runs the client:
  the windows tests 10 of 10. The intermittent failure of SChannel on a
  first run after a start of the VM is not explained: it is recorded here
  to be looked for again if it comes back.
- **D366 — The server under load measured again after D359 to D363
  (2026-11-06).** `measure.sh` at 5d7f925 (fixed frequency, the server on
  4 CPUs, wrk on 4 others, 10 s a point, `results/http_servers.md`):
  `serve` 70564, 201587, 206674 and 155344 requests a second with 1, 16,
  256 and 1024 connections (1.40, 2.10 and 1.81 cores busy with 16 to
  1024), +2.1, -0.2 and -0.5 % against D357 (197367, 207104, 156049);
  84, 73, 72 and 58 % of the ceiling of one loop (83536, 275431, 288547,
  265892, the same as at D355). The benchmark of one thread took 8.6 % of
  a request away (`serve.pipelined` 1491.87 -> 1363.77 ns), all in the
  program; under wrk a request takes 4.96 µs of the loop with 16
  connections (3.63 for the ceiling), the kernel about 59 % of the cycles
  of a request (3045.8 of 5162.6 at D362): the gains of the program are a
  few per cent of it. nginx 630152 to 785171 on 4 cores, lighttpd 187464 to
  196662 on one. Open: a loop of events on each core, the cost of a
  request of the loop.
- **D367 — The payload of a `match` on a temporary taken out of it
  (2026-11-06).** The open point of D346: `f(x) ?? d` and `match (f(x)) {
  Ok(v) => v, ... }` bind the result of the call to a temporary (`let drop
  match = f(x)`), copy the payload in the branch that gives it (`copy (proj
  1 match)`: a block made and its bytes copied for a string) and free the
  whole temporary after the `if`.
  - `take_payloads` (after the pass of ownership): in such a `let drop`
    whose body is the `if`s of the match (their conditions reading only
    scalar parts of it), the branch that gives a payload takes it (`take k
    match`; the other parts that own memory taken and freed there, zero in
    that branch: the error of an `Ok`), the other branches end by freeing
    the temporary (`Free(match, e)`), and the temporary is no longer freed
    after the `if`. Nothing taken: the body as it was. A function whose
    memory check (`verify_fn`) fails after the rewrite is kept as it was,
    `main` too (`verify`).
  - The checker (D96) kept the parts taken out of a tuple (`Take`, D138)
    for the whole program: a part taken in one branch made the read of the
    tuple in the other an error (`match is used after a part of it was
    taken`). The parts taken are now restored before the second branch, as
    the states of the variables are, and those of both kept after the
    `if`.
  - No function of the benchmarks of the ten modules is kept as it was.
    `env` of `sys` (`sys.env("HOME") ?? ""`), an A/B at fixed frequency
    (medians of 9 runs): 46.81 -> 41.59 ns (-11.2 %); `parse`, `decode`,
    `stringify`, `encode`, the forms of `http` and of `time` within -0.8 and
    +1.1 %. `results/primitives.md` (a whole run): `env` 46.83 -> 40.90 ns
    (4.1 -> 3.6 times its floor). The other forms beyond 2.5 % (`bytes`,
    `endsWith.2`, `slice`, `startsWith.1`, `startsWith.8` of `strings` -6.0
    to -6.9 %, `roundtrip.64`, `send.64k`, `connect` of `net` +2.5 to +4.1,
    `normalize` +3.2, `writeFile.4k` -3.1): the IR of `strings` and `net`
    the same (0 functions of 46 and of 67 changed), that of `sys` changed
    in `envLoop` only, the runtime the same: the drift between runs (D363).
  - The ten programs: their IR the same, the code generator and the
    runtime unchanged since D363.
  - Tested by `match_payloads_taken` (`result.rs`,
    `data/match_payloads_taken.isc`: `??`, a `Result` whose error owns
    memory, a match of three arms, a payload that is a tuple, an arm that
    reads the temporary, the threads of a `map`; the text of the
    interpreter with 1 and 4 threads, Valgrind; the takes and the frees in
    the IR).
- **D368 — A concatenation with a literal written by the compiled code;
  short copies up to 64 bytes by moves (2026-11-06).** `concat` of
  `strings` (`l + "!"`, a line of 60 bytes) 3.0 times its floor: alone,
  29.93 cycles and 133.89 instructions a call, `isc_str_concat` 48 % (its
  allocation inlined, six registers saved, two calls of `memcpy` through a
  pointer), `free` 27 %, `memcpy` 15 %.
  - A concatenation one of whose parts is a literal (a variable bound to
    one, `lit_vars`) takes the way of D363 too: the length summed by the
    compiled code, one block, the literals stored as constants, the texts
    by `isc_put_text`, a left text given by `isc_str_reserve`.
  - `copy_short` (`isc_put_text`, the replacements of D280 and D308)
    copies 33 to 64 bytes by two moves of 32 bytes that overlap, a call of
    `memcpy` beyond (from 32 before).
  - `concat` alone: 29.93 -> 20.34 cycles and 133.89 -> 87.89 instructions
    a call. An A/B of the groups at fixed frequency (medians of 9 runs):
    `concat` 9.215 -> 6.278 ns (-31.9 %), `encode` -28.9 %, `stringify`
    -20.9 %, `normalize` -17.4 %, `parseIso` -4.1 %, `decode` -3.6 %,
    `parse` -1.0 %, the server -0.8 and -0.9 %, `template`, `padStart`,
    `trim`, `formatIso`, `basename`, the exchanges within -0.9 and +0.5;
    `env` +2.3 % (`envLoop` the same IR, `main` changed: the places of the
    stack, D363). `results/primitives.md` (a whole run): `concat` 9.28 ->
    6.27 ns (3.0 -> 2.0 times its floor), `encode` 181.66 -> 128.17 (0.8 ->
    0.6), `stringify` 576.81 -> 451.63 (1.5 -> 1.2), `normalize` 209.57 ->
    165.78 (4.2 -> 3.3), `env` -4.5 %, `decode` -3.3 %; no form slower by
    2.5 % or more.
  - The ten programs: nine binaries the same bytes as at D367; k-nucleotide
    (the templates of its lines of output) and regex-redux (`copy_short` of
    its replacements) an A/B of 16 runs on 4 threads, the lowest and the
    median: +0.19 / +0.03 % and +1.17 / +0.16 %, the same outputs.
  - Tested by `concatenations_with_literals_written_by_the_code`
    (`strings.rs`, `data/strings_concat_literals.isc`: texts of 0 to 80
    bytes around literals, an accumulator grown by literals, replacements
    whose matches are 0 to 80 bytes apart; the text of the interpreter with
    1 and 4 threads, Valgrind; no `isc_str_concat` in the IR).
- **D369 — A text padded by a fill of one byte written by moves
  (2026-11-06).** `padStart` of `strings` (a slice of 10 bytes padded to
  20 by `"0"`) 5.8 times its floor: alone, 37.69 cycles and 178.89
  instructions a call, `isc_pad` 45.6 %, `free` 27.0 %, `memcpy` 7.6 %,
  `memset` 4.7 %: the fast way of D302 wrote the text by
  `write_copy_of_slice` and the fill by a loop that LLVM made a call of
  `memset`.
  - `isc_pad` and `isc_pad_int` write the text by `copy_short` and the fill
    by `fill_short` (D359) into the block of `isc_alloc`.
  - `padStart` alone: 37.69 -> 32.18 cycles and 178.89 -> 152.89
    instructions a call. An A/B of the group at fixed frequency (medians of
    9 runs): `padStart` 11.755 -> 10.181 ns (-13.4 %), `concat`,
    `template`, `trim`, `toUpperCase`, `formatIso`, `parseIso` within -0.3
    and +0.8 %. `results/primitives.md` (a whole run): `padStart` 11.81 ->
    10.21 ns (5.8 -> 5.1 times its floor); `env` +3.8 % (it calls neither
    function, the runtime of the binary changed only in them: the places of
    the code and of the stack, D363). What is left: the block made and
    freed (`free` 30 % of the 32.18 cycles), where the floor writes into a
    buffer.
  - The ten programs: nine binaries the same bytes as at D368; pidigits
    (`padEnd` of its lines) and regex-redux an A/B of 16 runs on 4
    threads, the lowest and the median: +0.06 / -0.01 % and -0.03 / -0.40
    %, the same outputs.
  - Tested by `texts_padded_by_moves` (`strings.rs`: fills of 0 to 40
    bytes before and after texts of 0 to 20 bytes, a fill of two bytes;
    compiled, interpreted, Valgrind).
- **D370 — The variables of the environment found by their names where
  they are (2026-11-06).** `env` of `sys` (`sys.env("HOME") ?? ""`) 3.6
  times its floor (`getenv` of glibc): alone, 134.21 cycles and 499.34
  instructions a call, `isc_sys_getenv` 63.7 % (the name copied with a
  zero byte into a buffer of 256 bytes cleared at each call, scanned twice
  for `=` and a zero byte, its length counted again, then each of the 36
  variables compared byte by byte), `from_utf8` of the value 20.6 %.
  - `env_value` (Linux) compares the name, without a copy, with each entry
    of the environment noted by `isc_args`: rejected at its first byte, the
    rest compared up to the first difference (an entry shorter than the
    name differs at its zero byte); the value tested ASCII by words
    (`is_ascii`) before `from_utf8`. Windows and WASI keep their `getenv`.
  - `env` alone: 134.21 -> 66.61 cycles and 499.34 -> 302.34 instructions a
    call. An A/B of the group `sys` at fixed frequency (medians of 9 runs):
    `env` 43.313 -> 22.559 ns (-47.9 %), `normalize`, `basename` -0.1 %,
    `exists` +2.3 % (a system call of 680 ns, its code the same).
    `results/primitives.md` (a whole run): `env` 40.52 -> 21.17 ns (3.6 ->
    1.9 times its floor); `bytes`, `endsWith.2`, `slice`, `startsWith.8`,
    `startsWith.1` of `strings` +5.3 to +6.9 %, the binary of the group the
    same bytes as at D369: the drift between runs.
  - The ten programs: their binaries the same bytes as at D369.
  - Tested by `environment_names_compared_where_they_are` (`std_sys.rs`,
    `data/std_env_names.isc`: a name that is the start of another, a name
    longer than the variable that starts like it, an empty value, a value
    that is not UTF-8, an empty name, a name with `=`; compiled and
    interpreted).
- **D373 — Actions in parallel: `mapParallel` (2026-11-06).** The author
  put the loop of events on each core next (« ça après », then « ok et
  ensuite »), which needs actions run at the same time (D352, D355).
  - The language (SPEC, « Actions en parallèle »): `mapParallel(x, f)`,
    `action (T[n], action (T) => U) => U[n]`, the action applied to each
    element at the same time on the threads of the pool, its results in
    the order of `x`; the effects of one application in its order, none
    between two; with one thread (the interpreter, `ISC_THREADS=1`) one
    after the other, in order. Refused at compile time: in a `function`,
    in a pure lambda, over arrays or of arrays (the POC: the rows of an
    array of arrays are written by `materialize_rows`), a lambda that
    captures a connection or a listener (linear, D339: two tasks never
    hold the same socket or server).
  - The compiler: a node `Tasks(x, lambda)` of the IR (`x` bound to a
    variable by the lowering, the lambda that of `map`), never fused,
    never removed (`is_pure` false), its results each new (the ownership
    pass, the verifier); the code: the lambda outlined as the task of a
    `map` is (`outline_task`, its captures in the context), each task
    reading its element and writing its result, given to
    `isc_parallel_actions`. The runtime: a dynamic job of the pool, each
    task taken by the next thread free, the caller's too, its wait after
    its share asleep as the workers' (an action may wait for the system
    for long), never on the calling thread alone for being faster (D357:
    two actions that wait for each other must run at the same time).
    A `map` written in the lambda is compiled inline, as in any task (`if
    self.in_task` in the code generator); one inside a function that the
    lambda calls goes through the pool, a nested loop (D201) that the
    threads free share.
  - The primitives of the runtime from several threads: sockets,
    listeners and servers of `net` are linear, so each belongs to one
    task; files go by their path; the allocator has its caches by thread.
    The buffer of `print` was a global without a lock: while tasks run
    (`ACTIONS`, a count), the thread that writes the first part of a line
    holds it (`OUT_HOLDER`, the address of its cache of the heap) until
    its newline, `printBytes` for its bytes; a panic waits for the line of
    another task 2048 pauses of at least 50 µs (about 0.1 s), then takes
    the buffer. Without tasks, one load of the
    count per call of `out` and one per newline: 6.5 instructions more a
    line of `print(i)` (`perf stat`, 10 million lines: 2850456620 ->
    2915749942 instructions), 0.7 and 2.0 cycles more a line in two runs
    (`cycles:u`, of about 125 a line), +1.10, +1.37 and +1.06 % for a
    program that only prints 10 million short lines into `/dev/null`
    (medians of 11 alternated runs; old against old +0.00 %). A first
    version with a third load (the holder at each newline) had 9.02
    instructions more a line, 1.15 and 0.85 cycles, and +0.37 % in its one
    A/B: why the version with fewer instructions measured slower is not
    shown (TODO).
  - `sysThreads()` (the threads of the pool, 1 in the interpreter and
    under WASI, whose `thread_count` is 1) for the modules.
  - The ten programs, A/B at fixed frequency (7 alternated runs, 4
    threads): the same outputs, binary-trees -0.07 % (medians), fannkuch
    -0.23, fasta +1.07 (5 ms runs; minimums +0.17), k-nucleotide +0.25,
    mandelbrot -0.04, n-body +0.20, pidigits -0.43, regex-redux -0.33,
    reverse-complement -0.06, spectral-norm -0.11.
  - Tests (`actions.rs`): `actions_in_parallel` (16 tasks of 300 lines
    each, no line cut, an action by its name, a pure function,
    `mapParallel` inside a task, an empty array; the lines and results of
    the interpreter, compiled on 1 and 4 threads),
    `actions_in_parallel_wait_for_each_other` (one task listens and reads,
    the other connects again until the listener is there and writes; 2
    and 4 threads), `actions_in_parallel_refused`; an example in
    `docs/language.md`.
- **D374 — A loop of events on each thread of the pool for `serve`
  (2026-11-06).** One loop of events took 1.40 to 2.10 cores and 58 to 73
  % of the ceiling of one loop (D366), against a ceiling of 4 cores of
  742655 to 808459 requests a second.
  - Under Linux, `serve(port, handler)` runs `mapParallel` over the
    threads of the pool (`sysThreads`), each task opening a listener of
    its own of the port with `SO_REUSEPORT` (`sysListenShared`: the kernel
    shares the new connections out among the listeners of the group,
    `socket(7)`; seen here: 64 connections answered by the 4 loops, the CPU
    time of the server on its 4 threads) and running `serveOn` on it; the first error of the
    loops is the result (they end only on one). A listener that cannot be
    opened ends its loop at once; the others share the port. One loop
    under Windows (its headers, those of mingw-w64, define `SO_REUSEADDR`
    and `SO_EXCLUSIVEADDRUSE` and no `SO_REUSEPORT`; whether `SO_REUSEADDR`
    would share the connections out was not checked: `sysListenShared`
    gives the code of `EOPNOTSUPP`), with one thread, for
    the port 0 and for `serveOn`; (port 0: each listener gets a port of its own, even with
    `SO_REUSEPORT`, checked: three sockets bound to 0, three ports);
    `serveAction` keeps one loop (its handler answers one
    request at a time, D352), and so do the servers of `https` (not
    measured yet). The interpreter has one loop (one thread);
    `sysListenShared` is `sysListen` there.
  - A port already listened to is refused first: `serve` opens a listener
    of its own on it (without `SO_REUSEPORT`), which fails if another
    socket listens there (`AlreadyExists`), and closes it before opening
    the shared ones. Without it, a second copy of a server on the same
    port joined the group of the first (the kernel lets the sockets of one
    user with the option share a port) and served too: the test of the
    README (`every_example_of_the_readme_works`, a second copy that must
    end with `already exists`) waited for it without end in the suite. A
    process of the same user that binds the port with `SO_REUSEPORT` in
    the moment between the two, or later, still shares it (the kernel's
    rule).
  - Measured in `isocron-http` (`isocron_only.sh`, 10 s a point, fixed
    frequency, the server on 4 CPUs, wrk on 4 others): 1, 16, 256 and 1024
    connections 71749, 581516, 776540, 656243 requests a second (3.85,
    4.00, 3.96 cores with 16 or more), against 70564, 201587, 206674,
    155344 with one loop (D366): 2.9, 3.8 and 4.2 times with 16, 256 and
    1024 connections; 78, 96 and 84 % of the ceiling of 4 cores (742655,
    808459, 782530), nginx 630152, 785171, 753388.
  - By busy core (requests a second over cores): Isocron 151043, 194135,
    165718; nginx 173596, 198777, 190249; the ceiling 229924, 246481,
    253246: Isocron 87.0, 97.7 and 87.1 % of nginx. In this measure the
    ceiling keeps only 3.09 to 3.28 of its 4 cores busy and is 3.0 %
    above nginx with 256 connections, 3.9 % with 1024: something outside
    the servers limits them, most likely wrk (its CPU time not measured,
    a hypothesis). Being far above nginx needs a measure where the server
    is the limit (fewer cores for it than for the client), and the cost
    of a request of the loop brought down to the ceiling's (TODO).
  - Tested by `serve_runs_a_loop_on_each_thread` (`std_http.rs`: 4
    listeners of the port with 4 threads, 1 interpreted; a second server on
    the port refused, the listeners of the first the same; 64 connections
    of 50 requests each answered) and by the server of the README.
- **D375 — The claims of this log checked again (2026-11-06).** I wrote
  « sous WASI, le programme n'a pas accès aux fichiers » to explain a
  failing test and changed an example on it; it was false (the test runs
  wasmtime with `--dir=/::/`: only a relative path failed, the current
  folder not being opened). The author: « tu racontes n'importe quoi et du
  coup tu te bases sur ces fake news pour coder ? », « ça me fait peur par
  rapport à tout ce que tu as pu faire dans ce sens ». Two independent
  readings of the whole log listed every claim about the system, a
  library or a cause that its entry does not support by a measure, a test
  or a source: 96, of which 35 confirmed afterwards (a header, a manual,
  `/sys`, the code), 10 contradicted, 51 not checked. The tests check what
  the programs do, not the reasons written here: none of the contradicted
  claims makes the code wrong, two weighed on a choice (the first two
  below).
  - Contradicted, each marked where it stands:
    - The entry « the product of a big integer by a word in assembly »
      (after D214): the loop « at the floor of the machine (0.739 ns per
      word) ». `results/machine_floor.txt` has `mul_word_asm 0.332
      ns_per_word`; D222 corrected it (0.33 ns in assembly). The assembly
      was dropped on a wrong floor.
    - D285: a tmpfs « after the memory of the machine was short » holding
      a file in pages of 4 KiB. `transparent_hugepage/shmem_enabled` is
      `[never]` here and `/tmp` has no `huge=`: a tmpfs has pages of 4 KiB
      always. The cause of the faults is not shown (a hypothesis: pages
      swapped out).
    - D248: « from glibc 2.34 on, `__libc_start_main` ignores » the init.
      glibc 2.41 here: `__libc_start_main@@GLIBC_2.34` and `@GLIBC_2.2.5`
      at the same address (0x29ce0), `mov %rcx,%r14` ... `test %r14,%r14`
      ... `call *%r14`: a non-null init is called. Embedding
      `__libc_csu_init` stays right, for another reason.
    - D137: the shift amount modulo the bits « as the x86 instructions ».
      Measured on this processor: `shlb` of 1 by 8, 9 and 10 gives 0,
      `shlw` by 16 to 19 gives 0 (the count masked to 5 bits, not to the
      width). The language is right (an explicit `and`; `u8` and `u16`
      shifts by 0 to 35 the same interpreted and compiled).
    - D332: « `nlink_t` is 32 bits » under WASI. `__typedef_nlink_t.h` of
      wasi-libc: `unsigned long long`. The offsets measured there are right.
    - D246: « Windows cannot move pages ». True of `VirtualAlloc` only:
      the headers of mingw-w64 have `VirtualAlloc2`, `MapViewOfFile3`,
      `MEM_RESERVE_PLACEHOLDER` and `MEM_REPLACE_PLACEHOLDER` (memory of a
      section mapped again elsewhere).
    - D299: the output by lines « as C's standard output does ».
      `stdout(3)`: C buffers by lines only into a terminal; into a pipe or
      a socket by blocks, where Isocron writes each line.
    - D312: « the 39 accesses to the stack » of n-body; D261 measured 30
      after its split. Not measured again.
    - D195 and the entry « kept elements compressed into a register »: the
      compression into memory « slow on Intel's cores ». D119 measured that
      form within about 1 % (5444 against 5522 ns, 4.018 against 3.973
      ms); the 0.697 against 0.625 s of D195 does not isolate it.
    - D120: « the L2 cannot hold the chunk being computed and the next
      one »: chunks of 16 Ki lost too, two of which fit in its 512 KiB.
  - Not checked: hypotheses written as facts. Those that led to a choice
    are to be measured (TODO): the reason of V = 32 (D55: latency and
    adders not counted), the prefetchers of L2 (D77, D78) and the worst
    case of 4K aliasing at 64 bytes (D81) behind the places of large
    arrays, the 32-byte windows of the decoders behind the alignment of
    the whole runtime (D256), the states of a core while its sibling
    sleeps (D261) and the desktop session on cores 0 to 3 (D300) behind
    the protocol of the measures, and the static programs with only
    `R_X86_64_RELATIVE` relocations (D289: pidigits has 24 of them and no
    `RELR` table, but `isc_start` skips any other kind without a word).
    The others: D2 (the default target of `zig cc`), D46, D48, D64, D65
    and D74 (the memory modules), D67, D68, D74, D81 (the prefetcher, a
    locked add), D85, D92, D95, D115, D116, D117, D119 (AMD Zen), D121,
    D122, D140, D170 (« tens of cycles » a division, against D228), D174,
    D189, D198, D199, D205, D222, D223, the entry after D214 (`mulx` and
    `rdx`), D235, D239, D241, D265, D268, D270, D285 (ext4), D288 (two
    causes), D294, D298 and D361 (the alignment of the blocks of WASI's
    dlmalloc), D305, D343 (OpenSSL and `MSG_NOSIGNAL`), D354, D359 and
    D360 (a coincidence of the binary).
  - Confirmed with limits the entry left out: D257, a read lease breaks
    within `/proc/sys/fs/lease-break-time` (45 s here), after which the
    kernel removes it; an `O_NONBLOCK` open fails at once.
  - From now on: no claim about the system, a library or a cause is
    written here, in a comment or to the author without the check that
    shows it (a run, a counter, a header, a manual, the source) named next
    to it; otherwise it is written as a hypothesis.
- **D371 — The small blocks moved between the caches of the threads by
  whole lists (2026-11-06).** The profile of `serve` (D374: a loop on each
  of 2 threads, 1024 connections, wrk on 6 other cores, `perf record -p`):
  44.38 % of the cycles in the program, of which the allocator 25.75 %
  of all the cycles, `refill` 12.81, `isc_free` 4.94, `isc_alloc` 4.37,
  `spill` 3.63; the LBR stacks give 90.6 % of the samples of `refill` to
  `serveLoop` (`isc_fn_11`, by its signature in the IR). A turn of the
  loop takes and gives back more blocks of a class than the cache of the
  thread holds (256 at most): `refill` took half of the limit from the
  global list one block at a time under its lock (each `next` read a
  miss), `spill` gave half back the same way.
  - Now, for the 16 first classes (64 bytes to 1 KiB): a full list becomes
    the batch in reserve of the thread in one step, the batch before goes
    to the batches of all the threads (one lock, one step); an empty list
    takes the batch in reserve, else a batch of the others, else the old
    path (`SMALL_BATCHES`, the count of a batch in the `len` of its first
    block, the next batch in its `prev`). The counts of the cache in 16
    bits, so that the reserve fits in its 1024 bytes (2048 bytes moved the
    cache of the nodes and slowed binary-trees, measured before this
    commit). The larger classes keep the old path.
  - The server after: `refill` and `spill` gone from the top of the
    profile, `isc_alloc` 13.90 % (the header of a block of a batch read
    when it is taken: the miss moved there), `isc_free` 5.50 %, the
    program 39.0 %. Requests a second on 2 cores, 3 alternated runs of 10
    s: 1024 connections 317385.04 -> 343805.29 (+8.3 %, medians) and
    319112.74 -> 340456.92 (+6.7 %) in a second series; 16 connections
    347674.98 -> 342406.56 and 339923.83 -> 335309.11 (-1.5 and -1.4 %,
    the runs of both overlapping: 337338 to 345308 against 338370 to
    348551, and 334522 to 344767 against 334808 to 343136).
  - The forms (`results/primitives.md`, a whole run against the run of
    D370): `json.parse` -15.2 %, `serveAction.pipelined` -9.9 %,
    `stringify` -6.2 %, `bytes`, `endsWith.2`, `startsWith`, `slice` -3.0
    to -3.7 %, `serve.pipelined` -2.7 %, `decode` -2.6 %. Above +2 %:
    `parseI64.7` +7.2 %, `toString.i64` +2.9 %, `connect` +2.6 %,
    `template` +2.4 %, `env` +2.1 %. Alternated (9 runs): `toString.i64`
    +0.0 %, `template` +0.7 %, `concat` +1.2 %, `parseI64.7` +7.1 %.
    `parseI64.7` allocates nothing (`sum(map(nums, parseI64))`): the same
    code placed elsewhere. A program of that form alone, padded so that
    the runtime lands at four places 64 bytes apart: 9.112 and 9.111 ns
    with `parse_i64` at 0x13f40 and 0x13f80, 9.431 and 9.433 ns at 0x13fc0
    and 0x14000 (the place in the binary of `strings` now), the same
    1031 M instructions; 238.06 to 239.46 M cycles against 246.18 to
    247.57 M, `idq_uops_not_delivered.core` 302.42 and 302.62 M against
    343.99 and 344.62 M slots (about 5 slots a cycle more, the width of
    the core), the cycles with no micro-op delivered, the uops of the DSB
    and of the decoders, the mispredicted branches, the 4K aliasing and
    the blocked stores the same. The uop cache delivers fewer micro-ops a
    cycle with the code 64 bytes further; why, not shown (it is not the
    page: at 0x14000 the function is in one page). A coincidence of the
    place of the runtime, shown by moving it, not by D371's code.
  - The ten programs (7 alternated runs, 4 threads, against the runtime of
    D374): the same outputs, minimums -1.15 % (fasta, runs of 5 ms) to
    +0.07 % (mandelbrot), binary-trees -0.12 %. binary-trees counted:
    instructions 26416 -> 26428 M, 4K aliasing 283.7 -> 83.2 and 91.2 M,
    misses of L1 176.0 -> 176.2 M; its user cycles 7550.7 and 7567.5 M
    against 7597.9 and 7734.1 M, its medians +1.52 and -1.56 % in two
    series of 11 runs: the waiting of its pool, which these counters
    count, varies from one run to the next.
  - Tested by `small_blocks_move_between_threads_by_batches`
    (`parallel.rs`, `data/heap_batches.isc`: blocks made by threads and
    freed by others, in batches, the results of the interpreter) and the
    tests of the allocator (`heap.rs`).
- **D372 — The cache of the heap at a fixed place of its page; the
  benchmark of `now` (2026-11-06).** With D371, 640 bytes of static data
  more moved the block of TLS of the main thread (`MAIN_TLS` of
  `process.rs`, in `.bss`): `counts[0]` came where the header of the block
  that `concat` takes and gives back lay, +51 % (measured before this
  commit, in this session). Now the TLS section of the cache is aligned on
  4 KiB, 64 bytes into its page: `place_tls` aligns the thread pointer of
  every thread on the largest of the alignment of the segment and 64 (the
  code), so `heads` and `counts` are at the same place of their pages in
  every program; the block of the main thread no longer fits in
  `MAIN_TLS` and is mapped. In a program linked with glibc, glibc places
  it (not checked). The test `thread_caches_at_a_fixed_place_of_their_page`
  (`parallel.rs`) reads the alignment of the TLS segment of a program
  (4096). The benchmark of `now` timed r calls (200 calls, 4 µs: a cost
  paid once of about 2 µs counted as 10 ns a call); it now times r times n
  calls (`time.isc`, `time.c`): 20.28 ns against a floor of 16.92.
- **Rejected after measurement: the header of the next block prefetched
  at each allocation (2026-11-06).** After D371 the miss of `serve` is the
  header of the block taken (86.5 % of the samples of `isc_alloc` on the
  instruction after its read). A `prefetcht0` of the next block of the
  list in `alloc_cached`: the server on 2 cores, 3 alternated runs, 1024
  connections 345705.85 against 340456.92 for D371 (+1.5 %), 16
  connections 344588.82 against 335309.11, all runs overlapping; the small
  forms of `strings` +0.5 to +4.8 % against the runtime of D374 (`concat`
  6.251 -> 6.548 ns) where D371 alone is -3.3 to +0.7 %: a loop that takes
  and gives back one block fetches a line it does not use, at every call.
  Not kept.
- **D376 — The vectors of a wait of the server kept from one wait to the
  next (2026-11-06).** The cost of a request of `serve` on 2 cores with 16
  connections (`perf stat -p` over 5 s against the requests of wrk):
  18534 cycles, 12205 of them in the kernel and 6329 in the program (11515
  instructions); the ceiling in C 13113, 12425 and 688 (570 instructions).
  The kernel the same: the same system calls a request for both (`strace
  -c` over a load of 4 s: 249157 `sendto` and 249170 `recvfrom` for 249151
  requests, one `epoll_wait` for 5.9 of them; the ceiling 184470, 184482
  and one for 6.0, for 184465 requests); all the gap in the program. The
  allocations counted (a counter of each size in `isc_alloc`, not kept):
  about 49 a request (117.4 M for 2405614 requests), 35 of them of at most
  64 bytes, and at each turn of the loop (588132 turns, 4.1 requests each)
  a block of 64 KiB then one of 128 KiB: the vector of the bytes of
  `isc_sys_serverwait`, which reserved 64 KiB for the first connection
  ready, then grew by a copy, then was freed.
  - Now the three vectors of a wait (numbers, lengths, bytes) are kept in
    the state of the server with their room, cleared at each wait (the
    bytes not kept past 1 MiB of room: a large body read once), under
    Linux and Windows.
  - The server on 2 cores, 3 alternated runs of 10 s: 16 connections
    340013.07 -> 378100.35 requests a second (+11.2 %, medians; 335927 to
    344731 against 366973 to 381463), 1024 connections 345238.01 ->
    352274.40 (+2.0 %, the runs overlapping). The pipelined forms the same
    (`serve.pipelined` -0.3 %, `serveAction.pipelined` -0.2 %: hundreds of
    requests a turn there).
  - Tested by the tests of the server and of TLS (`std_http.rs`,
    `std_https.rs`, `std_net.rs`), whose waits read many sizes (bodies of
    1 MiB, responses of 2000000 bytes, a body over the limit).
  - Next, by the same counts: `readyOf` (30.1 % of the calls of
    `isc_alloc` in the LBR stacks, 24 of them in its code), the task of the
    responses (15.2 %, 40), the rest of `serveLoop` (46.5 %).
- **D377 — The arrays of the requests ready of a connection kept as they
  are when all are taken (2026-11-06).** `readyOf` returned the slices
  `0 .. take` of its four arrays; a slice of an array of records (`frames`,
  `heads`) is a new array (`slice_is_view`: views only for arrays of
  scalars), the arrays of the lines of each head copied with it. When all
  the requests are taken (the common case) the arrays are now returned as
  they are. The allocations counted (a counter in `isc_alloc`, not kept),
  `serve` on 2 threads, 16 connections, one load of 6 s each: 49.96 ->
  42.69 a request (125370368 for 2509619 requests, 116391936 for
  2726632). Requests a second at fixed frequency, 3 alternated runs of
  10 s: 16 connections 380782.29 -> 373927.01 (-1.8 %, the runs
  overlapping: 370107 to 381545 against 370937 to 379388), 1024
  connections 348906.57 -> 353523.06 (+1.3 %): 7 allocations fewer of
  about 50 are within the noise of this measure, about 1 % of the cost of
  a request (18534 cycles, D376). Tested by the tests of the server
  (`std_http.rs`: pipelines cut by a body, by `Connection: close`, faults).
- **Rejected after measurement: the loops of a task of `mapParallel` run
  at once when no thread of the pool is free (2026-11-06).** In the
  profile of the user cycles of `serve` (2 threads, 16 connections),
  `isc_parallel_for_site` and `parallel_job` took 9.70 and 1.75 % of the
  samples: the maps of `serveLoop` given to the pool, where no thread is
  free (each runs its loop of events), through the counts of their place
  (two `rdtsc`, stores into words that the two loops share) and a slot of
  the nested loops. A test (`ACTIONS`, `BUSY`, `JOB.finished`) ran them at
  once instead: `isc_parallel_for_site` 1.46 % of the samples after, the
  other functions larger by as much; the server, 3 alternated runs of 10
  s: 16 connections 384464.60 -> 383414.16 (-0.3 %), 1024 connections
  361042.48 -> 361912.14 (+0.2 %), the runs overlapping; the user cycles
  a request 4630 and 5663 before, 4620 and 4236 after (two loads each,
  noisy), its instructions 10801 and 10680 against 11377 and 10584. No
  gain shown: not kept. That the 9.70 % were samples of the work around
  these calls (skid) rather than their own cost is a hypothesis.
- **D378 — The gap of `padStart` to its floor shown by measure
  (2026-11-06).** `padStart` of `strings` is 4.9 times its floor (9.711
  against 1.963 ns, `results/primitives.md`): `l.slice(0, 10).padStart(20,
  "0").length` summed over 4096 texts. Alone in a program (one thread, at
  fixed frequency, 2000 rounds): 31.3 user cycles and 152.5 instructions
  a call (`perf stat`); by function (`perf record`): `isc_pad` 50.76 %
  (15.9 cycles: the block of the result taken from the cache of the
  thread, the fill, the copy), `isc_free` 27.79 % (8.7 cycles: the block
  given back), the loop 20.59 % (6.4 cycles). The floor writes the 20
  bytes into a buffer (`take`, a bump of a pointer) and frees nothing. The
  gap is the block that a string value owns, made and freed at each call;
  it goes only if a string that lives inside one expression is placed
  elsewhere than in the heap (on the stack, by an analysis of where it
  escapes: TODO), a general change of the compiler, not of `padStart`.
- **D379 — No block for an allocation of 0 bytes (2026-11-06).** The
  allocations of `serve` counted by size (a counter in `isc_alloc`, not
  kept; 2 threads, 16 connections, 2373379 requests): 45.67 a request, of
  which 12.77 of 0 bytes (empty arrays and texts: an empty array made by a
  `map`, a `filter` or a slice, the bytes left of a connection after its
  requests), 14.26 of 1 to 16 bytes, 6.10 of 33 to 64, 12.07 above. Each
  took a block of 64 bytes and gave it back. Now `isc_alloc(0)` and
  `isc_alloc_zeroed(0)` give the empty block of the runtime (`EMPTY_BYTES`,
  read only, 64 bytes aligned on 64, the one of the empty strings of
  `new_str`, D337), which `isc_free` recognizes first and `room` gives no
  room in; every free of a block goes through `isc_free` (the generated
  code never touches the caches of the heap).
  - The server on 2 cores, 3 alternated runs of 10 s: 1024 connections
    354310.59 -> 411264.45 requests a second (+16.1 %, medians; 350764 to
    367420 against 409756 to 412953), 16 connections 382278.92 ->
    384972.39 (+0.7 %, the runs overlapping). That the gain at 1024
    connections comes from the empty bytes left of each connection kept
    in its table (up to 1024 blocks of 64 bytes alive, now one block for
    all) is a hypothesis.
  - The ten programs (7 alternated runs): the same outputs, minimums -3.71
    % (fasta, runs of 5 ms) to +1.19 % (n-body; 11 runs again: +0.45 %,
    medians +0.02 %, the code of its program the same bytes). The forms of
    `json`, `http`, `strings` and `sys` (9 alternated runs): `env` -3.5 %,
    `parseI64.7` -3.3 %, `serveAction.pipelined` -3.0 %, the others within
    -1.7 to +0.5 % but `lines.line` +2.3, +2.4 and +1.9 % in three series;
    alone in a program 14.585 -> 14.707 ns (+0.8 %) with the same
    instructions (995.59 against 994.78 M) and 1.6 % more cycles, the uops
    of the decoders and the DSB the same: the place of the code again
    (D371), not more work.
  - Tested by the whole suite (its runs under Valgrind among them: a write
    into a block of 0 bytes would now be a fault in read-only memory).
- **D380 — The runtimes embedded in `iscc` kept as they are, measured
  (2026-11-06).** TODO.md asked for a smaller `iscc`, its runtimes
  compressed. `iscc` is 66895400 bytes, its six runtimes 43348588 of them
  (8193344, 8426600, 8451420 for x86-64 and its levels v3 and v4, 5657162
  and 5711684 for WebAssembly, 6908378 for Windows); compressed by `gzip
  -6`, 10721556 bytes. But each build writes the runtime it links next to
  the object file: inflating it first costs 31 to 34 ms with `gzip -d` for
  the 8193344 bytes of x86-64, where a whole build of a small program
  takes 38 to 49 ms (239 ms the first time). The archives of the releases
  are compressed already (`tar.gz`, `zip`): the download gains little.
  Not done: the size of the installed `iscc` is not worth the time of every
  build; an inflated copy kept between builds would remove that time, with
  a cache to keep valid.
- **D381 — The Benchmarks Game measured again in Docker (2026-11-06).**
  After the changes of the runtime of D371 to D380 (the batches of blocks,
  the cache at a fixed place of its page, the print lock, the vectors of a
  wait, no block for 0 bytes), the publication again at f3e1ef8: the image
  `isocron-bg` rebuilt with this `iscc` (`image.sh`), every program built
  and checked (`run.sh build`, `run.sh check`: 98 of their outputs the
  reference's; `mandelbrot.gpp-4` differs from it at N=200 and is
  Isocron's at N=16000, as before), then measured at fixed frequency
  (`run.sh measure 4 12 native`, the lowest of 12 runs). Isocron the
  fastest on 10 of 10, its times 664 (677 before), 1659 (1666), 5 (5), 676
  (684), 637 (638), 1887 (1906), 490 (491), 82 (83), 40 (41) and 195 (196)
  ms for binary-trees, fannkuch-redux, fasta, k-nucleotide, mandelbrot,
  n-body, pidigits, regex-redux, reverse-complement and spectral-norm.
  `results/floors.md` generated again: every program below three times its
  floor, at most regex-redux 2.8, k-nucleotide 2.6 -> 2.5;
  `results/benchmarks_game.md`, the summary of the README and its figure
  generated again (`report.py`).
- **D382 — An iscc on Windows links the runtime of the level of its
  processor (2026-11-06).** TODO.md asked for the runtimes of x86-64-v3 and
  v4 under Windows. `build.rs` builds the runtimes of the target of `iscc`
  at the base level and for x86-64-v3 and v4 (D188, D241); for `iscc.exe`
  that target is Windows (D247), so those three are runtimes of Windows
  already. But a build for Windows took `RUNTIME_WINDOWS` (the base level,
  built for the option `--target x86_64-windows` of an iscc of Linux),
  `Options::windows` being `cfg!(windows)`: an iscc on Windows never linked
  its own v3 or v4. Now an iscc that runs on Windows chooses among its own
  runtimes by its processor, as under Linux (`runtime_v4`, `runtime_v3`);
  `RUNTIME_WINDOWS` stays for an iscc of Linux that builds for Windows.
  Measured in the virtual machine of Windows (4 threads, 7 alternated
  runs, `Measure-Command`): regex-redux built by the `iscc.exe` before and
  after, the same output, 158.0 -> 113.4 ms (minimums; medians 168.2 ->
  122.6). Tested by the tests of Windows (`windows.rs`, 11 of 11, among them
  `iscc_on_windows`: programs built by `iscc.exe` in the machine).
- **D383 — The memory report names the arrays made by parts
  (2026-11-06).** TODO.md: the report counted as allocated whole the arrays
  that the code generator makes by blocks (D225: an array of bytes made only
  for `printBytes`, by blocks of 1 MiB) and by windows (D293: a `filter`
  read only by the blocks of an array written by blocks). Their recognition
  (`print_stream_ok`, `windowed_filter` and their helpers: `streamable`,
  `stream_map_ok`, `windowed_uses`, `windowed_stream`, `windowed_lambda`,
  `interval_ok`, `inner_loop`, `free_vars`, `mentions`) moves from the code
  generator to `isc_ir::blocks`, unchanged, so that both use the same: the
  binaries of reverse-complement and regex-redux the same bytes after the
  move. The report gives the value of such a `let`, outside any loop (where
  the code generator takes these paths), the context `Blocks` or `Windows`,
  and reports the array as `blocks` or `windows`, without bytes, instead of
  `alloc`. reverse-complement: its lines (`blocks`) and its bases
  (`windows`). Tested by `arrays_made_by_parts_are_reported_so`
  (`report.rs`, on reverse-complement); SPEC section 8.
- **D384 — The rest of regex-redux left as it is (2026-11-06).** TODO.md
  kept open the engine of shapes and the replacements of regex-redux. In the
  publication of D381 regex-redux takes 82 ms: 2.8 times its floor of
  29.7 ms (under the 3 of the goal), 0.11 of the fastest of the other
  languages (Rust, 753 ms). The rest measured (2a59c94): the vector of the
  bounds of the matches of the third replacement, kept as starts of 4 bytes
  instead of pairs of 8, about 2.5 ms in a program in C alone, about 3 % of
  regex-redux; it would change the bounds of every match through the engine
  (`Found`, `merge_runs_with`, `Run`, the writers of the replacements). Not
  made: the author judged it not worth it (2026-11-06), the effort going to
  the server (D374 onward).
- **D385 — `serveAction` with a loop on each thread, its responses
  written in one pass (2026-11-06).** `serveAction` (a handler that is an
  action, D352) answered at 94.7, 55.0, 55.8 and 52.7 % of nginx with 1,
  16, 256 and 1024 connections (results/http_servers.md, 2 cores), on
  1.07 to 1.30 of its 2 cores: one loop, one request at a time. The author:
  « tu peux passer là-dessus en priorité », then « idéalement il faudrait
  arriver au moins au même niveau de performance que serve ».
  - The language: under Linux `serveAction(port, handler)` runs, as `serve`
    does (D374), a loop on each thread of the pool, each on a listener of
    its own of the port, by `mapParallel`. Each loop answers its requests
    one at a time, in their order; the requests of one connection stay in
    their order (a connection belongs to one loop). Two loops run at the
    same time, the effects of their requests in no order between them: the
    rule of the actions of `mapParallel` (D373). Chosen over a new name for
    the parallel form: `serve` changed the same way in D374, and the order
    of all the requests stays where it is asked for, `serveActionOn(l,
    handler, limit)` (one loop, as `serveOn`), for a handler that reads a
    file then writes it again. One loop under Windows, with one thread,
    for the port 0, and for the servers of TLS. SPEC sections 12 and 18,
    the guide.
  - The loop: `respondFrom` wrote each `Response` into an array filled
    first with `textResponse(500, "")`, after an array of the places of the
    requests (`items`), and a second `map` made their bytes; it now writes
    the `Out` of each request (`responseBytes`) into an array of `Out`s,
    as the `map` of `serveLoop` does, in one pass.
  - Measured (the server on 2 cores, wrk on 6, `ISC_THREADS=2`, 3
    alternated runs of 10 s, medians, requests a second, 16 / 256 / 1024
    connections): the loops 199602 -> 368261 (+84.5 %), 215475 -> 410494
    (+90.5 %), 188792 -> 385182 (+104.0 %), then `serve` 376703, 446502,
    422177 (97.8, 91.9, 91.2 %); the one pass 364589 -> 380690 (+4.4 %),
    416267 -> 437783 (+5.2 %), 385768 -> 408195 (+5.8 %), `serve` 381057,
    440508, 414704 (99.9, 99.4, 98.4 %; the runs overlap with 16 and 256).
    With 1024 connections, four runs more (under `perf`): `serveAction`
    419232, 413859, 408149, 413348, `serve` 413371, 421346, 405381,
    408537; the means of the seven 99.4 %. A request (`perf stat`, 5 s in
    a run, two runs each): user instructions 10915 and 10934 against 10507
    and 10461 (about 430 more, not attributed: the two forms are compiled
    into other functions, the handler of `serve` inlined into its task),
    user cycles 4554 and 4466 against 4331 and 4452, kernel instructions
    13863 and 13954 against 14008 and 13852.
  - Tests: `serve_action_runs_a_loop_on_each_thread` (`std_http.rs`, the
    test of `serve`'s loops made a function of the program: 4 listeners
    with 4 threads, 1 interpreted, a second server refused, 64
    connections of 50 requests each answered in their order, the handler
    an action that prints); `server_with_a_handler_that_is_an_action`
    unchanged.
- **D386 — One `serve`, for a pure function or an action: the purity of
  a lambda inferred, `isPure` (2026-11-06).** After D385 `serveAction`
  answered as fast as `serve`; the author asked what `serve` was still for,
  then: « Une seule fonction serve qui accepte les deux, et qui garde le
  chemin parallèle quand le gestionnaire est pur, serait l'idéal : peux tu
  partir là-dessus ? et d'ailleurs si d'autres items du langage sont dans
  le même cas, en faire de même pour eux ». The pairs of the language: the
  servers (`serve` / `serveAction`, `serveOn` / `serveActionOn`,
  `serveServer` / `serveActionServer`, `serveTls` / `serveTlsAction`,
  `serveTlsOn` / `serveTlsActionOn`) and `map` / `mapAction` (D387).
  - The language: (1) a lambda written where an action is expected is an
    action only when its body has an effect (the call of an action, of a
    value of an action type or of a primitive of effects, the reference to
    an action as a value, an expression statement); without one its type is
    that of a pure function. The checker keeps for each enclosing lambda
    that may act whether it had one (`act_effects`, `note_effect` where each
    effect is allowed). The name of a declared `action` is an action, its
    body with an effect or not. (2) `isPure(f)`: whether the function `f` is
    pure, a constant of each call compiled, since each call is specialized
    for the function it receives (no closure at run time): a `function`, a
    lambda without an effect, an operator, a primitive that is not an
    action, a choice or a table of such functions (`fn_pure` of the
    lowering). (3) In the first branch of `if (isPure(h))`, `h` a parameter
    or a local of an action type, `h` has the type of the pure function
    (`pure_test` of the checker): it can be given to `map`. The lowering
    compiles only the branch of the function received. Chosen over an
    overloading by the purity of an argument (two declarations of one name)
    and over a polymorphism of effects in the types: one test, checked
    once, in the code of the library, and nothing new in the types.
  - The library: `serve`, `serveOn`, `serveServer` of `http` and
    `serveTls`, `serveTlsOn` of `https` take a handler `action (Request) =>
    Response`; `serveServer` answers by `serveLoop` (the `map` in parallel,
    no data race by construction) when `isPure(handler)`, else by
    `serveActionLoop` (one request at a time, D352, D385). `serveAction`,
    `serveActionOn`, `serveActionServer`, `serveTlsAction` and
    `serveTlsActionOn` are removed (0.x: no promise of compatibility yet);
    the programs that called them call `serve`, `serveOn`, `serveTls`,
    `serveTlsOn`. The message of an effect in a `function` names
    `http.serve` given an action.
  - The code: the server of `bench/http_servers/hello.isc` built by the
    `iscc` of ac32ac6 (`serveAction` given a declared action, `serve` a
    function) and by the new one (`serve` given each): the same 125
    functions of the program at the same addresses and sizes; their code
    the same but for four constants that are places of the sources in the
    messages of panics (lines of `http.isc`, columns of the program) and
    the name of a function of the runtime (the old `iscc` a debug build):
    the measures of D385 stand.
  - Tests (`actions.rs`): `the_purity_of_the_function_received`
    (`isPure` of a `function`, of a declared action without an effect, of
    an action, of lambdas with and without effects, of a choice; the first
    branch given to `map`, its results 1000 more to show the way; the
    interpreter and compiled on 1 and 4 threads),
    `the_purity_test_refused`, `serve_takes_the_way_of_its_handler` (the
    functions of the IR: `serveLoop` and no `serveActionLoop` for a
    function and a lambda without an effect, the reverse for an action and
    a lambda that calls one); the tests of `std_http.rs` and `std_https.rs`
    on the new names; an example in `docs/language.md`; SPEC sections 12
    and 18.
- **D387 — `map` for a pure function or an action; a lambda that reads a
  function of its context given to a recursive function (2026-11-06).**
  The other pair of D386: `map` (pure, in parallel) and `mapAction` (an
  action, in order, D352).
  - The language: in an action, the function of `map(x, f)` may be an
    action (a lambda written there checked as one may act, its purity
    inferred as in D386): applied to each element in order, once each, the
    result of the size of `x`; a pure function, or a lambda without an
    effect, applied in parallel as before. `fold`, `filter` and the other
    functions of arrays stay pure. `mapAction` is removed from the
    language: its code stays in the prelude as `mapInOrder`, which the
    lowering calls for a `map` of an action (`fn_pure` false), not
    documented. The message of an effect in a pure lambda names `fold`,
    `filter` and the others, `map` and `mapParallel` aside.
  - The lowering: `map(x, (v) => h(v) * 2)`, `h` an action given to the
    action that calls it, reached the limit of the POC « a recursive
    function given a function that reads a function of its context » (TODO,
    D352: `mapInOrder` is recursive). A function of the context of a lambda
    is known when the program is compiled (defunctionalization, D107): the
    key of the specialization (`FnKey::Lambda`) now holds the keys of the
    functions the lambda reads, two such lambdas two specializations; a
    function read that reads values or sizes of its own context is still
    refused (they would not be in the specialized function).
  - The code: the ten programs of the Benchmarks Game built by the `iscc`
    of 785b2cf and by the new one: the code of their functions the same
    (`objdump`, the addresses of the runtime set aside: no line differs),
    `.text` and `.rodata` of the same sizes.
  - Tests: `map_takes_an_action` (`actions.rs`: an action by its name, a
    lambda that prints, a lambda that calls the action it is given, an
    empty array, maps inside maps, a sum of 100000 in parallel, `apply` of
    a lambda that reads `g` for two `g`; the interpreter and compiled on 1
    and 4 threads); the IR test of D152 (`ir.rs`: two specializations of
    `apply`, one per function read; a function read that reads a value of
    its context refused); the refusals of `actions.rs` moved from `map` to
    `fold` and `filter`; `std_env_names.isc` on `map`; an example of the
    guide; SPEC section 12.
- **D388 — A row of bytes that reads a table in registers in one vector
  of 64 lanes (2026-11-06).** TODO.md: the lines of reverse-complement
  about 19 ms against about 6, « one byte at a time ». Profile (4 threads,
  fixed frequency, 10 runs of `perf record`, the input of 254166745
  bytes): the lines are vector code already, by rows of 61 (D214, D318),
  two vectors of 32 lanes a row, each with its reversed masked load
  (`vpermb`), the table of 256 bytes permuted by four `vpermi2b` of 32
  lanes and three selections, its masks and its reloads from the stack
  (the « one byte at a time » of TODO was wrong). `isc_task_3` (the bases
  of each block and the lines) 73.39 % of the 874 million user
  instructions and 61.08 % of the user cycles, `isc_task_4` (the count of
  the heads, 16 bytes a step) 18.35 and 17.61 %, `isc_task_1` (the count of
  the bases, D293) 7.35 and 15.50 %; those two wait for memory (78.6 % and
  52.2 % of their cycles on the instruction after their load), at about
  the speed of the floor; the times of the samples: the heads from 0 to
  6.6 ms, the records from 6.9 to about 37 ms. The rows of 33 to 64 bytes
  in one vector of 64 lanes (D274) left out the `map`s that read a table
  in registers (D198), whose permutations were of 32 lanes at most.
  - The code: `permute_bytes` for 64 lanes, the table in 4 parts of 64
    bytes (its 8 registers of 32 joined, out of the loop), two
    `vpermi2var.qi.512` of 128 entries (bits 0 to 6 of each index) and the
    selection of bit 7; the read of a table on 64 lanes only for a table in
    registers; `byte_window` (32 lanes at most) a gather beyond; the rows
    of D274 with tables. A first version read any array of bytes on 64
    lanes through the test of 256 bytes and the permutations: fasta +4.0 %
    instructions (143.7 -> 149.4 million), +1.10 and +0.88 % (minimums of
    41 alternated runs, the old against itself +0.00 %); with the read
    for the tables in registers only, fasta's code is the old one. A rule
    tried on the way (no 64 lanes for rows that compute floats, from the
    hypothesis that fasta's draws spilled) changed nothing: removed.
  - Measured (4 threads, fixed frequency, 11 alternated runs, the same
    outputs): reverse-complement 40.0 -> 31.9 ms (-20.14 %, medians
    -20.67 %), 873.7 -> 582.6 million user instructions, 458.4 -> 350.9
    million user cycles, 157.35 -> 123.66 ms of CPU: about 1.9 times its
    floor of 16.832 ms. The code of the other nine programs the same
    (`objdump`, the addresses of the runtime set aside).
  - Test `rows_that_read_a_table_in_registers_take_64_lanes`
    (`vector_index.rs`: rows of 61 downwards and of 33 upwards, the
    source's bytes from 0 to 255 through the four parts and bit 7, tables
    of 256 and 200 bytes, lengths cut in a row, `vpermi2var.qi.512` in
    the IR on a host with AVX-512 VBMI; the interpreter's results, 1 and 4
    threads).
- **D389 — The quotient of a multiply-add of bigints from the leading
  words (2026-11-06).** TODO.md: pidigits made `num * 3 + acc` and `num * 4
  + acc` whole only to divide them by `den`, a quotient of one digit; the
  floor (bench/floor/floors.py) takes it from the leading words. Profile
  (one CPU, fixed frequency, 10000 digits): 490 ms, 1.56 G user cycles,
  4.61 G instructions; `limbs::mul_word` 58.82 % of the cycles,
  `big256::op` 26.10 % (the division of Knuth inlined: both operands
  copied and shifted, then multiplied and subtracted), `limbs::sub` 10.57
  %.
  - The runtime: `muladd_quot(a, k, b, c)` (isc-runtime-core) for all of
    them non-negative, `k` (or `a`) of one word, `c` of more than 128 bits,
    `a` and `b` of at most 56 bits more than `c`: each cut at the bit `e =
    bits(c) - 64` (`a = A 2^e + a'` and the same for `b` and `c`, C from
    2^63 to 2^64), so that `(A k + B) 2^e <= a k + b < (A k + B + k + 1)
    2^e` and `C 2^e <= c < (C + 1) 2^e`: the quotient lies from `(A k + B)
    / (C + 1)` to `(A k + B + k) / C`, two divisions on 128 bits; when they
    are the same it is the quotient, without a pass over the words.
    Otherwise, and outside these conditions, `muladd_div` makes the
    product, the sum and the quotient whole (`isc_big_muladd_div`): the
    same values and panics as before. A first version took `a` and `b` of
    at most the bits of `c` only: pidigits -5.49 %, its `num` most of the
    time longer than `den` (the sum up to ten times `den`); the windows of
    120 bits then.
  - The compiler: `let t = (let p = x * y in p + z) in t / c` of bigints
    (the temporaries of the lowering, each read once there, no operand
    moved), `z + p` too, is one call (`muladd_div` of the code generator),
    its operands in the same order, the panic at the place of the
    division. The interpreter unchanged (its IR the same).
  - Measured (one CPU, fixed frequency, 11 alternated runs, the same
    outputs for 1, 27, 1000 and 10000 digits): pidigits 490.3 -> 337.8 ms
    (-31.10 %, medians -31.17 %), 4606 -> 3220 million instructions, 1557
    -> 1071 million user cycles: about 1.7 times its floor of 0.194 s. The
    code of the other nine programs the same.
  - Test `big_quotients_of_a_multiply_add` (`stdlib.rs`: numbers of 1 to
    40 words, `k` first or second, `b` first or second, sums that are exact
    multiples of `c` and one less, negative operands, a `k` of two words,
    small numbers; ten calls in the code for its ten quotients; the
    interpreter's results; a division by zero: the panic of the
    interpreter and of the code before, at line 3, column 24).
- **D390 — Floors closer to the machine: small tables of k-mers, trees in
  the L3 cache, the next permutation without a branch, the flips by eight
  chains (2026-11-06).** TODO.md and the warning of `results/floors.md`:
  some primitives of `bench/floor/floor.c` were still above what the
  machine allows (D222). `floor.c` now measures, `floors.py` uses (4
  threads, fixed frequency, the third of three runs of `run.sh`):
  - the counts of k-mers in tables of 4, 16, 64 and 256 counters (k = 1 to
    4, in the L1 cache), as for 4096: 0.786, 0.692, 0.665 and 0.643 ns an
    update against 0.657: no cheaper (the 4 counters of the 1-mers
    increased one after the other); each frame counted at the cost of its
    table;
  - trees of depth 16 and 18 (in the L3 cache) besides 12 and 20: by a loop
    and an explicit stack 1.598 and 3.331 ns a node against 4.451 at 20;
    each depth of binary-trees at the cost measured at it or at the next
    one above;
  - the next permutation of fannkuch-redux without a branch per level: the
    counter in factorial base in one vector, the level of its carry by one
    comparison with the radixes, the digits below it cleared and the next
    one increased by tables, the rotations `rot[1] .. rot[level]` one
    shuffle composed beforehand (checked against the counter with a branch
    on each of the 479001599 permutations of 12: the same levels and
    permutations): 1.969 -> 1.447 ns a permutation;
  - the flips with each of the 8 vectors taking its next permutations from
    a part of its own of the pool (8 chains of the place in the pool
    instead of one through the vectors): 0.691 -> 0.673 ns a flip.
  The file read by blocks in the caches measured 34.2 and 35.7 GB/s in the
  runs of the day (30.2 on 2026-11-05). Floors: binary-trees 0.3947 ->
  0.3286 s, fannkuch-redux 0.8835 -> 0.8057, reverse-complement 0.01683 ->
  0.01424, k-nucleotide 0.2680 -> 0.2725 (the 1-mers dearer), the six
  others within 1 %. The warning of `floors.md` gone with the next
  publication (D391).
- **D391 — The Benchmarks Game measured again in Docker after D388 to
  D390 (2026-11-06).** At c6614dc, `image.sh`, `run.sh build`, `check`
  and `measure 4 12 native` at fixed frequency (the protocol of D381):
  Isocron the fastest on 10 of 10. Its times (ms, the lowest of 12 runs):
  binary-trees 664, fannkuch-redux 1660, fasta 5, k-nucleotide 678,
  mandelbrot 637, n-body 1882, pidigits 338 (490 at D381: D389),
  regex-redux 82, reverse-complement 31 (40: D388), spectral-norm 195; the
  fastest of the others rust-5 841, gcc-6 2462, rust-7 754, gpp-2 1161,
  gpp-4 782, gcc-9 1944, gcc-2 680, rust-7 754, gcc-7 62, rust-6 204.
  Against the floors of D390 (`results/floors.md` regenerated): regex-redux
  2.79, k-nucleotide 2.49, fasta 2.19, reverse-complement 2.18 (2.4 at
  D381), fannkuch-redux 2.06 (1.9 against its floor before D390),
  binary-trees 2.02 (1.7 before D390), spectral-norm 1.82, pidigits 1.73
  (2.5), mandelbrot 1.46, n-body 1.43: each below 3.
  `results/benchmarks_game.md`, the summary of the README and its chart
  regenerated (`report.py`).
- **D392 — A microbenchmark and a floor for every function of `sys`,
  `path` and `time` (2026-11-06).** TODO.md (audit 6): « a microbenchmark
  and a floor for each primitive of the library »; 28 forms measured for
  about 91 exported functions. Added to `bench/primitives` (each in
  Isocron and the same work in C, the checksums equal):
  - `time`: `isLeapYear`, `daysInMonth`, `weekday`, `addDays`,
    `addMonths`, `daysBetween`, `toInstant`, `formatDateTime`, `format`, on
    4096 dates, times and years;
  - `path`: `isAbsolute`, `join`, `dirname`, `extension` (one path in three
    absolute);
  - `sys`: `ioError.message` (the 8 kinds), `envAll`, `cwd`, `isFile`,
    `isDir`, `fileSize`, `modified`, `makeDir.remove`,
    `makeDirs.removeAll`, `rename.2`, `readText.4k`, `writeText.4k`,
    `appendFile.64`, `appendText.64`, by an action `loopOf` given the call
    to repeat (D387); the floor of `makeDirs` the cheapest way (the full
    path first, the parents when missing), of `removeAll` a walk of the
    folders; the C program starts and ends with a new folder (the files of
    its runs before were counted by `listDir`, now `listDir.66`).
  Two floors corrected on the way: `daysBetween`'s checksum 0 (a sum that
  cancels: each date paired with the one at half its index), `toInstant`
  beaten by Isocron (0.4: the C made the date again from the day; now
  given, as the `DateTime` holds it: 1.0). `run.py` keeps the rows of the
  groups not measured (measuring two groups wrote a page of those two
  alone). Measured (one CPU, fixed frequency, the medians of 5 runs of 200
  rounds): 75 forms; new gaps above 3: `time.format` 12.1 (eight
  `replaceAll` in a row), `sys.envAll` 7.0, `readText.4k` 3.6,
  `ioError.message` 3.4 (TODO); the others of `time` 1.0 to 1.8, the system
  calls 1.0 to 1.3.
- **D393 — Release 0.4.0 (2026-11-06).** The author: « quand tu
  l'estimeras nécessaire il faudra penser à faire puis pousser une release
  ». After 0.3.2: one `serve` and one `map` for a pure function or an
  action (D385 to D387), the loops of events on each core (D373, D374),
  reverse-complement and pidigits faster (D388, D389), the floors closer to
  the machine (D390), the Benchmarks Game measured again (D391: the
  fastest on 10 of 10, each program below three times its floor), the
  microbenchmarks of `sys`, `path` and `time` (D392). A version 0.4.0 (as
  Cargo counts): `mapAction`, `serveAction`, `serveActionOn`,
  `serveActionServer`, `serveTlsAction` and `serveTlsActionOn` removed.
  The archives as for 0.3.2 (`build_iscc.sh` for Linux and Windows in the
  images of the releases, from the sources of the commit of the release;
  `package.sh`; the extension of VS Code at 0.4.0), tried before they are
  published (`selftest_linux.sh`, `selftest_windows.py`, the example of the
  README by the `iscc` of the archive). `CHANGELOG.md`, the links of the
  README and of `tools/vscode/README.md` at 0.4.0.
- **D394 — The microbenchmarks of the pure functions of `json`, `sha256`
  and `http`; floors beaten made the cheapest known (2026-11-06).** After
  D392, as it: `json.message`, `json.quote` (`quote.plain`: nothing to
  escape; `quote.escaped`: a quote and a newline), `sha256.digestText.55`,
  and the pure functions of `http`: `parseUrl`, `headerOf`, `header`,
  `text.1k`, `textResponse`, `httpDate`, `message` (4096 of each, the
  checksums equal to those of C); `serveAction.pipelined` named
  `serve.action.pipelined` (D386). Floors that Isocron beat, so not the
  cheapest: `quote.plain` (0.7, the C byte by byte: now the 32 bytes of the
  text tested in one vector of AVX2 and written in one store, 1.65 ns),
  `sha256.hex.32` (0.6, a loop of the bytes: now two shuffles of the 16
  digits in AVX2, checked against the loop on 100000 random inputs, 2.63
  ns); `json.encode` still beaten (0.6: the floats of C slower than those of
  Isocron; TODO). Measured (one CPU, fixed frequency, 5 runs of 200
  rounds), ratios above 3: `http.header` 14.7, `json.quote.escaped` 14.1,
  `http.headerOf` 14.6, `http.httpDate` 11.1, `http.message` 10.3,
  `json.quote.plain` 9.4, `http.text.1k` 9.3, `http.textResponse` 8.4,
  `sha256.hex.32` 7.9, `http.parseUrl` 5.7, `json.message` 4.0 (TODO).
- **D395 — The names of headers compared byte by byte, the case of ASCII
  ignored (2026-11-07).** D394: `http.headerOf` and `header` 14.6 and 14.7
  times their floor (379 ns for 8 headers): each name made in lower case by
  `toLowerCase` to be compared, and the headers found put in an array. Now
  `sameName` compares the lengths then the bytes, each in lower case for
  ASCII, and a `fold` keeps the place of the last one found: no text, no
  array. SPEC said « sans casse »: the case of ASCII only now, as RFC 9110
  compares the names of fields (tokens of ASCII; the module already refuses
  to send a name that is not one, `tokenOk`): a name that is not ASCII is
  compared by its bytes (`été` no longer finds `Été`, as `toLowerCase` made
  it). A version that kept `toLowerCase` for the names that are not ASCII
  (a letter of Unicode in lower case may change the length, or be one of
  ASCII: the sign of Kelvin) scanned each name for them: 92 ns, not kept.
  The floor in C, `strcasecmp`, was beaten: now the lengths then the bytes
  too. Measured (one CPU, fixed frequency, 7 alternated runs of 200
  rounds): `headerOf` 378.77 -> 19.05 ns, `header` 379.57 -> 20.62 ns;
  against the floor 1.6 and 1.7. Test `header_of_by_bytes` (`std_http.rs`:
  the last of a name, mixed cases, a name missing, empty, a prefix, longer,
  not ASCII; interpreted and compiled); SPEC section 18, the guide.
- **D396 — A count of bytes by the masks of its condition (2026-11-07).**
  D394: `http.text.1k` 9.3 times its floor. Profile (one CPU, fixed
  frequency): the task of `count(b, (x) => x >= 0x80u8)`, the test of UTF-8
  of `text`, 69.06 % of the cycles: `count` of the prelude is `sum(map(x,
  (v) => p(v) ? 1 : 0))`, each byte widened to 64 bits (`vpmovzxbw`,
  `vpmovzxwq`, `vpaddq`), 32 a turn. `filter` already counts by the bits of
  its masks (`count_kept`, D195). Now `reduce_block` takes such a sum (of
  `i64`, from 0, by `+`, a `map` of one input whose body is `c ? 1 : 0`) of
  bytes to `count_kept`, the condition `c` its predicate, 32 bytes a vector.
  Tried on the way: `count` written `filter(x, p).length` in the prelude:
  fasta 5.0 -> 1578.0 ms (a `filter` in each lane of its lines, made for
  each element); counts of every width by the masks: `json.quote.escaped`
  +8.7 % (its counts over indices of texts of 16 to 19 bytes: the elements
  left one at a time); with a test at run time (4 vectors at least) the
  short counts paid it, `http.parseUrl` +14.1 %: bytes only, no test.
  Measured (one CPU, fixed frequency, 7 alternated runs of the old and new
  binaries of the groups `strings`, `sys`, `http`, `json`, 63 forms):
  `http.text.1k` 151.10 -> 89.91 ns (-40.5 %), no form slower by 3 % or
  more; the code of the ten programs of the Benchmarks Game the same. Test
  `counts_by_the_masks_of_their_condition` (`vector_index.rs`: bytes and
  `i64`, lengths 0, 31, 33, 1000, 300007, a condition that reads past its
  array, one on the index; the interpreter's results, 1 and 4 threads).
- **D397 — `json.quote` in one pass (2026-11-07).** D394: `quote.escaped`
  14.1 times its floor, `quote.plain` 9.4; `stringify` quotes every text
  and key by it. The text to escape went through six `replaceAll` over the
  whole text, then a count of the text for each of the 32 controls; the
  plain one through two concatenations. Now the bytes tested by masks
  (D396) and one template for a text without anything to escape; else
  `escapeFrom`, a tail recursion that appends to its owned accumulator
  (rule A, D351) each run of plain bytes and the escape of the byte after
  it (`escapeOf`: `\"`, `\\`, `\n`, `\r`, `\t`, `\u00XY` in lower case).
  The same texts: 2002 texts of every control, quotes, backslashes,
  Unicode, empty, quoted by the old and the new module, interpreted and
  compiled, the same bytes. Measured (one CPU, fixed frequency, 9
  alternated runs of the old and new binaries): `quote.escaped` 273.42 ->
  96.29 ns (-64.8 %), `quote.plain` 15.38 -> 13.93 ns (-9.5 %), the other
  forms of `json` within 1.7 %; the code of the ten programs the same.
  Test `json_quotes_in_one_pass` (`stdlib.rs`).
- **D398 — `time.format` in one pass (2026-11-07).** D392: 12.1 times its
  floor, eight `replaceAll` of the whole pattern in a row. Now
  `formatFrom`, one pass from left to right appending to its owned
  accumulator (rule A) each run of the pattern and the text of the field
  after it (`fieldAt`: YYYY, MM, DD, HH, mm, ss, SSS, Z, tried in that
  order). The same texts as before: the texts put in are digits and the
  zone comes last, so that no replacement made a field, and no two fields
  overlap (`MMM` 01M, `ZZ` two zones): 500 times of 19 patterns the same
  bytes, interpreted and compiled. A test of the first byte before the
  fields tried: 294.40 against 294.76 ns, not kept. Measured (one CPU,
  fixed frequency, 7 alternated runs): `format` 455.16 -> 294.76 ns (-35.2
  %), the other forms of `time` within 1.2 %; the code of the ten programs
  the same. Left (8.1 times its floor of 36.5 ns, profile of `format`
  alone): the loop of the pattern 38.55 % of the cycles, the 8 texts of the
  padded fields made (`isc_pad_int` 13.62 %) and freed (`isc_free` 12.68
  %), the appends (`append_parts` 13.36 %, `memcpy` 8.51 %), where the floor
  writes its digits into one buffer. Test `time_formats_in_one_pass`
  (`stdlib.rs`).
- **D399 — `stringOf` takes the block of bytes at their last use
  (2026-11-07).** D394: `sha256.hex.32` 7.9 times its floor. Profile (one
  CPU, fixed frequency): `stringOf` of the new array of 64 digits checks it
  as UTF-8 (`from_utf8` 9.66 %), copies it into a new block
  (`isc_str_from_bytes` 12.50 %, `memcpy` 3.41 %, `isc_alloc` 10.80 %), then
  the array is freed (`isc_free` 15.34 %). Now the ownership pass moves the
  bytes of `stringOf(b)` into it when `b` is owned, without an alias, at its
  last use (as the left text of a concatenation, D351), the verifier knows
  that `stringOf` takes them, and the compiled code calls
  `isc_str_adopt_bytes`: checked as UTF-8 (the same panic at the same
  place), then the text is their block, no copy. Bytes still read after it
  are copied as before. Measured (one CPU, fixed frequency, 7 alternated
  runs of the old and new binaries of `strings`, `sys`, `http`, `json`,
  `time`, `sha256`, 81 forms): `sha256.hex.32` 27.05 -> 20.93 ns (-22.6 %),
  no form slower by 3 % or more; k-nucleotide (its `name` takes its bytes)
  678.8 -> 680.8 ms (+0.29 %, medians +0.16 %, the same output), the code of
  the nine others the same. Test `string_of_takes_the_bytes_at_their_last_use`
  (`stdlib.rs`: new bytes in a recursion, an empty array, bytes read after
  it copied, bytes that are not UTF-8; 5 texts take their bytes, 1 copies
  them).
- **D400 — `envAll`: its entries measured by vectors, checked by
  `from_utf8`, copied once (2026-11-07).** D392: `sys.envAll` 7.0 times its
  floor. The floor did less than `envAll`: it did not sort the entries,
  which `sysEnviron` gives in the order of the bytes of their names; now it
  sorts them (`qsort`, `strcmp`), 586 -> 969 ns, against which `envAll` was
  4.1 times. Profile of `envAll` alone (one CPU, fixed frequency, 18
  entries of 2490 bytes, `LS_COLORS` 1916 of them): `from_utf8_lossy` 52.5 %
  of the cycles (its chunks byte by byte, for every entry), the length of
  each entry byte by byte 19.0 %, then each entry copied twice (a `Vec`,
  then the string). Now `sorted_strings` checks each entry by `from_utf8`
  (its ASCII by words; `from_utf8_lossy` only for an entry that is not
  UTF-8, the same replaced text) and copies it once into its string, the
  entries sorted as slices of the environment (`sort_unstable`: equal
  entries are the same bytes); the same for the names of `readDir`, under
  Linux, Windows and WASI. Then 51 % in the length of the entries: the
  `strlen` of `isc-mem`, byte by byte, now one aligned vector at a time (32
  bytes with AVX2, 16 otherwise), its load written in assembly as glibc and
  `compiler-builtins` do (the block may hold bytes outside the string,
  which Rust may not read; an aligned block lies within one page): every
  `strlen` of the runtime under Linux (the environment, the arguments).
  Measured: `envAll` alone 4569 -> 2787 (the check and one copy) -> 1354 ns
  (the length by vectors); the 81 forms of six groups alternated (7 runs):
  `envAll` 4634.34 -> 1881.88 ns (-59.4 %), `env` 22.24 -> 18.79 ns (-15.5
  %); against the floor (`run.py`, 5 runs): 1602.70 against 973.12 ns, 1.6
  times. One form slower, `http.parseUrl` 113.60 -> 122.18 ns (+7.6 %, 15
  alternated runs), by the place of the code, not by its work: the same
  program cut after `parseUrl` 82.07 and 82.12 ns, and the program of
  `parseUrl` alone 81.5 and 82.1; in the whole program, the task of
  `parseUrl` at the same address (0x5f510) and the functions of the
  runtime it calls moved by 0xC0 bytes (their size changed before them):
  the misses of the cache of decoded instructions doubled
  (`frontend_retired.dsb_miss` 7.31 -> 15.94 million over the whole
  program, `dsb2mite_switches.penalty_cycles` 23.2 -> 35.5 million), their
  samples in that task 18 -> 166, in `isc_free` 78 -> 163, in `isc_str_new`
  25 -> 99, in `memchr` 9 -> 80. Tests `the_length_of_strings_at_every_place`
  (`isc-mem`, SSE2, AVX2 and AVX-512 builds: every start in 64 bytes,
  every length to 300, the end of a page) and
  `all_the_environment_by_vectors` (`std_sys.rs`: 132 entries of every
  length to 130, a value that is not UTF-8, compiled and interpreted).
- **D401 — An array literal of string literals is data of the program
  (2026-11-07).** D394: `http.httpDate` 11.1 times its floor. Its code made,
  at each call, the arrays of the 7 days and the 12 months: two arrays and
  19 strings allocated, copied, then freed (`isc_free_strs`), as D173 found
  for the tables of scalars of n-body; a table of names in a function is a
  common form (days, months, units, the texts of a status). Now a literal
  whose elements are all string literals is `ConstStrs` in the IR: the
  bytes of its strings and the array of their places, global constants of
  the compiled program aligned as an array; borrowed from no variable (the
  ownership pass and the verifier give a copy to any consumer, each string
  copied, D154), never freed; read without an allocation
  (`@requireNoAlloc` accepts it, SPEC section 8). The interpreter makes the
  same array. Code of `httpDate`: 16 -> 12 allocations, 5 -> 1 frees of
  arrays of strings. Measured (one CPU, fixed frequency, 7 alternated runs
  of the old and new binaries of six groups, 81 forms): `http.httpDate`
  138.20 -> 98.81 ns (-28.5 %); k-nucleotide (its `QUERIES`) 676.9 -> 678.9
  ms (+0.30 %, medians -0.30 %) and regex-redux (its `VARIANTS`) 81.6 ->
  81.4 ms, the same outputs, the code of the eight others the same. Four
  forms moved by the place of the code, not by their work: in `strings`
  and `sys`, whose binaries are the same but for their build note (the
  same code, the same size), `startsWith.8` +6.8 %, `startsWith.1` +5.8 %,
  `envAll` +4.4 % (the scatter of the measure); in `http`, whose code
  changed (its tables), `textResponse` 20.13 -> 22.54 ns and `text.1k`
  85.76 -> 90.08 ns (11 alternated runs), while the programs of each alone,
  the same binaries with both compilers, give 22.59 and 22.59 ns, 87.03 and
  87.18 ns: the old place was the faster one, as `parseUrl` the other way
  (118.55 -> 109.54 ns, D400). Test `arrays_of_string_literals`
  (`stdlib.rs`: indexed, joined, concatenated, copied by `with`, `sort`, a
  `fold`, returned, a constant of the top level given to a function; 1 and
  4 threads, the interpreter, Valgrind).
- **D402 — `httpDate`: its fields of two digits written by their digits
  (2026-11-07).** After D401, `httpDate` still made a text for each of its
  four fields of two digits (`two`: `0${x}` or `${x}`), then freed them. Now
  `${x / 10}${x % 10}`: two integers of one digit, which the compiled code
  writes into the text itself (D359); the same text for the values of the
  fields (0 to 99). 4000 instants from 1678 to 2261 the same bytes,
  interpreted and compiled, old against new. Code: 12 -> 4 allocations.
  Measured: alone, 68.0 -> 59.6 ns; in the benchmark (one CPU, fixed
  frequency, 15 alternated runs) 99.08 -> 86.15 ns (-13.1 %); against its
  floor (`run.py`, 5 runs) 86.03 against 13.10 ns, 6.6 times (11.1 at
  D394). `text.1k` 90.06 -> 74.77 ns in the same runs, its code the same:
  the place of the code (D400, D401), which moves it from 74.8 to 90.1 ns
  between binaries (87.0 alone). Test `http_dates_by_their_digits`
  (`std_http.rs`: the dates of Python's `datetime` for the example of RFC
  9110, the epoch, an instant before it, a leap day, the first and last
  instants).
- **D403 — `time.format`: each field a part of a template after its
  text (2026-11-07).** After D398, the profile of `format` alone gave
  `isc_pad_int` 13.62 % and `isc_free` 12.68 % of the cycles: `fieldAt`
  returned the text of each field in a tuple, so each was made, appended,
  then freed. Now `fieldAt` gives the length of the field only (each field
  starts by a byte of its own, tried in the same order), and `withField`
  appends it to the owned text as `${acc}${pad(x, n)}`: `pad` is inlined,
  and `toString(x).padStart(n, "0")` as a part of a template is written by
  the compiled code into the text itself (D359), its sign as before. The
  same texts: 1000 times of 22 patterns, a fifth of them with fields out of
  their ranges and negative, old against new, interpreted and compiled.
  Measured (one CPU, fixed frequency, 7 alternated runs): `format` 299.72
  -> 203.34 ns (-32.2 %), the other forms of `time` within 1.2 %; against
  its floor (`run.py`, 5 runs) 202.39 against 36.44 ns, 5.6 times (12.1 at
  D392). Left (profile of `format` alone): the loop over the bytes of the
  pattern 45.96 %, the appends of the parts of the pattern between the
  fields (`append_parts` 19.60 %), the digits (`isc_put_int` 11.88 %), the
  text grown at each field (`isc_str_reserve` 8.79 %, `heap::usable` 4.99
  %), where the floor writes into one buffer of its size. Test
  `time_format_fields_as_parts` (`stdlib.rs`).
- **D404 — A text that its scope only reads, written on the stack
  (2026-11-07).** D378 and the audit (D392, D394): many forms of the
  library are a text made, read, then freed (`message(e).length`, a key, a
  line printed); profile of `http.message` alone (one CPU, fixed
  frequency): `isc_alloc` 33.68 % and `isc_free` 15.96 % of the cycles, the
  two loads of the list of the cache of the thread one after the other (the
  head of the list, then the header of the block: 80 and 92 of the 325
  samples of `isc_alloc`), where the floor writes into a buffer. Now a
  `let` of a text, freed at the end of its scope (`drop`) and never moved in
  it (`moves_var`: no consumer, no call that takes it, not returned), gets
  a buffer of 128 bytes on the stack (`alloca` in the entry of the
  function, aligned on 16: never 4 bytes past a multiple of 16, the place
  of a literal, D361); the concatenations whose text is its value (itself,
  the branches of an `if`, the body of a `let` or a `Free`: `result_concats`,
  not one given its left text, D351) write the text into that buffer when
  its length is at most 128 bytes, into a block of the heap otherwise; the
  value is freed only when it is not in its buffer (`drop_value`, by the
  value: the end of the scope, a `Free`, the frees before a tail call). The
  text dies with its scope, so the next time the `let` runs (a loop, a
  tail recursion) the buffer is free again; a recursion has a buffer in
  each frame. Measured (one CPU, fixed frequency, 7 alternated runs of the
  old and new binaries of six groups, 81 forms): `http.message` 13.05 ->
  6.49 ns (-50.3 %), `strings.concat` 6.20 -> 3.10 (-49.9 %), `sys.join`
  6.89 -> 4.40 (-36.2 %), `json.quote.plain` 13.90 -> 9.72 (-30.1 %),
  `json.message` 17.19 -> 13.09 (-23.9 %), `time.formatDateTime` 84.77 ->
  76.61 (-9.6 %), `http.httpDate` 86.02 -> 80.84 (-6.0 %), nine others -4.2
  to -5.6 %. Slower in that run: five forms of `strings` of 1.7 ns
  (`startsWith.8` +8.7 %, `startsWith.1`, `slice`, `endsWith.2`, `bytes`
  +6.5 to +7.6 %), `sys.envAll` +8.7 %, `strings.parseI64.7` +3.3 %. The
  program of `startsWith.8` and `endsWith.2` alone: their timed functions
  the same code at the same addresses in both binaries (`main` 64 bytes
  longer, the runtime after it moved by 64 bytes); 200 rounds (1.4 ms a
  form) +6.5 % and +6.9 %, 6000 rounds the same medians (1.72 and 1.72 ns,
  1.76 and 1.77 ns) and the same instructions (519922597 and 519922486):
  not their code. `envAll` alone 1341.33 -> 1354.65 ns (+1.0 %). The ten
  programs: binary-trees, fannkuch-redux, mandelbrot and regex-redux
  changed (the lines they print), the same outputs, binary-trees 663.8 ->
  663.5 ms (21 alternated runs, medians +0.20 %), fannkuch-redux +0.10 %,
  mandelbrot +0.03 %, regex-redux -0.13 % (11 runs); the six others the
  same code. Against the floors (`run.py`, 5 runs): `http.message` 5.6
  times (10.3 at D394), `json.quote.plain` 5.8 (9.4), `json.message` 3.0
  (4.0). Test `texts_on_the_stack` (`stdlib.rs`: texts of a `match`, of
  126 to 130 bytes and longer, an `if` of a new text and a borrowed one, a
  tail recursion, a recursion that keeps its text over the call below, the
  tasks of a `map`, a text read by lambdas in its scope, one returned; the
  interpreter, 1 and 4 threads, Valgrind with the allocator of glibc).
- **D405 — Release 0.4.1 (2026-11-07).** The author: « quand tu
  l'estimeras nécessaire il faudra penser à faire puis pousser une release
  ». After 0.4.0: the pure functions of `json`, `sha256` and `http`
  measured against their floors (D394), then their largest gaps reduced by
  general changes: names of headers compared by bytes (D395), counts of
  bytes by masks (D396), `json.quote` and `time.format` in one pass (D397,
  D398, D403), `stringOf` taking its bytes (D399), `envAll` and `strlen`
  by vectors (D400), arrays of string literals as data of the program
  (D401), `httpDate` by its digits (D402), a text that its scope only reads
  written on the stack (D404). No name removed, the language the same but
  `@requireNoAlloc`, which accepts an array of string literals (SPEC
  section 8): 0.4.1. The Windows tests in the VM before it (11 passed, the
  `iscc.exe` of the commit of D404). The archives as for 0.4.0
  (`build_iscc.sh` for Linux and Windows in the images of the releases,
  from the sources of the commit of the release; `package.sh`; the
  extension of VS Code at 0.4.1), tried before they are published
  (`selftest_linux.sh`, `selftest_windows.py`, the example of the README by
  the `iscc` of the archive). `CHANGELOG.md`, the links of the README and of
  `tools/vscode/README.md` at 0.4.1.
- **D406 — `z + x * y` and `z - x * y` of bigints in one pass with ADX
  (2026-11-07).** TODO: pidigits 337.8 ms against its floor of 194,
  `(acc + num * 2) * k2` and `(acc - den * d) * 10` three passes each
  (the product by a word, the sum, the product) where the floor counts
  two; one pass in Rust (a carry of 128 bits) was slower than the two
  passes in assembly (+11.97 %, D389). Now the compiled code recognizes a
  product bound to a temporary that only a sum or a difference reads
  (`muladd_parts`: `z + x * y`, `x * y + z`, `z - x * y`) and calls
  `isc_big_muladd`; when a factor is of one word and the processor has ADX
  (read once by `cpuid`: not part of the levels x86-64-v3 and v4), one pass
  in assembly, 4 words a turn: the product of each word by `mulx`, its high
  word added to the next low one by the chain of `adcx` (CF), the word of
  `z` added by the chain of `adox` (OF), the loop counted by `jrcxz` (no
  flag read or written); a difference adds the complement of one side and
  1 (OF set first), the larger magnitude chosen first from the lengths and
  the top words (`p` is between `x[top] * w` and that plus `w` in the words
  from the top of `x`), a difference below zero negated after only when
  those leave it undecided. Otherwise the product then the sum, as before
  (the same bytes: the values are canonical). The first version took `z -
  p` always: pidigits 337.2 -> 348.5 ms (+3.35 %, 11 alternated runs), its
  profile 474 of 3300 samples in the negation (with `acc` below zero, `acc
  + num * 2` is a difference of magnitudes, often below zero). Alone (one
  CPU, fixed frequency, a test of the runtime): 520 words 1.02 ns a word
  against 1.21 for the two passes, 64 words 1.46 against 1.50. pidigits
  (11 alternated runs, the same output): 337.2 -> 298.3 ms (-11.53 %,
  medians -11.00 %), 1.54 times its floor of 194; the code of the nine
  others the same. Tests `muladd_as_the_two_passes` (`isc-runtime-core`,
  built for x86-64-v3 and v4: 30000 cases of 0 to 330 bytes, words of all
  ones, every sign, `z` the product and one from it; more than 10000 of
  them in one pass) and `big_multiply_adds_in_one_pass` (`stdlib.rs`: the
  three forms, the interpreter's results).
- **D407 — The Benchmarks Game measured again at 6fae7ee (2026-11-07).**
  After D406 (pidigits 337.2 -> 298.3 ms alone): the image built with the
  `iscc` of 6fae7ee, the programs built and checked against the
  references (102 checks, the same outputs), then measured in Docker (4
  threads, 12 runs, the lowest, `bench/machine.sh apply parallel`). The
  fastest on 10 of 10: pidigits 338 -> 298 ms (C 681 ms, 0.44 of it, 1.5
  times its floor of 195.5 ms); the nine others within their scatter
  (binary-trees 664 -> 667 ms, reverse-complement 31 -> 32 ms, n-body 1882
  -> 1878 ms, the others the same to 1 ms). `results/benchmarks_game.md`,
  `benchmarks_game.tsv`, `floors.md`, `floors_isocron.tsv`, the table of
  the README and its figure made again from these lines.
- **D408 — The output to a Windows console in UTF-8 (2026-11-07).** TODO:
  « the console in UTF-8 ». A program for Windows wrote its UTF-8 by
  `WriteFile`; a console reads each byte as a character of its code page:
  in the VM (code page 437), `print("été 日本 ✓ ñ")` showed `├⌐t├⌐ µùÑµ£¼
  Γ£ô ├▒`. Now `write` of the standard output or error, when it is a
  console (`GetFileType` and `GetConsoleMode`, read once a stream), turns
  the UTF-8 into UTF-16 (`MultiByteToWideChar`) and writes it by
  `WriteConsoleW`; the bytes of a character cut at the end of a write (the
  buffer of `print` flushed when full, in the middle of a line) are held
  for the next one (`utf8_whole` in `isc-runtime-core`; the writes of a
  stream come one at a time); bytes that are not UTF-8 show as U+FFFD. A
  pipe or a file: the bytes as before. Checked in the VM, in a console of
  an interactive `ssh -tt` (a one-command session of OpenSSH loses the
  output of ConPTY: `cmd /c echo bonjour` showed nothing): the same program
  showed `├⌐t├⌐ µùÑµ£¼ Γ£ô ├▒` with the runtime before, `été 日本 ✓ ñ` with
  this one; a line of `x` and 40000 `é` (80001 bytes, the buffer of 64 KiB
  cut in a character) then `fin`: no U+FFFD, `fin` after it. In a pipe both
  wrote the same bytes. The Windows tests in the VM (11 passed; one first
  failed by the master connection of ssh, passed again). Test
  `the_last_whole_character` (`isc-runtime-core`). Left: the input from a
  console (`ReadConsoleW`), the MSVC ABI.
- **D409 — The input from a Windows console in UTF-8 (2026-11-07).** After
  D408: read by `ReadFile`, a console gives its characters in its code
  page; in the VM (code page 437), `été 日本` typed for `readStdinText()`
  stopped the program: `readStdinText: the standard input is not UTF-8
  text (invalid from byte 0)` (`é` the byte 0x82). Now `read` of the
  standard input, when it is a console (read once), reads UTF-16 by
  `ReadConsoleW`, at most `n / 3` units (3 bytes of UTF-8 at most a unit, 4
  a pair of 2: what is read fits in the place of the caller), turned into
  UTF-8 by `WideCharToMultiByte`; Ctrl+Z first is the end, as `ReadFile`
  gives it; the first half of a surrogate pair at the end of a read is
  held for the next, and the bytes beyond `n` (a place of fewer than 3
  bytes) too. Checked in the VM, typed in a console of an interactive `ssh
  -tt`: `été 日本` then Ctrl+Z gives `[été 日本] 14 bytes` (12 bytes and the
  `\r\n` of the console), `a😀b` gives `[a😀b] 8 bytes`, Ctrl+Z alone `[] 0
  bytes`; the runtime before stopped on the first. A pipe or a file: read
  as before. The Windows tests in the VM: 11 passed.
- **D410 — A loop inside a task writes no line the other threads read
  (2026-11-07).** The server measured against 0.4.0 (2 cores, 3 alternated
  runs of 10 s, medians): the same within 0.6 % (`serve` 386409 -> 388526
  requests a second with 16 connections, 443949 -> 445724 with 256, 414410
  -> 426379 with 1024, the runs overlapping; an action -0.6 to +0.4 %):
  D394 to D409 are not on its path. Its profile (`perf record -p` of its
  user cycles for 5 s, 16 connections): `isc_parallel_for_site` 12.01 %,
  `parallel_job` 1.65 %. Each loop of events is a task of the pool and the
  `map` of its arrivals a loop inside it, whose site read the clock twice
  and wrote its four words (`runs`, `seq`, `last`) at each call, from both
  threads: the line went from one core to the other (3078 samples on the
  load of its words, 3514 after its `lock xadd`, 1571 on `rdtsc`). Now:
  inside a task (`BUSY`) a loop goes to the pool's loop of tasks
  (`nested_for`) without the site (there is no pool to choose); `BUSY` is
  read before it is swapped (a swap writes its line even when it finds it
  set); the count of the open loops is gone (written twice by each loop
  inside a task; the threads with nothing left read the slots below
  `high`), and `high` is written only when it grows. Profile after it:
  `parallel_job` 7.40 %, `isc_parallel_for_site` 1.22 %; 82026 samples for
  388125 requests a second against 84303 for 383575 (0.2113 against 0.2198
  a request, -3.9 % of its user cycles, about a third of its cycles: the
  kernel has the rest); the requests a second within the scatter of the
  measure (6 points of 3 runs: -1.4 to +1.3 %). The ten programs (15
  alternated runs for those with loops inside tasks): binary-trees -0.10 %,
  k-nucleotide -0.22 %, reverse-complement -0.28 %, regex-redux -0.09 %
  (medians), the others within 0.35 % (7 runs), the same outputs. Not
  kept: (1) a loop inside a task run in place when no thread has finished
  its share: binary-trees +28.9 %, k-nucleotide +46.4 % (a thread freed
  later cannot take its tasks); (2) the same, its rest published as soon
  as a thread is free: the server 0.2068 a request (-5.9 %), but
  k-nucleotide +1.33 % (minimum, +0.81 % medians, 15 runs: a freed thread
  waits for the end of the task in progress, long there).
- **D411 — The third read of reverse-complement measured (2026-11-07).**
  TODO: its input read three times where the floor reads it twice. Profile
  (4 threads, user cycles): the count of the heads (`isc_task_4`, a
  comparison with `>` and a count of its mask, by blocks of 4096) 5.01 %,
  the count of the bases of each record (`isc_task_1`, the count pass of
  the `filter` of `b != 10`) 5.07 %, the bases and the lines (`isc_task_3`)
  8.62 %; the rest is the waiting of the threads while the output is
  written (D286). In time (4 threads, the lowest of 15 alternated runs, the
  input of 254 MB): the heads alone 10.69 ms (the reading of the input
  included), the heads and the counts of the bases 17.16 ms, the whole
  program 31.76 ms: the count pass 6.47 ms, 20 % of it. Its count is needed
  before the bases are written (the base `i` of the output is the base
  `m - 1 - i` of the record, `m` the count); the floor counts the newlines
  in the same read as its search of the heads. In the program they are two
  expressions: the heads over all the input, then a `filter` over each
  range the heads give. Removing the read needs the first pass to gather,
  by blocks, the counts of a predicate that only a later expression over
  its ranges uses (a fusion across expressions, the counts of the ranges
  from the counts of their blocks and their two ends): left open in TODO
  with this measure, the program at 2.2 times its floor.
- **D412 — The client of `http` and more of `net` against their floors
  (2026-11-07).** TODO, the audit (D301): `http.request`, `get` and the
  functions of `net` not measured. `bench/primitives/hello_server.c`, a
  server of C started by `run.py` for a group that declares `// bench-server:
  http` (on a free port of the local machine, one connection at a time:
  the request read to the end of its head and of its body, `hello, world!`
  or 64 KiB for `/64k`, `Connection: close`). The forms: `get.small`,
  `get.64k`, `request.post` (64 bytes and a header) in `http.isc`, each a
  connection of its own; `listen.close` (a listener on a port of the
  system, closed) and `resolve.hosts` (`ip6-localhost`, a name of
  `/etc/hosts`) in `net.isc`. Their floors: the same bytes of request sent
  in one write, the response read to the end of its body by its
  `Content-Length`, the connection closed (`http.c`); `socket`, `bind`,
  `listen`, `close`; `/etc/hosts` read and its lines scanned for the name
  (`net.c`). The same checksums (the status and the length of the body;
  the addresses, 100 each and their lengths). Measured (`run.py`, one CPU,
  fixed frequency, 5 runs of 200 rounds): `get.small` 41581.86 against
  37439.71 ns (1.1), `request.post` 41710.26 against 37567.40 (1.1),
  `get.64k` 67064.79 against 53090.46 (1.3), `listen.close` 3874.55
  against 3776.41 (1.0), `resolve.hosts` 5609.73 against 1925.74 (2.9: the
  file split into lines and words, `replaceAll`, `trim`, a text for each,
  where the floor scans its bytes once). Left in TODO: `lookup` (a server of
  DNS on the local machine), the servers of `https`.
- **D413 — `resolve` cuts into words only the lines of `/etc/hosts` that
  hold the name (2026-11-07).** D412: `resolve.hosts` 2.9 times its floor,
  each line of the file stripped of its comment, its tabs replaced, trimmed
  and split into words. A word of a line is a part of its text (the tabs
  and spaces between words, the comment cut away), so a line that does not
  hold the name as a part of its text cannot name it: now each line is
  first tested by `indexOf`, and only those that hold it are cut into
  words; the same addresses. Checked: the parser of before and this one,
  both copied into a program, over 98 cases (7 texts: tabs, comments,
  blank lines, `\r`, a name inside a longer word, an empty name; 14 names):
  the same; `resolve` of the names of the machine's `/etc/hosts`,
  interpreted and compiled, old against new: the same text; the tests of
  `std_net.rs` (4 passed). Measured (`run.py`, 5 runs of 200 rounds):
  `resolve.hosts` 5609.73 -> 4431.34 ns (-21.0 %), 2.3 times its floor; the
  other forms of `net` the same.
- **D414 — `net.lookup` against its floor (2026-11-07).** The last function
  of `net` without a microbenchmark (D412). `bench/primitives/dns_server.c`,
  a server of DNS on a free port of UDP of the local machine, started by
  `run.py` for a group that declares `// bench-server: dns`: each query
  answered with one record, 127.0.0.1 for A, ::1 for AAAA. The form
  `lookup` (`bench.test` asked to it, A then AAAA: two addresses, the
  checksum 212); its floor (`net.c`) one socket of UDP with a time limit,
  the same two queries (the bytes of `net.lookup`), each answer read and
  its records written as texts (`inet_ntop`). Measured (`run.py`, 5 runs of
  200 rounds): 29883.68 against 23607.51 ns, 1.3 times (the runtime makes a
  socket for each query: two sockets made, given time limits and closed
  where the floor makes one); the other forms of `net` the same.
- **D415 — The floor of the numbers of `json`: the shortest decimal by
  Schubfach (2026-11-07).** TODO: the floor of `json.encode` beaten by
  Isocron (0.6): it wrote each number by `snprintf("%.17g")`, slower than
  the shortest form that Isocron writes (the `Display` of `core`). A floor
  is the cheapest way known for the same work: the shortest decimal that
  reads back to the double, written as Isocron writes it (no exponent,
  `-0`). Now `bench/primitives/f64_text.h`: Schubfach (R. Giulietti, the
  algorithm of Java's `Double.toString`), its table of 10^-k on 126 bits
  for k from -324 to 292 computed exactly with integers by `gen_pow10.py`
  (the formula of the floor of log2 10^e checked against the exact value
  for each); the two smallest subnormals written directly (Schubfach, as
  Java, gives them two digits, 4.9e-324; their shortest forms are 5e-324
  and 1e-323). Checked: 3999358 doubles (random bits, the powers of 10 and
  of 2 from -30 to 30, the limits, the 1000 halves of the benchmark) the
  same texts as the shortest digits of Python's `repr` laid out as Rust
  does; 17 limit values the same texts as `print` in Isocron. Measured
  (`run.py`, 5 runs of 200 rounds): the floor of `encode` 216.42 -> 81.41
  ns, Isocron 117.03 ns, 1.4 times; the floor of `stringify` 391.80 ->
  114.35 ns, Isocron 398.74 ns, 3.5 times (a gap the floor hid: added to
  TODO).
- **D416 — The server of `https` against its floor (2026-11-07).** The last
  form of the library without a microbenchmark (the audit, D301): the
  servers of TLS. A new group, `https_serve`: the program serves
  (`https.serveTlsOn`, `hello, world!`, one thread) a client of OpenSSL
  started by `run.py` once it listens (`// bench-client: tls`, the
  certificates of the group `https`), and stops after the requests the
  client makes; its floor `https_serve.c`, a server of OpenSSL called
  directly, one connection at a time, the response of Isocron made once
  (its date, its headers; `Connection: close` when the request asks for
  it). The client `https_client.c`, the same for both: `serve.handshake`, a
  request on a connection of its own (the handshake of TLS 1.3, the
  request, the response, the end), `serve.keepalive`, 64 requests on a
  connection kept; the checksums the status and the length of the body.
  Both set `TCP_NODELAY` as the runtime does on the connections it accepts
  (without it the floor waited for the delayed acknowledgements: 802542
  ns a request kept, 28.1 times Isocron). Measured (`run.py`, the server on
  one CPU at fixed frequency, 5 runs of 200 rounds): `serve.handshake`
  1168175.68 against 1159198.66 ns (1.0: the cryptography of OpenSSL, the
  same library), `serve.keepalive` 39432.08 against 34059.70 ns (1.2).
- **D417 — The keys of a count hashed once, the found ones counted in a
  loop that holds the table (2026-11-07).** TODO: k-nucleotide, its 12-mers
  and 18-mers. The counting alone (`countBy` of the 125 million keys of the
  sequence THREE, one thread, fixed frequency, `perf stat`): the 18-mers
  660.9 ms, 72.1 instructions and 20.7 cycles a key in the whole program,
  `add_block` 56.38 % of its instructions and 55.7 % of its cycles (about
  40.6 instructions and 11.7 cycles a key: 1.7 times the 2.221 ns of a
  random increment of the floor). Its code: each key hashed twice (once for
  the prefetch of the key AHEAD places later, once for itself), and the
  table, its length and `bits` read again from `self` at each key. Now the
  hashes of a chunk of 1024 keys in a loop of their own (vector code: a
  multiplication of 64 bits for 8 keys with AVX-512), then a loop over the
  keys that counts those found in their line with the table, its mask and
  `bits` held in it (a slice borrowed, nothing else of `self` written); a
  new key or a small one goes to `add_count`, after which the table is read
  again (it may have grown). The same keys in the same order: the order of
  the first occurrences kept. Measured: the 18-mers 660.9 -> 618.5 ms
  (-6.4 %), 72.1 -> 61.9 instructions a key; the 12-mers 622.4 -> 575.1 ms
  (-7.6 %), 68.6 -> 58.5; k-nucleotide (4 threads, 11 alternated runs, the
  same output) 682.7 -> 667.2 ms (-2.27 %, medians -1.69 %); the nine
  others within 0.65 % (minimums, 7 runs). Test
  `counts_of_keys_with_the_table_held` (`stdlib.rs`: small keys among
  large ones in the same chunks, the table grown inside a chunk, 1 and 4
  threads, the interpreter's results).
- **D418 — The fast loop of a count: the first line of a key only, no
  test of an empty slot (2026-11-07).** After D417, `add_block` 32.3
  instructions and 10.3 cycles a key for the 18-mers (52.10 % and 51.78 %
  of the 7.74 G instructions and 2.49 G cycles of the count alone). Its
  code: the probe of the next lines in a loop that LLVM aligned with
  padding the fast path ran through (297 samples on one `nop`), the test of
  an empty slot at each key (`vptestmq`, `kandb`), and the bound of the
  prefetch AHEAD places later tested at each key. Now the fast loop looks
  for a key in the first line of its hash only, by `holds` (`e >> bits ==
  k` for the 8 slots: an empty slot, 0, holds no key but 0, which goes to
  the general path); a key not in that line, new, small, or 0 goes to
  `add_count` (the probe of the next lines, the insertion); the hashes
  after the last key of a chunk are its own, so that the prefetch has no
  bound to test. The same counts and order. Measured (one thread, fixed
  frequency): the 18-mers 615.4 -> 565.0 ms (-8.2 %), 61.9 -> 52.5
  instructions a key in the whole program; the 12-mers 578.2 -> 524.4 ms
  (-9.3 %), 58.5 -> 49.2; k-nucleotide (4 threads, 11 alternated runs, the
  same output) 666.5 -> 638.8 ms (-4.15 %, medians -3.94 %); the nine
  others within 0.65 % (minimums, 7 runs), n-body +1.23 % in that run, its
  code the same, +0.05 % in 11 runs. Test `counts_of_keys_with_the_table_held`
  (D417) and the 39 of `stdlib.rs`.
- **D419 — The probes of 8 keys at once in vectors closed by measure
  (2026-11-07).** TODO: k-nucleotide, its keys to probe 8 at once. After
  D418, the count of the 18-mers alone (one thread, 3.2 GHz, `perf stat`
  and `perf record`): `add_block` 44.80 % of 6.563 G instructions and 44.38
  % of 2.283 G cycles, 23.5 instructions and 8.1 cycles (2.53 ns) a key.
  The floor of these counts is the random increment of a table of 2 MiB by
  4 independent streams, 2.220 ns an update a core (`floor.c`), which
  compares no key and adds to a counter of 4 bytes. A count must at least
  load the line of each key and compare its key: each key has a line of
  its own, and its 8 slots are already compared by one vector (`holds`,
  D418). Probing 8 keys at once could take at most the 0.31 ns a key left
  (1.14 times the floor), about 78 ms of CPU for the 250 million keys of
  the 12-mers and 18-mers, of the order of 20 ms of the 638.8 ms of
  k-nucleotide on 4 threads: not done. Left in TODO: the base, and the
  k-mers computed whole at each place (`isc_task_9`, 20.66 % of the cycles
  of the count of the 18-mers).
- **D420 — The k-mers rolled from the one before: tried, not kept
  (2026-11-07).** TODO (D419): the k-mers computed whole at each place
  (`isc_task_9`, 20.66 % of the cycles of the count of the 18-mers, 9.7
  instructions and 3.8 cycles a key). In C (one thread, 3.2 GHz, 125
  million places of 18 bases): computed whole 2.18 ns a key (vector code of
  gcc), rolled `v * 4 - x[i - 1] * 4^18 + x[i + 17]` 0.94 ns (one chain of
  shifts and additions), four chains 0.785 ns; Isocron about 1.18 ns. Tried
  in the compiler: a `map` over `iota` whose value is `fold(iota(K), 0, (c,
  j) => c * A + g(i + j))` (64 bits: exact modulo 2^64; the same reads of
  `g`) filled by blocks from the element before, its first by its `fold`.
  Measured (the count alone, one thread, 2 runs each): the 18-mers 390.0
  -> 389.0 and 391.2 ms, the 12-mers 361.6 -> 392.3 ms (+8.5 %): the vector
  code of Isocron computes 8 k-mers at once as fast as the chain, which pays
  the checks of bounds of its two reads at each key. Not kept.
- **D421 — A tested example in the guide for each function of the library
  (2026-11-07).** The goal of the library: « chacune a un exemple testé dans
  docs/language.md ». Counted function by function (the 91 exported by the
  8 modules, each looked for as `module.name` or imported by name in the
  guide): 43 had an example, each module had at least one. Now one more
  example for each module that left some out: `sys` and `path` (a folder
  made, bytes written and appended, a file renamed and removed, its kind,
  size and time, the folder of the program, the environment sorted,
  `ioError`), `time` (leap years, the days of months, days to dates and
  back, `addDays`, `toDateTime`, `formatDateTime`), `json.quote`, `net` (a
  `Server`: `serverOf`, `wait`, `sendTo`, `closeConn`, `closeServer`, with
  `write`, `setTimeout`, `closeWrite`, `resolve`), `http` (`httpDate`,
  `headerOf`, `request`, `exchange` with `header` and `text`,
  `serveServer`), `https` (`request`, `getWith`, `requestWith`): run by the
  test of the guide, interpreted and compiled with the same output, and as
  WebAssembly or refused for it; `lookup`, `startTls`, `serverOfTls`,
  `serveTlsOn` and `serveTls`, which need a server of DNS or a certificate
  with its key, in examples checked by the compiler (`isc-check`). 91 of
  91 now.
- **D422 — The programs of the guide run on Windows (2026-11-07).** The
  goal of the library: each function tested « dans la vraie VM Windows ».
  Counted function by function (D421), 65 of the 91 were called by the
  programs of the tests of Windows (each module by some). Now the test
  `the_guide_on_windows` (`windows.rs`) builds for Windows each program of
  the guide that runs alone (no standard input, no module of its own) and
  runs it in the machine on one thread: the output the guide gives, which
  the test of the guide checks interpreted, compiled for Linux and as
  WebAssembly. 73 programs, whose examples call each function of the
  library but the five checked only by the compiler (D421: `lookup`,
  `startTls`, `serverOfTls`, `serveTlsOn`, `serveTls`, the last three run by
  `https_server_on_windows`). Its first run found a defect: `envAll` not in
  the order of the names under Windows (D423). The tests of Windows: 12
  passed (`https_client_on_windows` once refused to start, its program
  quarantined by Windows Defender, threat 2147735505, as before; passed
  again).
- **D423 — `envAll` in the order of the bytes of the names (2026-11-07).**
  `sys.envAll` promises the variables « in the order of the bytes of their
  names »; the runtime and the interpreter sorted the entries `name=value`
  whole, so that a name followed by a byte below `=` came before its own
  prefix: `ISC_SORT(x86)`, `ISC_SORT-`, `ISC_SORT1` before `ISC_SORT`
  (under Windows, `ProgramFiles(x86)` before `ProgramFiles`: the example of
  D421 printed `false` there). Now both sort by the bytes before the first
  `=`, then by the whole entry (`sorted_entries` in the runtime, under
  Linux, Windows and WASI); `readDir` sorts its names as before. Test
  `all_the_environment_by_vectors` with these four names (compiled and
  interpreted; the compiler before them gave `ISC_SORT` last).
- **D424 — A microbenchmark for each function of the library, by its name
  or through the one that is it (2026-11-07).** Counted function by
  function (D421), 76 of the 91 were named by a form of
  `bench/primitives`. New forms and floors (the same calls of the system,
  `run.py`, 5 runs of 200 rounds): `net.setTimeout` 1.2 (two `setsockopt`),
  `connect.closeWrite` 1.0 (`shutdown`), `server.open.close` 1.0
  (`serverOf`, `closeServer`: `epoll_create1`, `fcntl`, `epoll_ctl`),
  `server.roundtrip.64` 1.1 (`wait`, `sendTo`: `epoll_wait`, `recv`,
  `send`), `server.conn` 1.1 (a connection waited for and closed by
  `closeConn`); `https.requestWith.small` 1.0 and `startTls` 1.0 (the
  handshake alone, `SSL_connect`); `serve.action.pipelined` now written
  `serverOf` then `http.serveServer`, what `serveOn` does (1.4, as before).
  86 of 91 named. The five others are made of measured ones, word for word:
  `https.get` and `https.request` are `requestWith("", ...)`, the
  authorities of the system, which no server of the local machine has
  (one would have to be installed in the system: not done), measured as
  `requestWith` and `getWith`; `https.serveTls` is `listen` then
  `serveTlsOn`, and `net.serverOfTls` the first step of `serveTlsOn`
  (`serve.handshake` 1.0, `serve.keepalive` 1.2, D416); `http.serve`, a
  loop of events on each core with no end, measured against nginx and the
  ceiling of the machine by `bench/http_servers` (`results/http_servers.md`).
- **D425 — The MSVC ABI at the boundary of a DLL (2026-11-07).** TODO:
  « Windows: the MSVC ABI » (D246: MinGW-w64 first, MSVC later). What that
  ABI brings to a program of Isocron is the use of code built by Visual
  Studio. The convention of calls of Win64 is the same for both ABIs, so
  that a program of the target of MinGW calls the functions of a DLL
  compiled by Microsoft: test `msvc_compiled_c_functions_on_windows`
  (`windows.rs`, in the machine of Windows), `GetTickCount64`, `lstrlenA`
  and `MulDiv` of kernel32 and `strlen` of msvcrt through `declare
  function`; and those of `shlwapi.dll` (`StrToIntA`, `PathIsRelativeA`,
  `ChrCmpIA`), which iscc does not link, through an import library in the
  format of Microsoft's `lib.exe` (short import members, made by the test
  from a `.def` by `lld-link /lib`, the LLD of Rust) given by `--link`, as
  one given with a DLL built by Visual Studio; without it the build fails
  (the functions not found). Not done: static libraries built by Visual
  Studio and a target `x86_64-pc-windows-msvc`. Their objects refer to the
  helpers of Microsoft's C runtime (`__security_cookie` and
  `__security_check_cookie` for a buffer on the stack under the default
  `/GS`, `__chkstk` for a frame of more than a page, `_fltused` for
  floats), none of which the files of MinGW-w64 that iscc links define
  (`nm`: only `___chkstk_ms` in `libgcc.a`); that runtime and the
  libraries of the SDK are Microsoft's, under its licence, neither in iscc
  nor in the machine (no `vswhere.exe`, no `Windows Kits`). Installing
  Visual Studio Build Tools in the machine to build and test them is the
  author's decision: the item left open in TODO with that need. The tests
  of Windows: 13 passed (`https_client_on_windows` once refused to start,
  its program quarantined by Windows Defender, threat 2147735505, as in
  D422; passed again).
- **D426 — The counts of a later `filter` made in the pass of an earlier
  one over the same array (2026-11-07).** TODO, the third read of
  reverse-complement (D411: the count of the bases, 6.47 ms of 31.76).
  Cost model first (`fused.c` of the session, 4 threads over the mapped
  input of 254 MB, AVX-512, the lowest of 31 rounds): the search of the
  heads alone 5.464 ms, the count of the bases alone 5.546 ms, both in one
  pass 6.183 ms: a pass over the memory, the second count nearly free
  while each block is in the caches. In the program they are two
  expressions in two functions: `filter(iota(data.length), (i) => data[i]
  == 62u8)` in `main`, then in `records` (recursive) a windowed `filter`
  (D293) of `b != 10u8` over `data.slice(...)`. Now, in the code
  generator:
  - `count_users`: the functions whose windowed `filter` reads a range of
    one of their parameters (a `slice` of it, or itself) with a predicate
    of its elements alone (no free variable);
  - a `let` whose `init` makes a `filter` over `iota(a.length)` that reads
    `a[i]`, and whose body gives `a` to such a function at that
    parameter, makes a block `[base, length, count of block 0, ...]`: the
    pass of that `filter` (the parallel one, also on one thread) counts
    the predicate of the callee by blocks of 1024 of `a`, after each of its
    blocks of 4096 (in the caches), in vectors of 64 bytes; `base` is set
    once they are made (null otherwise: an array of at most 4096
    elements);
  - the call gives the block to a variant of the callee
    (`isc_fn_{f}_sm`, its first parameter; its own calls with the same
    parameter give it on; the tail calls are its loop);
  - there, when the source of the windowed `filter` lies within `base ..
    base + length` (checked at run time), its blocks are those of the
    array from an origin (its offset in the first one, kept after the
    places): the count of each whole block read from the block, the two
    ends counted; otherwise the origin is 0 and it counts in parallel as
    before. Its windows (`windowed_fill`, `windowed_whole`) read the
    blocks from that origin.
  Measured (fixed frequency, 4 threads, the input of 254 MB):
  reverse-complement 31.9 -> 27.7 ms (lowest of 31 alternated runs,
  -13.13 %, medians -13.04 %), the same output; 1.65 times its floor of
  16.8 ms. Profile of 20 runs: user cycles 7191.3 -> 6202.1 million,
  instructions 11668 -> 11353 million; the count of the bases (20.31 % of
  the samples, 1460 million cycles) gone, the pass of the heads 1709 ->
  1958 million (+14.6 %, against +13 % for the fused pass of C). The
  counts first in vectors of 32 bytes (the width of the byte filters)
  cost +25 % of that pass (29.2 ms); in vectors of 64, -4.50 % more. The
  LLVM IR of the nine other programs is unchanged. Test
  `counts_made_by_a_caller` (`bitwise.rs`): records of bytes and of
  `i32` from heads every `step` elements (within a block, across many, a
  single record), the same function given another array and arrays of
  at most 4096 elements (their own counts), 1 to 8 threads, the bytes
  computed by the test and by the interpreter, valgrind; with the counts
  read from the block falsified by one, it fails.
- **D427 — A large join by ranges in parallel; one search a line
  (2026-11-07).** TODO, k-nucleotide, « the base »: its text said the codes
  were an array of `i64` of 1 GB; they are bytes since D218 (the IR:
  `codes%17: u8[]`). Measured instead, the base alone (`kn_base` of the
  session: the lines, the heads, the join, the codes; 4 threads, fixed
  frequency, lowest of 15 runs): the lines 77.4 ms, with the heads 85.8,
  with the join 115.4, with the codes 129.5 ms; per run 653 million user
  cycles and 439 million of the kernel (clearing new pages: 336 MB of
  lines, 67 MB of their strings, 125 MB of the join, 125 MB of codes).
  The join added 29.6 ms for about 100 million cycles: `isc_join` copied
  the 2.08 million lines on one thread. Now `join` (runtime, safe code
  but the reads of the strings) makes a result of 1 MiB or more by ranges
  of its strings in parallel (each about `total / 4 x threads` bytes, cut
  where a string starts, the separator before it the range's; each
  thread writes its own pages); `IscStr` is `Sync` (a pointer and a
  length, read only). Measured (21 alternated runs): the join stage 115.5
  -> 107.6 ms, the base 129.9 -> 122.0 ms, k-nucleotide 635.6 -> 630.1 ms
  (-0.86 %, medians -1.09 %), the same output; what the join costs still
  (+22 ms over the heads, 60 ms of CPU) is its copies, 2.08 million of 60
  bytes (133 million user cycles), and the clearing of its 125 MB (64
  million in the kernel). Also, the lines of a part of the text (D252)
  searched once each (the end of a line is the `\n` before the next one;
  each was searched twice): the instructions of the base 11651 -> 9205
  million for 10 runs, its user cycles -2.4 %, its time the same (130.3
  -> 130.0 ms): the searches were not the bound, the reads of the 250 MB
  of the input are (twice: the count of the lines, then the lines). Tests
  `a_large_join_made_in_parallel` (`stdin.rs`: strings of 0 to 299 bytes,
  empty ones, one of 3 MB, separators of 0, 1 and 3 bytes, 1 to 8
  threads, the bytes of Rust's `join` and of the interpreter; without the
  separator at the start of a range, it fails on 2 threads) and
  `lines_of_a_large_text_made_in_parallel`.
- **D428 — A join read once in order by a `map` of its bytes is never
  made (2026-11-07).** TODO, the arrays made by blocks for other consumers
  (k-nucleotide's join, D427: 125 MB written, cleared by the kernel, then
  read once in order by the `map` of the codes). Now `let s = join(parts,
  sep) in let t = bytes(s) in let c = map(t, l) in rest`, `s` and `t`
  read nowhere else (the parts and the separator borrowed), makes `c`
  without `s`: the ranges of the join as `join` makes them in parallel
  (`isc_join_ranges`, D427), each by a task that gathers its bytes 16 KiB
  at a time into a buffer on its stack (`isc_join_next`: the strings and
  the separators of the range from a cursor) and maps them from there
  into `c` in vector code (`fill_range`, the lambda as any `map`'s: its
  narrowed result, D218, its tables). Measured (fixed frequency, 4
  threads, 21 alternated runs): the base of k-nucleotide 122.5 -> 108.4
  ms (-11.51 %), per run 740 -> 668 million user cycles, 477 -> 384
  million in the kernel, 1890 -> 1339 page faults; k-nucleotide 632.1 ->
  615.0 ms (-2.70 %, medians -3.52 %), the same output; the LLVM IR of the
  nine other programs is unchanged. Test
  `a_join_read_by_a_map_is_never_made` (`stdin.rs`): separators of 0, 1
  and 3 bytes, empty strings, one longer than the buffer, no string,
  results of 8 bits, of 64 bits and through a table, 1 to 8 threads, the
  values of Rust and of the interpreter, valgrind, a read past the table
  with the panic of the interpreter; without the separator before the
  second string, it fails.
- **D429 — The Benchmarks Game measured again at 5f615bb (2026-11-07).**
  After D426 to D428 (reverse-complement 31.9 -> 27.7 ms, k-nucleotide
  632.1 -> 615.0 ms on the host): the image built with the `iscc` of
  5f615bb, the programs built and checked against the references (102
  checks, the same outputs; the known difference of C++ #4 of mandelbrot
  at N = 200 only, the same at 16000), then measured in Docker (4 threads,
  12 runs, the lowest, `bench/machine.sh apply parallel`). The fastest on
  10 of 10: reverse-complement 32 -> 27 ms (C and Rust 62 ms, 0.44 of
  them, 1.9 times its floor), k-nucleotide 677 -> 622 ms (C++ 1165 ms,
  0.53, 2.3 times its floor); the others within their scatter (n-body
  1878 -> 1904 ms with the same LLVM IR since D407, the scatter of its
  series, D261; the others the same to 1 ms). `results/benchmarks_game.md`,
  `benchmarks_game.tsv`, `floors.md`, `floors_isocron.tsv`, the table of
  the README and its figure made again from these lines.
- **D430 — Release 0.4.2 (2026-11-07).** The author: « quand tu
  l'estimeras nécessaire il faudra penser à faire puis pousser une release
  ». After 0.4.1: pidigits by ADX (D406), the keys of k-nucleotide hashed
  once (D417, D418), the counts of reverse-complement made in the pass of
  its heads (D426), a large join in parallel (D427), a join read by a
  `map` never made (D428), the Windows console in UTF-8 (D408, D409),
  `envAll` in the order of the names (D423), the MSVC ABI at the boundary
  of a DLL (D425), an example in the guide (D421, run on Windows, D422)
  and a microbenchmark (D412 to D416, D424) for each function of the
  library; the Benchmarks Game measured again (D429). No name removed, the
  language the same: 0.4.2. The Windows tests in the VM with the
  `iscc.exe` of the release's sources first; the archives as for 0.4.1
  (`build_iscc.sh` for Linux and Windows in the images of the releases,
  from the sources of the commit of the release; `package.sh`; the
  extension of VS Code at 0.4.2), tried before they are published
  (`selftest_linux.sh`, `selftest_windows.py`, the example of the README
  by the `iscc` of the archive). `CHANGELOG.md`, the links of the README
  and of `tools/vscode/README.md` at 0.4.2.
- **D431 — The last five functions of the library measured by their names
  (2026-11-07).** After D424, 86 of the 91 were named by a form; the five
  others only through the functions they are made of. Now each by its own
  (`run.py`, 5 runs of 200 rounds, fixed frequency):
  - `https.get` and `https.request`, the authorities of the system: an
    empty authority makes the runtime call
    `SSL_CTX_set_default_verify_paths`, which reads `SSL_CERT_FILE`;
    `run.py` sets it to the authority of the session for the program and
    its floor (whose client takes the system's paths the same way), so
    that nothing is installed in the system: `get.system.small` 1.0,
    `request.system.small` 1.0;
  - `net.serverOfTls`: a server of TLS opened on a new listener and
    closed, against the calls of the runtime in C (the context, the
    certificate and the key read from their texts, `epoll`):
    `serverOfTls.close` 1.0;
  - `http.serve` and `https.serveTls`, which serve without end: groups
    measured by a client in C (`// bench-client: http`, `http_client.c`,
    `https_client.c`), the server stopped by a signal once the client is
    done (`run_served`); against servers of one thread on the system's
    calls (`http_serve.c`, `https_serve.c`): `http_serve` a request by
    connection 1.1, on a connection kept 1.2; `https_serve_tls` 1.0 and
    1.2 (as `serveTlsOn`, D416).
  91 of 91 functions named by a form of `bench/primitives`
  (`results/primitives.md`).
- **D432 — The waiting of the threads of reverse-complement measured again
  after D426 (2026-11-07).** D286 explained the waiting at the end of its
  writing passes when the program took 59 ms: the pages at the end of the
  grown block of each `filter`, cleared by the thread of the last range.
  Since D293 and D426 the bases are never made whole, so that cause is
  gone; measured again (4 threads, fixed frequency, 20 runs, `perf record`
  of user cycles at 20 kHz, the samples of each thread): the threads are
  busy for the whole run (111 ms of CPU a run for 27.7 ms on 4 threads),
  and the waiting of the pool (`worker`, `parallel_job`) is 6.2 % of the
  samples: 5.11 % on the three workers, 1.08 % on the thread of the body.
  The body alone (its sequential parts: the sums of the places, the
  counts taken from the pass of the heads, the writes) is 0.87 % of the
  samples, while the three workers wait: about 2.6 %; the rest, about
  3.6 %, is the end of each of the four parallel phases (the heads, then
  the output of each of the three records), the last thread finishing its
  range (contiguous ranges, D254). About 1.7 ms of each thread's 27.7 ms;
  no clearing of pages left in it. n-body's 17 cycles (D312) stand: its
  LLVM IR is the same since D407.
- **D433 — The 17 cycles of n-body measured again; the publication of
  D429 holds for the code since (2026-11-07).** D312 explained them by
  measure (the ports 0 and 1, the divider, `llvm-mca` 116.04 cycles a step
  against 119.62). Measured again on the binary of today (one thread, CPU
  0, its sibling 8 not taken offline, 50 million steps, `perf stat` of 3
  runs, fixed frequency): 121.65 cycles and 243.00 instructions a step;
  the ports 0, 1, 5 and 6 78.58, 79.69, 48.76 and 1.00 uops a step, the
  loads 28.00, the stores 14.00 and 14.00: the same work as in D312 to a
  tenth of a uop (78.75, 79.47, 48.84, 1.00, 28, 14, 14), so the same loop
  and the same model, 95.4 % of the step (the 2 cycles more than D312, its
  sibling online: D261). And the LLVM IR of the ten programs built by
  the compiler of today is the one of 5f615bb (only the versions of the
  crates changed in `crates/` since, D430), so the publication of D429
  measures the code of today.
- **D434 — A component of a new tuple that dies with it moved out, not
  copied (2026-11-07).** TODO, the forms above 3 times their floor:
  `http.textResponse` 22.5 ns against 2.2 (10.3). Its IR made three blocks
  a response: the array of headers, the body copied into the record, and
  the body copied again out of it (`copy (proj 0 tmp)`), because the value
  of `let tmp = <record> in proj 0 tmp` (the body read out of the
  response a function made) is a component of a tuple that dies with the
  `let`, which the ownership pass copies. Now `bind_projections`
  (`passes.rs`) binds each component that owns memory of such a tuple
  (`let c = proj j r`), the value read through its variable, so that
  `split_tuples` (D138) takes the tuple apart: the component moved out
  (`take`), the others freed on their own. Measured (`run.py`, 5 runs of
  200 rounds, fixed frequency): `textResponse` 22.5 -> 13.9 ns (6.3 times
  its floor). A first version took apart every new tuple read only by
  projections: `http.header` 19.4 -> 29.3 ns (the headers read through a
  captured variable instead of `proj 1 resp`) and the IR of n-body and
  k-nucleotide changed with the same times (1877.5 -> 1879.1 ms, 615.4 ->
  613.9 ms); not kept, the rule only for the value of the `let` being the
  component (where the copy was): the LLVM IR of the ten programs is
  unchanged, of the groups of `bench/primitives` only `http`'s. Test
  `a_component_read_out_of_a_new_tuple_is_moved` (`tuples.rs`: components
  of bytes, of an array of tuples of strings, of integers, of a string,
  read in a parallel `map`, in a recursive action, kept in an array; the
  interpreter's output on 1, 3 and 8 threads; valgrind; without the pass
  no `Take` is made and it fails).
- **D435 — The names of `envAll` measured once for its sort (2026-11-07).**
  The sort by names of D423 split both entries at their `=` at each
  comparison: `sys.envAll` 1684 -> 2376 ns (measured with D434's run; the
  floor still sorted whole entries, 953 ns). Now the length of each name is
  found once (`sorted_entries`; the interpreter by a cached key), and the
  floor sorts as `envAll` does since D423 (by names measured once, then
  the entries): `envAll` 1958 ns against 1237 (1.6).
- **D436 — The ASCII prefix of a text checked 64 bytes at a time
  (2026-11-07).** `http.text` of 1 KiB took 89 to 90 ns, 277 cycles and
  1040 instructions an iteration; its profile: `from_utf8` (the check of
  `stringOf`) 43.7 % of the samples, the check of the module (`utf8`, in
  vector code) 31.5 %, the copy 11.7 %. `from_utf8` tests ASCII by two
  words a step. Now `utf8_check` (`isc-runtime-core`, safe code) tests
  the ASCII prefix by blocks of 64 bytes, the `|` of each block compared
  once (vector code), and gives the rest to `from_utf8` from the first
  block with a byte of 128 or more (a character starts there): the same
  answer and the same place of the fault. Used by `stringOf` (copied or
  taking its block), `readText`, the text of the standard input. Measured
  (`run.py`, 5 runs of 200 rounds): `http.text.1k` 90.4 -> 74.8 ns (5.2
  times its floor); `readText.4k` 6484 -> 6433 ns, `json` and the other
  forms within their scatter; regex-redux 81.0 -> 80.8 ms, k-nucleotide
  612.7 -> 615.0 ms (15 alternated runs, the same outputs): their checks
  were a small part. Test `utf8_check_is_from_utf8` (`isc-runtime-core`:
  every length around the blocks, characters of 1 to 4 bytes, faults and
  cut characters before, across and after the blocks, the answer of
  `from_utf8`). Left: the body still checked twice (`utf8` of the module,
  then `stringOf`): a text of bytes already checked needs a primitive that
  checks once and says so, pure, which the primitives (actions) have not.
- **D437 — The rest of a count in one masked vector (2026-11-07).** The
  count of `json.quote` (D396) over a text of 16 bytes, shorter than its
  vector of 32, went to the loop of single elements. Now `count_kept`
  counts the rest (after its whole vectors) in one vector whose lanes
  past the end are masked, when the elements are read from memory (a
  masked load reads nothing past the end) and the predicate cannot fail
  in a lane (`masked_safe`: variables, literals, comparisons, logic,
  arithmetic but division and remainder, conversions, choices, `let`s),
  on a target with masked loads of bytes (AVX-512 BW and VL; AVX2 has
  none for bytes and keeps the loop). The model was wrong: LLVM had made
  the loop of the rest vector code already. Measured (one CPU, fixed
  frequency, 11 alternated runs): `quote.escaped` alone in its program
  95.15 -> 93.27 ns (-2.0 %), `quote.plain` 9.67 -> 9.61 ns (-0.6 %);
  reverse-complement 28.1 -> 27.7 ms (-1.45 %), k-nucleotide 614.4 ->
  612.6 ms (-0.30 %, 15 alternated runs, the same outputs). In the binary
  of the group of `json`, `quote.escaped` 98.06 -> 106.29 ns, and in
  `http`'s `text.1k` 74.19 -> 63.17 ns, both by the place of the code: the
  program of `text.1k` alone, 68.57 -> 61.40 ns, runs the same
  `utf8_check` (the same instructions, at a multiple of 64 in both) with
  0.79 then 0.03 mispredicted branches an iteration (218 -> 199 cycles,
  818 -> 809 instructions). Test `counts_with_a_masked_rest` (`bitwise.rs`:
  arrays of bytes and of `i32` of 0 to 100 elements, slices at every
  offset, counts and `filter`s, the code for the machine and for AVX2,
  the interpreter's values; without the mask of the lanes it fails at 33
  elements).
- **D438 — The Benchmarks Game measured again at 67cd983 (2026-11-07).**
  After D434 to D437 (the code of k-nucleotide and reverse-complement
  changed by D437, the runtime by D435 and D436): the image built with the
  `iscc` of 67cd983, the programs built and checked against the
  references (102 checks, the same outputs; the known difference of C++ #4
  of mandelbrot at N = 200 only), then measured in Docker (4 threads, 12
  runs, the lowest, `bench/machine.sh apply parallel`). The fastest on 10
  of 10: k-nucleotide 622 -> 614 ms, reverse-complement 27 -> 28 ms and
  n-body 1904 -> 1879 ms within their scatter (D429: 1 ms of 27; n-body's
  series, D261), the others the same to 1 ms. `results/benchmarks_game.md`,
  `benchmarks_game.tsv`, `floors.md`, `floors_isocron.tsv`, the table of
  the README and its figure made again from these lines.
- **D439 — `net.lookup` tested on Windows (2026-11-07).** The goal of the
  library: each function tested in the machine of Windows. Counted again:
  `net.startTls` runs there through `https.requestWith`
  (`https_client_on_windows`), `serverOfTls`, `serveTlsOn` and `serveTls`
  in `https_server_on_windows`, but `net.lookup` nowhere: `net_on_windows`
  runs `std_net.isc` without the port of a server of DNS, so it skips its
  queries. Now `lookup_on_windows` (`windows.rs`): a server of DNS of the
  test at the address of this machine that the virtual machine reaches
  (`ISC_WINDOWS_HOST_ADDR`, 172.17.0.1), `example.test` with A 10.1.2.3
  and AAAA 2001:db8::7, every other name NXDOMAIN, and a port where no
  server answers; the program compiled for Windows and for Linux, on 1 and
  4 threads: `Ok([10.1.2.3, 2001:db8::7])`, the fault of a name without
  records, the delay; the same outputs on both. The tests of Windows: 14
  passed.
- **D440 — A short text checked as ASCII without `from_utf8` (2026-11-07).**
  TODO, the server: its profile (`serve.pipelined` alone, one CPU, 60
  rounds of 1600 requests) gave `isc_str_from_bytes` 5.19 %, `from_utf8`
  3.32 % and `utf8_check` 0.63 % of the user cycles (1830 cycles a
  request): the lines and names of a request, shorter than a block of 64
  bytes, went whole to `from_utf8` (D436 tested whole blocks only). Now
  the bytes after the whole blocks are tested the same way, by the `|` of
  their bytes, when the blocks were ASCII: `from_utf8` only for a text
  with a byte of 128 or more. Measured: `serve.pipelined` alone 1323.9 ->
  1299.9 ns a request (medians of 11 alternated runs, -1.82 %), its UTF-8
  9.14 -> 6.27 % of the cycles (`from_utf8` 0.16 %); in `run.py` (5 runs
  of 200 rounds) `serve.pipelined` 1318.0 -> 1299.9 ns (1.36 times its
  floor), `serve.action.pipelined` 1374.3 -> 1352.7 ns, the other forms
  of `http` and `json` within their scatter. Test
  `utf8_check_is_from_utf8` (every length around the blocks).
- **D441 — The integers of one or two digits of a template written by
  their bytes (2026-11-07).** TODO, the forms above 3 times their floor:
  `http.httpDate` 82.4 ns against 13.1 (6.3). Profile (alone, one CPU):
  258 cycles and 947 instructions a date, `isc_put_int` 20.7 % of them:
  its fields written digit by digit (D402), nine calls a date. Now an
  integer of a template without padding that is not negative and below
  100 is written in the generated code: one digit as its byte, two by a
  pair of a table of `00` to `99` (a global of 200 bytes); the others
  call the runtime as before. Measured (`run.py`, 5 runs of 200 rounds):
  `httpDate` 82.4 -> 57.1 ns (4.4 times its floor), the form `template`
  (`${i}-${length}`) 14.8 -> 13.1 ns, `serve.pipelined` 1299.9 -> 1286.3
  ns (its lengths); one digit only first: `template` 14.8 -> 15.6 ns (the
  test before each call), so two. The LLVM IR of six programs changed
  (their outputs): binary-trees 669.0 -> 660.6, fannkuch-redux 1660.0 ->
  1659.3, k-nucleotide 615.7 -> 614.4, mandelbrot 637.6 -> 637.2,
  pidigits 298.3 -> 298.5, regex-redux 80.7 -> 80.3 ms (9 alternated
  runs, the same outputs). Test `small_integers_in_templates`
  (`strings.rs`: -120 to 120 and their multiples, 999 and 1000, the
  extremes of `i64`, `u8` and `i32`, a padded one, for the machine and
  for AVX2, the interpreter's text; with a table shifted by one it fails).
- **D442 — The texts on the stack known by function (2026-11-07).** The
  suite of D441 failed: `json_decoded_into_types_and_encoded` under
  valgrind (AVX2, glibc's allocator) freed an address of the stack. Found:
  the reduction to the decoded value printed and encoded fails whole and
  passes without any one of its six statements; the code of the main body
  the same as before D441 register for register but for one test: the
  text of the name (`quote`, written into a buffer on the stack, D404)
  freed without the test of its buffer. `stack_strs`, the texts on the
  stack, is keyed by the names of registers, which each function and each
  task numbers from 0; a task emitted inside the main body (the encoding
  of the numbers of the points) had a text on the stack under the same
  name, and took the main body's out when it ended. D441 had only moved
  the numbers of the registers of that task (its integers written in the
  code); the fault is D404's. Now the table is saved and emptied around
  each task (`outline_task`) and each function (`emit_fn`), as the
  environment is. The test of the buffer back in that IR; `stdlib.rs` and
  `strings.rs` pass whole; the LLVM IR of the ten programs the same as
  with D441.
- **D443 — A count of fewer elements than a vector in one masked vector
  (2026-11-07).** D437's model was wrong for `json.quote` of a name: its
  count did not reach `count_kept`. Profile (alone, one CPU): 29 cycles
  and 121 instructions a quote; an exact reduction of fewer elements than
  its lanes (16 bytes, 32 lanes) goes to `reduce_loop`, one element at a
  time, before the blocks. Now such a reduction that is a count
  (`count_parts`) is made by `count_kept`, so by one masked vector on a
  target with masked loads of bytes (AVX-512 BW and VL; elsewhere as
  before). Measured (`run.py`, 5 runs of 200 rounds): `json.quote.plain`
  9.7 -> 6.7 ns (4.1 times its floor), `json.message` 13.0 -> 8.4 ns,
  `json.quote.escaped` 98.5 -> 93.7 ns; the LLVM IR of the ten programs
  unchanged. In the same run `sys.env` 21.6 -> 27.0 ns with its floor
  11.4 -> 16.1 ns (its ratio 1.9 -> 1.7: the measure of that series, its
  code unchanged). Test `counts_with_a_masked_rest` (counts of 0 to 100
  elements, so these short ones too, for the machine and for AVX2).
- **D444 — The Benchmarks Game measured again at efded9d (2026-11-07).**
  After D439 to D443 (the outputs of six programs written by D441, the
  runtime by D440): the image built with the `iscc` of efded9d, the
  programs built and checked against the references (102 checks, the same
  outputs; the known difference of C++ #4 of mandelbrot at N = 200 only),
  then measured in Docker (4 threads, 12 runs, the lowest). The fastest on
  10 of 10, the times within their scatter of D438 (binary-trees 666 ->
  663 ms, n-body 1879 -> 1885 ms, regex-redux 81 -> 82 ms,
  reverse-complement 28 -> 27 ms, the others the same to 1 ms).
  `results/benchmarks_game.md`, `benchmarks_game.tsv`, `floors.md`,
  `floors_isocron.tsv`, the table of the README and its figure made again
  from these lines.
- **D445 — A null pointer freed without the cold path (2026-11-07).**
  TODO, `http.parseUrl` 5.8 times its floor. Profile (alone, one CPU): 352
  cycles and 1472 instructions a call, a third of them in frees: `isc_free`
  20.0 %, `free_cold` 7.9 %, `heap::free` 6.4 %. Under gdb, the pointers
  that reached `free_cold` were null: a value of a sum type (`Result`, the
  types of the program) holds the fields of every variant, and its drop
  frees each, those of the variants it is not being null; `isc_free` tested
  the empty string and the cache of the thread, then went to the cold path
  and to `heap::free`, which returned. Now `isc_free` returns at once on a
  null pointer. Measured (`run.py`, 5 runs of 200 rounds): `parseUrl`
  108.2 -> 86.9 ns (4.6 times its floor), `sys.ioError.message` 45.6 ->
  31.0 ns (2.2, below 3 now), `json.parse` 864.3 -> 819.2 ns,
  `http.exchange.small` 7182 -> 6870 ns, `time.parseIso` 36.4 -> 34.0 ns;
  binary-trees, k-nucleotide and regex-redux within their scatter (9
  alternated runs, the same outputs). Test
  `sum_values_with_fields_of_the_heap_dropped` (`stdlib.rs`: `Result`,
  `Option` and a type of the program whose variants hold strings and
  arrays, by thousands, kept in arrays and dropped in parallel `map`s, the
  interpreter's output on 1 and 4 threads, valgrind).
- **D446 — An integer padded to two digits by zeros written from the
  table of pairs (2026-11-07).** `time.format` 5.7 times its floor; its
  fields of two digits are `pad(x, 2)`, an integer padded by zeros at its
  start, which a template writes by `isc_put_int` (D359), one call a field.
  Now such an integer that is not negative and below 100 is written as the
  pair of the table of D441 (`00` to `99`: the zero of padding included);
  the others call the runtime as before. Measured (`run.py`, 5 runs of 200
  rounds): `time.format` 205.7 -> 182.9 ns (5.0 times its floor),
  `time.formatDateTime` 73.8 -> 51.7 ns (1.07), `time.formatIso` 76.7 ->
  64.9 ns; the forms of `http` within their scatter; the LLVM IR of the
  ten programs unchanged. Test `small_integers_in_templates` (`strings.rs`)
  with integers from -20 to 129 padded to 2 by zeros at the start, by
  another fill, at the end, for the machine and for AVX2. TODO: the forms
  above 3 times their floors given again with today's numbers.
- **D447 — The Benchmarks Game measured again at 4ca9fb0 (2026-11-07).**
  After D445 (the runtime) and D446 (no change of the code of the ten
  programs): the image built with the `iscc` of 4ca9fb0, 102 checks against
  the references (the same outputs; the known difference of C++ #4 of
  mandelbrot at N = 200 only), measured in Docker (4 threads, 12 runs, the
  lowest). The fastest on 10 of 10, every time within its scatter of D444
  (k-nucleotide 614 -> 618 ms, n-body 1885 -> 1881 ms, binary-trees 663 ->
  662 ms, the others the same to 1 ms). `results/benchmarks_game.md`,
  `benchmarks_game.tsv`, `floors.md`, `floors_isocron.tsv`, the table of
  the README and its figure made again from these lines.
- **D448 — A value of a sum type frees the fields of its variant only
  (2026-11-07).** D445 left about 5 cycles to each null field of the
  variants a value is not (a call of `isc_free` each). A sum type is a
  tuple in the IR (its tag, then the slots of every variant) whose slots
  only `print` knew. Now the lowering keeps the tuple of each sum type laid
  out with the slots of its variants, and the tuples of records, tuples
  and pairs of `zip` (`plain_tuples`): `Program::sum_slots` holds those
  told apart by their type alone (the tag first, two variants at least,
  one layout of slots, no record, tuple or pair of the same tuple). The
  drop of such a value reads its tag and frees the fields of that variant
  only (`switch`); every other tuple is freed whole as before. Measured
  (`run.py`, 5 runs of 200 rounds): `http.parseUrl` 86.8 -> 63.2 ns (3.3
  times its floor), `sys.ioError.message` 31.0 -> 12.6 ns (0.88),
  `sys.cwd` 415.0 -> 397.3 ns; the other forms of `http`, `json`, `sys`
  and `time` within their scatter; the LLVM IR of the ten programs
  unchanged. Test `sums_freed_by_their_variant` (`stdlib.rs`: sums of
  strings, arrays and nested sums, a variant without fields, and a record,
  a tuple and the pairs of `zip` of the same tuple as a sum, on 1 and 4
  threads, the interpreter's output, valgrind; with the records not left
  out, valgrind finds their texts lost).
- **D449 — Release 0.4.3 (2026-11-07).** After 0.4.2: the fix of D442 (a
  text on the stack freed as a block of the heap, in 0.4.2 already), and
  D431 to D448: a microbenchmark for each function of the library by its
  name, `net.lookup` on Windows, a component of a tuple moved out, the
  small integers of templates written by their bytes, counts in masked
  vectors, ASCII checked by blocks, null and the other variants of a sum
  freed without a call; the Benchmarks Game measured again (D447, whose
  ten programs have the same LLVM IR as today's). No name removed, the
  language the same: 0.4.3. The steps of 0.4.2 (D430): the Windows tests
  in the VM with the `iscc.exe` of the release's sources, the archives
  built from those sources in the images of the releases, tried before
  they are published.
- **D450 — Bytes known below 0x80 given to a text without checking them
  (2026-11-07).** `sha256.hex.32` 5.4 times its floor. One call, one CPU
  at fixed frequency (`perf stat`, 20 M calls): 37.9 cycles and 163
  instructions, against about 7 cycles for the floor (2.05 ns). Profile:
  `utf8_check` 31.4 % (the 64 digits reduced to their largest byte, a
  chain of about 10 cycles), the digits 30.3 %, `isc_alloc` 12.0 %,
  `isc_str_adopt_bytes` 10.1 %, `isc_free` 9.4 %. The digits come from the
  table `0123456789abcdef`: ASCII, so UTF-8, known at compilation.
  - Now `passes::byte_bounds` gives the largest byte each `let` can hold
    when it is known from its value (a literal, the bytes of a string, an
    element, a slice, a copy, the branches of an `if`, a map, `&`, `>>` by
    a constant, `min`; a variable bound twice takes the larger, a parameter
    none), and `stringOf` of bytes at their last use below 0x80 is their
    block, without a call (`isc_str_adopt_bytes`, its check and its
    panic, which cannot happen).
  - Rejected before it: the ASCII test of `utf8_check` by words of 8 bytes
    or'ed together. LLVM read the block of 64 bytes by one load of 64
    bytes over the two stores of 32 that wrote it just before, which the
    processor cannot forward: 11.60 -> 15.92 ns, 50 % of the samples on
    that load.
  - Measured: `hex` alone, 7 alternated runs of 2 M calls: 11.60 -> 6.58
    ns; `run.py` (5 runs of 200 rounds) `sha256.hex.32` 14.3 -> 7.2 ns, 2.7
    times its floor. The code of the forms of `strings`, `sys`, `json`,
    `http` and `time` the same (their LLVM IR compared with the compiler of
    9df47bb); `sys.env` 19.9 -> 28.1 ns with its floor 11.7 -> 16.1 (the
    environment of the processes, the ratio 1.70 -> 1.75), the forms of
    `strings` of 1.5 to 2 ns within their scatter (two runs of the same
    binaries in a row: `bytes` 1.475 and 1.631, about a cycle). Of the
    ten programs only k-nucleotide changes: its `name` of a k-mer, from the
    table `ACGT`, no longer checked (a few dozen names).
  - Test `ascii_bytes_given_to_texts_unchecked` (`stdlib.rs`: a table of
    ASCII, `sha256.hex`, `& 0x7f`, `>> 1`, an `if` of literals not
    checked; a table with `é` checked, the same panic as the interpreter
    for a text cut in a character; with the bound taken up to 0xff the
    test fails).
- **D451 — The checks of UTF-8 of the library as exact as the runtime's
  (2026-11-07).** The modules `http`, `sys` and `json` each checked bytes
  as UTF-8 by a fold that only looked at the classes of the bytes: it took
  an encoded surrogate (`ED A0 80`), the overlong forms of three and four
  bytes (`E0 80 80`, `F0 80 80 80`) and the characters above U+10FFFF
  (`F4 90 80 80`), which the runtime (`from_utf8`) refuses; `stringOf`
  then panicked on them. Found while measuring `http.text`. Its effects:
  - the server (`serve`, `serveOn`, `serveAction`): a request with such
    bytes in its head passed the check of its frame, then `stringOf` of a
    header panicked (`http.isc`, line 711): one request ended the program
    (a refusal of service from any client);
  - the client: a head of response with such bytes panicked instead of
    `Malformed("the head is not text")`, and `text` of such a body
    panicked instead of giving `?` for its bytes;
  - `sys.readText` of such a file panicked instead of `not UTF-8`;
  - `json`: its check reads the bytes of a text, already UTF-8 (no
    effect).
  Now the fold keeps the range of the next continuation by the lead byte
  (`E0`: A0 to BF, `ED`: 80 to 9F, `F0`: 90 to BF, `F4`: 80 to 8F, the
  others 80 to BF), as RFC 3629 and the runtime. Tests: `text` of 65536
  bodies of a lead byte from 0x80 and three bytes at the edges of the
  ranges, against Rust's `from_utf8` (`text_of_bodies_at_the_edges_of_utf8`,
  compiled and interpreted; the old module panics); `readText` of 9 files
  (`read_text_refuses_bytes_not_utf8`); the client against a head and a
  body with a surrogate (`client_against_a_server_of_the_test`); the
  server against a header with a surrogate (400, the server going on) and
  a header of UTF-8 (`server_on_requests_of_every_form`; with the old
  module the server panics).
- **D452 — Counts of bytes 64 at a time on AVX-512 BW (2026-11-07).**
  `http.text.1k` 3.4 times its floor. One call, one CPU at fixed
  frequency (`perf stat`, 8.2 M calls): 193.6 cycles; the code of the
  call 49.9 % (the count of the bytes from 0x80 of `utf8`: a load, a
  compare, `vpmovmskb`, `popcnt` and an add for each 32 bytes),
  `utf8_check` 27.2 %, `memcpy` 13.1 %, `isc_str_from_bytes` 5.2 %,
  `isc_free` 2.9 %.
  - Now a count of bytes (`count_parts`) takes 64 lanes where the target
    has masks of 64 bytes (`vpcmpb` into a mask of 64 bits, `kmovq`,
    `popcnt`): half the turns. A count of fewer than 32 elements keeps one
    masked vector of 32 lanes. The lanes of a masked vector are the low
    bits of a word (`shl`, `sub`), not a compare of `w` indices.
  - Over 1024 bytes or more, the vectors are read from a boundary of 64
    bytes, the bytes before it in one masked vector: without it the count
    of the bases of revcomp (from any offset of its input) took 1.3 % more
    user cycles (`perf stat`, 9 runs: 283.6 -> 287.3 M cycles, 562.0 ->
    561.5 M instructions; an unaligned load of 64 bytes crosses two lines
    of cache) and revcomp 27.5 -> 28.0 ms (21 runs). From 128 bytes the
    test cost `json.quote.escaped` 2.4 %; at 1024 revcomp 27.4 -> 27.3 ms
    (21 alternated runs, medians -0.89 %), k-nucleotide 616.9 -> 617.4 ms
    (7 runs, medians +0.74 %, its counts the same).
  - Rejected first: the check of 64 bytes of `utf8_check` written with a
    block of a fixed size and a test of each byte (LLVM writes it with
    `vpmovmskb` alone, but as a reduction to the largest byte inside the
    loop), then with words or'ed together (a frame of 424 bytes, gathers):
    not without `unsafe` and the intrinsics.
  - Measured: `text` alone, 7 alternated runs: 60.63 -> 51.97 ns;
    `run.py` (5 runs of 200 rounds) `http.text.1k` 63.2 -> 53.3 ns (its
    floor, a `malloc` of 1 KiB, a copy and a `free` in C, measured at 14.1,
    19.8 and 15.6 ns: 2.7 to 3.4 times), `json.quote.plain` 6.8 -> 5.0 ns.
    `json` by 9 alternated runs of the old and new binaries: `decode`
    +1.2 %, `quote.escaped` +2.0 %, `message` +2.9 %, whose code is the
    same but for the lines of its panics (`json.isc` changed by D451; the
    LLVM IR of the 12 functions with counts alone differs): the layout of
    the binary.
  - Test `counts_with_a_masked_rest` (`bitwise.rs`): lengths 0 to 140,
    200, 1000, 1100 and 1200, slices at the offsets 0 to 69 (every start
    before a boundary), the machine and AVX2; it fails when the start is
    not moved past the bytes before the boundary.
- **D453 — Release 0.4.4 (2026-11-07).** The fix of D451 (one request
  with an encoded surrogate in its head ended a server of `http`, in 0.4.3
  and before) released at once, with D450 and D452. No name removed, the
  language the same. The steps of 0.4.3 (D449). The ten programs of the
  Benchmarks Game: revcomp and k-nucleotide changed by D450 and D452,
  measured against the code of D447 (D452: within their scatter).
- **D454 — The Benchmarks Game measured again at 2047582 (2026-11-07).**
  After D450 (k-nucleotide: the names of its k-mers not checked) and D452
  (revcomp and k-nucleotide: their counts of bytes 64 at a time): the
  image built with the `iscc` of 2047582 (0.4.4), 102 checks against the
  references (the same outputs; the known difference of C++ #4 of
  mandelbrot at N = 200 only, the same as Isocron at N = 16000), measured
  in Docker (4 threads, 12 runs,
  the lowest). The fastest on 10 of 10, every time within its scatter of
  D447 (k-nucleotide 618 -> 613 ms, revcomp 27 ms, the others the same to
  1 ms). `results/benchmarks_game.md`, `benchmarks_game.tsv`, `floors.md`,
  `floors_isocron.tsv`, the table of the README and its figure made again
  from these lines.
- **D455 — Two floors of `http` without the work they describe
  (2026-11-07).** The floors of `http.text.1k` and `http.textResponse`
  (`bench/primitives/http.c`) copy a body into a block of its own
  (`malloc`, `memcpy`, `free`), as Isocron makes its text; the block is
  never read, and GCC (`-O3`) removed the three calls: the assembly of
  `http.c` had no call of `malloc` (`json.c`, `strings.c`: all theirs
  kept). The floors measured the check of the bytes alone (`text.1k`) and
  `strlen` alone (`textResponse`), and the ratios 3.4 and 6.3 compared
  Isocron with less work than its own. Now an empty `asm` takes the
  pointer of the block before its `free` (the block kept, nothing else
  added): `text.1k` floor 15.6 -> 38.3 ns, Isocron 53.3 ns, 1.39 times;
  `textResponse` floor 2.2 -> 11.3 ns, Isocron 13.8 ns, 1.22 times
  (`run.py`, 5 runs of 200 rounds); the other forms of `http` within
  their scatter. Not a gain of Isocron: a fault of the measure, closed.
  The other floors with blocks of the heap keep their calls.
- **D456 — The writes of the floors of texts kept (2026-11-07).** After
  D455, the other floors of the library whose checksum takes the length of
  a text written into a buffer, not its bytes: `http.httpDate`,
  `http.message`, `json.message`, `json.quote` (both forms), `sys.normalize`,
  `sys.join`, `sys.ioError.message`, `time.formatIso`,
  `time.formatDateTime` (its text at an offset), `time.format`. Each now
  passes its buffer to `KEEP`, an empty `asm` that may read the memory, so
  that no compiler can drop the writes. Measured (5 alternated runs of
  the old and new floors, one CPU at fixed frequency): every form within
  4.5 % (`http.header` -4.4 %, `http.textResponse` +3.5 %, `json.message`
  +4.5 %, the others within 3 %): GCC kept these writes already (the 0.92
  ns of `http.message`: its constant texts written by a few stores of
  immediates). The ratios of `results/primitives.tsv` stand; the floors
  are now safe from it.
- **D457 — The floor of `http.message` reads its errors as data
  (2026-11-07).** `http.message` 6.0 times its floor. One call, one CPU at
  fixed frequency (`perf stat`, 20.5 M calls): 21.7 cycles, 100
  instructions, no block of the heap (its text on the stack, D404); the
  code of the call 64.7 %, `isc_put_int` 17.5 % (the number of
  `TooLarge`), `isc_put_text` 12.2 %. The floor wrote
  `hput(p, 65536, 5)`: a constant of its loop, whose digits GCC wrote at
  compilation as stores of immediates, and constant texts of fixed
  lengths, where the program reads the error, its number and its text
  from an array made at run time: 0.92 ns, about 3 cycles, less than the
  work. Now the floor reads the errors from an array (their kind, text,
  length, number; the number `65536 + (r < 0)`, a value of the run), counts
  the digits of the number and copies the texts by their lengths: the
  same checksum (91474). `run.py` (5 runs of 200 rounds): floor 1.1 ->
  4.9 ns, Isocron 7.0 ns, 1.42 times. A fault of the measure, closed;
  not a gain of Isocron.
- **D458 — Padded texts only read written on the stack (2026-11-07).**
  `strings.padStart` 5.3 times its floor. One call, one CPU at fixed
  frequency (`perf stat`, 20.5 M calls): 31.9 cycles, 150 instructions;
  `isc_pad` 52.1 % (its block of the heap), `isc_free` 28.3 %, the loop
  15.6 %; the floor takes its 20 bytes from a buffer made before, and
  writes them.
  - Now `padStart` and `padEnd` whose text its scope only reads (a `let`
    that frees it, never moved, as the concatenations of D404) write it
    into the buffer of 128 bytes on the stack of that `let` when it fits:
    `isc_pad_into` (the fill of one byte, or the text itself when it is not
    shorter than the width), the heap otherwise; the drop frees the text
    only when it is not that buffer. A fill of one byte known to the
    compiler calls `isc_pad1_into`, six arguments in registers (its byte
    and its side in one; the nine of `isc_pad_into` put three on the
    stack): 9.91 -> 6.98 ns with `isc_pad_into`, 5.53 ns with it (7
    alternated runs of 2000 rounds).
  - Measured: `run.py` (5 runs of 200 rounds) `strings.padStart` 10.2 ->
    5.7 ns, 2.88 times its floor; the old and new binaries of `strings`, 9
    alternated runs: `padStart` -43.2 %, every other form within 2.7 %
    (their LLVM IR the same but the two functions of `padStart`; the
    +5.9 % of `split.6` in that run of `run.py` its scatter). The ten
    programs: their LLVM IR the same but the two declarations.
  - Test `padded_texts_on_the_stack` (`stdlib.rs`: widths of 0 to 300
    bytes around 128, fills of one byte (literal and variable), of two and
    empty, texts longer than their widths, an empty text, texts kept in an
    array and returned; the interpreter's output, 1 and 4 threads,
    Valgrind; with the sides of `isc_pad1_into` swapped it fails).
- **D459 — Texts added to a text by moves, its room read inline
  (2026-11-07).** `json.quote.escaped` 5.2 times its floor. One call, one
  CPU at fixed frequency (`perf stat`, 2 M calls): 282 cycles for a text
  of 15 bytes and 3 escapes; its loop 37.2 %, `append_parts` 28.7 %,
  `memcpy` 9.5 %, `isc_free` 5.7 %, `isc_str_append_n` 5.2 %,
  `heap::usable` 3.7 %. Each escape adds a part of a few bytes and an
  escape of one or two to the text at its last use (D351): `append_parts`
  copied each part by `copy_nonoverlapping` (a call of `memcpy`), and read
  the room of its block by `heap::usable`, marked cold and never inlined.
  Now the parts and the text moved to a larger block are copied by
  `copy_short` (moves that overlap, `memcpy` past 64 bytes, as the
  concatenations of D368), and `usable` is inlined. Measured: `quote`
  alone, 7 alternated runs: 88.22 -> 77.41 ns (250 cycles left:
  `append_parts` 40.2 %, the loop over the bytes 36.0 %, one byte at a
  time; no search by a predicate in the language to skip to the next
  escape); `run.py` (5 runs of 200 rounds) `json.quote.escaped` 100.3 ->
  81.7 ns (4.15 times), `time.format` 184.5 -> 155.1 ns (4.21), the
  other forms of `json`, `strings` and `time` within their scatter or
  faster. regex-redux 81.6 -> 81.6 ms, pidigits 299.0 -> 298.6 ms (7
  alternated runs), k-nucleotide 613.1 -> 616.1 ms (15 runs, medians
  -0.34 %), the same outputs. Test `texts_added_by_moves` (`stdlib.rs`:
  parts of 0 to 130 bytes, one to three at once, the block grown; the
  interpreter's output, Valgrind; with the longer parts cut by a byte it
  fails).
- **D460 — A parallel loop on one thread run at once (2026-11-07).**
  `json.parse` 4.8 times its floor, on one thread (`ISC_THREADS=1`, as
  every form of the library). Profile (one CPU at fixed frequency): the
  functions of the module (`buildAny` 13.9 %, `anyAt` 8.8 %,
  `stringEnd` 6.6 %, ...), and `isc_parallel_for_site` 3.6 %: at each
  parallel loop the choice of its site (the pool or the calling thread
  alone, D357) read the clock twice (`rdtsc`) and added to a word of the
  site atomically, where on one thread there is nothing to choose. Now a
  loop of at most one element, or on one thread (`ISC_THREADS=1`, WASI
  without threads, a machine of one core), runs at once: no clock read,
  nothing written to the words of its site; on more threads the same
  choice as before. Measured (the old and new binaries of `parse` and
  `decode`, 7 alternated runs): one thread `parse` 828.7 -> 785.6 ns
  (-5.2 %), `decode` 448.1 -> 445.4; four threads 614.1 -> 612.1 and
  246.2 -> 245.7 (the same). `run.py` (5 runs of 200 rounds): `time.formatIso`
  -10.2 %, `sys.isAbsolute` -4.9 %, `sys.normalize` -3.7 %,
  `json.stringify` -3.5 %, the other forms of `json`, `http`, `sys` and
  `time` within their scatter (`sys.env` 20.1 -> 21.3 ns, its floor the
  same: the old and new binaries of `sys`, 9 alternated runs, 15.73 ->
  15.67 ns, every form within 2.7 %; the time of `env` in `run.py` goes
  from 19.6 to 28.1 ns from one run to the other, D450, D452). Test
  `loops_on_one_thread_run_at_once` (`parallel.rs`: loops of 0, 1, 2 and
  1000 elements, nested, on 1 and 4 threads, the interpreter's output).
- **D461 — `httpDate`: its fields from the table of pairs, its day of the
  week from the days (2026-11-07).** `http.httpDate` 4.4 times its floor.
  One call, one CPU at fixed frequency (`perf stat`, 8.2 M calls): 183
  cycles, 651 instructions, 89.8 % in the loop (the date, the template):
  the samples on divisions by constants (`imul`, `shr`) and the writes of
  the parts. Each field of two digits was two parts, `x / 10` and
  `x % 10` (D402), and `time.weekday` found the days since 1970 again
  from the year, month and day that `toDateTime` had just made from them;
  the floor takes the day of the week from its days.
  - Now each field is `toString(x).padStart(2, "0")`, which a template
    writes as a pair of the table of digits (D446), and the day of the
    week is `(days + 3) mod 7` of the days of the instant (floored), the
    division of `toDateTime`. The same texts: test
    `http_dates_against_a_reference` (`std_http.rs`: 20000 instants over
    the range of `i64`, none computed past it, against a reference in
    Rust, compiled and
    interpreted; with the days not floored below 1970 it fails), and
    `http_dates_by_their_digits`.
  - Measured: `httpDate` alone, 7 alternated runs: 57.20 -> 48.01 ns (the
    pairs), 37.48 ns (and the day of the week); the old and new binaries
    of `http`, 9 alternated runs: `httpDate` 56.90 -> 37.76 ns (-33.6 %),
    the others within 2.7 % (`headerOf` +2.7 %, its code the same but the
    lines of `http.isc`); `run.py` (5 runs of 200 rounds) 56.9 -> 37.7 ns,
    2.93 times its floor.
- **D462 — `time.format`: one addition a field, its fields compared byte
  by byte; its floor reads its pattern (2026-11-07).** `time.format` 4.2
  times its floor. One call, one CPU at fixed frequency (`perf stat`, 4.1
  M calls): 512 cycles, 2419 instructions; the loop 45.3 %,
  `append_parts` 28.9 %, `isc_str_reserve` 10.4 %, `isc_str_append`
  6.5 %. Each field added two texts to the text made so far: the literal
  text before it (`acc + p.slice(from, i)`), then the field (a template
  of its own); `YYYY` and `SSS` were found by a `count` over an `iota` of
  4 and 3 bytes.
  - Now the literal text and the field are added by one template
    (`withField(acc, lit, d, c)`), and the bytes after the first of a field
    are compared one by one. The same texts: test
    `time_format_patterns_as_before` (`stdlib.rs`: fields cut short, side by
    side, `Z` at every place, an empty pattern, offsets of 0, -754 and +330
    minutes, against the texts of the version before it; with `SSS` found
    by two bytes it fails), and the tests of `time` before.
  - The floor (`bench/primitives/time.c`) wrote the fields in the order of
    the pattern of the measure, known at compilation, without reading it,
    and computed the date again in its loop, where the program gives
    `format` a `DateTime` made before and reads its pattern: now the fields
    are made before the loop and the pattern, read at run time
    (`volatile`), is read byte by byte by the cheapest test of each field;
    the same checksum (122880). Floor 25.6 -> 28.3 ns on one run outside
    the fixed frequency; `run.py` 36.8 -> 41.0 ns.
  - Measured: `format` alone, 7 alternated runs: 167.15 -> 134.40 ns (a
    noisy run, from 128.0 to 172.8); `run.py` (5 runs of 200 rounds)
    `time.format` 155.5 -> 126.0 ns, 3.07 times its floor (4.22 before: the
    program -19 %, its floor +11 %); the other forms of `time` within their
    scatter.
- **D463 — `readText` checks ASCII by vectors first (2026-11-07).**
  `sys.readText.4k` 3.3 times its floor. The same system calls as the
  floor (`strace -c`: per file one `openat`, two `read`, one `close`); the
  check of `utf8` of `sys.isc` folded each of the 4096 bytes through its
  state (D451), where the one of `http.isc` counts the bytes from 0x80 by
  vectors first (D396) and folds only a text that has some. Now the check
  of `sys` does the same: an ASCII text is taken without the fold.
  Measured (the old and new binaries of `sys`, 7 alternated runs):
  `readText.4k` 6420.90 -> 3273.28 ns (-49.0 %), the other forms within
  their scatter (from -6.5 % to +3.3 %, their code the same but the lines
  of `sys.isc`); `run.py` (5 runs of 200 rounds) 6409.3 -> 3227.3 ns,
  1.68 times its floor. Tested by `read_text_refuses_bytes_not_utf8` (D451:
  texts with bytes from 0x80, through the fold) and the tests of `sys`
  (texts of ASCII).
- **D464 — `path.normalize` reads its parts by their places
  (2026-11-07).** `sys.normalize` 3.3 times its floor: `normalize` split
  the path into strings (`split("/")`: an array and a string for each of
  the 8 parts of the measured path), then kept or dropped each, where the
  floor reads each part where it is, by its place. Now `normAt` finds the
  next `/` from a place, tells `.` and `..` by their bytes, and writes a
  part (`p.slice(i, e)`, a part of the text added to the result) only when
  it is kept. The same texts: test `normalize_by_the_places_of_its_parts`
  (`std_sys.rs`: the 2801 paths of 0 to 4 parts among "", ".", "..", "a",
  "bc", ".x" and "...", with and without a root, against a reference in
  Rust of the rules before, which the old module gives line for line; with
  `..` told by its first byte alone it fails). Measured (the old and new
  binaries of `sys`, 7 alternated runs): `normalize` 168.03 -> 91.30 ns
  (-45.7 %), the other forms within 3.3 %; `run.py` (5 runs of 200 rounds)
  164.9 -> 90.2 ns, 1.82 times its floor (`isAbsolute` +6.1 %, 1.9 ->
  2.0 ns, its code the same).
- **D465 — `sys.isAbsolute`: the gap is the layout of an array of strings
  (2026-11-07).** `sys.isAbsolute` 3.4 to 3.6 times its floor (2.0 ns
  against 0.6). Its loop calls nothing: `startsWith` and the bytes tested
  inline. The floor reads its 4096 paths in one array of 4096 rows of 48
  bytes (`qs[4096][48]`, contiguous, tested up to their zero byte), where a
  program of Isocron holds an array of strings: a pointer and a length for
  each, its bytes in a block of its own (64 bytes aligned). Measured
  (`bench/primitives/isabsolute_layout.c`, one CPU at fixed frequency, 3
  runs of 2000 rounds): the same test in C on that layout 1.886, 1.926,
  1.899 ns; on the rows with `strlen` 2.651, 2.654, 2.650 ns; Isocron
  1.889, 1.876, 1.854 ns. Isocron is the machine on its own layout; the
  rest is that layout (a line of the cache a string where the rows share
  them), which no change of the code makes contiguous: the representation
  of strings, as the lines of k-nucleotide (TODO). Closed by this measure.
- **D466 — A byte searched in a short text by windows of 16 bytes
  (2026-11-07).** `http.parseUrl` 3.2 times its floor. One call, one CPU
  at fixed frequency (`perf stat`, 4.1 M calls): 198 cycles; the loop
  42.3 %, `find_bytes` 10.9 % and `memchr` (`find_avx2`) 10.4 % (the
  searches of `://` and `:` in texts of 17 to 40 bytes), `isc_str_new`
  8.0 %, `isc_free` 6.8 %, `isc_to_case` 6.8 %, `memcmp` 6.0 %. `memchr`
  set up its vectors for a few bytes, and the rest of a needle of a few
  bytes was compared as slices, a call of `memcmp`.
  - Now `find_bytes` (`indexOf`, `includes`, `split`, `replaceAll` of a
    text) finds a byte in at most 64 bytes by windows of 16 compared at
    once (`vpcmpeqb`, `vpmovmskb`, read where they are, the last over the
    end of the one before), fewer than 16 one by one, and `memchr` beyond;
    the rest of a needle is compared byte by byte. `find_byte_short` is a
    call of its own: inlined, `indexOf` of a byte in a line of 60 lost its
    gain (6.72 -> 6.62 ns, against 5.93 called).
  - Measured (the old and new binaries of `strings` and `http`, 7
    alternated runs): `includes.3` 24.95 -> 20.18 ns (-19.1 %),
    `indexOf.1` 6.75 -> 5.93 ns (-12.1 %), `parseUrl` 61.17 -> 58.57 ns
    (-4.3 %), `replaceAll.1` -3.7 %, every other form within 2.5 %;
    `run.py` (5 runs of 200 rounds) `parseUrl` 61.1 -> 58.7 ns (3.05 times
    its floor), `includes.3` -19.3 %, `indexOf.1` -11.0 %, `replaceAll.1`
    -4.4 % (`json.message` +4.9 %, no search in it; `sys.env` +11.6 %, its
    scatter of D450). The ten programs do not call it (regex-redux replaces
    by patterns). Test `find_bytes_is_a_search_byte_by_byte`
    (`isc-runtime-core`: texts of 0 to 70 bytes, needles of 1 to 17 bytes
    at every place, false starts, against a search byte by byte; with the
    last window a byte off, or the place of a window, it fails).
- **D467 — Doubles written by Schubfach (2026-11-07).** `json.stringify`
  3.2 times its floor. Profile (one CPU at fixed frequency): the writing
  of its numbers by `core::fmt` a third of its cycles (Grisu,
  `format_shortest_opt` 21.9 %, its fallback and formatter
  `float_to_decimal_common_shortest` 7.6 %, `isc_to_string_f64` 2.9 %,
  `fmt::write` and the parts of `Formatter` 3 %), where its floor writes
  them by Schubfach (D415).
  - Now `write_f64` of the runtime (`print`, templates, `toString`,
    `json`; the interpreter and compiled code alike) writes a finite double
    by Schubfach (`f64text.rs`, ported from the floor, its table of 617
    pairs of words from `tools/gen_pow10_rs.py`, no `unsafe`), laid out as
    Rust's `Display`. Two differences from Schubfach found by the test and
    kept to `Display`: a value halfway between two texts of its length
    takes the larger (2^-25: `...313`, where Schubfach takes the even
    `...312`), and a subnormal of a few bits takes as few digits as read
    back to it (20 times the least double: `1e-322`, Schubfach `99e-324`:
    it tries a digit less only from a scaled value of 100), by reading
    the shorter candidates back (`parse` of `core`).
  - Test `shortest_as_display` (`isc-runtime-core`): the text of Rust's
    `Display` for zeros, the 100000 least subnormals, the powers of 2 from
    -1074 to 1023 and of 10, the integers around 2^53, the largest and
    smallest values, and two million doubles of random bits.
  - Measured (the old and new binaries of `json` and `strings`, 7
    alternated runs): `json.stringify` 366.17 -> 283.56 ns (-22.6 %),
    `json.encode` 114.44 -> 88.52 ns (-22.6 %), the other forms within
    their scatter; `run.py` (5 runs of 200 rounds) `json.stringify` 368.0
    -> 284.1 ns (2.48 times its floor), `json.encode` 115.7 -> 88.4 ns
    (1.08). The ten programs write their doubles by `toFixed` (another
    path): their code the same.
  - The suite of this change failed once in `tcp_and_dns_on_the_local_machine`
    (`std_net.rs`, the interpreter): its connection to the port of a
    listener just closed, to be refused, was accepted: the port freed and
    taken by a test run at the same time (4 at once in the suite); alone
    it passed 5 times of 5. Now the refusal is tested at the port 1, where
    nothing listens.
- **D468 — Release 0.4.5 (2026-11-07).** After 0.4.4: D455 to D467, the
  texts, dates, paths and numbers of the library faster by general
  changes, the same results; the floors of four forms corrected. No name
  removed, the language the same. The steps of 0.4.3 (D449): the Windows
  tests in the VM with the `iscc.exe` of the release's sources, the
  archives built from those sources in the images of the releases, tried
  before they are published. The ten programs of the Benchmarks Game:
  their LLVM IR the same as at D454 but two declarations (D458).
- **D469 — The checks of `publish.sh` read the whole file (2026-11-07).**
  The dry run of 0.4.5 failed with `CHANGELOG.md of 756583d... has no
  section 0.4.5`, the section there; the real run then published (tag
  `v0.4.5` at 756583d, its four files and their sums checked). Under
  `set -o pipefail`, `git show ... | grep -q` fails when `grep -q` leaves
  at the first line found while `git show` still writes: `git show` dies
  of SIGPIPE (141) and the pipeline fails. Shown with a long output:
  `(echo "## 0.4.5 x"; seq 1 5000000) | grep -q "^## 0.4.5 "` gives 141,
  the same with `grep -c ... > /dev/null` 0. Now both checks of the commit
  of a release (`Cargo.toml`, `CHANGELOG.md`) count with `grep -c`, which
  reads its whole input.
- **D470 — The Benchmarks Game measured again at f6a0533 (2026-11-07).**
  After 0.4.5 (D455 to D469: the runtime changed by D459, D460, D466 and
  D467; the LLVM IR of the ten programs the same as at D454 but two
  declarations): the image built with the `iscc` of f6a0533, 102 checks
  against the references (the same outputs; the known difference of C++
  #4 of mandelbrot at N = 200 only), measured in Docker (4 threads, 12
  runs, the lowest). The fastest on 10 of 10, every time within 5 ms of
  D454 (n-body 1880 -> 1885 ms, binary-trees 661 -> 659 ms, pidigits 300
  -> 298 ms, the others the same to 1 ms). `results/benchmarks_game.md`,
  `benchmarks_game.tsv`, `floors.md`, `floors_isocron.tsv`, the table of
  the README and its figure made again from these lines.
- **D471 — `json.parse` and `json.decode` against C with the
  representation of Isocron (2026-11-07).** `json.parse` 4.7 and
  `json.decode` 4.8 times their floors. Profile of `parse` alone (one CPU
  at fixed frequency): `buildAny` 23.8 %, `anyAt` (the check) 13.0 %, the
  blocks of its strings and containers made and freed about 27 %
  (`isc_str_new` 8.2 %, `isc_free` 6.3 %, `isc_free_strs` 5.1 %,
  `isc_alloc` 4.0 %, `isc_rec_drop_0` 3.4 %), the index and the check of
  the text about 13 %. The floor (`json.c`) keeps its strings unescaped in
  one buffer and its nodes in one tape, both used again from one parse to
  the next: no block made or freed; a value of Isocron owns each of its
  strings and the arrays of each container. Measured: the floor with that
  representation (`ISC_JSON_OWNED=1 json <rounds>`: after each parse each
  string copied into a block of its own, each array its values, each
  object its keys and values, 7001 blocks a document, all freed; after
  each decode the records in an array of their own and each name in a
  block of its own), one CPU at fixed frequency, 3 runs of 200 rounds:
  `parse` 170.1, 170.2, 171.6 ns, owned 378.3, 374.1, 374.5 ns; `decode`
  91.8, 92.0, 91.8 ns, owned 118.1, 117.7, 118.2 ns. Isocron (781 ns,
  447 ns) is 2.1 times C with its representation for `parse`, whose
  representation is a factor of 2.2 of its 4.7; 3.8 times for `decode`,
  whose gap is its code (a check of the members, then their building, by
  the codec of the type): open (TODO).
- **D472 — `parseUrl` tells its scheme by its bytes (2026-11-07).**
  `http.parseUrl` 3.05 times its floor; `isc_to_case` 6.8 % of its cycles
  (D466): the scheme copied into a new text in lower case, then compared
  with `http` and `https`, the two only texts it can be. Now its 4 or 5
  bytes are compared in any case (`| 32`, which only `h`/`H`, `t`/`T`,
  `p`/`P`, `s`/`S` give for those letters) and the literal `http` or
  `https` is kept. The same results: test `url_schemes_in_any_case`
  (`std_http.rs`: schemes in mixed cases, `httpx`, `htt`, `httpss`, `h`,
  an empty one, a colon without its slashes, against the texts of the
  version before it; with the fifth byte not tested it fails). Measured
  (the old and new binaries of `http`, 9 alternated runs): `parseUrl`
  59.87 -> 57.38 ns (-4.2 %), the others within 2.4 %; `run.py` (5 runs
  of 200 rounds) 58.7 -> 57.2 ns, 2.98 times its floor.
- **D473 — The next redesign, and a lead that was not one (2026-11-07).**
  After D472 the forms of the library above 3 times their floors need a
  redesign or the author: `json.decode` and `json.parse` (D471),
  `json.quote.escaped` (its bytes one at a time: no search by a predicate
  in the language), `sys.isAbsolute` (explained, D465), `time.format`
  3.14 and `json.quote.plain` 3.11. Chosen next, without a question to the
  author (the rules of the project): the decoder of `json` in one pass, the general one of
  them (no licence, no change of the model of ownership). Its profile
  (`decode` alone, one CPU at fixed frequency): `codecBuildMembers1` 20.9
  %, `codecMembers1` (the check) 14.7 %, `codecField1` 7.7 % (the key of a
  member found in both passes), `stringEnd` 8.3 %, the blocks of its
  strings 10.5 %; the code of the call (the index of the text, the array
  of the records) 26.1 %. One pass would save at most the check and one
  finding of each key, about 20 %: `decode` about 3.9 times its floor, not
  below 3; the index by `scan` and the building of the elements by `map`
  (for the threads) stay. Not a lead: the slices of a text read where they
  are are already views (D303, `str_slice_view`); the call of
  `isc_str_slice` in their code is the path of their panic only, and the
  strings made by `parseUrl` and `parse` are those they return.
- **D474 — The decoder of `json` in one pass: rejected after measure
  (2026-11-07).** The choice of D473. First a guard: the test
  `json_decode_faults_as_before` (`stdlib.rs`,
  `std_json_decode_faults.isc` and `.out`): every document one byte away
  from three valid ones (an array of records; a record of an option, a
  tuple, an array of records of arrays and an array of options; a tuple at
  the top), each byte removed then replaced by each of 12 bytes, 2499
  results written by the decoder of 0.4.5 (265 values, the rest errors
  with their places and paths), compiled and interpreted. Then each form
  given a reader of its value, its end and its fault (`codecRead`): the
  members of a record by one loop (each key found once, its value read
  by its field's reader), the elements of an array and of a tuple read
  once after the checks of their structure in their order; the 2499
  results the same. Measured (`decode` of 1000 records, 7 alternated
  runs, one CPU at fixed frequency): 447.65 -> 458.85 ns (+2.5 %). The
  check saved, the elements of an array are tuples of their value, end
  and fault, whose values are copied out of them (a string each record):
  a prototype, 1000 records of a name made directly or taken out of such
  tuples, 10.1 against 19.0 ns each. The decoder stays a check then a
  building; the test stays. Taking a value out of the elements of an
  array that dies without copying it would be the change of the compiler
  that makes one pass pay (TODO).
- **D475 — The waiting of reverse-complement and the 17 cycles of n-body
  measured again on the code of 0.4.5 (2026-11-07).** The two gaps of
  D432 and D433, measured again with the binaries of the compiler of
  374dc5f, fixed frequency:
  - n-body (one thread, 50 million steps, `perf stat` of 3 runs): 121.20
    cycles and 243.10 instructions a step; the ports 0, 1, 5 and 6 78.62,
    79.62, 48.83 and 1.03 uops a step, the loads 28.03, the stores 14.02
    and 14.02: the work of D433 (121.65, 243.00, 78.58, 79.69, 48.76,
    1.00, 28.00, 14.00, 14.00) and of the model of D312 to a tenth of a
    uop: the same loop on the ports 0 and 1 and the divider.
  - reverse-complement (4 threads, `perf record` of user cycles at 20 kHz,
    20 runs, 18043 samples): `isc_task_9` 60.31 %, `isc_task_4` 30.94 %;
    the waiting of the pool 7.07 % (`worker` 5.25 % on the three other
    threads, `parallel_job` 1.82 % on the thread of the body), the body
    alone 0.65 %: the picture of D432 (5.11, 1.08, 0.87 %), the ends of
    its four parallel phases (the last thread finishing its contiguous
    range) and its sequential parts; no clearing of pages.
- **D476 — The Benchmarks Game measured again at 7667fbd (2026-11-07).**
  After D471 to D475 (`http.isc` alone changed in `crates/` since f6a0533,
  D472; the ten programs import no module): the image built with the
  `iscc` of 7667fbd, 102 checks against the references (the same outputs;
  the known difference of C++ #4 of mandelbrot at N = 200 only), measured
  in Docker (4 threads, 12 runs, the lowest). The fastest on 10 of 10,
  every time within its scatter (n-body 1885 -> 1896 ms, its code the
  same: from 1880 to 1896 ms since D447; the others within 1 ms).
  `results/benchmarks_game.md`, `benchmarks_game.tsv`, `floors.md`,
  `floors_isocron.tsv`, the table of the README and its figure made again
  from these lines.
- **D477 — Integers padded to 3 and 4 digits by zeros written from the
  table of pairs (2026-11-07).** `time.format` 3.14 times its floor;
  profile (one CPU at fixed frequency): 407 cycles a call, `isc_put_int`
  6.9 % (its year padded to 4, its milliseconds to 3, by zeros, through a
  call; D446 writes those padded to 2 from the table of pairs). Now a
  template writes an integer padded to 3 by zeros, not negative and below
  1000, as a digit then a pair, and one padded to 4, below 10000, as two
  pairs; the others call the runtime as before. `isc_put_int` left the
  profile of `format`. Measured (the old and new binaries of `time`, 15
  alternated runs): `formatDateTime` 51.68 -> 49.06 ns (-5.1 %),
  `formatIso` 59.46 -> 57.85 ns (-2.7 %), `format` 126.00 -> 125.35 ns
  (-0.5 %); `parseIso` 33.69 -> 36.48 ns (+8.3 %) in that binary, whose
  code is the same (four functions of formatting alone differ, by their
  division by 100): alone in a binary of its own, built by the old and the
  new compiler, 9 alternated runs, 35.08 -> 34.16 ns: the place of its
  code among the others. `run.py` (5 runs of 200 rounds): `format` 127.4
  -> 125.6 ns (3.12 times its floor), `formatDateTime` -5.2 %, `formatIso`
  -3.1 %, `parseIso` 34.0 -> 38.1 ns (1.62 times its floor, the same
  place). The LLVM IR of the ten programs unchanged. Test
  `small_integers_in_templates` (`strings.rs`) with integers from -70 to
  about 64000 padded to 3 and 4 by zeros, both sides of 1000 and 10000,
  for the machine and for AVX2 (with the second pair of the 4 written a
  byte early it fails).
- **D478 — The short texts of a template copied in its code
  (2026-11-07).** `json.quote.plain` 3.11 times its floor; one call, one
  CPU at fixed frequency (`perf stat`, 82 M calls): 15.6 cycles, 70
  instructions; the hot samples on the call of `isc_put_text` that copies
  the text between the quotes and the `vzeroupper` the call needs before
  it (code of AVX calling code without it), as many as the count of its
  bytes to escape.
  - Now a text of a template (a concatenation, a template, a padded text
    of D458 aside) of at most 64 bytes is copied in the generated code by
    moves that overlap, as `copy_short` of the runtime: two of 32, 16, 8 or
    4 bytes, or its first, middle and last bytes; longer, the call as
    before.
  - Rejected first: a masked load and store of 32 bytes on AVX-512 BW.
    `json.quote.plain` 4.863 -> 4.372 ns, but `http.parseUrl` +5.7 % and
    `strings.concat` +10.2 % (the old and new binaries, 7 alternated runs):
    a masked store does not forward its bytes to the loads that read them
    back soon after, as `parseUrl` reads the texts it made; with plain
    moves `parseUrl` the same. With the moves up to 32 bytes `concat` was
    still +4.0 % (its parts of 60 bytes paid a test more before the call);
    up to 64, -17.6 %.
  - Measured (the old and new binaries of `strings` and `http`, 7
    alternated runs): `strings.concat` 2.66 -> 2.19 ns (-17.6 %), `split.6`
    80.86 -> 75.95 ns (-6.1 %), `http.message` -6.4 %, `http.httpDate`
    -4.6 %, the others within 2.7 %; `quote` of a name alone 4.863 -> 4.515
    ns (9 runs). `run.py` (5 runs of 200 rounds): `time.format` 125.6 ->
    112.1 ns (2.78 times its floor), `json.quote.plain` 5.1 -> 4.5 ns
    (2.75), `json.message` -13.2 %, `time.parseIso` -9.2 %, `json.encode`
    -5.1 %, `time.formatIso` -5.5 %; the forms of `strings` of 1.5 to 2 ns
    from -10 % to +12 %, their scatter (within 2 % alternated). k-nucleotide
    and regex-redux, whose outputs are templates (their other code the
    same): 619.4 -> 620.3 ms and 82.0 -> 82.2 ms (15 alternated runs,
    medians +0.50 % and +0.42 %), the same outputs. Test
    `short_texts_of_templates_by_masks` (`strings.rs`: parts of 0 to 70
    bytes and of 100, both sides of every size of move, on the stack and in
    a block, for the machine and AVX2; with the moves of 32 a byte early it
    fails).
- **D479 — The parts of a path searched from the end: rejected after
  measure (2026-11-08).** `sys.dirname` about 3.0 times its floor: its
  last `/` and the `/` at the end of its path found by folds over all its
  bytes (`lastByte`, `trimEnd`, three folds). First a guard: the test
  `path_parts_as_before` (`std_sys.rs`, `std_path_parts.isc` and `.out`):
  `dirname`, `basename` and `extension` of every path of 0 to 3 parts among
  9, with and without a root and a `/` at the end, 820 lines written by the
  module of 0.4.5, compiled and interpreted. Then `lastByte` and `trimEnd`
  searched from the end, one byte at a time (the same texts). Measured (the
  old and new binaries of `sys`, 7 alternated runs): `basename` 9.17 ->
  7.42 ns, `extension` 12.41 -> 10.47 ns, `normalize` -3.2 %, but
  `dirname` 8.71 -> 9.91 ns (+13.8 %); with `trimEnd` by its fold again,
  `dirname` +9.1 %: for its path of 23 bytes the fold in vectors costs less
  than 8 steps one at a time. `dirname` is the form above 3: the module
  stays as it was; the test stays.
- **D480 — The Benchmarks Game measured again at 0d8a28d (2026-11-08).**
  After D477 (no change of the ten programs) and D478 (the outputs of
  k-nucleotide and regex-redux, templates, copied in their code): the
  image built with the `iscc` of 0d8a28d, 102 checks against the
  references (the same outputs; the known difference of C++ #4 of
  mandelbrot at N = 200 only), measured in Docker (4 threads, 12 runs, the
  lowest). The fastest on 10 of 10, every time within its scatter
  (k-nucleotide 613 -> 614 ms, regex-redux 81 -> 82 ms, n-body 1896 ->
  1885 ms, the others within 1 ms). `results/benchmarks_game.md`,
  `benchmarks_game.tsv`, `floors.md`, `floors_isocron.tsv`, the table of
  the README and its figure made again from these lines.
- **D481 — The escapes of `json.quote` found by `filter`: rejected after
  measure (2026-11-08).** `json.quote.escaped` 4.15 times its floor; its
  escapes found by a recursion over every byte (D473: no search by a
  predicate in the language). First a guard: the test
  `json_quote_against_its_rules` (`stdlib.rs`: every text of 0 to 4
  characters of an alphabet of a letter, `"`, `\`, a newline, a carriage
  return, a tab, the controls 1 and 31, `é` and a space, 11111 texts,
  against the rules of JSON written in the test, compiled and
  interpreted). Then the places to escape found at once by `filter` (its
  predicate in vectors) and the text between them added from one to the
  next (the same texts). Measured (`quote` of 4096 texts of 15 bytes and 3
  escapes, the old and new binaries, 9 alternated runs): 77.44 -> 83.19 ns
  (+7.4 %): the array of the places (a block, its compression) costs more
  than 15 bytes read one at a time. The module stays; the test stays.
- **D482 — `s.lastIndexOf(t)`, and the parts of a path by it
  (2026-11-08).** `sys.dirname` about 3.0 times its floor: 27.5 cycles and
  118 instructions a call, nothing made on the heap; its last `/` and the
  last byte of its path that is not `/` found by folds of the index of the
  last match (D479), indices of 64 bits for bytes, then a reduction across
  the vector, where the floor calls `strrchr`. The language had no search
  from the end. Now a text has `lastIndexOf` (as TypeScript: the byte index
  of the last `t`, -1 if none, the length of the text for an empty `t`),
  decided without a question to the author: one byte by windows of 16
  bytes from the end up to 64 bytes (`find_last_byte_short`, as
  `find_bytes` of D466; `memrchr` set up its vectors for a few bytes:
  `extension` +16.6 % with it), `memrchr` beyond, `memmem::rfind` for more
  bytes; the interpreter and compiled code by `isc-runtime-core`. In the
  guide (`docs/language.md`, its example run: `(7, -1, 8)`), in SPEC
  section 10's list, in the grammar of VS Code. `path` finds the last `/`
  of `basename`, `dirname` and `normalize` by it; `extension` keeps its
  fold for the dot of a name of a few bytes (the call +12.7 % there).
  Measured (the old and new binaries of `sys`, 7 alternated runs):
  `basename` 9.18 -> 7.32 ns (-20.2 %), `extension` 12.39 -> 10.62 ns
  (-14.3 %), `dirname` within 2 %, `envAll` -8.4 %; `run.py` (5 runs of
  200 rounds) `basename` -20.1 %, `extension` -14.4 %, `dirname` 8.7 ->
  8.6 ns (2.96 times its floor), `isAbsolute` 1.8 -> 2.0 ns (its code the
  same: from 1.8 to 2.0 ns from one run to the other, D465). The LLVM IR of
  the ten programs the same but a declaration. Tests
  `last_index_of_texts` (`strings.rs`: a byte in texts of 0 to 70 bytes at
  every place, needles of two bytes and empty, overlapping, characters of
  two bytes; the interpreter's values), `last_index_of_is_a_search_from_the_end`
  (`isc-runtime-core`: against a search byte by byte; with the window a
  byte off it fails), `path_parts_as_before` (D479) and
  `normalize_by_the_places_of_its_parts` (D464).
- **D483 — The Benchmarks Game measured again at 543d932 (2026-11-08).**
  After D481 (no change of the code) and D482 (`lastIndexOf`; the LLVM IR
  of the ten programs the same but its declaration): the image built with
  the `iscc` of 543d932, 102 checks against the references (the same
  outputs; the known difference of C++ #4 of mandelbrot at N = 200 only),
  measured in Docker (4 threads, 12 runs, the lowest). The fastest on 10
  of 10, every time within its scatter (k-nucleotide 614 -> 619 ms, its
  code the same: from 613 to 622 ms since D429; regex-redux 82 -> 81 ms;
  the others within 1 ms). `results/benchmarks_game.md`,
  `benchmarks_game.tsv`, `floors.md`, `floors_isocron.tsv`, the table of
  the README and its figure made again from these lines.
- **D484 — Release 0.4.6 (2026-11-08).** After 0.4.5: D469 to D485, a
  search from the end for texts (`lastIndexOf`, D482), the short texts of
  a template copied in its code (D478), integers padded by zeros from the
  table of pairs (D477), `parseUrl` by the bytes of its scheme (D472);
  measures that explain `json.parse` and `json.decode` (D471) and the two
  gaps of D432 and D433 (D475); three leads rejected after measure, their
  guards kept as tests (D474, D479, D481); the key of a server of HTTPS
  on Windows in the store of the machine (D485). No name removed; one
  method added. The steps of 0.4.3 (D449): the Windows tests in the VM
  with the `iscc.exe` of the release's sources, the archives built from
  those sources in the images of the releases, tried before they are
  published. The ten programs of the Benchmarks Game measured at 543d932
  (D483); nothing in `crates/` changed since but the version and the code
  of TLS for Windows (`mod schannel`, D485), which they do not reach.
- **D485 — The key of a server of HTTPS on Windows in the store of the
  machine (2026-11-08).** The Windows tests of 0.4.6 (the `iscc.exe` of
  8541507, the VM just started) failed on `https_server_on_windows`:
  `serve failed: tls: the certificate or the key of the server:
  0x8009030d of SChannel`, in the log of the system the event 36870 ("a
  fatal error occurred when attempting to access the TLS server credential
  private key ... the internal error state is 10001"), as once at D365,
  where it was left unexplained.
  - Measured in the VM: the whole file of the Windows tests at once after
    a start of the VM failed 3 times of 3; the pair alone, started after a
    first PowerShell (59 to 63 s after the start), 6 of 6 passed; the
    client of the tests then the pair, after 90 s, with `KeyIso` started
    by hand, 4 of 4 passed; the pair alone at once when `ssh` answers,
    3 of 3 failed (started 34.7 s after the start, refused 1.0 s later).
    The log of the starts of the night (00:28 to 01:22, the clock of the
    VM): the 7 failures from 35.2 to 58.1 s after a start, from 9.9 s
    before to 6.2 s after the session of the user opened with its password
    by `AutoAdminLogon` (event 4624 of type 2, 38.7 to 62.1 s after the
    start); no failure of a server started later.
    Started 1.76 s after that session (seen by `quser`), refused; 1.79 s
    after, served.
  - So `lsass`, where SChannel uses the key of a server (D343), reads a
    key of the store of the user (CNG, protected by DPAPI with the secret
    of the user) only a few seconds after a session of that user opened
    with its password, more under load (6.2 s while the 14 tests ran); a
    session of `ssh` by a key (type 3) gives no password. A server started
    by `ssh` with a key, or by a task, on a machine where the user has no
    such session, was refused.
  - Now the key is imported into the store of the machine
    (`CRYPT_MACHINE_KEYSET`, protected by the DPAPI of the system), into
    that of the user when the process may not write there, and removed by
    `NCRYPT_MACHINE_KEY_FLAG` when it is the machine's; a refusal of a key
    of the user says that it needs a session opened with the password.
    Measured: a server with the key of the machine started at once when
    `ssh` answers, 4.2 and 5.6 s before the session of the user, served,
    no event 36870; the folder of the keys of the machine 3 files, 4 while
    it serves, 3 after; the whole file of the Windows tests at once after a
    start of the VM, 14 passed, no event 36870 (twice, the second with the
    count of the test below).
  - Test: `https_server_on_windows` counts the files of the keys of the
    machine before, while and after the server (`dir /a-d`: they are
    hidden files of the system; without it the count was 0): one more,
    then as before. With the key not removed (its flag of the machine
    dropped) it fails, `keys of the machine: 3 then 4`; with the key in the
    store of the user, `4 then 4 while serving` (the key left by the first
    removed from the VM after; three keys of the machine of 2026-11-05
    20:33 to 20:35, probably of the trials of D343, left there).
  - 0.4.6, not yet published, takes this change (D484, its notes).
- **D486 — The commit of a release the last of `main` (2026-11-08).**
  0.4.6 was published with its tag on its commit of release, 8541507
  (D484), and its archives built from 6795010, the commit after it that
  adds D485 (the Windows tests of the archives of 8541507 had failed):
  `publish.sh` tags the last commit of `main` whose message starts with
  `Release 0.4.6:`. The tag stays on 8541507: moving it is a forced push
  of a published tag, refused here; the author's call (TODO). The notes of
  the release on GitHub say that its archives come from 6795010 and that
  the sources of the tag lack D485 (its four files and their sums checked
  after download). Now `publish.sh` refuses a release whose commit is not
  the last of `main`: a fix after it needs a new commit `Release VERSION:`
  and archives built from it. Tried in a clone of the repository: with a
  commit after that of the release, refused with that message; with the
  commit of the release last, past that check (stopped by the next one,
  `main` not ahead of `origin/main`, as that clone was made).
- **D487 — The group of numbers measured, and `toFixed` by the integer
  of a double (2026-11-08).** The audit of the forms (TODO: "the other
  groups of forms: numbers") had no microbenchmark of the texts and the
  reading of numbers. Now `bench/primitives/numbers.isc` and its floor
  `numbers.c` (4096 values in the caches, one thread): `toFixed(2)` of
  thirds from 0.3 to 4e7 of both signs (none halfway between two numbers
  of 2 decimals), the shortest text of a double (`toString`), `parseI64`
  of integers of up to 9 digits and a sign, `parseF64` of thousandths from
  -500 to 500. The floors: `toFixed(2)` by the integer and the power of
  two of the double, `f64_text.h` (Schubfach, D415), `parseI64` by a loop
  over its known length without a check of overflow up to 18 digits,
  `parseF64` by Clinger's fast path (the digits as an integer divided by
  an exact power of ten); a first floor of `parseI64` by `strlen` and of
  `parseF64` by `strtod` was beaten by Isocron (7.06 against 7.77 ns, 14.8
  against 60.6 ns, not at a fixed frequency): wrong, replaced.
  - `toFixed` wrote the 1100 decimals of `|x|` at each call (`{:.1100}`,
    then kept `d + 1`): 1156 ns a call, 88 times its floor (not at a fixed
    frequency). Now for `d` up to 18, its first `d + 1` decimals come from
    the integer `m` and the power of two `e` of `x`: `m 10^(d+1) / 2^-e` in
    128 bits (`m < 2^53`, `10^(d+1) < 2^64`), rounded half away from zero,
    divided in 64 bits when it fits (a division of 128 bits is a call: 74
    -> 34 ns), its digits and its point written at once; `d` above 18, or
    a product beyond 128 bits, by the exact value as before.
  - Measured (the old and new binaries, 7 alternated runs, fixed
    frequency): `toFixed.2` 1609.32 -> 48.62 ns (-97.0 %), the other
    forms of the group within 3.5 % (their code the same); `run.py` (5 runs
    of 200 rounds): `toFixed.2` 49.44 ns, 2.58 times its floor;
    `parseF64` 1.56, `toString.f64` 1.38, `parseI64` 1.27. n-body,
    spectral-norm and k-nucleotide, which write by `toFixed`: the same
    outputs (n-body 1000 and 50000, spectral-norm 100 and 1000,
    k-nucleotide of 250000 lines of fasta).
  - Tests: `fixed_short_is_fixed_exact` (`isc-runtime-core`: 200000
    doubles from 1e-30 to 1e21 and the edges, `d` from 0 to 18, against
    the exact value; with `> 5` for `>= 5` it fails at `0.5` with 0
    decimals), `to_fixed_of_many_doubles` (`strings.rs`: 2000 doubles from
    1e-9 to 1e20 with 0 to 20 decimals, compiled against interpreted), and
    the cases of JavaScript of D155.
- **D488 — Numbers sorted by their bytes, and the group of sorts measured
  (2026-11-08).** The audit of the forms had no microbenchmark of the
  sorts. Now `bench/primitives/sorts.isc` and its floor `sorts.c` (4096
  values in the caches, one thread, the nanoseconds of one element and a
  checksum of the order): `sort` of integers of both signs, of doubles, of
  texts of 1 to 7 bytes, `sortBy` of those texts by their length (a key of
  few values: the order of equal keys checked), `toReversed`. The floors:
  numbers by a sort of their keys alone by their bytes (radix, as many
  bytes as their range needs; a first floor that sorted pairs of a key
  and a value was beaten by Isocron, 13.93 against 14.78 ns: wrong,
  replaced), texts by a quicksort of their bytes one place at a time
  (Bentley and Sedgewick), the key of few values by counting, a reversed
  copy.
  - `sort` and `sortBy` order by `argsort`: indices sorted by
    comparisons, each reading two elements by their indices and choosing
    their type by a `match`, the index as the last key. Measured first (not
    at a fixed frequency): `sort` of integers 3.1 times its floor, `sortBy`
    by a length 17 times. Now from 64 elements the numbers and booleans
    are sorted by their keys as unsigned integers (`radix_order` in
    `isc-runtime-core`: integers with the bit of the sign flipped, floats by
    `order_key_f64`, every NaN last and -0 before +0 as `order_f64`), a
    sort by the bytes of `key - min` from the lowest, the bytes of the range
    only, a byte that every key shares skipped: stable, so the order of a
    comparison sort with the index as the last key; a key of a few values
    is one pass of counting. Below 64 elements, and for texts, by
    comparisons as before.
  - Measured (the old and new binaries of `sorts`, 7 alternated runs,
    fixed frequency; the maintenance of Plex, a `Plex Transcoder` on
    another core, ran then): `sort.i64` 45.70 -> 13.60 ns (-70.2 %),
    `sort.f64` 50.29 -> 26.06 ns (-48.2 %), `sortBy.length` 61.09 ->
    22.40 ns (-63.3 %), `sort.string` and `toReversed` within 0.5 %.
  - `sortBy` of texts: profile (one CPU at fixed frequency): the array of
    the result filled by `isc_str_new` (a block and a copy of each text)
    and those freed by `isc_free_strs`, about 63 % of the cycles; the sort
    20 %, the keys 10 %. A text of an array owns its bytes, where the floor
    moves shared pointers. The floor with that representation
    (`ISC_SORTS_OWNED=1 sorts <rounds>`: each text of the sorted array
    copied into a block of its own, the blocks freed after), alternated
    with Isocron (5 runs, fixed frequency, the machine quiet, D489):
    `sortBy.length` Isocron 22.19 ns, the floor 3.52 (6.30 times it), the
    floor that owns 17.92 (1.24 times it); `sort.string` 121.94, 66.73
    (1.83) and 81.14 (1.50). The representation is 14.4 of the 18.7 ns of
    the gap of `sortBy` of texts (the change of the representation of
    texts, TODO). (Measured first while Plex ran: 37.09, 5.18 and 40.95;
    every side slower, the side that makes blocks more.)
  - The Benchmarks Game measured at 36c84c7 (after D487) while that
    maintenance ran (a `Plex Transcoder` from 03:22:59 to 03:39:25, the
    measure from 03:22 to 03:41): the fastest on 10 of 10, but every
    language slower (fasta 5 -> 11 ms, reverse-complement 27 -> 35 ms,
    binary-trees 660 -> 707 ms; Rust #7 of regex-redux 752 -> 781 ms, C #2
    of pidigits 681 -> 704 ms): not published, measured again on a quiet
    machine.
  - Tests: `radix_order_is_a_stable_sort` (`isc-runtime-core`: keys of
    every range, 0 to 300 keys and 5000, against a stable sort by the key;
    with the passes counted by `/ 8` it fails), `order_key_f64_is_order_f64`
    (every pair of NaNs, zeros, infinities, subnormals), and
    `sorts_of_every_type_of_number` (`stdlib.rs`: 2000 elements of each
    type of number and booleans, unsigned above 2^63, NaN, -0 and the
    infinities, keys of few values, compiled against interpreted; with the
    key of doubles by `b ^ 2^63` alone it fails at the sort of `f32`).
- **D489 — The Benchmarks Game and the sorts measured on a quiet machine
  (2026-11-08).** After the maintenance of Plex (D488; no `Plex
  Transcoder` from three minutes before to the end, the load below 1.5
  before): the image built with the `iscc` of 4ae540e (D487 `toFixed`,
  D488 the sorts of numbers), 102 checks against the references (the same
  outputs; the known difference of C++ #4 of mandelbrot at N = 200 only),
  measured in Docker (4 threads, 12 runs, the lowest). The fastest on 10
  of 10, every time within its scatter of D483 (k-nucleotide 619 -> 610
  ms, n-body 1885 -> 1873 ms, binary-trees 660 -> 662 ms, fasta 5 ms,
  reverse-complement 27 ms, the others within 1 ms; the other languages
  within 4 ms of D483: the measure of D488 was the load of Plex).
  `results/benchmarks_game.md`, `benchmarks_game.tsv`, `floors.md`,
  `floors_isocron.tsv`, the table of the README and its figure made again
  from these lines. Then `run.py` (5 runs of 100 rounds) of the group of
  sorts, no transcoder of Plex before or after: `sortBy.length` 6.24
  times its floor (its representation, D488), `sort.string` 1.81,
  `sort.i64` 1.69, `sort.f64` 1.47, `toReversed` 1.01.
- **D490 — The tasks of a fill that reads by windows of 16 KiB too
  (2026-11-08).** The question left by D321: reverse-complement was
  8.91 % faster with B = 16384, not by its reductions; "another use of B,
  to be found". Measured again on the code of D489 (the compiler built
  with B = 16384, the old and new binaries of reverse-complement, 21
  alternated runs, fixed frequency, no transcoder of Plex): -14.11 %
  [27.6 -> 23.7 ms]. B is in two places of its LLVM IR: the blocks of its
  parallel `filter` of the heads (`filter(iota(data.length), ...)`, D178)
  and the tasks of `printBytes(lines(m, ...))`, a fill that reads `bases`
  by windows (D293), which D321 had left at tasks of B elements ("the
  windows of D293 are made for tasks of B elements").
  - Blocks of 16384 elements for the parallel `filter` alone: +0.08 %
    (the same 27.5 ms): not that.
  - Tasks of 16 KiB (`fill_block`, as the other fills of D321) for the
    fill that reads by windows, its first task checked at that size and
    its widest window 4 of those tasks (`WINDOW_WIDE` in tasks): -14.31 %
    [27.5 -> 23.6 ms], the same outputs; `perf stat` (4 threads, 3 runs,
    the first cold one of the old binary left out): 563.5 -> 484.5 million
    instructions, 289 -> 238 million cycles, 105 -> 88.5 ms of CPU. A task
    filters again the blocks of 1024 of the source that its window covers,
    past both ends: about 6 blocks for 4 of output with tasks of 4096,
    about 18 for 16 with tasks of 16384, and a quarter of the tasks
    (61000 -> 15000) with their fixed costs (a model; 79 million
    instructions less measured). A window of 16 KiB of bytes and its
    margins stays on the stack (32 KiB); tasks of elements of 8 bytes the
    same (4096 elements). The LLVM IR of the nine other programs the same.
  - Test `filters_read_by_windows` (`bitwise.rs`) with sizes around the
    tasks of 16384 bytes and their windows (16383 to 16449, 32769, 65537,
    65600), 1 to 8 threads, against the bytes computed in the test and the
    interpreter.
- **D491 — The Benchmarks Game measured again at 2182c59 (2026-11-08).**
  After D490 (the tasks of the fill of reverse-complement that reads by
  windows, the LLVM IR of the nine others the same), on a quiet machine
  (no `Plex Transcoder` from three minutes before to the end, the load
  below 1.5 before): the image built with the `iscc` of 2182c59, 102
  checks against the references (the same outputs; the known difference
  of C++ #4 of mandelbrot at N = 200 only), measured in Docker (4 threads,
  12 runs, the lowest). The fastest on 10 of 10; reverse-complement 27 ->
  24 ms (Rust 64 ms), the others within their scatter (k-nucleotide 610 ->
  607 ms, n-body 1873 -> 1876 ms, the others within 1 ms).
  `results/benchmarks_game.md`, `benchmarks_game.tsv`, `floors.md`,
  `floors_isocron.tsv`, the table of the README and its figure made again
  from these lines.
- **D492 — Maps by an index of hashes, and the group of maps measured
  (2026-11-08).** The audit of the representations chosen without a
  measure (TODO): D158 laid a map out as its keys by first insertion, its
  values, and the ranks of its keys sorted, searched by bisection ("the
  same order and bits everywhere"). Now `bench/primitives/maps.isc` and its
  floor `maps.c` (4096 elements, 1000 distinct keys, in the caches, one
  thread): `countBy`, `Map(entries)`, `get` of integers and of texts that
  find, `has` of integers that do not; the floor a table of hashes by
  linear probing (a power of two, at most half full) with the keys in the
  order of their first insertion. Measured first (not at a fixed
  frequency): `get` of an integer 67.5 ns, 51 times its floor; the profile
  (one CPU at fixed frequency): the bisection, a mispredicted branch and two
  dependent loads (the rank, then the key) at each of its 10 steps.
  - The order of a map that a program sees is that of first insertion
    (`keys`, `values`, `entries`); the index is not seen. Now its third
    array is an index of hashes: a power of two of slots at least twice its
    keys, each 0 or the position of a key plus one, linear probing, the hash
    of D167 (`hash_word` of an integer widened to 64 bits as the interpreter
    holds it, `hash_bytes` of a text or a `bigint`): fixed, so the same
    bits everywhere. Two primitives, `$hashIndex(keys)` and `$hashFind(keys,
    index, k)`, over `hash_index` and `hash_find` of `isc-runtime-core`
    (the interpreter and the runtime); `$mapOf` and `$mapParts` type the
    index by a size of its own. Integer keys are probed in the generated
    code (the hash, then the slots, each key read with its type): through
    the call, `get` 7.42 ns; in the code 4.13 ns. The table of counts of
    `$groupCounts` (D317) now starts with room for min(n, 8192) keys at
    most 3/4 full: a count of 4096 keys grew once from 1024 slots, 18 % of
    its time.
  - Measured (the old and new binaries of `maps`, 7 alternated runs, fixed
    frequency, no transcoder of Plex): `get.i64` 56.08 -> 7.48 ns with the
    call, then 7.42 -> 4.13 inline; `get.string` 167.69 -> 20.13 ns;
    `has.miss.i64` 12.31 -> 8.88 -> 2.97 ns; `Map.build.i64` 5.70 -> 4.96
    ns; `countBy.i64` 9.72 -> 9.00 ns, then 9.03 -> 7.34 ns with the room.
    `run.py` (5 runs of 50 rounds): `get.string` 1.81 times its floor,
    `has.miss.i64` 1.90, `Map.build.i64` 2.13, `get.i64` 2.17,
    `countBy.i64` 3.11 (its profile: the insertion into the lines of 8 slots
    compared in vectors of D317, made for tables larger than the caches,
    57 %; the columns of the result 11.6 %; the index of the map 7.0 %; open:
    a probe of one slot for a table in the caches). k-nucleotide, which
    counts by `countBy` and reads by `get` (its LLVM IR the only one of the
    ten that changes): the same outputs, 608.0 -> 607.2 ms, then 607.9 ->
    607.6 ms with the room (15 alternated runs).
  - In the guide and SPEC section 4: a map built in O(n), `get` and `has`
    in O(1) on average.
  - Tests: `hash_index_finds_each_key` (`isc-runtime-core`: 0 to 300 keys
    and 5000, integers of every range with equal hashes modulo the table,
    texts, keys absent), `maps_of_every_class_of_key` (`stdlib.rs`: maps
    of integers of 8 to 64 bits of both signs, unsigned above 2^63,
    booleans, texts and `bigint`, 3000 entries, `get`, `has`, `set`,
    `delete`, `entries`, `countBy`, compiled against interpreted; with the
    hash of the probe in the code shifted by 30 for 29 it fails at its first
    line), and the example of the guide (the same output).
- **D493 — The Benchmarks Game measured again at 3d3b036 (2026-11-08).**
  After D492 (maps by an index of hashes: the LLVM IR of k-nucleotide, its
  counts and its lookups; the nine others the same), on a quiet machine (no
  `Plex Transcoder` from three minutes before to the end): the image built
  with the `iscc` of 3d3b036, 102 checks against the references (the same
  outputs; the known difference of C++ #4 of mandelbrot at N = 200 only),
  measured in Docker (4 threads, 12 runs, the lowest). The fastest on 10
  of 10, every time within its scatter of D491 (k-nucleotide 607 ms, n-body
  1876 -> 1880 ms, the others within 1 ms). `results/benchmarks_game.md`,
  `benchmarks_game.tsv`, `floors.md`, `floors_isocron.tsv`, the table of
  the README and its figure made again from these lines.
- **D494 — Counts by value of the keys below the number of keys
  (2026-11-08).** `countBy` 3.11 times its floor after D492: its keys
  inserted into the lines of 8 slots compared in vectors (D317, for tables
  larger than the caches), 57 % of its time. The table of counts already
  counts the keys below 65536 by their value (`small`, grown to the largest
  key seen), but only from 65536 keys on (`GROUP_DIRECT_MIN`, the room of
  that array). Now every table of counts counts by value the keys below
  `direct_max`, 65536 bounded by the number of keys (a power of two, at
  least 256): that array is at most as long as the keys counted; the keys
  above it by hashes as before; from 65536 keys on the same as before.
  - A first form tested that bound in the loop of `add_block` (the keys of
    a chunk that packs, sent to `add_count`): k-nucleotide +2.33 % [605.3
    -> 619.4 ms] (21 alternated runs, a quiet machine), 21.50 -> 21.98
    billion instructions, its bound being the same (65536). That test only
    sends a key to `add_count`, which decides: the loop keeps its test of
    before (`direct && k < 65536`, a constant there), and k-nucleotide
    609.1 -> 608.4 ms (-0.11 %, its outputs the same).
  - Measured (the binaries of `maps` before and after, 7 alternated runs,
    fixed frequency): `countBy.i64` 7.39 -> 3.51 ns (-52.5 %), the others
    within 2.2 %; `run.py` (5 runs of 50 rounds): `countBy.i64` 1.48 times
    its floor, `get.i64` 2.22, `Map.build.i64` 2.09, `has.miss.i64` 1.93,
    `get.string` 1.75.
  - Test `counts_by_value_and_by_hashes` (`stdlib.rs`: keys on both sides
    of the bound, 0 and negative ones, 10 to 70000 elements, 1 to 4
    threads, the keys in the order of their first element and their counts,
    against the interpreter; with the bound taken as `<=` in `add_count`
    the compiled program does not end, the test does not pass).
- **D495 — The group of arrays measured, and a `map` of a zip made one
  `map` (2026-11-08).** The audit of the forms (TODO: "arrays") had no
  microbenchmark of the arrays. Now `bench/primitives/arrays.isc` and its
  floor `arrays.c` (4096 integers in the caches, one thread, 25 times the
  rounds asked: a form of 0.1 ns an element measured over 200 rounds lasted
  80 µs, and two runs differed by half): the sum of a `map`, a `map` made
  into an array, `filter` counted and summed, `concat`, the sum of a `map`
  of a `zip`, `fold`, `scan`, `fill`, the sum of a slice. The floors: sums
  in 32 lanes (4 vectors of 8), a prefix sum within vectors of 8 and a
  carry kept in a vector, stores of whole vectors into arrays aligned on 64
  bytes. First floors were beaten by Isocron, so wrong (fixed frequency):
  loops of GCC (one vector accumulator; the sum of a `map` 0.54, `scan`
  0.53, the sum of a slice 0.64), `x << 1` for `x + x` (a shift of 512 bits
  has one port, an addition two: 0.108 -> 0.079 ns), a carry of `scan`
  extracted to a scalar and broadcast again (0.262 against 0.245 ns).
  - The sum of `map(zip(xs, ys), (p) => { const [x, y] = p; ... })` 3.36
    times its floor: the pairs a source that the reduction does not read in
    vectors, so the `map` was made into a buffer first; the same written by
    indices (`map(iota(n), (i) => xs[i] ^ ys[i])`) one loop in vectors.
    Now a pass of the IR (`compose_zip_maps`, after the first fusion):
    `map(zip(a, b), (p) => f)`, the components scalars and `f` reading `p`
    only by its components, is one `map` over `a` and `b` whose parameters
    are the components; otherwise it stays as it was (a `p` used whole, a
    zip of texts).
  - Measured (the old and new binaries of `arrays`, 7 alternated runs,
    fixed frequency): `zip.sum` 0.56 -> 0.11 ns (-80.0 %), the nine others
    the same; `run.py`: every form of the group from 1.01 to 1.79 times its
    floor (`filter.count` 1.79, `filter.sum` 1.55, the sum of a slice 1.21,
    `concat` 1.11, `zip.sum` 1.09, `fold` 1.04, the others 1.01 to 1.04).
    The LLVM IR of the ten programs the same (none reads a zip so).
  - Test `maps_of_zips_by_their_components` (`stdlib.rs`): each form of a
    `map` of a zip (mixed types, summed, made into an array, filtered, a
    pair as the result, `p` whole by a function and a call, texts, sizes
    known at run time and their difference a panic) against the same value
    computed by indices, which that pass does not change, compiled and
    interpreted (the interpreter runs the optimized program too: compiled
    against interpreted alone cannot see a fault of a pass of the IR); with
    the components of pairs of one type swapped by the pass it fails
    (1484466 against -4490822).
- **D496 — The Benchmarks Game measured again at 62d70aa (2026-11-08).**
  After D494 (counts by value below the number of keys: k-nucleotide's
  counts the same, 609.1 -> 608.4 ms alternated) and D495 (a `map` of a zip:
  the LLVM IR of the ten programs the same), on a quiet machine (no `Plex
  Transcoder` from three minutes before to the end): the image built with
  the `iscc` of 62d70aa, 102 checks against the references (the same
  outputs; the known difference of C++ #4 of mandelbrot at N = 200 only),
  measured in Docker (4 threads, 12 runs, the lowest). The fastest on 10
  of 10, every time within its scatter (k-nucleotide 607 -> 611 ms, within
  605 to 622 ms since D429; n-body 1880 -> 1884 ms; the others within 1
  ms). `results/benchmarks_game.md`, `benchmarks_game.tsv`, `floors.md`,
  `floors_isocron.tsv`, the table of the README and its figure made again
  from these lines.
- **D497 — The group of control and calls measured (2026-11-08).** The
  audit of the forms (TODO: "control and calls") had no microbenchmark of
  them. Now `bench/primitives/control.isc` and its floor `control.c` (one
  thread): a call of a recursion that is not a tail call (`fib(25)`, per
  call), Euclid's algorithm by a tail recursion (`gcd(b, a % b)`) of 4096
  pairs, a `match` over a sum of three variants (25 times the rounds: 0.6 ms
  at 200), the steps of Collatz of 4096 numbers by a tail recursion (branches
  that the data choose). Measured (`run.py`, 5 runs of 200 rounds, fixed
  frequency): `collatz` 1.39 times its floor, `gcd` 1.32, `fib.call` 1.26,
  `match.area` 1.07.
  - Floors beaten first, so wrong: Collatz in a scalar loop of C was 4
    times slower than Isocron, which runs the loops of a `map` in vectors
    (32 numbers in 4 vectors of 8, every lane stepped at each turn, the
    turn written into a lane when it reaches 1); a floor that gave a lane
    done its next number (`expandload`, a reduction at each turn) was still
    twice as slow; the floor now steps 32 numbers as Isocron does (17.1
    against 23.3 ns, not at a fixed frequency). Euclid's division in 64
    bits was Isocron's speed; the floor divides in 32 bits once both
    numbers fit (the remainders only decrease, tested once before the
    loop).
  - `gcd` 1.32: Isocron divides in 32 bits too (LLVM's `idivq-to-divl` for
    this CPU: 5 `divl` in its code with and without that feature asked,
    64.87 -> 64.81 ns), but tests at each division whether both fit, on the
    chain of the loop; the floor tests once.
- **D498 — The group of the standard output and input measured, and an
  integer printed in one call (2026-11-08).** The audit of the forms (TODO:
  "input and output") had no microbenchmark of the standard streams (the
  files are in `sys`). Now `bench/primitives/io.isc` and its floor `io.c`
  (one thread): `print` of 4096 numbers a round (a line each), `printBytes`
  of 64 KiB a round, `readStdinLines` of a million lines of numbers summed
  once. `run.py` gives a group that declares `// bench-io` a standard
  input it makes once (`build/io_input.txt`), a regular file as its
  standard output, and the path of a file for its lines of results as its
  last argument (its standard output is what it measures). The floor: the
  digits written into a buffer of 64 KiB written by `write` when full, the
  64 KiB written at once, the input read whole and its lines parsed as
  `memchr` finds them; the outputs of the floor and of Isocron the same
  bytes.
  - `print` of a number: profile (one CPU at fixed frequency, 2048000
    lines): 26.5 ns a line; `memcpy` 15.3 % (the digits copied into the
    buffer by `copy_from_slice`), two calls a line (`isc_print_i64`, then
    `isc_print_newline`, the buffer taken twice), the loop of `forEach` (a
    call of its action by its address an element) 32.7 %. Now a few bytes
    written to the buffer of the standard output are copied by moves
    (`copy_short` of D459), and `print` of one integer calls
    `isc_print_i64_line` (or `_u64_`): its digits and the newline in one
    buffer, one test of the tasks, the end of the line for a terminal or a
    pipe as before.
  - Measured (the old and new binaries of `io` and the floor, 7 alternated
    runs, fixed frequency, the same outputs): `print.line` 26.735 -> 24.937
    ns (-6.7 %), `printBytes` and `readStdinLines` within 0.4 %; `run.py`:
    `readStdinLines.line` 2.01 times its floor, `print.line` 1.55,
    `printBytes.byte` 1.11. The LLVM IR of the ten programs the same but
    fannkuch-redux's `print(checksum)` (one call; its output the same).
  - Test `integers_printed_a_line_at_once` (`actions.rs`: integers of every
    width and their bounds, 16 actions of `mapParallel` printing 300 each
    at the same time to a pipe, against the interpreter, 1 and 4 threads;
    without the newline in `isc_print_i64_line` it fails).
- **D499 — The Benchmarks Game measured again at 5adf9f3 (2026-11-08).**
  After D497 (no change of the compiler) and D498 (`print` of an integer
  in one call: in the ten programs, fannkuch-redux's `print(checksum)`
  alone, its output the same), on a quiet machine (no `Plex Transcoder` from
  three minutes before to the end): the image built with the `iscc` of
  5adf9f3, 102 checks against the references (the same outputs; the known
  difference of C++ #4 of mandelbrot at N = 200 only), measured in Docker
  (4 threads, 12 runs, the lowest). The fastest on 10 of 10, every time
  within its scatter (binary-trees 659 -> 665 ms with Rust #5 842 -> 848
  ms, reverse-complement 24 -> 23 ms, n-body 1884 -> 1876 ms, the others
  within 3 ms). `results/benchmarks_game.md`, `benchmarks_game.tsv`,
  `floors.md`, `floors_isocron.tsv`, the table of the README and its figure
  made again from these lines.
- **D500 — Release 0.4.7 (2026-11-08).** After 0.4.6: D487 to D499, maps
  found by an index of hashes (D492), numbers and booleans sorted by their
  bytes (D488), `toFixed` from the integer of the double (D487), `countBy`
  by value below the number of keys (D494), a `map` of a zip made one `map`
  (D495), `print` of an integer in one call (D498), the tasks of a fill
  that reads by windows of 16 KiB (D490); the audit of every group of forms
  against its floor (numbers, sorts, maps, arrays, control and calls, the
  standard streams). No name removed, the language the same but the speed
  of its maps. The commit of the release the last of `main` (D486); the
  steps of 0.4.3 (D449): the Windows tests in the VM with the `iscc.exe` of
  the release's sources, the archives built from those sources in the
  images of the releases, tried before they are published. The ten
  programs of the Benchmarks Game measured at 5adf9f3 (D499); nothing in
  `crates/` changed since but the version.
- **D501 — A text in another case only read in its scope on the stack
  (2026-11-08).** TODO: "the strings that live inside one expression placed
  on the stack ... left: the other operations that make a text (`padStart`,
  `toString`, `toUpperCase`, the copies)"; `padStart` done by D458. Now
  `toUpperCase` and `toLowerCase` whose text is only read in its scope (the
  analysis of D404: `result_concats`) write it into the buffer of 128 bytes
  on the stack of D404 when it is ASCII (each byte mapped, the same length)
  and fits (`isc_to_case_into`); a text that is not ASCII (its length may
  change: `ß`) or longer, or given away, in a block as before.
  - Measured (the old and new binaries of `strings`, 7 alternated runs,
    fixed frequency): `toUpperCase` 17.69 -> 13.93 ns (-21.3 %), the others
    of the group within 3.6 %; `run.py` (5 runs of 200 rounds):
    `toUpperCase` 1.69 -> 1.33 times its floor; the highest of the group
    `padStart` 2.71, `trim` 2.38, `split.6` 2.37. The LLVM IR of the ten
    programs the same.
  - Test `texts_of_another_case_on_the_stack` (`stdlib.rs`: ASCII texts of
    0, 1, 127, 128, 129 and 300 bytes, texts that are not ASCII, compared,
    searched, measured, a text given away and returned; the interpreter's
    output on 1 and 4 threads; Valgrind; with the case of `toLowerCase`
    made upper there it fails).
