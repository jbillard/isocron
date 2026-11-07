//! The functions of memory that LLVM calls (`memcpy`, `memmove`, `memset`,
//! `memcmp`, `bcmp`) and `strlen`, for programs without a C library (D287,
//! D288): the copies, fills and comparisons of a variable length of the
//! generated code and of the runtime.
//!
//! The way of glibc on the same processors, measured against it (D288): up
//! to 128 bytes, words or vectors read from both ends, overlapping, all of
//! them before the first write (so that the same code is right for bytes
//! that overlap, `memmove`), without a loop; up to 8 vectors the same way;
//! then a loop of 4 vectors to aligned places, the first vector and the
//! last 4 written after it; from 2 KiB on, `rep movsb` and `rep stosb`,
//! which the processors with ERMS run by lines of 64 bytes; from half the
//! last-level cache on, stores that do not go through the caches, 4 pages
//! at a time. `memcmp` compares one vector after the other up to 8, each
//! with its exit, then 4 at a time.
//!
//! Safety: the functions take the pointers LLVM gives, and turn them once
//! into slices (`&[u8]`, `&mut [u8]`), by the contract of each function; the
//! rest is safe code on those slices (their bounds checked, which the
//! compiler removes where it can prove them), except a few small functions
//! with a safe interface around what Rust cannot express safely: the
//! conversion of an array of bytes into a vector, the instructions of the
//! vectors (`vec`), `rep movsb` and `rep stosb`, the stores past the caches,
//! the masked loads of AVX-512, and `strlen`, whose length is not known
//! before it is read. Each `unsafe` block says why it is sound (`SAFETY`).
//!
//! The crate is always built optimised (the profile of the workspace says
//! it for its tests): unoptimised, a copy of a fixed size is a call of
//! `memcpy`, which would be this crate itself.
//!
//! LLVM recognises a loop that copies, fills or measures bytes and makes it
//! a call of `memcpy`, `memset` or `strlen`, which would be these functions
//! themselves: the index of each such loop goes through `opaque`, which LLVM
//! cannot see through. (`#![no_builtins]` would forbid it for the whole
//! crate, but takes the crate out of the optimisation at the link of the
//! runtime, whose `core` then lacks the functions the crate calls.)

#![no_std]
#![deny(unsafe_op_in_unsafe_fn)]
#![deny(clippy::undocumented_unsafe_blocks)]
#![cfg(target_arch = "x86_64")]

#[cfg(test)]
extern crate std;

use core::arch::asm;
use core::arch::x86_64::*;
use core::sync::atomic::{AtomicUsize, Ordering};

#[cfg(feature = "symbols")]
pub mod symbols;

// The vector of the variant: 32 bytes with AVX2 (x86-64-v3 and v4: glibc
// also keeps to 32 bytes on processors with AVX-512, whose 64-byte stores
// lower the clock of some of them), 16 otherwise. Each function is safe:
// the instructions it uses are those of the set the crate is compiled for
// (the `cfg` of the module), on registers.
#[cfg(target_feature = "avx2")]
mod vec {
    #![allow(dead_code)]
    use core::arch::x86_64::*;
    pub type V = __m256i;
    pub const VEC: usize = 32;
    pub const FULL: u32 = u32::MAX;
    #[inline(always)]
    pub fn of(b: [u8; VEC]) -> V {
        // SAFETY: an array of 32 bytes and a vector of 32 bytes have the same
        // size, and every pattern of bits is a vector
        unsafe { core::mem::transmute::<[u8; VEC], V>(b) }
    }
    #[inline(always)]
    pub fn bytes(v: V) -> [u8; VEC] {
        // SAFETY: as `of`, the other way
        unsafe { core::mem::transmute::<V, [u8; VEC]>(v) }
    }
    #[inline(always)]
    pub fn of16(b: [u8; 16]) -> __m128i {
        // SAFETY: as `of`, 16 bytes
        unsafe { core::mem::transmute::<[u8; 16], __m128i>(b) }
    }
    #[inline(always)]
    pub fn bytes16(v: __m128i) -> [u8; 16] {
        // SAFETY: as `of16`, the other way
        unsafe { core::mem::transmute::<__m128i, [u8; 16]>(v) }
    }
    #[inline(always)]
    pub fn splat(c: u8) -> V {
        // SAFETY: AVX2 is enabled (the `cfg` of the module)
        unsafe { _mm256_set1_epi8(c as i8) }
    }
    #[inline(always)]
    pub fn eq(a: V, b: V) -> V {
        // SAFETY: as `splat`
        unsafe { _mm256_cmpeq_epi8(a, b) }
    }
    #[inline(always)]
    pub fn mask(v: V) -> u32 {
        // SAFETY: as `splat`
        unsafe { _mm256_movemask_epi8(v) as u32 }
    }
    #[inline(always)]
    pub fn xor(a: V, b: V) -> V {
        // SAFETY: as `splat`
        unsafe { _mm256_xor_si256(a, b) }
    }
    #[inline(always)]
    pub fn or(a: V, b: V) -> V {
        // SAFETY: as `splat`
        unsafe { _mm256_or_si256(a, b) }
    }
    /// Whether a vector has a byte that is not 0.
    #[inline(always)]
    pub fn any(v: V) -> bool {
        // SAFETY: as `splat`
        unsafe { _mm256_testz_si256(v, v) == 0 }
    }
    /// `(x ^ y) | acc`, one instruction (AVX-512).
    #[cfg(all(target_feature = "avx512f", target_feature = "avx512vl"))]
    #[inline(always)]
    pub fn or_xor(acc: V, x: V, y: V) -> V {
        // SAFETY: AVX-512 F and VL are enabled (the `cfg`), on registers;
        // with A = x, B = acc, C = y, the table 0xde is (A ^ C) | B
        unsafe { _mm256_ternarylogic_epi32::<0xde>(x, acc, y) }
    }
    /// `a | b | c`, one instruction (AVX-512).
    #[cfg(all(target_feature = "avx512f", target_feature = "avx512vl"))]
    #[inline(always)]
    pub fn or3(a: V, b: V, c: V) -> V {
        // SAFETY: as `or_xor`; the table 0xfe is A | B | C
        unsafe { _mm256_ternarylogic_epi32::<0xfe>(a, b, c) }
    }
}

