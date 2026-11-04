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

use core::ffi::{c_int, c_void};
use core::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};

extern "C" {
    fn mmap(addr: *mut c_void, len: usize, prot: c_int, flags: c_int, fd: c_int, off: i64) -> *mut c_void;
    fn munmap(addr: *mut c_void, len: usize) -> c_int;
    fn mremap(old: *mut c_void, old_len: usize, new_len: usize, flags: c_int, new: *mut c_void) -> *mut c_void;
    fn madvise(addr: *mut c_void, len: usize, advice: c_int) -> c_int;
    fn mprotect(addr: *mut c_void, len: usize, prot: c_int) -> c_int;
    fn raise(sig: c_int) -> c_int;
    fn fcntl(fd: c_int, cmd: c_int, ...) -> c_int;
    fn sigaction(sig: c_int, act: *const SigAction, old: *mut SigAction) -> c_int;
    fn __errno_location() -> *mut c_int;
}

/// `struct sigaction` of Linux on x86-64 (glibc).
#[repr(C)]
struct SigAction {
    handler: usize,
    mask: [u64; 16],
    flags: c_int,
    restorer: usize,
}

const SIGIO: c_int = 29;
const SIGSEGV: c_int = 11;
const SA_RESTART: c_int = 0x1000_0000;
const SA_SIGINFO: c_int = 4;
const SA_ONSTACK: c_int = 0x0800_0000;
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
static mut OLD_SEGV: SigAction = SigAction { handler: 0, mask: [0; 16], flags: 0, restorer: 0 };

