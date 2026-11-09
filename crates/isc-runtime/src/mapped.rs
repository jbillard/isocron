//! The standard input of a large regular file, mapped instead of read
//! (D257). Reading it copies every byte out of the page cache into new
//! pages that the kernel clears first: 254 MB in 28.9 ms with 4 threads.
//! Mapped, its pages are the ones of the page cache: 2.4 ms to map them,
//! 4.7 ms to unmap them.
//!
//! The array keeps the bytes the file had when it was read, as with
//! `read`: the file is held by a read lease (`F_SETLEASE`), which the
//! kernel takes only when no one has it open for writing, and which makes
//! a process that opens it for writing, or truncates it, wait until the
//! lease is given back. Before it gives it back, the handler of the signal
//! the kernel sends (`SIGIO`) copies the bytes into memory of the process
//! put at the place of the mapping (`mremap`): the writer then changes the
//! file, not the array. Pages made private by a copy on write would not
//! do: a truncation takes them back too (the kernel unmaps every page past
//! the new end, the private ones included, and reading them is a SIGBUS).
//! The mapping is read-only, so that no write of the program (an array
//! changed in place, D207) is lost while the bytes are copied: a write
//! faults, and the handler of SIGSEGV makes the same copy, then the write
//! is done again. No lease (another owner, a file open for writing, a file
//! system without leases), or a handler of SIGIO or SIGSEGV already there:
//! the file is read.
//!
//! One mapping at a time, the standard input's: it is read to its end once.

// every unsafe operation in a block of its own, each justified (D564)
#![deny(unsafe_op_in_unsafe_fn)]
#![deny(clippy::undocumented_unsafe_blocks)]

use core::ffi::{c_int, c_void};
use core::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};

// the system called directly (D287)
use super::linux::{fcntl, madvise, mmap, mprotect, mremap, munmap, raise, sigaction, SigAction};

const SIGIO: c_int = 29;
const SIGSEGV: c_int = 11;
const SA_RESTART: u64 = 0x1000_0000;
const SA_SIGINFO: u64 = 4;
const SA_ONSTACK: u64 = 0x0800_0000;
const F_GETFL: c_int = 3;
const O_ACCMODE: c_int = 3;
const O_RDONLY: c_int = 0;
const F_SETLEASE: c_int = 1024;
const F_RDLCK: c_int = 0;
const F_UNLCK: c_int = 2;
const PROT_READ: c_int = 0x1;
const PROT_READ_WRITE: c_int = 0x1 | 0x2;
const MAP_PRIVATE: c_int = 0x02;
const MAP_FIXED: c_int = 0x10;
const MAP_ANONYMOUS: c_int = 0x20;
const MADV_DONTNEED: c_int = 4;
const MREMAP_MAYMOVE: c_int = 1;
const MREMAP_FIXED: c_int = 2;
const PAGE: usize = 4096;

/// From this size on, the standard input is mapped (as D221 reads by
/// several threads from 8 MiB on).
pub const MAP_MIN: usize = 8 << 20;

// the state of the mapping
const NONE: u8 = 0;
/// Mapped, the lease held.
const LEASED: u8 = 1;
/// Its bytes being copied into memory of the process.
const COPYING: u8 = 2;
/// Its bytes in memory of the process, writable, the lease given back.
const PRIVATE: u8 = 3;

static STATE: AtomicU8 = AtomicU8::new(NONE);
/// The first byte of the array.
static DATA: AtomicUsize = AtomicUsize::new(0);
/// The bytes of the file mapped (whole pages).
static SPAN: AtomicUsize = AtomicUsize::new(0);
/// The mapping: a page before the array (its header), the file, a page.
static BASE: AtomicUsize = AtomicUsize::new(0);
static TOTAL: AtomicUsize = AtomicUsize::new(0);
/// A signal came since the lease was taken.
static BROKEN: AtomicBool = AtomicBool::new(false);
static HANDLER: AtomicBool = AtomicBool::new(false);
/// The handler of SIGSEGV before ours (put back for a fault elsewhere).
static mut OLD_SEGV: SigAction = SigAction { handler: 0, mask: 0, flags: 0, restorer: 0 };