#[cfg(not(target_feature = "avx2"))]
mod vec {
    #![allow(dead_code)]
    use core::arch::x86_64::*;
    pub type V = __m128i;
    pub const VEC: usize = 16;
    pub const FULL: u32 = 0xffff;
    #[inline(always)]
    pub fn of(b: [u8; VEC]) -> V {
        // SAFETY: an array of 16 bytes and a vector of 16 bytes have the same
        // size, and every pattern of bits is a vector
        unsafe { core::mem::transmute::<[u8; VEC], V>(b) }
    }
    #[inline(always)]
    pub fn bytes(v: V) -> [u8; VEC] {
        // SAFETY: as `of`, the other way
        unsafe { core::mem::transmute::<V, [u8; VEC]>(v) }
    }
    #[inline(always)]
    pub fn of16(b: [u8; 16]) -> V {
        of(b)
    }
    #[inline(always)]
    pub fn bytes16(v: V) -> [u8; 16] {
        bytes(v)
    }
    #[inline(always)]
    pub fn splat(c: u8) -> V {
        // SAFETY: SSE2 is part of x86-64
        unsafe { _mm_set1_epi8(c as i8) }
    }
    #[inline(always)]
    pub fn eq(a: V, b: V) -> V {
        // SAFETY: as `splat`
        unsafe { _mm_cmpeq_epi8(a, b) }
    }
    #[inline(always)]
    pub fn mask(v: V) -> u32 {
        // SAFETY: as `splat`
        unsafe { _mm_movemask_epi8(v) as u32 }
    }
    #[inline(always)]
    pub fn and(a: V, b: V) -> V {
        // SAFETY: as `splat`
        unsafe { _mm_and_si128(a, b) }
    }
}

use vec::V;
pub(crate) use vec::VEC;

/// The mask of the bytes that differ in two vectors.
#[inline(always)]
fn ne(a: V, b: V) -> u32 {
    #[cfg(all(target_feature = "avx512bw", target_feature = "avx512vl"))]
    {
        // SAFETY: AVX-512 BW and VL are enabled (the `cfg`), on registers
        unsafe { _mm256_cmpneq_epi8_mask(a, b) }
    }
    #[cfg(not(all(target_feature = "avx512bw", target_feature = "avx512vl")))]
    {
        !vec::mask(vec::eq(a, b)) & vec::FULL
    }
}

/// `x`, through an instruction of no effect: LLVM no longer knows how the
/// index of a loop goes on, and does not make the loop a call of `memcpy`,
/// `memset` or `strlen`.
#[inline(always)]
fn opaque(x: usize) -> usize {
    let r;
    // SAFETY: no instruction; the value goes through a register
    unsafe { asm!("/* {0} */", inout(reg) x => r, options(pure, nomem, nostack, preserves_flags)) };
    r
}

/// Up to this length, the short copies and fills (from both ends).
const SHORT_MAX: usize = 128;
/// From this length on, `rep movsb` (as glibc on the processors with ERMS:
/// below it, the start of the instruction costs more than a loop of
/// vectors; 11.5 ns against 5.0 for 129 bytes on the Xeon E-2388G).
pub(crate) const REP_MOVSB_MIN: usize = 2112;
/// Above this length, `rep stosb` (as glibc).
pub(crate) const REP_STOSB_MIN: usize = 2048;
/// The size of a page, for the copies past the caches 4 pages at a time.
const PAGE: usize = 4096;

/// From this length on, stores that do not go through the caches: the
/// bytes would push out of the last-level cache more than they gain (half
/// its size, as glibc). Read once by CPUID; 8 MiB if it says nothing.
static NON_TEMPORAL_MIN: AtomicUsize = AtomicUsize::new(0);

#[inline(always)]
fn non_temporal_min() -> usize {
    let t = NON_TEMPORAL_MIN.load(Ordering::Relaxed);
    if t != 0 {
        t
    } else {
        non_temporal_min_init()
    }
}

#[cold]
#[inline(never)]
fn non_temporal_min_init() -> usize {
    // the deterministic cache parameters: leaf 4 (Intel), 0x8000001D (AMD)
    let mut l3 = 0usize;
    for leaf in [4u32, 0x8000_001d] {
        // the leaves are tested first
        let max = __cpuid(leaf & 0x8000_0000).eax;
        if max < leaf {
            continue;
        }
        for sub in 0..16 {
            let r = __cpuid_count(leaf, sub);
            if r.eax & 0x1f == 0 {
                break;
            }
            if (r.eax >> 5) & 7 == 3 {
                let ways = ((r.ebx >> 22) & 0x3ff) as usize + 1;
                let parts = ((r.ebx >> 12) & 0x3ff) as usize + 1;
                let line = (r.ebx & 0xfff) as usize + 1;
                let sets = r.ecx as usize + 1;
                l3 = ways * parts * line * sets;
            }
        }
        if l3 != 0 {
            break;
        }
    }
    let t = if l3 >= 1 << 20 { l3 / 2 } else { 8 << 20 };
    NON_TEMPORAL_MIN.store(t, Ordering::Relaxed);
    t
}

/// The bytes a copy reads and writes: two slices (`memcpy`), or two places
/// of one (`memmove` of bytes that overlap). The algorithms read all the
/// bytes of a step before they write them.
pub(crate) trait Moves {
    fn len(&self) -> usize;
    /// The address of the first byte written (for its alignment).
    fn dst_addr(&self) -> usize;
    fn read<const N: usize>(&self, i: usize) -> [u8; N];
    fn write<const N: usize>(&mut self, i: usize, v: [u8; N]);
    #[inline(always)]
    fn ld(&self, i: usize) -> V {
        vec::of(self.read::<VEC>(i))
    }
    #[inline(always)]
    fn st(&mut self, i: usize, v: V) {
        self.write::<VEC>(i, vec::bytes(v))
    }
    /// The 4 vectors at `i`.
    fn ld4(&self, i: usize) -> [V; 4];
    /// The 4 vectors `v` written at `i`.
    fn st4(&mut self, i: usize, v: [V; 4]);
    /// The loop of `copy_vectors`: by 4 vectors from `p` while below `end`
    /// (the last piece ends below `end + 4 VEC`, inside).
    #[inline(always)]
    fn copy_pieces(&mut self, mut p: usize, end: usize) {
        while p < end {
            let v = self.ld4(p);
            self.st4(p, v);
            p = opaque(p + 4 * VEC);
        }
    }
    #[inline(always)]
    fn ld16(&self, i: usize) -> __m128i {
        vec::of16(self.read::<16>(i))
    }
    #[inline(always)]
    fn st16(&mut self, i: usize, v: __m128i) {
        self.write::<16>(i, vec::bytes16(v))
    }
}

/// `n` bytes of `src` to `dst`, which do not overlap.
pub(crate) struct Apart<'a> {
    dst: &'a mut [u8],
    src: &'a [u8],
}

