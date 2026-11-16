# records: a state and a result passed through a recursion

`walk.isc` walks a tree of 4096 intervals for each query, both children of
a node whose bounds hold the point (not a loop: a recursion), a state of 13
words given to each call and given back advanced, the closest hit a record
of 15 words. The shape of the walk of a ray tracer (`bench/rtiow`), of an
interpreter carrying its machine, of a parser carrying its position.
`rust/` is the same program in Rust, in two styles: the state lent
(`&mut State`, the default) and, with the second argument `value`, the
state given by value and given back, as Isocron writes it. The three print
the same number.

```
iscc build walk.isc -o walk && ISC_THREADS=1 ./walk 1000000
cd rust && cargo build --release && target/release/walk 1000000 [value]
```

Measured on 2026-11-16 (Isocron c1c4107, rustc 1.99.1, Xeon E-2388G at
3.2 GHz, one core): Rust with `&mut` 0.88 s (5.5 billion instructions),
Rust by value 1.18 s (8.7), Isocron 2.02 s (16.4). The causes and the
change of the compiler they call for: `docs/proposals/values-in-memory.md`.
