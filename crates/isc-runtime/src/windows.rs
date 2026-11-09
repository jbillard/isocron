//! The system under Windows (target `x86_64-pc-windows-gnu`, D246): the
//! functions of Linux that the runtime calls, with the same names and
//! meanings, over the Win32 API (`kernel32`, `WaitOnAddress` of the
//! synchronization API set), without a C library (D541): the program
//! starts at `isc_win_start`, its arguments and its environment are read in
//! UTF-16 from the system, its blocks of the system's allocator come from
//! the heap of the process; MinGW's C runtime only when the program links
//! C libraries, and then its `main` is called by it.
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
    fn GetConsoleMode(h: HANDLE, mode: *mut u32) -> BOOL;
    fn WriteConsoleW(h: HANDLE, buf: *const u16, n: u32, done: *mut u32, reserved: *mut c_void) -> BOOL;
    fn ReadConsoleW(h: HANDLE, buf: *mut u16, n: u32, done: *mut u32, control: *mut c_void) -> BOOL;
    fn WideCharToMultiByte(cp: u32, flags: u32, w: *const u16, wn: i32, s: *mut u8, n: i32, default: *const u8, used: *mut i32) -> i32;
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
    fn GetCommandLineW() -> *const u16;
    fn GetEnvironmentStringsW() -> *mut u16;
    fn FreeEnvironmentStringsW(p: *mut u16) -> BOOL;
    fn GetProcessHeap() -> HANDLE;
    fn HeapAlloc(heap: HANDLE, flags: u32, n: usize) -> *mut c_void;
    fn HeapFree(heap: HANDLE, flags: u32, p: *mut c_void) -> BOOL;
    fn ExitProcess(code: u32) -> !;
}

#[link(name = "synchronization")]
extern "system" {
    fn WaitOnAddress(addr: *const c_void, compare: *const c_void, size: usize, ms: u32) -> BOOL;
    fn WakeByAddressAll(addr: *const c_void);
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

/// The standard output is read as it comes (D299): a console or a pipe,
/// not a file on a disk nor a character device that is not a console (NUL).
pub fn stdout_read_as_it_comes() -> bool {
    const FILE_TYPE_DISK: u32 = 1;
    const FILE_TYPE_CHAR: u32 = 2;
    let h = handle(1);
    let mut mode = 0u32;
    // SAFETY: a handle of the process and a place for its mode
    match unsafe { GetFileType(h) } {
        FILE_TYPE_DISK => false,
        FILE_TYPE_CHAR => unsafe { GetConsoleMode(h, &mut mode) != 0 },
        _ => true,
    }
}

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
    let w = aligned_alloc(16, (wn as usize + 1) * 2) as *mut u16;
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
    free(w as *mut c_void);
    if h == INVALID_HANDLE_VALUE {
        SetLastError(e);
        return -1;
    }
    h as usize as c_int
}

/// The bytes of UTF-8 read from a console that did not fit in the place of
/// the caller, for the next read: up to 3, their count in the top byte
/// (D409). The reads of the standard input come one at a time.
static IN_HELD: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
/// The first half of a surrogate pair read last from a console, 0 none
/// (D409).
static IN_HIGH: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

