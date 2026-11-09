//! The symbols LLVM calls, for the runtime (the feature `symbols`): the
//! length tested first; below the long paths, the pointers turned into
//! slices, by the contract of each function (the only place where they
//! are, with `raw`), then the safe functions of the crate without a call;
//! from them on, a jump to the long path (`raw`).

use crate::{compare_from33, compare_upto32, copy_below32, copy_from65, copy_upto, copy_upto64, fill_from65, fill_short, fill_upto64, raw, Apart, Within, REP_MOVSB_MIN, REP_STOSB_MIN, VEC};
use core::ffi::{c_char, c_int, c_void};
use core::slice::{from_raw_parts, from_raw_parts_mut};

/// The two slices of a copy of `n` bytes.
///
/// # Safety
/// `n` bytes to read at `s` and to write at `d`, not overlapping, `n` not 0
/// (the pointers then not null).
#[inline(always)]
unsafe fn apart<'a>(d: *mut c_void, s: *const c_void, n: usize) -> Apart<'a> {
    // SAFETY: see above
    let (dst, src) = unsafe { (from_raw_parts_mut(d as *mut u8, n), from_raw_parts(s as *const u8, n)) };
    Apart::new(dst, src)
}

/// # Safety
/// `n` bytes to read at `s` and to write at `d`, not overlapping.
#[no_mangle]
pub unsafe extern "C" fn memcpy(d: *mut c_void, s: *const c_void, n: usize) -> *mut c_void {
    // as glibc: 32 to 64 bytes straight on, without a taken branch; fewer
    // than 32 (0 among them: a slice is never made of the null pointer a
    // copy of 0 bytes may be given); then the longer ones
    if n >= 32 {
        if n <= 64 {
            // SAFETY: the contract of `memcpy` (see above; `n` not 0)
            copy_upto64(&mut unsafe { apart(d, s, n) });
        } else if n < REP_MOVSB_MIN {
            // SAFETY: as above
            copy_from65(&mut unsafe { apart(d, s, n) });
        } else {
            // SAFETY: the contract of `memcpy` (see above)
            return unsafe { raw::copy_long(d as *mut u8, s as *const u8, n) as *mut c_void };
        }
    } else if n != 0 {
        // SAFETY: as above
        copy_below32(&mut unsafe { apart(d, s, n) });
    }
    d
}

/// # Safety
/// `n` bytes to read at `s` and to write at `d`, which may overlap.
#[no_mangle]
pub unsafe extern "C" fn memmove(d: *mut c_void, s: *const c_void, n: usize) -> *mut c_void {
    if (d as usize).abs_diff(s as usize) >= n {
        // SAFETY: the same contract, the two ranges apart
        unsafe { memcpy(d, s, n) }
    } else {
        // SAFETY: the same contract
        unsafe { move_overlapping(d, s, n) }
    }
}

/// `memmove` of two ranges that overlap (a function of its own: `memmove`
/// of ranges apart then jumps to `memcpy` without saving registers).
///
/// # Safety
/// As `memmove`; the ranges overlap.
#[inline(never)]
unsafe extern "C" fn move_overlapping(d: *mut c_void, s: *const c_void, n: usize) -> *mut c_void {
    let (to, from) = (d as usize, s as usize);
    // their union is one range of memory, each of its bytes in one of them,
    // which may be read and written (the contract of `memmove`)
    let lo = to.min(from);
    let len = to.max(from) + n - lo;
    let (f, t) = (from - lo, to - lo);
    if n <= 8 * VEC || (t < f && n < REP_MOVSB_MIN) {
        // SAFETY: the union of the two ranges (see above)
        let buf = unsafe { from_raw_parts_mut(lo as *mut u8, len) };
        copy_upto(&mut Within::new(buf, f, t, n));
    } else {
        // SAFETY: as above
        unsafe { raw::move_long(lo as *mut u8, len, f, t, n) };
    }
    d
}

/// # Safety
/// `n` bytes to write at `d`.
#[no_mangle]
pub unsafe extern "C" fn memset(d: *mut c_void, c: c_int, n: usize) -> *mut c_void {
    // as `memcpy`
    if n >= 32 {
        if n <= 64 {
            // SAFETY: the contract of `memset` (see above; `n` not 0)
            fill_upto64(unsafe { from_raw_parts_mut(d as *mut u8, n) }, c as u8);
        } else if n <= REP_STOSB_MIN {
            // SAFETY: as above
            fill_from65(unsafe { from_raw_parts_mut(d as *mut u8, n) }, c as u8);
        } else {
            // SAFETY: the contract of `memset` (see above)
            return unsafe { raw::fill_long(d as *mut u8, c as u8, n) as *mut c_void };
        }
    } else if n != 0 {
        // SAFETY: as above
        fill_short(unsafe { from_raw_parts_mut(d as *mut u8, n) }, c as u8);
    }
    d
}

/// # Safety
/// `n` bytes to read at `a` and at `b`.
#[no_mangle]
pub unsafe extern "C" fn memcmp(a: *const c_void, b: *const c_void, n: usize) -> c_int {
    // the short ones straight on (as glibc), then up to 8 vectors, then the
    // longer ones
    if n <= 32 {
        if n == 0 {
            return 0;
        }
        // SAFETY: the contract of `memcmp` (see above; `n` not 0)
        let (x, y) = unsafe { (from_raw_parts(a as *const u8, n), from_raw_parts(b as *const u8, n)) };
        compare_upto32(x, y)
    } else if n <= 8 * VEC {
        // SAFETY: as above
        let (x, y) = unsafe { (from_raw_parts(a as *const u8, n), from_raw_parts(b as *const u8, n)) };
        compare_from33(x, y)
    } else {
        // SAFETY: the contract of `memcmp` (see above)
        unsafe { raw::compare_long(a as *const u8, b as *const u8, n) }
    }
}

/// `memcmp` (as glibc: 0 if equal, else not 0).
///
/// # Safety
/// `n` bytes to read at `a` and at `b`.
#[no_mangle]
pub unsafe extern "C" fn bcmp(a: *const c_void, b: *const c_void, n: usize) -> c_int {
    // SAFETY: the same contract
    unsafe { memcmp(a, b, n) }
}

/// # Safety
/// `s` is a string ending with a zero byte.
#[no_mangle]
pub unsafe extern "C" fn strlen(s: *const c_char) -> usize {
    // SAFETY: the same contract
    unsafe { crate::length(s as *const u8) }
}