impl<'a> Apart<'a> {
    /// The two slices, of the same length (the shorter one's if they are
    /// not: never outside either).
    #[inline(always)]
    pub(crate) fn new(dst: &'a mut [u8], src: &'a [u8]) -> Self {
        debug_assert_eq!(dst.len(), src.len());
        let n = dst.len().min(src.len());
        let (dst, _) = dst.split_at_mut(n);
        let (src, _) = src.split_at(n);
        Apart { dst, src }
    }
}

impl Moves for Apart<'_> {
    #[inline(always)]
    fn len(&self) -> usize {
        self.src.len()
    }
    #[inline(always)]
    fn dst_addr(&self) -> usize {
        self.dst.as_ptr() as usize
    }
    #[inline(always)]
    fn read<const N: usize>(&self, i: usize) -> [u8; N] {
        piece(self.src, i)
    }
    #[inline(always)]
    fn write<const N: usize>(&mut self, i: usize, v: [u8; N]) {
        put(self.dst, i, v)
    }
    #[inline(always)]
    fn ld4(&self, i: usize) -> [V; 4] {
        vectors4(self.src, i)
    }
    #[inline(always)]
    fn st4(&mut self, i: usize, v: [V; 4]) {
        store4(self.dst, i, v)
    }
    /// The pieces of both slices in turn: their bounds checked once,
    /// before the loop (a load or a store of 32 bytes every 128 is not a
    /// loop LLVM makes a call of `memcpy`)
    #[inline(always)]
    fn copy_pieces(&mut self, p: usize, end: usize) {
        if p >= end {
            return;
        }
        let span = (end - p).div_ceil(4 * VEC) * 4 * VEC;
        if let (Some(dst), Some(src)) = (self.dst.get_mut(p..p + span), self.src.get(p..p + span)) {
            for (d, s) in dst.as_chunks_mut::<{ 4 * VEC }>().0.iter_mut().zip(src.as_chunks::<{ 4 * VEC }>().0) {
                store4(d, 0, vectors4(s, 0));
            }
        } else {
            debug_assert!(false, "pieces past the end");
        }
    }
}

// The accesses of the algorithms: always inside their slice (each
// algorithm says why); `get` rather than an index, so that no panic is
// possible in the short paths (a function that may call one aligns the
// stack: 2 instructions more for every length). A place outside would be
// a fault of the algorithm: the tests of the suite, built with the
// assertions of debugging, stop on it; the runtime would read zeros or
// write nothing, never outside.

/// The `N` bytes of `s` at `i`.
#[inline(always)]
fn piece<const N: usize>(s: &[u8], i: usize) -> [u8; N] {
    match s.get(i..i.wrapping_add(N)).and_then(|p| <[u8; N]>::try_from(p).ok()) {
        Some(v) => v,
        None => {
            debug_assert!(false, "read of {} bytes at {} past {}", N, i, s.len());
            [0; N]
        }
    }
}

/// The `N` bytes `v` written into `d` at `i`.
#[inline(always)]
fn put<const N: usize>(d: &mut [u8], i: usize, v: [u8; N]) {
    let len = d.len();
    match d.get_mut(i..i.wrapping_add(N)) {
        Some(p) => p.copy_from_slice(&v),
        None => debug_assert!(false, "write of {} bytes at {} past {}", N, i, len),
    }
}

/// The 4 vectors of `s` at `i` (one check of its bounds, the vectors at
/// constant places in the piece).
#[inline(always)]
fn vectors4(s: &[u8], i: usize) -> [V; 4] {
    let p: [u8; 4 * VEC] = piece(s, i);
    core::array::from_fn(|k| vec::of(piece(&p, k * VEC)))
}

/// The 4 vectors `v` written into `d` at `i`.
#[inline(always)]
fn store4(d: &mut [u8], i: usize, v: [V; 4]) {
    let mut p = [0u8; 4 * VEC];
    for (k, x) in v.into_iter().enumerate() {
        put(&mut p, k * VEC, vec::bytes(x));
    }
    put(d, i, p)
}

/// `n` bytes of `buf` from `from` to `to`, which may overlap.
pub(crate) struct Within<'a> {
    buf: &'a mut [u8],
    from: usize,
    to: usize,
    n: usize,
}

impl<'a> Within<'a> {
    /// The `n` bytes at `from` and at `to` in `buf` (fewer if they go
    /// past its end: never outside).
    #[inline(always)]
    pub(crate) fn new(buf: &'a mut [u8], from: usize, to: usize, n: usize) -> Self {
        debug_assert!(from.max(to) + n <= buf.len());
        let n = n.min(buf.len().saturating_sub(from.max(to)));
        Within { buf, from, to, n }
    }
}

impl Moves for Within<'_> {
    #[inline(always)]
    fn len(&self) -> usize {
        self.n
    }
    #[inline(always)]
    fn dst_addr(&self) -> usize {
        self.buf.as_ptr() as usize + self.to
    }
    #[inline(always)]
    fn read<const N: usize>(&self, i: usize) -> [u8; N] {
        piece(self.buf, self.from + i)
    }
    #[inline(always)]
    fn write<const N: usize>(&mut self, i: usize, v: [u8; N]) {
        put(self.buf, self.to + i, v)
    }
    #[inline(always)]
    fn ld4(&self, i: usize) -> [V; 4] {
        vectors4(self.buf, self.from + i)
    }
    #[inline(always)]
    fn st4(&mut self, i: usize, v: [V; 4]) {
        store4(self.buf, self.to + i, v)
    }
}

/// The offset in the destination of the first place aligned on `a` (a
/// power of 2) past its first byte.
#[inline(always)]
fn aligned_past<M: Moves>(m: &M, a: usize) -> usize {
    let d = m.dst_addr();
    ((d + a) & !(a - 1)) - d
}

/// Fewer than 32 bytes, from both ends, all read before any is written
/// (as the copies below: right for bytes that overlap).
#[inline(always)]
pub(crate) fn copy_below32<M: Moves>(m: &mut M) {
    let n = m.len();
    if n >= 16 {
        let (a, b) = (m.ld16(0), m.ld16(n - 16));
        m.st16(0, a);
        m.st16(n - 16, b);
    } else if n >= 8 {
        let (a, b) = (m.read::<8>(0), m.read::<8>(n - 8));
        m.write(0, a);
        m.write(n - 8, b);
    } else if n >= 4 {
        let (a, b) = (m.read::<4>(0), m.read::<4>(n - 4));
        m.write(0, a);
        m.write(n - 4, b);
    } else if n >= 2 {
        let (a, b) = (m.read::<2>(0), m.read::<2>(n - 2));
        m.write(0, a);
        m.write(n - 2, b);
    } else if n == 1 {
        let a = m.read::<1>(0);
        m.write(0, a);
    }
}

