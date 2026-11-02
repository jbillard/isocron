//! Shared kernels of the hand-optimized (`opt`) programs of phase 7 (SPEC
//! section 14): moving average, filter then sum, prefix sum, matrix-vector
//! product. Every float result is bit-identical to Isocron's fixed form
//! (section 10); see `PHASE7.md`.

use crate::{block_map, block_map2, fixed, tree, BLOCK, LANES};

/// An output buffer of `n` elements, allocated once before the timed loop and
/// written element by element, so that its pages are mapped before the
/// timing (`vec![0.0; n]` gets zero pages from the system, mapped at the
/// first write, which would fall in the first timed call).
pub fn output_buffer<T: Copy + Default>(n: usize) -> Vec<T> {
    let mut v = Vec::with_capacity(n);
    v.resize(n, std::hint::black_box(T::default()));
    v
}

// ---------------------------------------------------------------------------
// moving average

/// Size of the window.
pub const WINDOW: usize = 64;
/// Outputs per tile: the intermediate levels of a tile stay in L1.
pub const TILE: usize = 1024;

/// The windows of `WINDOW` elements share their partial sums: with V
/// accumulators, the fixed form of window `i` is `acc[j] = L0[i + j]` with
/// `L0[k] = (0.0 + x[k]) + x[k + V] + ...` (64 / V terms), and each step `d`
/// of the tree turns level `L` into `L'[k] = L[k] + L[k + d]`, again a
/// function of `i + j` only. Each level is computed once for all windows:
/// 64/V + log2(V) additions per output (7 for V = 32) instead of 64 + 31,
/// with exactly the operations, in the same order, of the per-window fixed
/// form.
const SHARED: bool = LANES <= WINDOW && WINDOW % LANES == 0;

/// The scratch buffers of `moving_average`: two levels of one tile.
pub struct MovingScratch {
    a: Box<[f64]>,
    b: Box<[f64]>,
}

impl MovingScratch {
    pub fn new() -> Self {
        MovingScratch { a: vec![0.0; TILE + WINDOW].into(), b: vec![0.0; TILE + WINDOW].into() }
    }
}

impl Default for MovingScratch {
    fn default() -> Self {
        Self::new()
    }
}

/// `out[i] = fixed-form sum of x[i..i + WINDOW] / WINDOW` for `i < out.len()`;
/// `x` holds at least `out.len() + WINDOW - 1` elements.
pub fn moving_average(x: &[f64], out: &mut [f64], s: &mut MovingScratch) {
    assert!(x.len() + 1 >= out.len() + WINDOW);
    for (t, o) in out.chunks_mut(TILE).enumerate() {
        if SHARED {
            moving_tile(&x[t * TILE..], o, s);
        } else {
            // other values of V (`ISC_LANES`): the fixed form window by window
            for (i, o) in o.iter_mut().enumerate() {
                let w = &x[t * TILE + i..][..WINDOW];
                *o = block_map(w, |v| v) / WINDOW as f64;
            }
        }
    }
}

fn moving_tile(x: &[f64], out: &mut [f64], s: &mut MovingScratch) {
    let m = out.len();
    let (mut src, mut dst) = (&mut s.a, &mut s.b);
    // level 0: acc[j] = e + w[j] + w[j + V] + ... from e = 0.0, in increasing order
    let mut len = m + LANES - 1;
    {
        let l0 = &mut src[..len];
        if WINDOW / LANES >= 2 {
            for ((o, p), q) in l0.iter_mut().zip(&x[..len]).zip(&x[LANES..LANES + len]) {
                *o = (0.0 + *p) + *q;
            }
        } else {
            for (o, p) in l0.iter_mut().zip(&x[..len]) {
                *o = 0.0 + *p;
            }
        }
        for t in 2..WINDOW / LANES {
            for (o, q) in l0.iter_mut().zip(&x[t * LANES..t * LANES + len]) {
                *o += *q;
            }
        }
    }
    // the steps d = V/2, ..., 2 of the tree
    let mut d = LANES / 2;
    while d >= 2 {
        let n = len - d;
        for ((o, p), q) in dst[..n].iter_mut().zip(&src[..n]).zip(&src[d..d + n]) {
            *o = *p + *q;
        }
        std::mem::swap(&mut src, &mut dst);
        len = n;
        d /= 2;
    }
    // the step d = 1, then the division
    if LANES >= 2 {
        for ((o, p), q) in out.iter_mut().zip(&src[..m]).zip(&src[1..1 + m]) {
            *o = (*p + *q) / WINDOW as f64;
        }
    } else {
        for (o, p) in out.iter_mut().zip(&src[..m]) {
            *o = *p / WINDOW as f64;
        }
    }
}

