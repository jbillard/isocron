# rtiow: a Rust project and its port to Isocron

[okaneco/rtiow](https://github.com/okaneco/rtiow) (MIT, `rust/LICENSE-MIT`; its sample images left out)
is a ray tracer in Rust following Peter Shirley's three books (*Ray Tracing
in One Weekend*, *The Next Week*, *The Rest of Your Life*): spheres, moving
spheres, rectangles, boxes, instances moved and turned, smoke, a tree of
bounding boxes (BVH), textures and the noise of Perlin, materials
(Lambertian, metal, glass, lights, isotropic), sampling by probability
densities (towards the lights and along the cosine), rendered in parallel
with rayon. `isocron/` is the same program in Isocron, written plainly
(not tuned): the two write the same image, byte for byte.

| | Rust (`rust/`) | Isocron (`isocron/`) |
| --- | --- | --- |
| Source | 4460 lines (26 files, comments included; 1615 of them the scenes of the three books, about 20), the crates rand, rand_chacha, rayon, image | 1248 lines (4 files: `rng`, `vec3`, `world`, `main`; every part of the library, the two scenes measured) |
| Polymorphism | traits (`dyn Hittable`, `dyn Texture`, `dyn Pdf`) behind `Arc` | sum types and `match` |
| Random numbers | `rand` 0.7, `rand_chacha` 0.2 | `rng.isc`: ChaCha8 and the draws of `rand` 0.7, the same numbers (`rng_check.isc`, `rng_check.out`, `rng_check.rs`) |
| Parallelism | `rayon` (`into_par_iter` over the pixels) | `map` over the pixels |

## What was changed in the Rust program

Only what a comparison needs, each place marked `bench/rtiow`:

* **Reproducible random numbers.** The program drew from `rand::thread_rng()`
  (seeded by the system, its draws shared by the pixels a thread happens to
  render): an image could not be rendered twice. `src/rng.rs` gives each
  thread a ChaCha8 generator (the generator its scenes use) started again
  from the seed plus the index of the pixel at the start of each pixel
  (`render.rs`); the smoke and the noise of Perlin draw from it as they drew
  from `thread_rng`.
* **The final scene of the second book** (`main.rs`: a sixth argument,
  `sphere` by default, or `final`): its light turned down
  (`FlipFace`), as the lights of the third book, whose `emitted` lights the
  front face only (it rendered black); it had been put behind the feature
  `images` although it uses no image.
* **The sort of the BVH** (`bvh.rs`): its comparison gave `Less` both ways
  for equal keys, which is no order: the order of the many equal boxes of
  the ground was left to the algorithm of `sort_unstable_by`. Now `Equal`
  for them, and a stable sort: the same order, defined.
* `ImageTexture` behind the feature `images` (the port has no JPEG decoder;
  without `images` the program builds without the `image` crate).

Kept as they are, in both: the BVH tries its right child up to `t_max` even
after a hit on its left (its right hit, when there is one, is kept), the box
of `RotateY` takes `1 - i` on its three axes, a translated hit is faced
against its ray again.

## Running it

```
# Rust
cd rust && cargo build --release --no-default-features --features threads
target/release/rtiow image.ppm 100 384 216 42 sphere
# Isocron
iscc build isocron/main.isc -o rtiow-isc
./rtiow-isc image.ppm 100 384 216 42 sphere
```

Arguments: the image, the samples per pixel, its width, its height, the
seed, the scene (`sphere` or `final`). `RAYON_NUM_THREADS` and `ISC_THREADS`
set the numbers of threads.

## Measures

`measure.py` (its steps in its header) runs under the protocol of the
machine (`bench/machine.sh`) and writes `results/*.json`; the numbers are in
`results/rtiow.md`, those before the compiler was changed for this program's
shapes in `results/before/rtiow.md`.

### Before any change (2026-11-16)

Isocron c1c4107, rustc 1.99.1, a Xeon E-2388G at a fixed 3.2 GHz; the port
as written:

* **The same images**: every render of the two programs, on 8 threads and
  on 1, the two scenes, byte for byte.
* **Builds**: `iscc build` 2.9 s (one core); Rust 6.3 s from clean (14.7 s
  of processor: its dependencies build in parallel), 3.0 s after one file
  touched (thin LTO). The programs: 363 KiB and 721 KiB.
* **8 threads**: `sphere` 2.73 s for Isocron, 2.78 s for Rust; `final`
  3.48 s and 2.40 s.
* **1 thread**: `sphere` 17.8 s and 7.0 s; `final` 23.7 s and 10.8 s.
* **Memory**: peaks of 20.7 MiB and 3.3 MiB (`sphere`), 28.7 and 3.8 MiB
  (`final`); 5 thousand blocks allocated and 858 thousand for a small
  `sphere` (Valgrind).