/// The `len` bytes of the standard input from its offset `cur` (a multiple
/// of a page), mapped, its pages not yet read; None when it cannot be (the
/// caller reads it). The offset of the standard input is not moved.
///
/// # Safety
/// Called by one thread at a time, with no mapping made yet (the standard
/// input is read once, by the thread that reads it).
pub unsafe fn map_stdin(cur: i64, len: usize) -> Option<*mut u8> {
    if len < MAP_MIN || cur < 0 || cur as usize % PAGE != 0 || STATE.load(Ordering::Acquire) != NONE {
        return None;
    }
    // SAFETY: one caller at a time (the contract of this function)
    if fcntl(0, F_GETFL, 0) & O_ACCMODE != O_RDONLY || !unsafe { install_handler() } {
        return None;
    }
    BROKEN.store(false, Ordering::SeqCst);
    if fcntl(0, F_SETLEASE, F_RDLCK) != 0 {
        return None;
    }
    let span = len.next_multiple_of(PAGE);
    let total = PAGE + span + PAGE;
    // SAFETY: a new mapping at a place the kernel chooses: no memory of the
    // program changed
    let base = unsafe { mmap(core::ptr::null_mut(), total, PROT_READ_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0) };
    if base as isize == -1 {
        fcntl(0, F_SETLEASE, F_UNLCK);
        return None;
    }
    // SAFETY: `base` is a mapping of `total` bytes, `PAGE` of them before
    let data = unsafe { (base as *mut u8).add(PAGE) };
    // SAFETY: the file put in place of `span` bytes of that mapping, which
    // no one else knows yet
    if unsafe { mmap(data as *mut c_void, span, PROT_READ, MAP_PRIVATE | MAP_FIXED, 0, cur) } != data as *mut c_void {
        // SAFETY: the mapping made above, given back whole, never read
        unsafe { munmap(base, total) };
        fcntl(0, F_SETLEASE, F_UNLCK);
        return None;
    }
    // the header of the allocator before the array: a class no block has,
    // so that `isc_free` gives the array back here (`owns`)
    // SAFETY: the 64 bytes before `data`, in the writable page before it,
    // aligned for a `u32`
    unsafe { (data.sub(64) as *mut u32).write(u32::MAX) };
    DATA.store(data as usize, Ordering::Relaxed);
    SPAN.store(span, Ordering::Relaxed);
    BASE.store(base as usize, Ordering::Relaxed);
    TOTAL.store(total, Ordering::Relaxed);
    // memory of the program, as if the allocator had given it
    crate::heap::held_outside(span, true);
    STATE.store(LEASED, Ordering::SeqCst);
    // a writer came before the mapping was known to the handler
    if BROKEN.load(Ordering::SeqCst) {
        // SAFETY: the mapping published above, in the state LEASED
        unsafe { give_back() };
    }
    Some(data)
}

/// `madvise` on `len` bytes from `p` (`p` on a page): 0 when done.
///
/// # Safety
/// `p` and `len` a part of the mapped array, whose bytes `advice` keeps
/// (`MADV_POPULATE_READ`: they are read in).
pub unsafe fn advise(p: *mut u8, len: usize, advice: c_int) -> c_int {
    // SAFETY: the contract of this function
    unsafe { madvise(p as *mut c_void, len, advice) }
}

/// The pages of `span` bytes from `p` (a part of the mapping) given back.
///
/// # Safety
/// `p` and `span` a part of the mapped array that no one reads again (its
/// pages of the file read again from the page cache if they are).
pub unsafe fn give_pages(p: *mut u8, span: usize) {
    // SAFETY: the contract of this function
    unsafe { madvise(p as *mut c_void, span, MADV_DONTNEED) };
}