/// 32 to 64 bytes: 32 from each end.
#[inline(always)]
pub(crate) fn copy_upto64<M: Moves>(m: &mut M) {
    let n = m.len();
    #[cfg(target_feature = "avx2")]
    {
        let (a, b) = (m.ld(0), m.ld(n - 32));
        m.st(0, a);
        m.st(n - 32, b);
    }
    #[cfg(not(target_feature = "avx2"))]
    {
        let (a, b, c, e) = (m.ld(0), m.ld(16), m.ld(n - 32), m.ld(n - 16));
        m.st(0, a);
        m.st(16, b);
        m.st(n - 32, c);
        m.st(n - 16, e);
    }
}

/// 65 to 128 bytes: 64 from each end.
#[inline(always)]
fn copy_upto128<M: Moves>(m: &mut M) {
    let n = m.len();
    #[cfg(target_feature = "avx2")]
    {
        let (a, b, c, e) = (m.ld(0), m.ld(32), m.ld(n - 64), m.ld(n - 32));
        m.st(0, a);
        m.st(32, b);
        m.st(n - 64, c);
        m.st(n - 32, e);
    }
    #[cfg(not(target_feature = "avx2"))]
    {
        let (a, b, c, e) = (m.ld(0), m.ld(16), m.ld(32), m.ld(48));
        let (f, g, h, i) = (m.ld(n - 64), m.ld(n - 48), m.ld(n - 32), m.ld(n - 16));
        m.st(0, a);
        m.st(16, b);
        m.st(32, c);
        m.st(48, e);
        m.st(n - 64, f);
        m.st(n - 48, g);
        m.st(n - 32, h);
        m.st(n - 16, i);
    }
}

/// More than 4 vectors and at most 8: 4 from each end, all read before any
/// is written, without a loop.
#[inline(always)]
fn copy_eight<M: Moves>(m: &mut M) {
    let n = m.len();
    let e = n - 4 * VEC;
    let a = [m.ld(0), m.ld(VEC), m.ld(2 * VEC), m.ld(3 * VEC)];
    let b = [m.ld(e), m.ld(e + VEC), m.ld(e + 2 * VEC), m.ld(e + 3 * VEC)];
    for (k, v) in a.into_iter().enumerate() {
        m.st(k * VEC, v);
    }
    for (k, v) in b.into_iter().enumerate() {
        m.st(e + k * VEC, v);
    }
}

/// More than 128 bytes, forwards by 4 vectors to places of the destination
/// aligned on a vector: the first vector and the last 4 read before the
/// loop, written after it. Right for bytes that overlap when the
/// destination is below the source (a load of the loop is never below a
/// store of it).
#[inline(always)]
fn copy_vectors<M: Moves>(m: &mut M) {
    let n = m.len();
    let head = m.ld(0);
    let end = n - 4 * VEC;
    let tail = [m.ld(end), m.ld(end + VEC), m.ld(end + 2 * VEC), m.ld(end + 3 * VEC)];
    let p = aligned_past(m, VEC);
    m.copy_pieces(p, end);
    m.st(0, head);
    for (k, x) in tail.into_iter().enumerate() {
        m.st(end + k * VEC, x);
    }
}

/// A store of a vector past the caches, at a place of `dst` aligned on a
/// vector.
#[inline(always)]
fn stream(dst: &mut [u8], at: usize, v: V) {
    let c = &mut dst[at..at + VEC];
    assert!((c.as_ptr() as usize).is_multiple_of(VEC));
    // SAFETY: `c` is VEC bytes of `dst`, aligned on VEC (both checked)
    unsafe {
        #[cfg(target_feature = "avx2")]
        _mm256_stream_si256(c.as_mut_ptr() as *mut V, v);
        #[cfg(not(target_feature = "avx2"))]
        _mm_stream_si128(c.as_mut_ptr() as *mut V, v);
    }
}

/// The line at `at` of `src` asked for in the caches (a prefetch never
/// faults, whatever the address).
#[inline(always)]
fn prefetch(src: &[u8], at: usize) {
    // SAFETY: `prefetcht0` reads nothing the program sees and never faults
    unsafe { _mm_prefetch::<_MM_HINT_T0>(src.as_ptr().wrapping_add(at) as *const i8) }
}

/// The waits for the stores past the caches to be done (before the bytes
/// are read by another thread).
#[inline(always)]
fn store_fence() {
    // SAFETY: a fence, on no memory
    unsafe { _mm_sfence() }
}

/// More than 128 bytes past the caches: the stores on lines of 64 bytes,
/// which they then fill whole (a line half written leaves a buffer of the
/// processor half full, which goes to memory in parts when other lines
/// come between: the 4 pages below, D288), 4 pages at a time, a line of
/// each in turn (as glibc: the memory reads and writes 4 streams), the 16
/// vectors of a turn read before any is written (a store and the next
/// load of the same offset in a page would otherwise share their 12 low
/// bits of address when `dst` and `src` do: a load waits for such a store).
fn copy_streaming(m: &mut Apart) {
    let n = m.len();
    let head = [m.ld(0), m.ld(VEC), m.ld((2 * VEC) % 64), m.ld((3 * VEC) % 64)];
    let end = n - 4 * VEC;
    let tail = [m.ld(end), m.ld(end + VEC), m.ld(end + 2 * VEC), m.ld(end + 3 * VEC)];
    let mut p = aligned_past(m, 64);
    while p + 4 * PAGE <= end {
        let mut i = 0;
        while i < PAGE {
            // the 2 lines of the next turn in each page
            for k in 0..4 {
                prefetch(m.src, p + k * PAGE + i + 128);
                prefetch(m.src, p + k * PAGE + i + 192);
            }
            let mut v = [vec::splat(0); 16];
            for k in 0..4 {
                for j in 0..4 {
                    v[4 * k + j] = m.ld(p + k * PAGE + i + j * VEC);
                }
            }
            for k in 0..4 {
                for j in 0..4 {
                    stream(m.dst, p + k * PAGE + i + j * VEC, v[4 * k + j]);
                }
            }
            i = opaque(i + 4 * VEC);
        }
        p = opaque(p + 4 * PAGE);
    }
    while p < end {
        let v = [m.ld(p), m.ld(p + VEC), m.ld(p + 2 * VEC), m.ld(p + 3 * VEC)];
        for (k, x) in v.into_iter().enumerate() {
            stream(m.dst, p + k * VEC, x);
        }
        p = opaque(p + 4 * VEC);
    }
    store_fence();
    for (k, x) in head.into_iter().enumerate().take(64 / VEC) {
        m.st(k * VEC, x);
    }
    for (k, x) in tail.into_iter().enumerate() {
        m.st(end + k * VEC, x);
    }
}

