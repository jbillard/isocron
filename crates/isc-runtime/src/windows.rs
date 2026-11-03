//! The system under Windows (target `x86_64-pc-windows-gnu`, D246): the
//! functions of Linux that the runtime calls, with the same names and
//! meanings, over the Win32 API (`kernel32`, `WaitOnAddress` of the
//! synchronization API set) and the few functions of the C library of
//! MinGW (`msvcrt`: `strlen`, `getenv`, `exit`, `abort`, `_aligned_malloc`).
//! A file descriptor is a handle (handles of kernel objects fit in 32 bits);
//! 0, 1 and 2 are the standard handles. The bytes are written and read as
//! they are (no text mode: no `\r\n`). Errors: `errno()` is the last Win32
//! error of the thread mapped to the POSIX code, and `strerror` gives the
//! text of glibc for it, so that the messages are those of Linux.

#![allow(non_snake_case, clippy::upper_case_acronyms)]

use core::ffi::{c_char, c_int, c_uint, c_void};

type HANDLE = *mut c_void;
type BOOL = i32;

const INVALID_HANDLE_VALUE: HANDLE = -1isize as HANDLE;
const STD_INPUT_HANDLE: u32 = -10i32 as u32;
const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;
const STD_ERROR_HANDLE: u32 = -12i32 as u32;
const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const FILE_SHARE_READ: u32 = 1;
const FILE_SHARE_WRITE: u32 = 2;
const FILE_SHARE_DELETE: u32 = 4;
const CREATE_ALWAYS: u32 = 2;
const OPEN_EXISTING: u32 = 3;
const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;
const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
const FILE_TYPE_DISK: u32 = 1;
const ERROR_HANDLE_EOF: u32 = 38;
const ERROR_BROKEN_PIPE: u32 = 109;
const CP_UTF8: u32 = 65001;
const MEM_COMMIT: u32 = 0x1000;
const MEM_RESERVE: u32 = 0x2000;
const MEM_DECOMMIT: u32 = 0x4000;
const MEM_RELEASE: u32 = 0x8000;
const PAGE_READWRITE: u32 = 4;
const STACK_SIZE_PARAM_IS_A_RESERVATION: u32 = 0x0001_0000;
const INFINITE: u32 = u32::MAX;

#[repr(C)]
struct Overlapped {
    internal: usize,
    internal_high: usize,
    offset: u32,
    offset_high: u32,
    event: HANDLE,
}

#[repr(C)]
struct MemoryBasicInformation {
    base: *mut c_void,
    allocation_base: *mut c_void,
    allocation_protect: u32,
    partition_id: u16,
    region_size: usize,
    state: u32,
    protect: u32,
    kind: u32,
}

#[link(name = "kernel32")]
extern "system" {
    fn GetStdHandle(which: u32) -> HANDLE;
    fn CreateFileW(name: *const u16, access: u32, share: u32, sa: *mut c_void, disposition: u32, flags: u32, template: HANDLE) -> HANDLE;
    fn ReadFile(h: HANDLE, buf: *mut c_void, n: u32, done: *mut u32, ov: *mut Overlapped) -> BOOL;
    fn WriteFile(h: HANDLE, buf: *const c_void, n: u32, done: *mut u32, ov: *mut Overlapped) -> BOOL;
    fn CloseHandle(h: HANDLE) -> BOOL;
    fn SetFilePointerEx(h: HANDLE, distance: i64, new: *mut i64, method: u32) -> BOOL;
    fn GetFileType(h: HANDLE) -> u32;
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn MultiByteToWideChar(cp: u32, flags: u32, s: *const u8, n: i32, w: *mut u16, wn: i32) -> i32;
    fn VirtualAlloc(addr: *mut c_void, size: usize, kind: u32, protect: u32) -> *mut c_void;
    fn VirtualFree(addr: *mut c_void, size: usize, kind: u32) -> BOOL;
    fn VirtualQuery(addr: *const c_void, info: *mut MemoryBasicInformation, len: usize) -> usize;
    fn CreateThread(sa: *mut c_void, stack: usize, start: extern "system" fn(*mut c_void) -> u32, arg: *mut c_void, flags: u32, id: *mut u32) -> HANDLE;
    fn WaitForSingleObject(h: HANDLE, ms: u32) -> u32;
    fn GetCurrentThreadStackLimits(low: *mut usize, high: *mut usize);
    fn GetCurrentProcess() -> HANDLE;
    fn GetProcessAffinityMask(p: HANDLE, process: *mut usize, system: *mut usize) -> BOOL;
    fn QueryPerformanceCounter(t: *mut i64) -> BOOL;
    fn QueryPerformanceFrequency(f: *mut i64) -> BOOL;
    fn SwitchToThread() -> BOOL;
    fn FlsAlloc(callback: Option<unsafe extern "system" fn(*mut c_void)>) -> u32;
    fn FlsSetValue(index: u32, value: *const c_void) -> BOOL;
    fn TerminateProcess(p: HANDLE, code: u32) -> BOOL;
}

