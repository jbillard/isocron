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