/// `rep movsb` from `src` to `dst` (as long), its destination aligned on 64
/// bytes (the processor then writes whole lines, as glibc does it): the
/// first 64 bytes read before, written after.
fn rep_movsb(m: &mut Apart) {
    let n = m.len();
    let mut head = [vec::splat(0); 64 / VEC];
    for (j, h) in head.iter_mut().enumerate() {
        *h = m.ld(j * VEC);
    }
    let k = aligned_past(m, 64) & 63;
    let (dst, src) = (&mut m.dst[k..], &m.src[k..]);
    // SAFETY: `rep movsb` copies `rcx` bytes from `rsi` to `rdi`, forwards
    // (the direction flag is clear, as the ABI requires): the `n - k` bytes
    // of the two slices, which do not overlap
    unsafe {
        asm!("rep movsb", inout("rcx") n - k => _, inout("rdi") dst.as_mut_ptr() => _, inout("rsi") src.as_ptr() => _, options(nostack, preserves_flags));
    }
    for (j, h) in head.into_iter().enumerate() {
        m.st(j * VEC, h);
    }
}

/// `rep movsb` of bytes that overlap, the destination below the source
/// (forwards, byte after byte as far as the result goes: right then).
fn rep_movsb_down(m: &mut Within) {
    assert!(m.to < m.from);
    let (to, from, n) = (m.to, m.from, m.n);
    let p = m.buf.as_mut_ptr();
    // SAFETY: `from + n` and `to + n` are inside `buf` (checked by
    // `Within::new`), `to < from`: the copy forwards reads each byte before
    // the write that could reach it
    unsafe {
        asm!("rep movsb", inout("rcx") n => _, inout("rdi") p.add(to) => _, inout("rsi") p.add(from) => _, options(nostack, preserves_flags));
    }
}

/// Bytes that overlap, the destination above the source: backwards by 4
/// vectors from the end, the first 4 vectors read first (a store at `k`
/// never reaches a byte still to be read: they are below `from + k`, and
/// `to > from`).
fn copy_up(m: &mut Within) {
    let n = m.len();
    let head = [m.ld(0), m.ld(VEC), m.ld(2 * VEC), m.ld(3 * VEC)];
    let mut k = n;
    while k > 8 * VEC {
        k = opaque(k - 4 * VEC);
        let v = m.ld4(k);
        m.st4(k, v);
    }
    // the 4 vectors above the head (from `4 VEC` to `k`, at most 4 vectors,
    // ending at `k`), then the head
    let e = k - 4 * VEC;
    let v = [m.ld(e), m.ld(e + VEC), m.ld(e + 2 * VEC), m.ld(e + 3 * VEC)];
    for (j, x) in v.into_iter().enumerate() {
        m.st(e + j * VEC, x);
    }
    for (j, x) in head.into_iter().enumerate() {
        m.st(j * VEC, x);
    }
}

// The entries of the symbols (`symbols.rs`) test the length first: below
// the long paths, they turn their pointers into slices and go on without a
// call; from them on, they jump to a function of the C convention that
// turns them into slices itself (`raw`). The short lengths then need
// neither the stack nor saved registers (with a call, 2 to 3 cycles more
// for a copy of 2 bytes, D288).

/// `memcpy` of two slices of the same length.
pub fn copy(dst: &mut [u8], src: &[u8]) {
    assert_eq!(dst.len(), src.len());
    if src.len() < REP_MOVSB_MIN {
        copy_upto(&mut Apart::new(dst, src))
    } else {
        copy_long(dst, src)
    }
}

/// `copy` below `REP_MOVSB_MIN` bytes (or `copy_within` of bytes apart, or
/// downwards).
/// The short lengths first, as glibc.
#[inline(always)]
pub(crate) fn copy_upto<M: Moves>(m: &mut M) {
    let n = m.len();
    if n < 32 {
        copy_below32(m)
    } else {
        copy_from32(m)
    }
}

/// `copy_upto` of 32 bytes or more.
#[inline(always)]
fn copy_from32<M: Moves>(m: &mut M) {
    if m.len() <= 64 {
        copy_upto64(m)
    } else {
        copy_from65(m)
    }
}

/// `copy_upto` of more than 64 bytes.
#[inline(always)]
pub(crate) fn copy_from65<M: Moves>(m: &mut M) {
    let n = m.len();
    if n <= SHORT_MAX {
        copy_upto128(m)
    } else if n <= 8 * VEC {
        copy_eight(m)
    } else {
        copy_vectors(m)
    }
}

/// `copy` from `REP_MOVSB_MIN` bytes on.
#[inline(always)]
fn copy_long(dst: &mut [u8], src: &[u8]) {
    let mut m = Apart::new(dst, src);
    if m.len() < non_temporal_min() {
        rep_movsb(&mut m)
    } else {
        copy_streaming(&mut m)
    }
}

/// `memmove` of `n` bytes of `buf` from `from` to `to`, which may overlap.
pub fn copy_within(buf: &mut [u8], from: usize, to: usize, n: usize) {
    assert!(from.max(to) + n <= buf.len());
    if n <= 8 * VEC || (to < from && n < REP_MOVSB_MIN) {
        copy_upto(&mut Within::new(buf, from, to, n))
    } else {
        move_long(buf, from, to, n)
    }
}

/// `copy_within` of more than 8 vectors: from `REP_MOVSB_MIN` bytes on
/// downwards, or upwards.
#[inline(always)]
fn move_long(buf: &mut [u8], from: usize, to: usize, n: usize) {
    let mut m = Within::new(buf, from, to, n);
    if to < from {
        rep_movsb_down(&mut m)
    } else if to > from {
        copy_up(&mut m)
    }
}

/// Fewer than 32 bytes set to `c`: two stores that overlap.
#[inline(always)]
pub(crate) fn fill_short(d: &mut [u8], c: u8) {
    let n = d.len();
    if n >= 16 {
        put(d, 0, [c; 16]);
        put(d, n - 16, [c; 16]);
    } else if n >= 8 {
        put(d, 0, [c; 8]);
        put(d, n - 8, [c; 8]);
    } else if n >= 4 {
        put(d, 0, [c; 4]);
        put(d, n - 4, [c; 4]);
    } else if n >= 2 {
        put(d, 0, [c; 2]);
        put(d, n - 2, [c; 2]);
    } else if n == 1 {
        put(d, 0, [c; 1]);
    }
}

