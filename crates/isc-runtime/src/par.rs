//! The parallel loops of the runtime itself, safe (D290): a closure called
//! for each part on the pool (`run`), or for each part of a slice cut in
//! disjoint ranges, each given as its own `&mut` slice (`parts_mut`), as
//! rayon does. The tasks of the pool take a pointer to their context; the
//! only code that turns it back into the closure, and the ranges into
//! slices, is here, each time with its reason.
//!
//! The closures are `Copy` (they capture references and numbers): the pool
//! copies a context of at most 64 bytes into its job line (D201), which is
//! then a copy of the closure, as good as the closure itself.

use super::{parallel_for, Task};
use core::ffi::c_void;
use core::marker::PhantomData;

/// How the tasks are shared out among the threads.
#[derive(Clone, Copy)]
#[allow(dead_code)]
pub enum Order {
    /// Every `threads`-th task to each thread (close places of memory for
    /// all of them: the loops that read, D65).
    Interleaved,
    /// A contiguous range of the tasks to each thread (each writes its own
    /// pages: the loops that fill, D254).
    Contiguous,
}

/// `f(k)` for each `k` of `0..n` on the pool; returns when all are done.
pub fn run<F: Fn(usize) + Sync + Copy>(n: usize, order: Order, f: F) {
    /// # Safety
    /// `ctx` the closure that `run` gives (P1).
    unsafe extern "C" fn task<F: Fn(usize) + Sync + Copy>(ctx: *mut c_void, k: i64) {
        // SAFETY: `ctx` is the closure `run` gave (or its copy in the job
        // line, as good: `F: Copy`), alive until `parallel_for` returns
        let f = unsafe { &*(ctx as *const F) };
        f(k as usize)
    }
    let t: Task = task::<F>;
    // SAFETY: P1: the task reads its closure, `F: Sync` (shared by the
    // threads), `F: Copy` (its bytes may be copied: `size_of::<F>()`), alive
    // until the loop returns; each `k` once
    unsafe { parallel_for(n as i64, t, &f as *const F as *mut c_void, core::mem::size_of::<F>() as i64, false, matches!(order, Order::Contiguous)) };
}

/// A slice cut at `bounds` (increasing, its first 0 and its last the
/// length of the slice): its parts, each given to one task.
pub struct Parts<'a, T> {
    base: *mut T,
    bounds: &'a [usize],
    _slice: PhantomData<&'a mut [T]>,
}

// SAFETY: the parts are disjoint ranges of a slice of `T: Send`: each task
// has its own (see `Parts::new`)
unsafe impl<T: Send> Sync for Parts<'_, T> {}

impl<'a, T: Send> Parts<'a, T> {
    /// The parts of `slice` at `bounds`; a panic if `bounds` does not cut
    /// the whole slice in increasing ranges.
    pub fn new(slice: &'a mut [T], bounds: &'a [usize]) -> Self {
        assert!(!bounds.is_empty() && bounds[0] == 0 && bounds[bounds.len() - 1] == slice.len());
        assert!(bounds.windows(2).all(|w| w[0] <= w[1]));
        Parts { base: slice.as_mut_ptr(), bounds, _slice: PhantomData }
    }

    /// The count of parts.
    pub fn len(&self) -> usize {
        self.bounds.len() - 1
    }

    /// The part `k`.
    ///
    /// # Safety
    /// One task at a time for each `k` (the tasks of a `run`: each `k` once).
    unsafe fn part(&self, k: usize) -> &'a mut [T] {
        let (a, e) = (self.bounds[k], self.bounds[k + 1]);
        // SAFETY: `[a, e)` is inside the slice and disjoint from the other
        // parts (`new` checked the bounds); one task has it (see above)
        unsafe { core::slice::from_raw_parts_mut(self.base.add(a), e - a) }
    }
}

/// `f(k, part)` for each part `k` of `parts` on the pool, each part its own
/// `&mut` slice; returns when all are done.
pub fn parts_mut<T: Send, F: Fn(usize, &mut [T]) + Sync + Copy>(parts: &Parts<'_, T>, order: Order, f: F) {
    run(parts.len(), order, |k| {
        // SAFETY: `run` calls each `k` once
        let part = unsafe { parts.part(k) };
        f(k, part)
    });
}

/// `f(k, part_a, part_b)` for each part `k` of two slices cut in as many
/// parts (`parts_mut` of both at once); returns when all are done.
pub fn parts_mut2<T: Send, U: Send, F: Fn(usize, &mut [T], &mut [U]) + Sync + Copy>(a: &Parts<'_, T>, b: &Parts<'_, U>, order: Order, f: F) {
    assert_eq!(a.len(), b.len());
    run(a.len(), order, |k| {
        // SAFETY: `run` calls each `k` once
        let (x, y) = unsafe { (a.part(k), b.part(k)) };
        f(k, x, y)
    });
}
