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
