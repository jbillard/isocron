//! Shared I/O, timing and the fixed reduction form of SPEC section 10.
//! Interface of every program: `<kernel> <inputs...> <output.f64> <reps>`;
//! it times the kernel only, prints nanoseconds, writes the result.

use std::time::Instant;

/// The kernels of phase 7 (moving average, filter then sum, prefix sum, matvec).
pub mod phase7;

/// V of the fixed form; `ISC_LANES=64 cargo build` for the experiments on V.
pub const LANES: usize = parse_lanes(option_env!("ISC_LANES"));

const fn parse_lanes(s: Option<&str>) -> usize {
    match s {
        None => 32,
        Some(s) => {
            let b = s.as_bytes();
            let mut v = 0usize;
            let mut i = 0;
            while i < b.len() {
                v = v * 10 + (b[i] - b'0') as usize;
                i += 1;
            }
            v
        }
    }
}
pub const BLOCK: usize = 4096;

const fn min(a: usize, b: usize) -> usize {
    if a < b {
        a
    } else {
        b
    }
}

/// Accumulators per pass over a block (speed only, never results), the rule
/// of Isocron's code generator: the accumulators of one pass, plus one
/// register per array read (one less with AVX, which folds a load into the
/// addition), must fit in the vector registers. `GROUP1`: one array read per
/// element, `GROUP2`: two. Each accumulator still receives its elements in
/// increasing order.
pub const GROUP1: usize = min(LANES, if cfg!(target_feature = "avx") { 64 } else { 16 });
pub const GROUP2: usize = min(
    LANES,
    if cfg!(target_feature = "avx512f") {
        64
    } else if cfg!(target_feature = "avx") {
        32
    } else {
        16
    },
);

pub fn read_f64(path: &str) -> Vec<f64> {
    let b = std::fs::read(path).unwrap_or_else(|e| panic!("cannot read {}: {}", path, e));
    b.chunks_exact(8).map(|c| f64::from_le_bytes(c.try_into().unwrap())).collect()
}

pub fn read_u8(path: &str) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|e| panic!("cannot read {}: {}", path, e))
}

pub fn write_f64(path: &str, xs: &[f64]) {
    let mut out = Vec::with_capacity(xs.len() * 8);
    for x in xs {
        let bits = if x.is_nan() { 0x7FF8_0000_0000_0000u64 } else { x.to_bits() };
        out.extend_from_slice(&bits.to_le_bytes());
    }
    std::fs::write(path, out).unwrap();
}

pub fn args() -> Vec<String> {
    std::env::args().skip(1).collect()
}

/// Calls `f` `reps` times, prints the total time in nanoseconds and returns
/// the last result.
pub fn bench<T>(reps: &str, mut f: impl FnMut() -> T) -> T {
    let reps: u64 = reps.parse::<u64>().unwrap_or(1).max(1);
    let t0 = Instant::now();
    let mut r = std::hint::black_box(f());
    for _ in 1..reps {
        r = std::hint::black_box(f());
    }
    println!("{}", t0.elapsed().as_nanos());
    r
}

fn tree(mut acc: [f64; LANES]) -> f64 {
    let mut d = LANES / 2;
    while d >= 1 {
        for j in 0..d {
            acc[j] += acc[j + d];
        }
        d /= 2;
    }
    acc[0]
}

/// `reduce_block` with `+`, over `f(element)` of one slice. The rows of V
/// elements are fixed-size arrays (`as_chunks`), so that LLVM vectorizes the
/// update of the accumulators without bounds checks.
#[inline(always)]
pub fn block_map(x: &[f64], f: impl Fn(f64) -> f64) -> f64 {
    let mut acc = [0.0f64; LANES];
    let (rows, tail) = x.as_chunks::<LANES>();
    // one pass per group of GROUP1 accumulators (a single pass when they fit)
    for p in 0..LANES / GROUP1 {
        let mut g = [0.0f64; GROUP1];
        for r in rows {
            let c: &[f64; GROUP1] = r[p * GROUP1..][..GROUP1].try_into().unwrap();
            for (a, v) in g.iter_mut().zip(c) {
                *a += f(*v);
            }
        }
        acc[p * GROUP1..][..GROUP1].copy_from_slice(&g);
    }
    for (a, v) in acc.iter_mut().zip(tail) {
        *a += f(*v);
    }
    tree(acc)
}

/// `reduce_block` with `+`, over `f(a[i], b[i])`.
#[inline(always)]
pub fn block_map2(a: &[f64], b: &[f64], f: impl Fn(f64, f64) -> f64) -> f64 {
    let mut acc = [0.0f64; LANES];
    let (ra, ta) = a.as_chunks::<LANES>();
    let (rb, tb) = b.as_chunks::<LANES>();
    for p in 0..LANES / GROUP2 {
        let mut g = [0.0f64; GROUP2];
        for (x, y) in ra.iter().zip(rb) {
            let x: &[f64; GROUP2] = x[p * GROUP2..][..GROUP2].try_into().unwrap();
            let y: &[f64; GROUP2] = y[p * GROUP2..][..GROUP2].try_into().unwrap();
            for ((s, u), v) in g.iter_mut().zip(x).zip(y) {
                *s += f(*u, *v);
            }
        }
        acc[p * GROUP2..][..GROUP2].copy_from_slice(&g);
    }
    for ((s, u), v) in acc.iter_mut().zip(ta).zip(tb) {
        *s += f(*u, *v);
    }
    tree(acc)
}

/// The fixed form: blocks of `BLOCK` elements, then the block results are
/// reduced the same way.
#[inline(always)]
pub fn fixed(n: usize, block: impl Fn(usize, usize) -> f64) -> f64 {
    if n <= BLOCK {
        return block(0, n);
    }
    let parts: Vec<f64> = (0..n.div_ceil(BLOCK)).map(|j| block(j * BLOCK, BLOCK.min(n - j * BLOCK))).collect();
    fixed_sum(&parts)
}

pub fn fixed_sum(x: &[f64]) -> f64 {
    fixed(x.len(), |s, m| block_map(&x[s..s + m], |v| v))
}

/// The fixed form with the blocks computed in parallel (rayon); each block
/// result has its own slot, so the result does not depend on the number of
/// threads.
pub fn fixed_par(n: usize, block: impl Fn(usize, usize) -> f64 + Sync) -> f64 {
    use rayon::prelude::*;
    if n <= BLOCK {
        return block(0, n);
    }
    let parts: Vec<f64> =
        (0..n.div_ceil(BLOCK)).into_par_iter().map(|j| block(j * BLOCK, BLOCK.min(n - j * BLOCK))).collect();
    fixed_sum(&parts)
}

/// Starts rayon's thread pool before the timed loop.
pub fn start_pool() -> usize {
    rayon::current_num_threads()
}

/// Four sub-histograms over a slice, added to `h`.
pub fn sub_histograms(x: &[u8], h: &mut [u32; 256]) {
    let mut t = [[0u32; 256]; 4];
    let mut it = x.chunks_exact(4);
    for c in &mut it {
        t[0][c[0] as usize] += 1;
        t[1][c[1] as usize] += 1;
        t[2][c[2] as usize] += 1;
        t[3][c[3] as usize] += 1;
    }
    for &b in it.remainder() {
        t[0][b as usize] += 1;
    }
    for j in 0..256 {
        h[j] += t[0][j] + t[1][j] + t[2][j] + t[3][j];
    }
}