/// At most `n` bytes of the console of the standard input, in UTF-8 (D409),
/// by `ReadConsoleW`: read by `ReadFile`, a console gives its characters in
/// its code page (`é` as the byte 0x82 under the code page 437). Ctrl+Z
/// first: the end, as `ReadFile` gives it; the first half of a surrogate
/// pair at the end of a read held for the next; bytes beyond `n` held.
unsafe fn read_console(p: *mut u8, n: usize) -> isize {
    use core::sync::atomic::Ordering::Relaxed;
    let held = IN_HELD.load(Relaxed);
    let hn = (held >> 24) as usize;
    if hn > 0 {
        let k = hn.min(n);
        for i in 0..k {
            *p.add(i) = (held >> (8 * i)) as u8;
        }
        let mut rest = ((hn - k) as u32) << 24;
        for i in k..hn {
            rest |= ((held >> (8 * i)) & 0xff) << (8 * (i - k));
        }
        IN_HELD.store(rest, Relaxed);
        return k as isize;
    }
    // at most 3 bytes a unit (4 a pair of 2): what is read fits in `n`
    let units = (n / 3).clamp(1, 4096);
    let mut w: alloc::vec::Vec<u16> = alloc::vec![0; units + 1];
    loop {
        let hi = IN_HIGH.swap(0, Relaxed);
        let start = (hi != 0) as usize;
        w[0] = hi as u16;
        let want = if start == 1 { (units - 1).max(1) } else { units };
        let mut done = 0u32;
        if ReadConsoleW(handle(0), w.as_mut_ptr().add(start), want as u32, &mut done, core::ptr::null_mut()) == 0 {
            return -1;
        }
        let mut len = start + done as usize;
        if done == 0 && start == 0 {
            return 0;
        }
        if start == 0 && w[0] == 0x1a {
            return 0;
        }
        if done > 0 && (0xd800..0xdc00).contains(&w[len - 1]) {
            IN_HIGH.store(w[len - 1] as u32, Relaxed);
            len -= 1;
        }
        if len == 0 {
            continue;
        }
        let bn = WideCharToMultiByte(CP_UTF8, 0, w.as_ptr(), len as i32, core::ptr::null_mut(), 0, core::ptr::null(), core::ptr::null_mut());
        if bn <= 0 {
            return -1;
        }
        let mut b: alloc::vec::Vec<u8> = alloc::vec![0; bn as usize];
        WideCharToMultiByte(CP_UTF8, 0, w.as_ptr(), len as i32, b.as_mut_ptr(), bn, core::ptr::null(), core::ptr::null_mut());
        let k = b.len().min(n);
        core::ptr::copy_nonoverlapping(b.as_ptr(), p, k);
        let mut rest = ((b.len() - k) as u32) << 24;
        for (i, &c) in b[k..].iter().enumerate().take(3) {
            rest |= (c as u32) << (8 * i);
        }
        IN_HELD.store(rest, Relaxed);
        return k as isize;
    }
}

