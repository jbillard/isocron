# Results

> [!NOTE]
> The machine of every measurement since 2026-11-01 is **xeon** (Intel Xeon
> E-2388G, 8 cores, 16 hardware threads), at fixed frequency, the other
> processes confined (`bench/machine.sh`). The proof of concept was first
> measured on a Celeron J3455.

## Today

| File | What |
| :-- | :-- |
| [**benchmarks_game.md**](benchmarks_game.md) | 🏁 The ten programs of the Benchmarks Game against the fastest in C, C++, Rust and Go, measured in Docker, with the floor of each (intermediate results: the work goes on) |
| [**floors.md**](floors.md) | 📏 The floor of each program: what its result requires at the cost of the cheapest way the machine has to do it |
| [**primitives.md**](primitives.md) | 🔬 Every form of the language against its floor: a microbenchmark of the form in Isocron and the same work in C the cheapest way known, on one core at fixed frequency, ranked by the gap (`bench/primitives/run.py`; the strings so far) |
| [`machine_floor.txt`](machine_floor.txt) | the costs of the machine the floors are made of (`bench/floor/floor.c`) |
| [`machine_counts.txt`](machine_counts.txt) | the work the floors count (flips, iterations, k-mers: `bench/floor/counts.c`) |
| [`benchmarks_game.tsv`](benchmarks_game.tsv), [`floors.tsv`](floors.tsv), [`floors_isocron.tsv`](floors_isocron.tsv), [`primitives.tsv`](primitives.tsv) | the raw lines of the measurements and of the floors |

## Campaigns of the proof of concept (kept as they were)

The four kernels of SPEC section 15 (`sum`, `dot`, `std_dev`, `histogram`,
and `matvec` later) against C, Rust, Go and Zig. Verdicts of SPEC section
14: 🟢 *beats* (the median lower by more than the dispersion), 🟡
*equals* (within it), 🔴 *slower*.

| File | What |
| :-- | :-- |
| [xeon-summary.md](xeon-summary.md) | the official campaign of 2026-11-02: Isocron has the lowest median in all 50 cases |
| [xeon-single.md](xeon-single.md), [xeon.md](xeon.md), [xeon-compile.md](xeon-compile.md) | its details: one core, 1 to 8 threads, compilation times |
| [stage2-summary.md](stage2-summary.md) | the campaign `stage2` (2026-11-02, afternoon), with its details [stage2-single.md](stage2-single.md), [stage2.md](stage2.md), [stage2-compile.md](stage2-compile.md) |
| [xeon-lanes.md](xeon-lanes.md) | the study of V, the number of accumulators of the fixed reduction form |
| [xeon-quick-single.md](xeon-quick-single.md), [xeon-quick.md](xeon-quick.md) | a quick campaign of the night of 2026-11-01 |
| [poc.md](poc.md) | the decision point of the proof of concept (Celeron J3455, 2026-11-01) |

Each `.md` of a campaign is made by its script from the `.json` beside it
(`bench/run.py`, `bench/parallel.py`, `bench/compile.py`, `bench/lanes.py`).
