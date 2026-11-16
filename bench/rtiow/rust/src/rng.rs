//! Reproducible stand-in for `rand::thread_rng` (bench/rtiow, not in the
//! original crate): one ChaCha8 generator per thread, started again from a
//! seed at the start of each pixel by the renderer, so that an image depends
//! only on its seed, not on the threads that drew it nor on the time.
//! `ThreadRng` is a handle to the generator of its thread, as the one of
//! `rand` is.
use std::cell::RefCell;

use rand::{RngCore, SeedableRng};
use rand_chacha::ChaCha8Rng;

thread_local! {
    static RNG: RefCell<ChaCha8Rng> = RefCell::new(ChaCha8Rng::seed_from_u64(0));
}

/// A handle to the generator of the current thread.
#[derive(Clone, Copy, Debug, Default)]
pub struct ThreadRng;

/// The generator of the current thread (seeded with 0 until `reseed`).
pub fn thread_rng() -> ThreadRng {
    ThreadRng
}

/// The generator of the current thread started again from `seed`.
pub fn reseed(seed: u64) {
    RNG.with(|r| *r.borrow_mut() = ChaCha8Rng::seed_from_u64(seed));
}

impl RngCore for ThreadRng {
    fn next_u32(&mut self) -> u32 {
        RNG.with(|r| r.borrow_mut().next_u32())
    }

    fn next_u64(&mut self) -> u64 {
        RNG.with(|r| r.borrow_mut().next_u64())
    }

    fn fill_bytes(&mut self, dest: &mut [u8]) {
        RNG.with(|r| r.borrow_mut().fill_bytes(dest))
    }

    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand::Error> {
        RNG.with(|r| r.borrow_mut().try_fill_bytes(dest))
    }
}
