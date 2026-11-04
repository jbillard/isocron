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
  4 017 541 ns against 3 973 068 ns on the large one, and very slow on AMD
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
  KiB of L2) cannot hold the chunk being computed and the next one. With
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
    shift amount is taken modulo the number of bits (as Java and the x86
    instructions): one `and`, no check, vectorizable. `(&)`, `(|)` and
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
  instead of 0.625 s, the instruction into memory being slow; and a
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
  memory, slow on Intel's cores). At fixed frequency, 4 threads:
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
  loop is at the floor of the machine (0.739 ns per word, `mulx` and
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
    `mremap` as a copy (Windows cannot move pages), the advice on huge
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
    2.28 calls; from glibc 2.34 on, `__libc_start_main` ignores them and
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