// ---------------------------------------------------------------------------
// filter then sum

/// Appends the elements `> 0.0` of `r` at `dst[0..]` (which has room for 8)
/// and returns how many were kept; `dst` beyond them is overwritten.
#[inline(always)]
fn compress8(r: &[f64; 8], dst: &mut [f64]) -> usize {
    #[cfg(all(target_arch = "x86_64", target_feature = "avx512f"))]
    {
        use std::arch::x86_64::*;
        let dst = &mut dst[..8];
        // SAFETY: AVX-512F is enabled at compile time; `r` and `dst` hold 8 f64.
        unsafe {
            let v = _mm512_loadu_pd(r.as_ptr());
            let m = _mm512_cmp_pd_mask::<_CMP_GT_OQ>(v, _mm512_setzero_pd());
            _mm512_storeu_pd(dst.as_mut_ptr(), _mm512_maskz_compress_pd(m, v));
            m.count_ones() as usize
        }
    }
    #[cfg(not(all(target_arch = "x86_64", target_feature = "avx512f")))]
    {
        let dst = &mut dst[..8];
        let mut k = 0;
        for &v in r {
            dst[k] = v;
            k += (v > 0.0) as usize;
        }
        k
    }
}

/// Kept elements gathered before they are added to the accumulators: a
/// multiple of V that divides BLOCK (1024 for V = 32). The gathering buffer
/// stays in L1 next to the input stream; a buffer of a whole block (32 KiB)
/// made the compaction about 40% slower (`PHASE7.md`).
const GATHER: usize = if LANES * 32 <= BLOCK && BLOCK % (LANES * 32) == 0 { LANES * 32 } else { BLOCK };

/// Adds `x` to the accumulators, element i to `acc[i mod V]`, in order.
#[inline(always)]
fn add_rows(acc: &mut [f64; LANES], x: &[f64]) {
    let (rows, tail) = x.as_chunks::<LANES>();
    for r in rows {
        for (a, v) in acc.iter_mut().zip(r) {
            *a += *v;
        }
    }
    for (a, v) in acc.iter_mut().zip(tail) {
        *a += *v;
    }
}

/// Streams the kept elements of `x` (those `> 0.0`) in order, skips the
/// first `skip` of them, and calls `emit` with the fixed-form block result
/// (`reduce_block`) of each following group of `BLOCK` kept elements, at
/// most `max_blocks` times. When `x` ends first, the last group is partial
/// (and emitted if not empty). The kept elements are compacted (AVX-512
/// `vcompresspd`) into a buffer of `GATHER`; each full buffer is added to the
/// V accumulators of the block (the k-th kept element of a block goes into
/// accumulator k mod V, `GATHER` being a multiple of V), and each full block
/// is reduced by the tree.
pub fn kept_blocks(x: &[f64], skip: usize, max_blocks: usize, mut emit: impl FnMut(f64)) {
    if max_blocks == 0 {
        return;
    }
    let mut p = 0;
    let mut s = skip;
    while s > 0 && p < x.len() {
        s -= (x[p] > 0.0) as usize;
        p += 1;
    }
    let (rows, tail) = x[p..].as_chunks::<8>();
    // the last elements, completed by elements that are not kept
    let mut last = [-1.0f64; 8];
    last[..tail.len()].copy_from_slice(tail);

    let mut buf = [0.0f64; GATHER + 8];
    let mut acc = [0.0f64; LANES];
    let mut k = 0usize; // kept elements in `buf`
    let mut kb = 0usize; // kept elements of the current block in `acc`
    let mut emitted = 0usize;
    // the rows, then `last` (one loop body, inlined: a closure kept `k` in memory)
    for i in 0..=rows.len() {
        let r = if i < rows.len() { &rows[i] } else { &last };
        k += compress8(r, &mut buf[k..]);
        if k >= GATHER {
            add_rows(&mut acc, &buf[..GATHER]);
            kb += GATHER;
            if kb == BLOCK {
                emit(tree(acc));
                emitted += 1;
                if emitted == max_blocks {
                    return;
                }
                acc = [0.0; LANES];
                kb = 0;
            }
            let rest: [f64; 8] = buf[GATHER..GATHER + 8].try_into().unwrap();
            buf[..8].copy_from_slice(&rest);
            k -= GATHER;
        }
    }
    if kb + k > 0 {
        add_rows(&mut acc, &buf[..k]);
        emit(tree(acc));
    }
}

