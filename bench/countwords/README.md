# countwords: Rust programs and a port to Isocron

[benhoyt/countwords](https://github.com/benhoyt/countwords) (MIT,
`rust/LICENSE-MIT`; commit d8c5cf3) is a benchmark of many languages: the
words of the standard input (separated by spaces and newlines), lowercased,
counted, written as `word count` lines, the most frequent first. Its Rust
programs are Andrew Gallant's, two of them here:

* `rust/simple`: "roughly how I would write this program if someone gave me
  the problem as an interview challenge and said, don't worry about
  performance": lines read one by one, `split_whitespace`, `to_lowercase`,
  a `HashMap<String, u64>`, the counts sorted.
* `rust/optimized`: the input read by blocks of 64 KiB, bytes lowercased in
  place, a `HashMap<Vec<u8>, u64>` hashed by `fxhash`, a word looked up
  before it is copied.

`isocron/countwords.isc` is the same task in Isocron, written plainly, as
the simple Rust program (not tuned): the whole text read and lowercased, its
words found by a regular expression, counted by `countBy`, the counts
sorted by `sortBy`, the lines joined and printed. The three write the same
counts (in another order among equal counts, which the comparison sorts by
word, as the benchmark's `normalize.py` does).

The only change to the Rust programs: an empty `[workspace]` in each
`Cargo.toml`, so that cargo builds them apart from the compiler's workspace.

## Running it

```
# Rust
cd rust/simple && cargo build --release
target/release/countwords < kjvbible_x10.txt
# Isocron
iscc build isocron/countwords.isc -o countwords-isc
./countwords-isc < kjvbible_x10.txt
```

`ISC_THREADS` sets the number of threads of the Isocron program; the Rust
programs use one.

## Measures

`measure.py` (its steps in its header) fetches the input of the benchmark
(the King James Bible, checked by its SHA-256, ten times: 43 MB), runs
under the protocol of the machine (`bench/machine.sh`) and writes
`results/*.json`; the numbers are in `results/countwords.md`, those before
the compiler was changed for this program's shapes in
`results/before/countwords.md`.

### Before any change (2026-11-17)

Isocron b8e39b7, rustc 1.99.1, a Xeon E-2388G at a fixed 3.2 GHz; the port
as written:

* **The same counts**: every run of the three programs.
* **Times**: 0.570 s for the simple Rust program, 0.252 s for the optimized
  one; Isocron 1.399 s on one thread, 0.848 s on 8.
* **Memory**: peaks of 6.0 and 6.1 MiB for the Rust programs, 1339 MiB for
  Isocron on one thread (1433 on 8).
* **Builds**: `iscc build` 0.34 s, the Rust programs 0.36 and 0.57 s from
  clean.

What explains them (`perf`, Isocron on one thread):

1. **The words found by the general engine of regular expressions**: about
   53 % of the time (`regex_automata`'s forward and reverse searches, the
   start states, `isc_rx_matches`): each of the 8.2 million words a search
   forwards to its end, then backwards to its start, for a pattern that is
   one class of bytes repeated (`[^ \n]+`).
2. **Every word a text of its own**, then an array of them, then the keys
   of `countBy` grouped (`isc_group`, 16.7 %): about 20 % more in the
   allocator and the kernel (`clear_page_erms` 9.2 %, the pages of 1.3 GiB
   cleared; `isc_free`, `refill`, `memcpy`).
3. **Nothing streamed**: the whole text read, then lowercased (a copy),
   then the words, then the counts; the Rust programs read by lines or by
   blocks of 64 KiB and count each word in their hash map as they find it,
   never an array of words.

### After the changes of the compiler (2026-11-17)

Isocron c6d8408, the port the same, line for line:

* **Times**: Isocron 0.307 s on one thread (1.399 s before), 0.263 s on 8
  (0.848 s); Rust 0.568 s (simple) and 0.251 s (optimized).
* **Memory**: 434 MiB at its peak (1339 MiB before); Rust 6 MiB.

What changed: the runs of a class of bytes are found without the engine of
regular expressions (D690: a class of ASCII characters or its complement
repeated, `[^ \n]+`, the places where a byte and the next differ in class
found in one pass by vectors). What remains: `countBy` groups the words
once all are made (an array of 8.2 million words, another of their
boundaries: `isc_group` about 60 % of the time, the pages of 434 MiB
cleared about 15 %), where the Rust programs count each word as they find
it; and the whole text read, then lowercased, where they read by blocks.

### After D697 to D700 (2026-11-18)

Isocron ea9cca1, the port the same, line for line:

* **Times**: Isocron 0.243 s on one thread (0.307 s before), 0.206 s on 8
  (0.263 s); Rust 0.572 s (simple) and 0.253 s (optimized).
* **Memory**: 307.6 MiB at its peak (434 MiB before); Rust 6 MiB.

What changed: `countBy` of texts counts each key as it comes, in a table
of hashes written in the prelude (D698), its input read once (no copy of
the words); a `map` of views of a text, the words of `matches`, is fused
into its consumer (D697); texts of up to 16 bytes are compared without a
call (D699); a `fold` reads a `filter` by windows (D700).

What remains: the port names the words in a statement of an action
(`const words = matches(...)`), and SPEC section 12 wants the value of
such a `const` computed whole before the next statement: the words
(125 MiB) and their places (125 MiB) are made. The same program with the
words in the call of `countBy` (`countBy(matches(text, /[^ \n]+/g), (w) =>
w)`, one statement), measured the same way against the optimized Rust
program: 0.238 s and 82.8 MiB (0.243 s and 307.6 MiB as written; Rust
0.252 s and 6.0 MiB), its peak the text read and the text lowered, both
whole. Below that: reading the text by blocks.

`measure.py memory` read the peak of a program from its fork on: between
the fork and the exec it was the script's own memory (about 22 MiB, seen
at random on one program or another, 21.7 MiB for the optimized Rust
program in one measure); now only once the program runs.