#[link(name = "synchronization")]
extern "system" {
    fn WaitOnAddress(addr: *const c_void, compare: *const c_void, size: usize, ms: u32) -> BOOL;
    fn WakeByAddressAll(addr: *const c_void);
}

extern "C" {
    fn _aligned_malloc(size: usize, align: usize) -> *mut c_void;
    fn _aligned_free(p: *mut c_void);
}

// ---------------------------------------------------------------------
// errors

/// The POSIX code of the last Win32 error of the thread (0 if none).
pub fn errno() -> c_int {
    isc_runtime_core::oserr::from_win32(unsafe { GetLastError() })
}

/// The text of glibc for a POSIX code.
pub unsafe fn strerror(code: c_int) -> *const c_char {
    isc_runtime_core::oserr::text_c(code).as_ptr() as *const c_char
}

// ---------------------------------------------------------------------
// files

pub const O_RDONLY: c_int = 0;
pub const O_WRONLY: c_int = 1;
pub const O_CREAT: c_int = 0o100;
pub const O_TRUNC: c_int = 0o1000;
pub const O_CLOEXEC: c_int = 0;

fn handle(fd: c_int) -> HANDLE {
    unsafe {
        match fd {
            0 => GetStdHandle(STD_INPUT_HANDLE),
            1 => GetStdHandle(STD_OUTPUT_HANDLE),
            2 => GetStdHandle(STD_ERROR_HANDLE),
            _ => fd as u32 as usize as HANDLE,
        }
    }
}

/// Opens the file named by the UTF-8 bytes `path` (NUL-terminated): to
/// read, or to write (created or truncated); its handle, or -1.
pub unsafe fn open(path: *const c_char, flags: c_int, _mode: c_int) -> c_int {
    let n = super::strlen(path);
    let wn = MultiByteToWideChar(CP_UTF8, 0, path as *const u8, n as i32, core::ptr::null_mut(), 0);
    if n > 0 && wn <= 0 {
        return -1;
    }
    let w = _aligned_malloc((wn as usize + 1) * 2, 16) as *mut u16;
    if w.is_null() {
        return -1;
    }
    if n > 0 {
        MultiByteToWideChar(CP_UTF8, 0, path as *const u8, n as i32, w, wn);
    }
    *w.add(wn.max(0) as usize) = 0;
    let write = flags & O_WRONLY != 0;
    let (access, disposition) = if write { (GENERIC_WRITE, CREATE_ALWAYS) } else { (GENERIC_READ, OPEN_EXISTING) };
    // a directory opens too (as on Linux), its read then fails
    let h = CreateFileW(w, access, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, core::ptr::null_mut(), disposition, FILE_ATTRIBUTE_NORMAL | FILE_FLAG_BACKUP_SEMANTICS, core::ptr::null_mut());
    let e = GetLastError();
    _aligned_free(w as *mut c_void);
    if h == INVALID_HANDLE_VALUE {
        SetLastError(e);
        return -1;
    }
    h as usize as c_int
}

/// At most `n` bytes (`u32` at a time); 0 at the end, -1 on an error.
pub unsafe fn read(fd: c_int, buf: *mut c_void, n: usize) -> isize {
    let mut done = 0u32;
    let m = n.min(1 << 30) as u32;
    if ReadFile(handle(fd), buf, m, &mut done, core::ptr::null_mut()) == 0 {
        // the end of a pipe whose writer is gone
        return if GetLastError() == ERROR_BROKEN_PIPE { 0 } else { -1 };
    }
    done as isize
}