/// More than 128 bytes set to `c`: the first 64 and the last 4 vectors, the
/// loop between them by 4 vectors to aligned places (through the caches,
/// or past them on lines of 64 bytes, as `copy_streaming`).
#[inline(always)]
fn fill_vectors(d: &mut [u8], c: u8, through: bool) {
    let n = d.len();
    let w = vec::bytes(vec::splat(c));
    let end = n - 4 * VEC;
    let a = d.as_ptr() as usize;
    let align = if through { VEC } else { 64 };
    let mut p = ((a + align) & !(align - 1)) - a;
    put(d, 0, [c; 64]);
    if through {
        let v = vec::splat(c);
        while p < end {
            store4(d, p, [v; 4]);
            p = opaque(p + 4 * VEC);
        }
    } else {
        let v = vec::splat(c);
        while p < end {
            for k in 0..4 {
                stream(d, p + k * VEC, v);
            }
            p = opaque(p + 4 * VEC);
        }
        store_fence();
    }
    for k in 0..4 {
        put(d, end + k * VEC, w);
    }
}

/// `memset` of a slice.
pub fn fill(d: &mut [u8], c: u8) {
    if d.len() <= REP_STOSB_MIN {
        fill_upto(d, c)
    } else {
        fill_long(d, c)
    }
}

/// `fill` of at most `REP_STOSB_MIN` bytes.
#[inline(always)]
pub(crate) fn fill_upto(d: &mut [u8], c: u8) {
    if d.len() < 32 {
        fill_short(d, c)
    } else {
        fill_from32(d, c)
    }
}

/// `fill_upto` of 32 bytes or more.
#[inline(always)]
fn fill_from32(d: &mut [u8], c: u8) {
    if d.len() <= 64 {
        fill_upto64(d, c)
    } else {
        fill_from65(d, c)
    }
}

/// 32 to 64 bytes set to `c`: 32 from each end.
#[inline(always)]
pub(crate) fn fill_upto64(d: &mut [u8], c: u8) {
    let n = d.len();
    put(d, 0, [c; 32]);
    put(d, n - 32, [c; 32]);
}

/// More than 64 bytes set to `c`, at most `REP_STOSB_MIN`.
#[inline(always)]
pub(crate) fn fill_from65(d: &mut [u8], c: u8) {
    let n = d.len();
    if n <= SHORT_MAX {
        put(d, 0, [c; 32]);
        put(d, 32, [c; 32]);
        put(d, n - 64, [c; 32]);
        put(d, n - 32, [c; 32]);
    } else {
        fill_vectors(d, c, true)
    }
}

/// `fill` of more than `REP_STOSB_MIN` bytes.
#[inline(always)]
fn fill_long(d: &mut [u8], c: u8) {
    if d.len() < non_temporal_min() {
        // SAFETY: `rep stosb` writes `rcx` bytes at `rdi` (forwards: the
        // direction flag is clear): the bytes of `d`
        unsafe {
            asm!("rep stosb", inout("rcx") d.len() => _, inout("rdi") d.as_mut_ptr() => _, in("al") c, options(nostack, preserves_flags));
        }
    } else {
        fill_vectors(d, c, false)
    }
}

/// The vector of the bytes of `s` at `i`.
#[inline(always)]
fn ld(s: &[u8], i: usize) -> V {
    vec::of(piece(s, i))
}

/// The difference of the bytes at `k` (as unsigned bytes). `k` is always
/// inside (the place of a lane of a mask of bytes of both); `get` instead
/// of an index keeps a panic out of the short paths, which would need the
/// stack.
#[inline(always)]
fn byte_order(a: &[u8], b: &[u8], k: usize) -> i32 {
    debug_assert!(k < a.len());
    match (a.get(k), b.get(k)) {
        (Some(&x), Some(&y)) => x as i32 - y as i32,
        _ => 0,
    }
}

/// The mask of the bytes that differ in the vectors at `i`.
#[inline(always)]
fn at(a: &[u8], b: &[u8], i: usize) -> u32 {
    ne(ld(a, i), ld(b, i))
}

/// The order of the first different byte of the vector at `i`, given its
/// mask (not 0).
#[inline(always)]
fn order_at(a: &[u8], b: &[u8], i: usize, m: u32) -> i32 {
    byte_order(a, b, i + m.trailing_zeros() as usize)
}

/// With AVX-512, the mask of the bytes that differ in `a` and `b` (at most
/// 32), by one masked load of each: the lanes past their length are not
/// read.
#[cfg(all(target_feature = "avx512bw", target_feature = "avx512vl"))]
#[inline(always)]
fn masked_ne(a: &[u8], b: &[u8]) -> u32 {
    debug_assert!(a.len() == b.len() && a.len() <= 32);
    let n = a.len().min(b.len()).min(32);
    let k = ((1u64 << n) - 1) as __mmask32;
    // SAFETY: the masked loads read the lanes of `k`, the first `n` bytes of
    // each slice (`n` at most the length of each), and no other (the processor does not fault on the others);
    // AVX-512 BW and VL are enabled (the `cfg`)
    unsafe { _mm256_mask_cmpneq_epu8_mask(k, _mm256_maskz_loadu_epi8(k, a.as_ptr() as *const i8), _mm256_maskz_loadu_epi8(k, b.as_ptr() as *const i8)) }
}

/// The first 4 bytes of `s`, then its last 4, as one big-endian word of 8.
#[cfg(not(all(target_feature = "avx512bw", target_feature = "avx512vl")))]
#[inline(always)]
fn first_and_last4(s: &[u8]) -> u64 {
    let n = s.len();
    (u32::from_be_bytes(piece(s, 0)) as u64) << 32 | u32::from_be_bytes(piece(s, n - 4)) as u64
}

/// Below a vector, without AVX-512: 16 bytes twice, or words of 8 and 4
/// bytes read from both ends, or the bytes themselves.
#[cfg(not(all(target_feature = "avx512bw", target_feature = "avx512vl")))]
#[inline(always)]
fn compare_short(a: &[u8], b: &[u8]) -> i32 {
    let n = a.len();
    if n >= 16 {
        for start in [0, n - 16] {
            let x = u128::from_be_bytes(piece(a, start));
            let y = u128::from_be_bytes(piece(b, start));
            if x != y {
                return if x > y { 1 } else { -1 };
            }
        }
        return 0;
    }
    if n >= 8 {
        for start in [0, n - 8] {
            let x = u64::from_be_bytes(piece(a, start));
            let y = u64::from_be_bytes(piece(b, start));
            if x != y {
                return if x > y { 1 } else { -1 };
            }
        }
        return 0;
    }
    if n >= 4 {
        // the first 4 bytes, then the last 4, as one word of 8
        let (x, y) = (first_and_last4(a), first_and_last4(b));
        return (x > y) as i32 - (x < y) as i32;
    }
    for (k, (x, y)) in a.iter().zip(b).enumerate() {
        if x != y {
            return byte_order(a, b, k);
        }
    }
    0
}