/// Some array is mapped.
pub fn mapped() -> bool {
    STATE.load(Ordering::Acquire) != NONE
}

/// `p` is the mapped array.
#[inline]
pub fn owns(p: *const u8) -> bool {
    STATE.load(Ordering::Acquire) != NONE && p as usize == DATA.load(Ordering::Relaxed)
}

/// The mapped array freed (or the program at its end, D258): its pages
/// given back by `zap` (by every thread, where `munmap` alone gives the
/// pages of a file back one by one on one thread: 254 MB in 3.58 ms, 1.11
/// ms by 4 threads), unmapped, the lease given back.
///
/// # Safety
/// The array is no longer read (its owner frees it, or the program ends);
/// `zap` may be given the whole array.
pub unsafe fn free(zap: unsafe fn(*mut u8, usize)) {
    loop {
        let s = STATE.load(Ordering::Acquire);
        match s {
            LEASED | PRIVATE => {
                if STATE.compare_exchange(s, NONE, Ordering::AcqRel, Ordering::Acquire).is_ok() {
                    // SAFETY: the array, no longer read (the contract of
                    // this function); this thread alone took it out of its
                    // state
                    unsafe { zap(DATA.load(Ordering::Relaxed) as *mut u8, SPAN.load(Ordering::Relaxed)) };
                    // SAFETY: the mapping of `map_stdin`, no longer read
                    unsafe { munmap(BASE.load(Ordering::Relaxed) as *mut c_void, TOTAL.load(Ordering::Relaxed)) };
                    crate::heap::held_outside(SPAN.load(Ordering::Relaxed), false);
                    if s == LEASED {
                        fcntl(0, F_SETLEASE, F_UNLCK);
                    }
                    return;
                }
            }
            // the handler, on another thread, makes the pages private
            COPYING => core::hint::spin_loop(),
            _ => return,
        }
    }
}

/// The handlers of SIGIO and SIGSEGV, installed once; false when the
/// program has its own.
///
/// # Safety
/// One caller at a time (it writes `OLD_SEGV`, which the handler of
/// SIGSEGV reads once installed).
unsafe fn install_handler() -> bool {
    if HANDLER.load(Ordering::Acquire) {
        return true;
    }
    let mut io = SigAction { handler: 0, mask: 0, flags: 0, restorer: 0 };
    let mut segv = SigAction { handler: 0, mask: 0, flags: 0, restorer: 0 };
    // SAFETY: the actions read into places of this frame; none changed
    if unsafe { sigaction(SIGIO, core::ptr::null(), &mut io) } != 0 || io.handler != 0 {
        return false;
    }
    // SAFETY: as above
    if unsafe { sigaction(SIGSEGV, core::ptr::null(), &mut segv) } != 0 || segv.handler != 0 {
        return false;
    }
    // SAFETY: one caller at a time (the contract of this function), before
    // the handler that reads it is installed
    unsafe { OLD_SEGV = segv };
    let act = SigAction { handler: on_sigio as *const () as usize, mask: 0, flags: SA_RESTART, restorer: 0 };
    // SAFETY: a handler of this module, sound in a handler (atomics and the
    // system called directly, D287)
    if unsafe { sigaction(SIGIO, &act, core::ptr::null_mut()) } != 0 {
        return false;
    }
    let act = SigAction { handler: on_sigsegv as *const () as usize, mask: 0, flags: SA_SIGINFO | SA_RESTART | SA_ONSTACK, restorer: 0 };
    // SAFETY: as above
    if unsafe { sigaction(SIGSEGV, &act, core::ptr::null_mut()) } != 0 {
        return false;
    }
    HANDLER.store(true, Ordering::Release);
    true
}