/// At most `n` bytes (`u32` at a time); 0 at the end, -1 on an error.
pub unsafe fn read(fd: c_int, buf: *mut c_void, n: usize) -> isize {
    if fd == 0 && n > 0 && is_console(0) {
        return read_console(buf as *mut u8, n);
    }
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

/// Whether the standard input (0), output (1) and error (2) are consoles
/// (D408, D409): 0 not known yet, 1 a console, 2 not.
static CONSOLE: [core::sync::atomic::AtomicU8; 3] = [const { core::sync::atomic::AtomicU8::new(0) }; 3];
/// The bytes of a character of UTF-8 cut at the end of a write to such a
/// console, for the next write: up to 3 bytes, their count in the top byte
/// (D408). The writes of one stream come one at a time (the buffer of
/// `print` has one holder).
static HELD: [core::sync::atomic::AtomicU32; 3] = [const { core::sync::atomic::AtomicU32::new(0) }; 3];

fn is_console(fd: c_int) -> bool {
    use core::sync::atomic::Ordering::Relaxed;
    if !(0..=2).contains(&fd) {
        return false;
    }
    let c = &CONSOLE[fd as usize];
    match c.load(Relaxed) {
        1 => true,
        2 => false,
        _ => {
            const FILE_TYPE_CHAR: u32 = 2;
            let h = handle(fd);
            let mut mode = 0u32;
            // SAFETY: a handle of the process and a place for its mode
            let yes = unsafe { GetFileType(h) == FILE_TYPE_CHAR && GetConsoleMode(h, &mut mode) != 0 };
            c.store(if yes { 1 } else { 2 }, Relaxed);
            yes
        }
    }
}

/// `n` bytes of UTF-8 written to a console as UTF-16 (D408), by
/// `WriteConsoleW`: written by `WriteFile`, a console takes each byte as a
/// character of its code page (`é` shown as `Ã©` under the code page 850 or
/// 437). The bytes held from the last write first; those of a character
/// cut at the end held for the next; all `n` counted as written. Bytes that
/// are not UTF-8 shown as U+FFFD.
unsafe fn write_console(fd: c_int, p: *const u8, n: usize) -> isize {
    use core::sync::atomic::Ordering::Relaxed;
    let held = HELD[fd as usize].load(Relaxed);
    let hn = (held >> 24) as usize;
    let mut b: alloc::vec::Vec<u8> = alloc::vec::Vec::with_capacity(hn + n);
    for k in 0..hn {
        b.push((held >> (8 * k)) as u8);
    }
    b.extend_from_slice(core::slice::from_raw_parts(p, n));
    let cut = isc_runtime_core::utf8_whole(&b);
    let mut h = ((b.len() - cut) as u32) << 24;
    for (k, &c) in b[cut..].iter().enumerate() {
        h |= (c as u32) << (8 * k);
    }
    HELD[fd as usize].store(h, Relaxed);
    let body = &b[..cut];
    if body.is_empty() {
        return n as isize;
    }
    let wn = MultiByteToWideChar(CP_UTF8, 0, body.as_ptr(), body.len() as i32, core::ptr::null_mut(), 0);
    if wn <= 0 {
        return -1;
    }
    let mut w: alloc::vec::Vec<u16> = alloc::vec![0; wn as usize];
    MultiByteToWideChar(CP_UTF8, 0, body.as_ptr(), body.len() as i32, w.as_mut_ptr(), wn);
    let hd = handle(fd);
    let mut at = 0usize;
    while at < w.len() {
        let mut done = 0u32;
        let k = (w.len() - at).min(1 << 14) as u32;
        if WriteConsoleW(hd, w.as_ptr().add(at), k, &mut done, core::ptr::null_mut()) == 0 || done == 0 {
            return -1;
        }
        at += done as usize;
    }
    n as isize
}

pub unsafe fn write(fd: c_int, buf: *const c_void, n: usize) -> isize {
    if n > 0 && is_console(fd) {
        return write_console(fd, buf as *const u8, n);
    }
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

/// A block of `size` bytes at a multiple of `align` (a power of two, at
/// least 16) from the heap of the process (D541: `_aligned_malloc` of
/// MinGW's C library before): `align` bytes more than asked, the address
/// given by the heap kept in the 8 bytes before the block. Null when
/// refused.
pub unsafe fn aligned_alloc(align: usize, size: usize) -> *mut c_void {
    // SAFETY: the heap of the process, which the system made
    let raw = unsafe { HeapAlloc(GetProcessHeap(), 0, size.max(1).saturating_add(align)) } as usize;
    if raw == 0 {
        return core::ptr::null_mut();
    }
    // the heap gives multiples of 16: the block starts 16 to `align` bytes in
    let p = (raw + align) & !(align - 1);
    // SAFETY: 8 bytes of the block of the heap, before `p`
    unsafe { ((p - 8) as *mut usize).write(raw) };
    p as *mut c_void
}

/// Frees a block of `aligned_alloc` (null: nothing).
pub unsafe fn free(p: *mut c_void) {
    if !p.is_null() {
        // SAFETY: `p` from `aligned_alloc`, the address of its block of the
        // heap before it
        unsafe { HeapFree(GetProcessHeap(), 0, ((p as usize - 8) as *const usize).read() as *mut c_void) };
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

// ---------------------------------------------------------------------
// the start, the arguments and the environment (D541)

/// The string of UTF-16 at `p`, ended by a zero.
///
/// # Safety
/// `p` a string of UTF-16 ended by a zero, that lives as long as the
/// process (the command line of the process).
unsafe fn wide_str(p: *const u16) -> &'static [u16] {
    // SAFETY: see above
    unsafe { core::slice::from_raw_parts(p, isc_runtime_core::winargs::wide_len(p)) }
}

/// The arguments of the program after its name, in UTF-8, each given to
/// `each`: read from its command line in UTF-16 by the rules of Windows,
/// whatever started the program (its own start, or MinGW's, whose `argv` is
/// in the code page of the system).
pub fn args(each: &mut dyn FnMut(&[u8])) {
    // SAFETY: the command line of the process, which lives as long as it
    let w = unsafe { wide_str(GetCommandLineW()) };
    let mut cur = alloc::vec::Vec::new();
    let mut bytes = alloc::vec![0u8; 3 * w.len()];
    let mut first = true;
    isc_runtime_core::winargs::command_line_args(w, &mut cur, &mut |a| {
        if !core::mem::take(&mut first) {
            let n = isc_runtime_core::winargs::utf8_of_utf16(a, &mut bytes);
            each(&bytes[..n]);
        }
    });
}

/// The environment of the process in UTF-8 (`NAME=value` strings each ended
/// by a zero byte, the block by an empty one), made once at the first
/// `getenv` from the system's in UTF-16, in the heap of the process (the
/// first reads come before the runtime's allocator, which reads
/// `ISC_ALLOC`): the address of its length (8 bytes) followed by it, 0
/// before.
static ENV: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

fn env_block() -> &'static [u8] {
    use core::sync::atomic::Ordering::Acquire;
    let p = ENV.load(Acquire);
    if p != 0 {
        // SAFETY: a block made below, never freed nor changed
        return unsafe { core::slice::from_raw_parts((p + 8) as *const u8, (p as *const usize).read()) };
    }
    // SAFETY: the environment of the process, units ended by two zeros; a
    // block of the heap of `8 + cap` bytes
    unsafe {
        let base = GetEnvironmentStringsW();
        let mut n = 0;
        if !base.is_null() {
            while *base.add(n) != 0 || *base.add(n + 1) != 0 {
                n += 1;
            }
            n += 1;
        }
        let w: &[u16] = if base.is_null() { &[] } else { core::slice::from_raw_parts(base, n) };
        let cap = 3 * w.len() + 2;
        let q = HeapAlloc(GetProcessHeap(), 0, 8 + cap) as usize;
        if q == 0 {
            if !base.is_null() {
                FreeEnvironmentStringsW(base);
            }
            return &[];
        }
        let out = core::slice::from_raw_parts_mut((q + 8) as *mut u8, cap);
        // the strings with their zeros (units of `w`), then an empty one
        let mut len = isc_runtime_core::winargs::utf8_of_utf16(w, out);
        out[len] = 0;
        out[len + 1] = 0;
        len += 2;
        (q as *mut usize).write(len);
        if !base.is_null() {
            FreeEnvironmentStringsW(base);
        }
        // another thread may have made one first: its block is kept
        match ENV.compare_exchange(0, q, core::sync::atomic::Ordering::Release, Acquire) {
            Ok(_) => core::slice::from_raw_parts((q + 8) as *const u8, len),
            Err(other) => {
                HeapFree(GetProcessHeap(), 0, q as *mut c_void);
                core::slice::from_raw_parts((other + 8) as *const u8, (other as *const usize).read())
            }
        }
    }
}

/// The value of the variable `name` (a string ended by a zero byte) of the
/// environment, a string ended by a zero byte, or null; the case of the
/// ASCII letters of a name does not matter, as under Windows.
pub unsafe fn getenv(name: *const c_char) -> *const c_char {
    // SAFETY: `name` ends with a zero byte (see above)
    let name = unsafe { core::slice::from_raw_parts(name as *const u8, super::strlen(name)) };
    let block = env_block();
    match isc_runtime_core::winargs::env_find(block, name) {
        Some((at, _)) => block[at..].as_ptr() as *const c_char,
        None => core::ptr::null(),
    }
}

/// Ends the process with `code`.
pub fn exit(code: c_int) -> ! {
    // SAFETY: the end of the process
    unsafe { ExitProcess(code as u32) }
}

/// Ends the process at once with the code of `abort` (3).
pub fn abort() -> ! {
    terminate(3)
}

/// The start of a program for Windows without a C library (D541): its
/// `main` (the generated code, whose arguments `isc_args` reads from the
/// command line), then the end of the process with its code.
#[no_mangle]
pub extern "system" fn isc_win_start() -> ! {
    extern "C" {
        fn main(argc: c_int, argv: *const *const c_char) -> c_int;
    }
    // SAFETY: the `main` of the generated code, which does not read its
    // parameters under Windows but to give them to `isc_args`
    exit(unsafe { main(0, core::ptr::null()) })
}