/// At most `n` bytes from the offset `off`, the position of the file
/// unchanged for the other readers by offset.
pub unsafe fn pread(fd: c_int, buf: *mut c_void, n: usize, off: i64) -> isize {
    let mut done = 0u32;
    let m = n.min(1 << 30) as u32;
    let mut ov = Overlapped { internal: 0, internal_high: 0, offset: off as u32, offset_high: (off >> 32) as u32, event: core::ptr::null_mut() };
    if ReadFile(handle(fd), buf, m, &mut done, &mut ov) == 0 {
        return if GetLastError() == ERROR_HANDLE_EOF { 0 } else { -1 };
    }
    done as isize
}

pub unsafe fn write(fd: c_int, buf: *const c_void, n: usize) -> isize {
    let mut done = 0u32;
    let m = n.min(1 << 30) as u32;
    if WriteFile(handle(fd), buf, m, &mut done, core::ptr::null_mut()) == 0 {
        return -1;
    }
    done as isize
}

pub unsafe fn close(fd: c_int) -> c_int {
    if fd <= 2 {
        return 0;
    }
    if CloseHandle(handle(fd)) == 0 {
        -1
    } else {
        0
    }
}

/// The new position, or -1 (ESPIPE on a pipe or a console).
pub unsafe fn lseek(fd: c_int, off: i64, whence: c_int) -> i64 {
    let h = handle(fd);
    if GetFileType(h) != FILE_TYPE_DISK {
        SetLastError(87);
        return -1;
    }
    let mut new = 0i64;
    if SetFilePointerEx(h, off, &mut new, whence as u32) == 0 {
        return -1;
    }
    new
}

// ---------------------------------------------------------------------
// memory

pub unsafe fn aligned_alloc(align: usize, size: usize) -> *mut c_void {
    _aligned_malloc(size.max(1), align)
}

pub unsafe fn free(p: *mut c_void) {
    if !p.is_null() {
        _aligned_free(p)
    }
}

/// `mmap` of anonymous memory: reserved and committed (Windows has no
/// commit on first touch; the pages are still made only when touched).
/// MAP_FAILED (-1) when refused.
pub unsafe fn mmap(_addr: *mut c_void, len: usize, _prot: c_int, _flags: c_int, _fd: c_int, _off: i64) -> *mut c_void {
    let p = VirtualAlloc(core::ptr::null_mut(), len, MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE);
    if p.is_null() {
        -1isize as *mut c_void
    } else {
        p
    }
}

/// `munmap` of any part of a mapping: its pages decommitted; the address
/// space of the mapping released once none of its pages is committed.
pub unsafe fn munmap(addr: *mut c_void, len: usize) -> c_int {
    if len == 0 {
        return 0;
    }
    if VirtualFree(addr, len, MEM_DECOMMIT) == 0 {
        return -1;
    }
    let mut info = core::mem::zeroed::<MemoryBasicInformation>();
    if VirtualQuery(addr, &mut info, core::mem::size_of::<MemoryBasicInformation>()) == 0 {
        return 0;
    }
    let base = info.allocation_base;
    let mut p = base as usize;
    loop {
        if VirtualQuery(p as *const c_void, &mut info, core::mem::size_of::<MemoryBasicInformation>()) == 0 || info.allocation_base != base {
            break;
        }
        if info.state == MEM_COMMIT {
            return 0;
        }
        p += info.region_size;
    }
    VirtualFree(base, 0, MEM_RELEASE);
    0
}

pub const MADV_DONTNEED: c_int = 4;

/// MADV_DONTNEED: the pages decommitted and committed again, zeros when
/// touched next (as on Linux); the advice on huge pages ignored.
pub unsafe fn madvise(addr: *mut c_void, len: usize, advice: c_int) -> c_int {
    if advice == MADV_DONTNEED && len > 0 {
        VirtualFree(addr, len, MEM_DECOMMIT);
        VirtualAlloc(addr, len, MEM_COMMIT, PAGE_READWRITE);
    }
    0
}

/// `mremap(old, old_len, new_len, MREMAP_MAYMOVE | MREMAP_FIXED, dst)`, `dst`
/// inside a mapping of at least `new_len` bytes: the bytes copied there and
/// the old mapping given back (Windows cannot move pages).
pub unsafe fn mremap(old: *mut c_void, old_len: usize, _new_len: usize, _flags: c_int, dst: *mut c_void) -> *mut c_void {
    core::ptr::copy_nonoverlapping(old as *const u8, dst as *mut u8, old_len);
    munmap(old, old_len);
    dst
}