/// The `len` bytes of the standard input from its offset `cur` (a multiple
/// of a page), mapped, its pages not yet read; None when it cannot be (the
/// caller reads it). The offset of the standard input is not moved.
pub unsafe fn map_stdin(cur: i64, len: usize) -> Option<*mut u8> {
    if len < MAP_MIN || cur < 0 || cur as usize % PAGE != 0 || STATE.load(Ordering::Acquire) != NONE {
        return None;
    }
    if fcntl(0, F_GETFL) & O_ACCMODE != O_RDONLY || !install_handler() {
        return None;
    }
    BROKEN.store(false, Ordering::SeqCst);
    if fcntl(0, F_SETLEASE, F_RDLCK) != 0 {
        return None;
    }
    let span = len.next_multiple_of(PAGE);
    let total = PAGE + span + PAGE;
    let base = mmap(core::ptr::null_mut(), total, PROT_READ_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if base as isize == -1 {
        fcntl(0, F_SETLEASE, F_UNLCK);
        return None;
    }
    let data = (base as *mut u8).add(PAGE);
    if mmap(data as *mut c_void, span, PROT_READ, MAP_PRIVATE | MAP_FIXED, 0, cur) != data as *mut c_void {
        munmap(base, total);
        fcntl(0, F_SETLEASE, F_UNLCK);
        return None;
    }
    // the header of the allocator before the array: a class no block has,
    // so that `isc_free` gives the array back here (`owns`)
    (data.sub(64) as *mut u32).write(u32::MAX);
    DATA.store(data as usize, Ordering::Relaxed);
    SPAN.store(span, Ordering::Relaxed);
    BASE.store(base as usize, Ordering::Relaxed);
    TOTAL.store(total, Ordering::Relaxed);
    // memory of the program, as if the allocator had given it
    crate::heap::held_outside(span, true);
    STATE.store(LEASED, Ordering::SeqCst);
    // a writer came before the mapping was known to the handler
    if BROKEN.load(Ordering::SeqCst) {
        give_back();
    }
    Some(data)
}

/// The pages of `span` bytes from `p` (a part of the mapping) given back.
/// `madvise` on `len` bytes from `p` (`p` on a page): 0 when done.
pub unsafe fn advise(p: *mut u8, len: usize, advice: c_int) -> c_int {
    madvise(p as *mut c_void, len, advice)
}

pub unsafe fn give_pages(p: *mut u8, span: usize) {
    madvise(p as *mut c_void, span, MADV_DONTNEED);
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
pub unsafe fn free(zap: unsafe fn(*mut u8, usize)) {
    loop {
        let s = STATE.load(Ordering::Acquire);
        match s {
            LEASED | PRIVATE => {
                if STATE.compare_exchange(s, NONE, Ordering::AcqRel, Ordering::Acquire).is_ok() {
                    zap(DATA.load(Ordering::Relaxed) as *mut u8, SPAN.load(Ordering::Relaxed));
                    munmap(BASE.load(Ordering::Relaxed) as *mut c_void, TOTAL.load(Ordering::Relaxed));
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
unsafe fn install_handler() -> bool {
    if HANDLER.load(Ordering::Acquire) {
        return true;
    }
    let mut io: SigAction = core::mem::zeroed();
    let mut segv: SigAction = core::mem::zeroed();
    if sigaction(SIGIO, core::ptr::null(), &mut io) != 0 || io.handler != 0 {
        return false;
    }
    if sigaction(SIGSEGV, core::ptr::null(), &mut segv) != 0 || segv.handler != 0 {
        return false;
    }
    OLD_SEGV = segv;
    let act = SigAction { handler: on_sigio as usize, mask: [0; 16], flags: SA_RESTART, restorer: 0 };
    if sigaction(SIGIO, &act, core::ptr::null_mut()) != 0 {
        return false;
    }
    let act = SigAction { handler: on_sigsegv as usize, mask: [0; 16], flags: SA_SIGINFO | SA_RESTART | SA_ONSTACK, restorer: 0 };
    if sigaction(SIGSEGV, &act, core::ptr::null_mut()) != 0 {
        return false;
    }
    HANDLER.store(true, Ordering::Release);
    true
}

/// A fault: a write into the mapping (read-only) has its bytes copied, then
/// is done again; any other fault gets the handler from before (the
/// default one: the program stops as it would have), and faults again.
extern "C" fn on_sigsegv(_sig: c_int, info: *const SigInfo, _ctx: *mut c_void) {
    unsafe {
        let e = *__errno_location();
        let addr = (*info).addr;
        let data = DATA.load(Ordering::Relaxed);
        let inside = addr >= data && addr < data + SPAN.load(Ordering::Relaxed);
        if (*info).code > 0 && inside && STATE.load(Ordering::Acquire) != NONE {
            give_back();
        } else {
            sigaction(SIGSEGV, core::ptr::addr_of!(OLD_SEGV), core::ptr::null_mut());
            // sent by a process (not a fault, which happens again)
            if (*info).code <= 0 {
                raise(SIGSEGV);
            }
        }
        *__errno_location() = e;
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
/// given back. The errno of the interrupted code is kept.
extern "C" fn on_sigio(_sig: c_int) {
    unsafe {
        let e = *__errno_location();
        BROKEN.store(true, Ordering::SeqCst);
        give_back();
        *__errno_location() = e;
    }
}

/// The bytes of the mapping copied into memory of the process, put at its
/// place, writable (by the first caller; another one waits for it), then
/// the lease given back.
unsafe fn give_back() {
    loop {
        match STATE.load(Ordering::Acquire) {
            LEASED => {
                if STATE.compare_exchange(LEASED, COPYING, Ordering::AcqRel, Ordering::Acquire).is_ok() {
                    copy_out(DATA.load(Ordering::Relaxed) as *mut u8, SPAN.load(Ordering::Relaxed));
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
unsafe fn copy_out(p: *mut u8, span: usize) {
    let q = mmap(core::ptr::null_mut(), span, PROT_READ_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if q as isize != -1 {
        core::ptr::copy_nonoverlapping(p, q as *mut u8, span);
        if mremap(q, span, span, MREMAP_MAYMOVE | MREMAP_FIXED, p as *mut c_void) as isize != -1 {
            return;
        }
        munmap(q, span);
    }
    mprotect(p as *mut c_void, span, PROT_READ_WRITE);
}