/// A fault: a write into the mapping (read-only) has its bytes copied, then
/// is done again; any other fault gets the handler from before (the
/// default one: the program stops as it would have), and faults again.
extern "C" fn on_sigsegv(_sig: c_int, info: *const SigInfo, _ctx: *mut c_void) {
    // SAFETY: the kernel gives a handler of `SA_SIGINFO` its `siginfo_t`,
    // whose start is `SigInfo`
    let (addr, code) = unsafe { ((*info).addr, (*info).code) };
    let data = DATA.load(Ordering::Relaxed);
    let inside = addr >= data && addr < data + SPAN.load(Ordering::Relaxed);
    if code > 0 && inside && STATE.load(Ordering::Acquire) != NONE {
        // SAFETY: a fault in the mapping, which is published: copied out
        unsafe { give_back() };
    } else {
        // SAFETY: the action saved before ours was installed, read only
        // here and never written again
        unsafe { sigaction(SIGSEGV, core::ptr::addr_of!(OLD_SEGV), core::ptr::null_mut()) };
        // sent by a process (not a fault, which happens again)
        if code <= 0 {
            raise(SIGSEGV);
        }
    }
}

/// The start of `siginfo_t` for SIGSEGV: the address of the fault.
#[repr(C)]
struct SigInfo {
    signo: c_int,
    errno: c_int,
    code: c_int,
    _pad: c_int,
    addr: usize,
}

/// A writer waits for the lease: the pages made private, then the lease
/// given back (no `errno` to keep: the system is called directly, D287).
extern "C" fn on_sigio(_sig: c_int) {
    BROKEN.store(true, Ordering::SeqCst);
    // SAFETY: the mapping, if any is published, is copied out by one thread
    // (its state); with none, nothing is done but the lease given back
    unsafe { give_back() };
}

/// The bytes of the mapping copied into memory of the process, put at its
/// place, writable (by the first caller; another one waits for it), then
/// the lease given back.
///
/// # Safety
/// The mapping, when its state is LEASED, is the one `map_stdin` published.
unsafe fn give_back() {
    loop {
        match STATE.load(Ordering::Acquire) {
            LEASED => {
                if STATE.compare_exchange(LEASED, COPYING, Ordering::AcqRel, Ordering::Acquire).is_ok() {
                    // SAFETY: the mapping published (the contract of this
                    // function), taken by this thread alone (its state)
                    unsafe { copy_out(DATA.load(Ordering::Relaxed) as *mut u8, SPAN.load(Ordering::Relaxed)) };
                    STATE.store(PRIVATE, Ordering::Release);
                    break;
                }
            }
            COPYING => core::hint::spin_loop(),
            _ => break,
        }
    }
    fcntl(0, F_SETLEASE, F_UNLCK);
}

/// The `span` bytes from `p` (the read-only mapping: no one writes them
/// meanwhile) copied into new memory, moved to `p` in place of the mapping
/// (one call: the bytes are always there for the other threads). Without
/// memory for the copy, the mapping made writable, its pages copied on
/// their first write (a truncation of the file could then take them back).
///
/// # Safety
/// `p` and `span` the read-only mapping of the array, copied by this thread
/// alone.
unsafe fn copy_out(p: *mut u8, span: usize) {
    // SAFETY: a new mapping at a place the kernel chooses
    let q = unsafe { mmap(core::ptr::null_mut(), span, PROT_READ_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0) };
    if q as isize != -1 {
        // SAFETY: `span` bytes readable at `p` (the contract), writable at
        // `q` (just made), apart
        unsafe { core::ptr::copy_nonoverlapping(p, q as *mut u8, span) };
        // SAFETY: the copy put in place of the mapping at `p`, in one call
        // (the same bytes at every moment for the other threads)
        if unsafe { mremap(q, span, span, MREMAP_MAYMOVE | MREMAP_FIXED, p as *mut c_void) } as isize != -1 {
            return;
        }
        // SAFETY: the copy, never published, given back
        unsafe { munmap(q, span) };
    }
    // SAFETY: the mapping at `p` made writable (its pages private on their
    // first write)
    unsafe { mprotect(p as *mut c_void, span, PROT_READ_WRITE) };
}