/// `pthread_key_create` with a destructor run at the end of each thread
/// that set a value: a fiber local slot (`FlsAlloc`).
pub unsafe fn pthread_key_create(key: *mut c_uint, destructor: Option<unsafe extern "C" fn(*mut c_void)>) -> c_int {
    static mut DESTRUCTOR: Option<unsafe extern "C" fn(*mut c_void)> = None;
    unsafe extern "system" fn run(p: *mut c_void) {
        if !p.is_null() {
            if let Some(d) = *core::ptr::addr_of!(DESTRUCTOR) {
                d(p)
            }
        }
    }
    *core::ptr::addr_of_mut!(DESTRUCTOR) = destructor;
    let i = FlsAlloc(Some(run));
    if i == u32::MAX {
        return -1;
    }
    *key = i;
    0
}

pub unsafe fn pthread_setspecific(key: c_uint, value: *const c_void) -> c_int {
    if FlsSetValue(key, value) == 0 {
        -1
    } else {
        0
    }
}

// ---------------------------------------------------------------------
// time and threads

/// The monotonic clock, in seconds and nanoseconds.
pub unsafe fn monotonic(ts: *mut super::Timespec) {
    let (mut t, mut f) = (0i64, 1i64);
    QueryPerformanceCounter(&mut t);
    QueryPerformanceFrequency(&mut f);
    let f = f.max(1);
    (*ts).tv_sec = t / f;
    (*ts).tv_nsec = ((t % f) as i128 * 1_000_000_000 / f as i128) as core::ffi::c_long;
}

struct Start {
    f: extern "C" fn(*mut c_void) -> *mut c_void,
    arg: *mut c_void,
}

extern "system" fn trampoline(p: *mut c_void) -> u32 {
    let s = unsafe { core::ptr::read(p as *mut Start) };
    unsafe { free(p) };
    (s.f)(s.arg);
    0
}

/// A thread running `f(arg)` on a stack of `stack` bytes reserved (0: the
/// default); its handle, or None.
pub fn spawn(stack: usize, f: extern "C" fn(*mut c_void) -> *mut c_void, arg: *mut c_void) -> Option<HANDLE> {
    unsafe {
        let p = aligned_alloc(16, core::mem::size_of::<Start>()) as *mut Start;
        if p.is_null() {
            return None;
        }
        p.write(Start { f, arg });
        let flags = if stack > 0 { STACK_SIZE_PARAM_IS_A_RESERVATION } else { 0 };
        let h = CreateThread(core::ptr::null_mut(), stack, trampoline, p as *mut c_void, flags, core::ptr::null_mut());
        if h.is_null() {
            free(p as *mut c_void);
            return None;
        }
        Some(h)
    }
}

/// Waits for the end of the thread `h`.
pub fn join(h: HANDLE) {
    unsafe {
        WaitForSingleObject(h, INFINITE);
        CloseHandle(h);
    }
}

/// Lets the thread `h` run on its own (its handle closed).
pub fn detach(h: HANDLE) {
    unsafe { CloseHandle(h) };
}

/// Sleeps while the word at `p` is `seen` (spurious wake-ups allowed).
pub fn wait_word(p: *const u32, seen: u32) {
    unsafe { WaitOnAddress(p as *const c_void, &seen as *const u32 as *const c_void, 4, INFINITE) };
}

/// Wakes every thread sleeping on the word at `p`.
pub fn wake_word(p: *const u32) {
    unsafe { WakeByAddressAll(p as *const c_void) };
}

/// The CPUs the process may run on (at most 64: one processor group).
pub fn cpus() -> usize {
    let (mut process, mut system) = (0usize, 0usize);
    if unsafe { GetProcessAffinityMask(GetCurrentProcess(), &mut process, &mut system) } != 0 && process != 0 {
        process.count_ones() as usize
    } else {
        1
    }
}

/// A short pause of a thread that has nothing to do (the time slice given
/// to another thread: Windows sleeps by milliseconds).
pub fn pause_briefly() {
    unsafe { SwitchToThread() };
}

/// The lowest and highest addresses of the stack of the calling thread.
pub fn stack_limits() -> (usize, usize) {
    let (mut low, mut high) = (0usize, 0usize);
    unsafe { GetCurrentThreadStackLimits(&mut low, &mut high) };
    (low, high)
}

/// Ends the process at once with `code` (no end of thread run: see `die`).
pub fn terminate(code: c_int) -> ! {
    unsafe {
        TerminateProcess(GetCurrentProcess(), code as u32);
    }
    loop {
        core::hint::spin_loop();
    }
}