/// The fixed form of the kept elements from the results of their blocks:
/// one block is its own result, several are reduced again (`fixed_sum`).
pub fn combine_blocks(parts: &[f64]) -> f64 {
    match parts.len() {
        0 => block_map(&[], |v| v),
        1 => parts[0],
        _ => crate::fixed_sum(parts),
    }
}

// ---------------------------------------------------------------------------
// prefix sum

/// Inclusive prefix sums of `x` into `out`, from `carry` (wrapping); returns
/// the last sum. AVX-512: each row of 8 is scanned in its register (three
/// shifts and additions), then the carry is added; the carry chain is one
/// vector addition per row.
pub fn scan_i64(x: &[i64], out: &mut [i64], mut carry: i64) -> i64 {
    assert_eq!(x.len(), out.len());
    #[cfg(not(all(target_arch = "x86_64", target_feature = "avx512f")))]
    let done = 0;
    #[cfg(all(target_arch = "x86_64", target_feature = "avx512f"))]
    let done = {
        use std::arch::x86_64::*;
        let (rows, _) = x.as_chunks::<8>();
        let (orows, _) = out.as_chunks_mut::<8>();
        // SAFETY: AVX-512F is enabled at compile time; every row holds 8 i64.
        unsafe {
            let zero = _mm512_setzero_si512();
            let last = _mm512_set1_epi64(7);
            let mut c = _mm512_set1_epi64(carry);
            for (r, o) in rows.iter().zip(orows.iter_mut()) {
                let mut v = _mm512_loadu_epi64(r.as_ptr());
                v = _mm512_add_epi64(v, _mm512_alignr_epi64::<7>(v, zero));
                v = _mm512_add_epi64(v, _mm512_alignr_epi64::<6>(v, zero));
                v = _mm512_add_epi64(v, _mm512_alignr_epi64::<4>(v, zero));
                _mm512_storeu_epi64(o.as_mut_ptr(), _mm512_add_epi64(v, c));
                c = _mm512_add_epi64(c, _mm512_permutexvar_epi64(last, v));
            }
            carry = _mm_cvtsi128_si64(_mm512_castsi512_si128(c));
        }
        rows.len() * 8
    };
    for (o, &v) in out[done..].iter_mut().zip(&x[done..]) {
        carry = carry.wrapping_add(v);
        *o = carry;
    }
    carry
}

/// Wrapping sum of `x` (vectorized by LLVM: integer addition is associative).
pub fn sum_i64(x: &[i64]) -> i64 {
    x.iter().fold(0i64, |a, &v| a.wrapping_add(v))
}

// ---------------------------------------------------------------------------
// matrix-vector product

/// The fixed form of `sum(row * v)`: products rounded, then added (no FMA).
#[inline(always)]
pub fn row_dot(row: &[f64], v: &[f64]) -> f64 {
    fixed(v.len(), |s, m| block_map2(&row[s..s + m], &v[s..s + m], |p, q| p * q))
}