### After D701 to D706 (2026-11-18)

Isocron e0b480b, the port the same, line for line:

* **Times**: Isocron 0.225 s on one thread, 0.232 s on 8; Rust 0.570 s
  (simple) and 0.251 s (optimized).
* **Memory**: 15.3 MiB at its peak (307.6 MiB before); Rust 6.1 MiB.

What changed: the pure values of the statements of an action are computed
in the statement that reads them when no effect lies between (D701, the
rule of SPEC section 12 relaxed at the author's choice): the words of
`const words = matches(...)` are never made. The standard input read once
by the runs of a class of bytes that a `fold` reads is read by parts of 64
KiB (D702): neither the text nor its copy in lower case is ever whole. The
keys of the table are moved, not copied, into the `Map`, its pairs and its
sorted pairs (D705, D706).

What remains, 10.3 MiB while it counts and 15.3 at the end: the 32187 keys
in blocks of the allocator written in Isocron, each a block of 64 bytes
after a header of 64 (4 MiB; Rust's are about 1 MiB), the huge pages of
the heap partly used (measured with pages of 4 KiB instead: 10.5 MiB, but
other programs up to 6.3 % slower, D707: kept), and the arrays of the end
(the pairs, their keys of the sort, the text written).

### Against every fast program of the original benchmark (2026-11-18)

The programs of the original benchmark (benhoyt/countwords at d8c5cf3)
built from their sources with the options of its `test.sh`, and Isocron's
program, run in one container pinned to the CPU of the measures
(`others/others.py`; gcc 14, g++ 14, Go 1.24, Rust 1.84, Zig 0.7.1 as in
the original measures), 9 rounds interleaved, every output checked
(`results/others.txt`); Isocron after D708 and D709:

| Program | Min | Median |
| --- | ---: | ---: |
| **Isocron** (written plainly) | **0.175 s** | **0.178 s** |
| Rust, optimized with a trie | 0.181 s | 0.190 s |
| Zig, optimized | 0.191 s | 0.193 s |
| C, optimized | 0.195 s | 0.197 s |
| C++, optimized | 0.202 s | 0.204 s |
| Rust, optimized with its own table | 0.211 s | 0.212 s |
| Rust, optimized, `unsafe` | 0.235 s | 0.237 s |
| Rust, optimized | 0.250 s | 0.255 s |
| Go, optimized | 0.324 s | 0.326 s |
| Zig, simple | 0.414 s | 0.418 s |
| Rust, simple | 0.550 s | 0.568 s |
| C, simple | 0.791 s | 0.822 s |
| Go, simple | 0.808 s | 0.832 s |
| C++, simple | 1.312 s | 1.326 s |
| `wc -w` (reference) | 0.154 s | 0.155 s |

Before D708 and D709 Isocron was sixth (0.222 s): its table found a key
through four reads, each where the one before said, and compared it by
`memcmp`. Its peak is now 17.1 MiB (Rust 6.1).