/// Whether the 4 vectors at `i` differ: their differences (xor) or-ed and
/// tested once with AVX2 (as glibc; LLVM makes it 2 ternary operations with
/// AVX-512), the vectors of equal bytes and-ed otherwise.
#[inline(always)]
fn differ4(a: &[u8], b: &[u8], i: usize) -> bool {
    // AVX-512: 3 differences, the fourth and the or of all by 2 ternary
    // operations, one test (as glibc; LLVM makes the form of AVX2 below 4
    // comparisons into masks, which this processor runs on one port only:
    // 100.9 cycles against 86.4 for 2 KiB)
    #[cfg(all(target_feature = "avx512f", target_feature = "avx512vl"))]
    {
        let x0 = vec::xor(ld(a, i), ld(b, i));
        let x1 = vec::xor(ld(a, i + VEC), ld(b, i + VEC));
        let x2 = vec::xor(ld(a, i + 2 * VEC), ld(b, i + 2 * VEC));
        let t = vec::or_xor(x0, ld(a, i + 3 * VEC), ld(b, i + 3 * VEC));
        vec::any(vec::or3(t, x1, x2))
    }
    #[cfg(all(target_feature = "avx2", not(all(target_feature = "avx512f", target_feature = "avx512vl"))))]
    {
        let x0 = vec::xor(ld(a, i), ld(b, i));
        let x1 = vec::xor(ld(a, i + VEC), ld(b, i + VEC));
        let x2 = vec::xor(ld(a, i + 2 * VEC), ld(b, i + 2 * VEC));
        let x3 = vec::xor(ld(a, i + 3 * VEC), ld(b, i + 3 * VEC));
        vec::any(vec::or(vec::or(x0, x1), vec::or(x2, x3)))
    }
    #[cfg(not(target_feature = "avx2"))]
    {
        let e0 = vec::eq(ld(a, i), ld(b, i));
        let e1 = vec::eq(ld(a, i + VEC), ld(b, i + VEC));
        let e2 = vec::eq(ld(a, i + 2 * VEC), ld(b, i + 2 * VEC));
        let e3 = vec::eq(ld(a, i + 3 * VEC), ld(b, i + 3 * VEC));
        vec::mask(vec::and(vec::and(e0, e1), vec::and(e2, e3))) != vec::FULL
    }
}

/// Whether the 2 vectors at `i` differ.
#[inline(always)]
fn differ2(a: &[u8], b: &[u8], i: usize) -> bool {
    #[cfg(all(target_feature = "avx512f", target_feature = "avx512vl"))]
    {
        vec::any(vec::or_xor(vec::xor(ld(a, i), ld(b, i)), ld(a, i + VEC), ld(b, i + VEC)))
    }
    #[cfg(all(target_feature = "avx2", not(all(target_feature = "avx512f", target_feature = "avx512vl"))))]
    {
        vec::any(vec::or(vec::xor(ld(a, i), ld(b, i)), vec::xor(ld(a, i + VEC), ld(b, i + VEC))))
    }
    #[cfg(not(target_feature = "avx2"))]
    {
        vec::mask(vec::and(vec::eq(ld(a, i), ld(b, i)), vec::eq(ld(a, i + VEC), ld(b, i + VEC)))) != vec::FULL
    }
}

/// The order of the first different byte among the 4 vectors at `i`.
#[inline(always)]
fn locate4(a: &[u8], b: &[u8], i: usize) -> i32 {
    for k in 0..4 {
        let m = at(a, b, i + k * VEC);
        if m != 0 {
            return order_at(a, b, i + k * VEC, m);
        }
    }
    0
}

/// The last 4 vectors (the bytes before them equal).
#[inline(always)]
fn compare_last4(a: &[u8], b: &[u8]) -> i32 {
    let e = a.len() - 4 * VEC;
    if differ4(a, b, e) {
        locate4(a, b, e)
    } else {
        0
    }
}

/// The two slices of a comparison, of the same length (the shorter one's
/// if they are not: never outside either).
#[inline(always)]
fn same_length<'a>(a: &'a [u8], b: &'a [u8]) -> (&'a [u8], &'a [u8]) {
    debug_assert_eq!(a.len(), b.len());
    let n = a.len().min(b.len());
    (a.split_at(n).0, b.split_at(n).0)
}

/// `memcmp` of two slices of the same length: the sign of the first
/// different byte (as unsigned bytes), 0 if they are equal.
pub fn compare(a: &[u8], b: &[u8]) -> i32 {
    assert_eq!(a.len(), b.len());
    if a.len() <= 8 * VEC {
        compare_upto(a, b)
    } else {
        compare_long(a, b)
    }
}

/// `compare` of at most 8 vectors: one vector after the other, each with its
/// exit (the last ones ending at the end, overlapping: the first window with
/// a difference has the first one).
#[inline(always)]
fn compare_upto(a: &[u8], b: &[u8]) -> i32 {
    let (a, b) = same_length(a, b);
    if a.len() <= 32 {
        compare_upto32(a, b)
    } else {
        compare_from33(a, b)
    }
}

/// `compare` of at most 32 bytes: one masked load of each with AVX-512, as
/// glibc.
#[inline(always)]
pub(crate) fn compare_upto32(a: &[u8], b: &[u8]) -> i32 {
    #[cfg(all(target_feature = "avx512bw", target_feature = "avx512vl"))]
    {
        let m = masked_ne(a, b);
        if m == 0 {
            0
        } else {
            order_at(a, b, 0, m)
        }
    }
    #[cfg(not(all(target_feature = "avx512bw", target_feature = "avx512vl")))]
    {
        if a.len() < VEC {
            compare_short(a, b)
        } else {
            compare_from_vec(a, b)
        }
    }
}

/// `compare` of more than 32 bytes, at most 8 vectors.
#[inline(always)]
pub(crate) fn compare_from33(a: &[u8], b: &[u8]) -> i32 {
    compare_from_vec(a, b)
}