What explains them (`perf`, on the same renders):

1. **One thread, Isocron 2.5 times slower**: 11.3 billion instructions
   against 3.3 (`sphere`, 25 samples of 192 by 108), 4.2 billion loads
   against 1.0, 4.0 billion stores against 0.56, at a like number of
   instructions per cycle: copies. `hit` is one function of 3852
   instructions and a frame of 2.9 KiB (every kind of object inlined in its
   `match`); each call takes the ray and the generator (26 words) and gives
   back the generator and an `Option<HitRecord>` (its material inside, about
   160 bytes), through memory. The Rust program fills a `&mut HitRecord` and
   draws from a generator of its thread.
2. **Rust on 8 threads** spends 22.2 s of processor where one thread needs
   7.0 s: its `Arc`. In `AaRect::hit` the same work takes 1794 samples
   against 424 on one thread, the hottest instructions the `lock incq` of
   `rec.material = self.mp.clone()` and the loads of the rectangle beside
   counters other cores write. On one thread already, `malloc`, `free` and
   the drops of `Arc` are 35 % of its time (16 allocations a sample:
   `HitRecord::default()` makes an `Arc<Material>` and an `Arc<SolidColor>`,
   each diffusion two `Arc` of densities).
3. **Isocron on 8 threads** spends 21.4 s of processor for 17.8 s on one:
   the atomic counts of the textures of the world copied into each hit
   (D678): the `lock incq` of `hit` from about 1 % to about 9 % of its
   samples, `isc_rec_drop_1_atomic` from 2.1 % to 4.6 %.
4. **Memory**: Isocron's own is 2.6 MiB (Rust's 2.8); its peak grows by
   about 200 bytes a pixel, the port making the whole text of the image (the
   colors, a line a pixel, joined) before writing it, where the Rust program
   writes it line by line; the final scene adds 8 MiB while it is made
   (seen on an image of 2 by 2 pixels; which of its arrays, not measured
   yet).

### After the changes of the compiler (2026-11-17)

Isocron 3e66f95, the same machine and protocol (`results/rtiow.md`). The
port is the same, line for line: what changed is the compiler and its
runtime (D679 to D688), each change for a shape this program showed,
measured on the Benchmarks Game and `bench/records/walk` too, none tuned
for this program.

* **The same images**: every render, byte for byte, as before.
* **8 threads**: `sphere` 0.867 s for Isocron, 2.798 s for Rust (2.73 s
  before); `final` 1.395 s and 2.411 s (3.48 s before). Isocron spends 6.8
  s of processor where Rust spends 22.2.
* **1 thread**: `sphere` 6.65 s and 7.04 s (17.8 s before); `final` 9.85 s
  and 10.72 s (10.37 s with `target-cpu=native`; 23.7 s before).
* **Memory**: peaks of 1.3 MiB and 3.1 MiB (`sphere`), 3.6 and 3.9 MiB
  (`final`) on one thread; 2.3 and 3.6, 4.9 and 4.3 MiB on 8; 5 thousand
  blocks allocated and 858 thousand for a small `sphere` (Valgrind), 90
  thousand and 1.7 million for a small `final`.
* **Builds**: `iscc build` 2.6 s; the program 267 KiB (Rust's 721 KiB).

What changed, against the causes above:

* **Copies** (the first cause, one thread): large values given to and by
  recursive functions stay in memory, at the place the caller gives
  (D679); a state is written over the one given and a result at its place
  (D680); the state of a loop of tail calls stays at its caller's
  address, and LLVM no longer computes before such a loop what one of its
  arms needs (it computed the sines of the checker texture at every call
  of `textureValue`, whatever the texture: D681).
* **Atomic counts on 8 threads** (the third): a task whose results hold no
  node counts only the nodes made by its own iteration, so the threads
  read the world and never write it (D683); a node holds the slots of its
  own variant alone (D682: 728 bytes for every object of the world
  before), each slot a pattern reads read at its place.
* **Memory** (the fourth): the texts of the lines are appended in order
  (D685: no array of a text a pixel), written to the file by parts of 64
  KiB as they are made (D686: never the whole text, the header the start
  of it, not a copy), the colors are stored as bytes (D687: the compiler
  follows the values of the clamping of `intoU8`, NaN included, to the
  interval 0 to 255 of each part of the tuple), and the first chunk of
  the arrays of the allocator is in pages of 4 KiB, no longer a huge page
  of 2 MiB made resident by the first array (D688).

What remains, in memory, on `final` with 8 threads: each side of its
boxes keeps its material by value (a node of 192 bytes; Rust's six sides
of a box share one `Arc<Material>`), and each thread touches about 180
KiB of its stack (Rust: under 56 KiB), the frames of the large functions
(2.5 to 2.8 KiB for those of `hit`) never sharing their slots.
