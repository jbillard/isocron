//! The allocator checked (`ISC_ALLOC=check`, D546): the blocks of the
//! allocator of Isocron, each with a header before it and a guard after it,
//! so that a program without glibc is checked as Valgrind checks one with
//! it. A block freed is checked (its header: not freed twice, given by this
//! allocator; its guard: not written past its end), filled with a poison
//! and kept in a quarantine; leaving it, its poison is checked (not written
//! after its free). At the end of `main`, the quarantine is checked and the
//! blocks of the program still allocated are counted (a leak). The reads of
//! a block freed are not seen (Valgrind sees them).
//!
//! The compiled code calls the runtime for every block in this mode (that
//! of the system's allocator, `HEAP_MODE` 1): its cells of nodes, its
//! regions, its strings grown in place are blocks of their own.

use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// The header, in the 64 bytes before the block: its state, its size, its
/// kind (the program's, or the runtime's own: not counted as a leak).
const LIVE: u64 = 0x1500_c0de_a110_c8ed;
const FREED: u64 = 0x1500_c0de_f4ee_d0ed;
const GUARD: u8 = 0xCA;
const POISON: u8 = 0xDF;
const HEADER: usize = 64;
const TAIL: usize = 64;

/// The blocks freed, oldest first: a ring of `RING` blocks, at most
/// `QUARANTINE` bytes.
const RING: usize = 4096;
const QUARANTINE: usize = 64 << 20;

pub static CHECKED: AtomicBool = AtomicBool::new(false);
static LIVE_BLOCKS: AtomicUsize = AtomicUsize::new(0);
static LIVE_BYTES: AtomicUsize = AtomicUsize::new(0);

struct Quarantine {
    ring: [usize; RING],
    head: usize,
    len: usize,
    bytes: usize,
}

static LOCK: AtomicBool = AtomicBool::new(false);
static mut QUEUE: Quarantine = Quarantine { ring: [0; RING], head: 0, len: 0, bytes: 0 };

fn lock() {
    while LOCK.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
        core::hint::spin_loop();
    }
}

fn unlock() {
    LOCK.store(false, Ordering::Release);
}

fn fault(what: &str) -> ! {
    super::die(0, 0, &Fault(what))
}

struct Fault<'a>(&'a str);

impl core::fmt::Display for Fault<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        write!(f, "the allocator (ISC_ALLOC=check): {}", self.0)
    }
}

/// The header of the block `p`: its state, its size, its kind.
///
/// # Safety
/// `p` a block of `alloc` (its header readable before it).
unsafe fn header(p: usize) -> *mut [u64; 3] {
    (p - HEADER) as *mut [u64; 3]
}

/// A block of `size` bytes (a multiple of 64) at a multiple of 64, of the
/// program (`runtime` false) or of the runtime's own; null when the memory
/// is refused.
pub fn alloc(size: usize, runtime: bool) -> *mut u8 {
    // SAFETY: a block of the allocator of Isocron of the size asked, its
    // header and guard written in it
    unsafe {
        let raw = super::heap::alloc(HEADER + size + TAIL, false);
        if raw.is_null() {
            return raw;
        }
        let p = raw as usize + HEADER;
        *header(p) = [LIVE, size as u64, runtime as u64];
        core::ptr::write_bytes((p + size) as *mut u8, GUARD, TAIL);
        if !runtime {
            LIVE_BLOCKS.fetch_add(1, Ordering::Relaxed);
            LIVE_BYTES.fetch_add(size, Ordering::Relaxed);
        }
        p as *mut u8
    }
}

/// The places of a block of `size` bytes read to check a fill: every byte
/// of a small one, one in 4093 and the last of a large one.
fn sampled(size: usize, mut each: impl FnMut(usize) -> bool) -> bool {
    let step = if size <= 64 << 10 { 1 } else { 4093 };
    let mut i = 0;
    while i < size {
        if !each(i) {
            return false;
        }
        i += step;
    }
    size == 0 || each(size - 1)
}

/// Frees the block `p` of `alloc`: checked, poisoned, kept in the
/// quarantine (the oldest blocks leave it, checked, to the allocator).
pub fn free(p: *mut u8) {
    let p = p as usize;
    if p % 64 != 0 {
        fault("a free of an address that is no block");
    }
    // SAFETY: the header of a block of `alloc` (checked first)
    unsafe {
        let h = &mut *header(p);
        match h[0] {
            LIVE => {}
            FREED => fault("a block freed twice"),
            _ => fault("a free of an address that is no block"),
        }
        let size = h[1] as usize;
        if !sampled(TAIL, |i| *((p + size + i) as *const u8) == GUARD) {
            fault("a block written past its end");
        }
        h[0] = FREED;
        if h[2] == 0 {
            LIVE_BLOCKS.fetch_sub(1, Ordering::Relaxed);
            LIVE_BYTES.fetch_sub(size, Ordering::Relaxed);
        }
        core::ptr::write_bytes(p as *mut u8, POISON, size);
        lock();
        let q = &mut *core::ptr::addr_of_mut!(QUEUE);
        while q.len == RING || (q.len > 0 && q.bytes + size > QUARANTINE) {
            let old = q.ring[q.head];
            q.head = (q.head + 1) % RING;
            q.len -= 1;
            q.bytes -= (*header(old))[1] as usize;
            release(old);
        }
        q.ring[(q.head + q.len) % RING] = p;
        q.len += 1;
        q.bytes += size;
        unlock();
    }
}

/// A block leaving the quarantine: its poison checked, then given back.
///
/// # Safety
/// `p` a block of the quarantine.
unsafe fn release(p: usize) {
    // SAFETY: a block freed, its header and bytes still its own
    unsafe {
        let size = (*header(p))[1] as usize;
        if !sampled(size, |i| *((p + i) as *const u8) == POISON) {
            fault("a block written after its free");
        }
        super::heap::free((p - HEADER) as *mut u8);
    }
}

/// The end of `main`: every block of the quarantine checked; the blocks of
/// the program still allocated: their count and bytes (0, 0 none).
pub fn end() -> (usize, usize) {
    lock();
    // SAFETY: the quarantine, held
    unsafe {
        let q = &mut *core::ptr::addr_of_mut!(QUEUE);
        while q.len > 0 {
            let old = q.ring[q.head];
            q.head = (q.head + 1) % RING;
            q.len -= 1;
            q.bytes -= (*header(old))[1] as usize;
            release(old);
        }
    }
    unlock();
    (LIVE_BLOCKS.load(Ordering::Relaxed), LIVE_BYTES.load(Ordering::Relaxed))
}