/// `compare` of a vector at least, at most 8.
#[inline(always)]
fn compare_from_vec(a: &[u8], b: &[u8]) -> i32 {
    let n = a.len();
    let m = at(a, b, 0);
    if m != 0 {
        return order_at(a, b, 0, m);
    }
    if n <= 2 * VEC {
        let m = at(a, b, n - VEC);
        return if m == 0 { 0 } else { order_at(a, b, n - VEC, m) };
    }
    let m = at(a, b, VEC);
    if m != 0 {
        return order_at(a, b, VEC, m);
    }
    if n <= 4 * VEC {
        // the last 2 vectors tested at once
        let e = n - 2 * VEC;
        if !differ2(a, b, e) {
            return 0;
        }
        let m = at(a, b, e);
        return if m != 0 { order_at(a, b, e, m) } else { order_at(a, b, e + VEC, at(a, b, e + VEC)) };
    }
    for start in [2 * VEC, 3 * VEC] {
        let m = at(a, b, start);
        if m != 0 {
            return order_at(a, b, start, m);
        }
    }
    compare_last4(a, b)
}

/// `compare` of more than 8 vectors: the first 4, then by 4 vectors from
/// where `a` is aligned (as glibc: its loads then never span two lines),
/// the pieces of 4 vectors of both in turn (arrays: no bounds to check in
/// the loop), then the last 4.
#[inline(always)]
fn compare_long(a: &[u8], b: &[u8]) -> i32 {
    let (a, b) = same_length(a, b);
    if differ4(a, b, 0) {
        return locate4(a, b, 0);
    }
    // at most 4 vectors: inside, below the last 4 (more than 8 in all)
    let i0 = 4 * VEC - (a.as_ptr() as usize & (VEC - 1));
    let (pa, pb) = (a.get(i0..).unwrap_or_default(), b.get(i0..).unwrap_or_default());
    for (k, (x, y)) in pa.as_chunks::<{ 4 * VEC }>().0.iter().zip(pb.as_chunks::<{ 4 * VEC }>().0).enumerate() {
        if differ4(x, y, 0) {
            return locate4(a, b, i0 + k * 4 * VEC);
        }
    }
    compare_last4(a, b)
}

/// The `VEC` bytes of the block aligned on `VEC` bytes at `p`, by one load
/// written in assembly (the way of glibc and of `compiler-builtins` for
/// `strlen`): the block may hold bytes before and after the string, which
/// Rust may not read, but which the processor reads as any other.
///
/// # Safety
/// `p` is aligned on `VEC` bytes and one byte of its block is readable: a
/// block aligned on its size lies within one page, readable as a whole.
#[inline(always)]
unsafe fn aligned_block(p: *const u8) -> vec::V {
    let v;
    // SAFETY: the contract above; a load alone, no flag changed
    #[cfg(target_feature = "avx2")]
    unsafe {
        asm!("vmovdqa {v}, ymmword ptr [{p}]", p = in(reg) p, v = lateout(ymm_reg) v, options(pure, readonly, nostack, preserves_flags))
    };
    // SAFETY: the same
    #[cfg(not(target_feature = "avx2"))]
    unsafe {
        asm!("movdqa {v}, xmmword ptr [{p}]", p = in(reg) p, v = lateout(xmm_reg) v, options(pure, readonly, nostack, preserves_flags))
    };
    v
}

/// `strlen`: the bytes before the first zero byte of `s`, one aligned
/// vector at a time (D400: one byte at a time before, 1.4 us of the 2.8 of
/// `sys.envAll` for its 2490 bytes).
///
/// # Safety
/// `s` is a string ending with a zero byte.
pub unsafe fn length(s: *const u8) -> usize {
    let off = s as usize % vec::VEC;
    let zero = vec::splat(0);
    // SAFETY: the block of `s` aligned on VEC holds the byte at `s`, which
    // is readable (the contract)
    let m = vec::mask(vec::eq(unsafe { aligned_block(s.wrapping_sub(off)) }, zero)) >> off;
    if m != 0 {
        return m.trailing_zeros() as usize;
    }
    let mut n = vec::VEC - off;
    loop {
        // SAFETY: no zero byte in the `n` bytes from `s`: the byte at
        // `s + n` belongs to the string, readable, and `s + n` is aligned on
        // VEC
        let m = vec::mask(vec::eq(unsafe { aligned_block(s.add(n)) }, zero));
        if m != 0 {
            return n + m.trailing_zeros() as usize;
        }
        n += vec::VEC;
    }
}

/// The long paths in the C convention, for the symbols, which jump to them
/// (see above).
#[cfg(feature = "symbols")]
pub(crate) mod raw {
    use core::slice::{from_raw_parts, from_raw_parts_mut};

    /// # Safety
    /// The contract of `memcpy`, for `n` bytes.
    #[inline(never)]
    pub unsafe extern "C" fn copy_long(d: *mut u8, s: *const u8, n: usize) -> *mut u8 {
        // SAFETY: the contract of `memcpy` (see above; `n` not 0)
        let (dst, src) = unsafe { (from_raw_parts_mut(d, n), from_raw_parts(s, n)) };
        super::copy_long(dst, src);
        // through `opaque`: otherwise LLVM sees that the result is `d`, keeps
        // `d` in a saved register in `memcpy` and calls instead of jumping
        super::opaque(d as usize) as *mut u8
    }

    /// # Safety
    /// `n` bytes from `from` to `to` in the `len` bytes at `p`, which may be
    /// read and written.
    #[inline(never)]
    pub unsafe extern "C" fn move_long(p: *mut u8, len: usize, from: usize, to: usize, n: usize) {
        // SAFETY: see above
        let buf = unsafe { from_raw_parts_mut(p, len) };
        super::move_long(buf, from, to, n)
    }

    /// # Safety
    /// The contract of `memset`, for `n` bytes.
    #[inline(never)]
    pub unsafe extern "C" fn fill_long(d: *mut u8, c: u8, n: usize) -> *mut u8 {
        // SAFETY: the contract of `memset` (see above; `n` not 0)
        let dst = unsafe { from_raw_parts_mut(d, n) };
        super::fill_long(dst, c);
        // through `opaque`, as `copy_long`
        super::opaque(d as usize) as *mut u8
    }

    /// # Safety
    /// The contract of `memcmp`, for `n` bytes.
    #[inline(never)]
    pub unsafe extern "C" fn compare_long(a: *const u8, b: *const u8, n: usize) -> i32 {
        // SAFETY: the contract of `memcmp` (see above; `n` not 0)
        let (x, y) = unsafe { (from_raw_parts(a, n), from_raw_parts(b, n)) };
        super::compare_long(x, y)
    }
}

#[cfg(test)]
mod tests;
