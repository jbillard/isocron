//! Isocron runtime of compiled programs (SPEC section 13): aligned
//! allocation, I/O actions, printing, clock and panics, through the C ABI
//! functions `isc_*` called by the generated code.
//!
//! It uses no part of the Rust standard library, only `core` (number
//! formatting) and the C library (`open`, `read`, `write`, `aligned_alloc`,
//! `clock_gettime`, ...), so that compiled programs stay small. The texts of
//! messages come from `isc-runtime-core`, shared with the interpreter.
//! Targets: x86-64 Linux; wasm32 WASI without threads (`wasm32-wasip1`: the
//! thread pool is replaced by a sequential loop); wasm32 WASI with threads
//! (`wasm32-wasip1-threads`: wasi-threads, shared memory, the futex replaced
//! by `memory.atomic.wait32` / `memory.atomic.notify`).

#![no_std]
// the functions of the system that are safe under Linux (D287) are unsafe
// declarations of the C library elsewhere: their callers keep `unsafe`
// blocks (until the unsafe of the runtime is confined, TODO.md)
#![cfg_attr(target_os = "linux", allow(unused_unsafe))]
// without threads the pool is not compiled: its constants and statics are unused
#![cfg_attr(all(target_os = "wasi", not(isc_wasm_threads)), allow(dead_code))]

use core::ffi::{c_char, c_int, c_void};
use core::fmt::{self, Write};
use isc_runtime_core::{self as sem, Panic, ARRAY_ALIGN, PANIC_EXIT_CODE};

#[cfg(not(target_os = "wasi"))]
mod heap;

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod mapped;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct IscArray {
    pub ptr: *mut u8,
    pub len: i64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct IscStr {
    pub ptr: *const u8,
    pub len: i64,
}

// SAFETY: a string owns its bytes (from the allocator of the runtime, which
// threads share) or points to constant bytes of the program: it may be made
// on one thread and used on another (the tasks that fill an array of them)
unsafe impl Send for IscStr {}

// SAFETY: a string is a pointer and a length, no interior mutability: a
// reference to it shared by threads only reads those two (its bytes are
// read through `bytes_of`, the caller's promise that they are alive), as
// the tasks of `join` read the strings of an array (D427)
unsafe impl Sync for IscStr {}

#[repr(C)]
struct Timespec {
    tv_sec: i64,
    /// `long`: 64 bits on x86-64, 32 bits on wasm32.
    tv_nsec: core::ffi::c_long,
}

// WASI (D543): its own functions called directly, no C library
#[cfg(target_os = "wasi")]
mod wasi;
#[cfg(target_os = "wasi")]
use wasi::{abort, aligned_alloc, close, exit, free, getenv, lseek, open, read, strerror, strlen, write};

// Linux (D287): the system called directly, no C library
#[cfg(target_os = "linux")]
mod linux;

// the primitives of the system of the standard library (D322)
mod system;
mod net;
// The functions of memory that LLVM and `core` call (D288): those of
// `isc-mem`, the runtime's own under Linux; in a program linked with glibc,
// they are the ones of the program too (the definitions of an executable
// come first).
#[cfg(any(target_os = "linux", windows))]
use isc_mem::symbols::strlen;
#[cfg(target_os = "linux")]
use linux::{abort, close, exit, lseek, open, pread, read, strerror, write};

// The parallel loops of the runtime itself, safe (D290)
mod par;

// The start of the programs, their threads and their thread-local storage
// without a C library, or glibc's for the programs that declare C functions
// (D289): `process::os()`, which also gives glibc's allocator to those
// (`ISC_ALLOC=glibc`)
#[cfg(target_os = "linux")]
mod process;

/// The allocator of glibc (`ISC_ALLOC=glibc`) when the program has glibc.
#[cfg(target_os = "linux")]
unsafe fn aligned_alloc(align: usize, size: usize) -> *mut c_void {
    match process::os().alloc {
        // SAFETY: glibc's `aligned_alloc`
        Some(f) => unsafe { f(align, size) },
        None => core::ptr::null_mut(),
    }
}

#[cfg(target_os = "linux")]
unsafe fn free(p: *mut c_void) {
    if let Some(f) = process::os().free {
        // SAFETY: glibc's `free`, of a block of its `aligned_alloc`
        unsafe { f(p) }
    }
}

// Windows (D246): the same functions over the Win32 API
#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows::{abort, aligned_alloc, close, free, getenv, lseek, open, pread, read, strerror, write};

// The constants of the C library: Linux (glibc, musl) or WASI (wasi-libc);
// those of `windows` under Windows.
#[cfg(windows)]
mod sys {
    pub use super::windows::{monotonic, O_CLOEXEC, O_CREAT, O_RDONLY, O_TRUNC, O_WRONLY};
}

#[cfg(target_os = "linux")]
mod sys {
    use core::ffi::c_int;
    pub const O_RDONLY: c_int = 0;
    pub const O_WRONLY: c_int = 0o1;
    pub const O_CREAT: c_int = 0o100;
    pub const O_TRUNC: c_int = 0o1000;
    pub const O_CLOEXEC: c_int = 0o2000000;
    /// # Safety
    /// `ts` is a valid `Timespec`.
    pub unsafe fn monotonic(ts: *mut super::Timespec) {
        // SAFETY: the kernel writes one Timespec at `ts` (see above)
        unsafe { super::linux::clock_gettime(1, ts) };
    }
}

#[cfg(target_os = "wasi")]
mod sys {
    pub use super::wasi::{monotonic, O_CLOEXEC, O_CREAT, O_RDONLY, O_TRUNC, O_WRONLY};
}

use sys::{O_CLOEXEC, O_CREAT, O_RDONLY, O_TRUNC, O_WRONLY};
const SEEK_SET: c_int = 0;
const SEEK_CUR: c_int = 1;
const SEEK_END: c_int = 2;
const EINTR: c_int = 4;

// `alloc`, for the `bigint` of isc-runtime-core (D156): the blocks of the
// runtime (aligned on 64 bytes, the largest alignment asked)
extern crate alloc;

struct RuntimeAlloc;

unsafe impl core::alloc::GlobalAlloc for RuntimeAlloc {
    unsafe fn alloc(&self, l: core::alloc::Layout) -> *mut u8 {
        if l.align() > ARRAY_ALIGN {
            return core::ptr::null_mut();
        }
        isc_alloc(l.size().max(1) as i64)
    }
    unsafe fn dealloc(&self, p: *mut u8, _: core::alloc::Layout) {
        isc_free(p)
    }
}

#[global_allocator]
static RUNTIME_ALLOC: RuntimeAlloc = RuntimeAlloc;

/// Panics in the runtime itself (a bug) abort the process.
#[panic_handler]
fn rust_panic(_: &core::panic::PanicInfo) -> ! {
    unsafe { abort() }
}

/// The precompiled `core` is built for unwinding, so its unwind tables refer
/// to this personality routine. The runtime is built with `panic = "abort"`:
/// nothing ever unwinds, the routine is never called, but the linker needs
/// the symbol.
#[no_mangle]
pub extern "C" fn rust_eh_personality() {}

// ---------------------------------------------------------------------
// output

/// Standard output, buffered, flushed at the end of `main`, before a panic,
/// before the standard input is read and before a function of C is called;
/// and at each newline (the generated code ends every `print` with
/// `isc_print_newline`) when it is read as it comes (`out_by_lines`).
struct OutBuf {
    data: [u8; 1 << 16],
    len: usize,
}

static mut OUT: OutBuf = OutBuf { data: [0; 1 << 16], len: 0 };

/// The standard output flushed at each newline (1) or when its buffer is
/// full (2), decided at the first newline (0 before): by blocks into a
/// file or a device that is not a terminal (`> file`, `> /dev/null`: no one
/// reads them as they come), at each newline into a terminal, a pipe or a
/// socket (D299: a write of the system for each `print`, 187 ns a line of
/// one number written into `/dev/null`).
static OUT_LINES: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

fn out_by_lines() -> bool {
    match OUT_LINES.load(core::sync::atomic::Ordering::Relaxed) {
        1 => true,
        2 => false,
        _ => {
            let lines = stdout_read_as_it_comes();
            OUT_LINES.store(if lines { 1 } else { 2 }, core::sync::atomic::Ordering::Relaxed);
            lines
        }
    }
}

/// The standard output is read as it comes (`out_by_lines`).
#[cfg(target_os = "linux")]
fn stdout_read_as_it_comes() -> bool {
    const S_IFMT: u32 = 0o170000;
    const S_IFREG: u32 = 0o100000;
    const S_IFCHR: u32 = 0o020000;
    match linux::file_mode(1) {
        Some(m) if m & S_IFMT == S_IFREG => false,
        Some(m) if m & S_IFMT == S_IFCHR => linux::is_terminal(1),
        _ => true,
    }
}

#[cfg(windows)]
fn stdout_read_as_it_comes() -> bool {
    windows::stdout_read_as_it_comes()
}

#[cfg(not(any(target_os = "linux", windows)))]
fn stdout_read_as_it_comes() -> bool {
    true
}

fn write_all(fd: c_int, mut p: *const u8, mut n: usize) {
    while n > 0 {
        let r = unsafe { write(fd, p as *const c_void, n) };
        if r < 0 {
            if fail(r) == EINTR {
                continue;
            }
            return;
        }
        p = unsafe { p.add(r as usize) };
        n -= r as usize;
    }
}

/// The tasks of `mapParallel` running (D373). While there are any, a line
/// of `print` is written whole: the thread that writes its first part holds
/// the buffer (`OUT_HOLDER`) until its newline. Otherwise `out` reads only
/// this count.
static ACTIONS: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
/// The thread that holds the buffer of `print` (`thread_id`), 0 none.
static OUT_HOLDER: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

/// A number of the calling thread, not 0, that no other running thread has.
#[cfg(not(target_os = "wasi"))]
fn thread_id() -> usize {
    heap::thread_id()
}

/// One thread under WASI.
#[cfg(target_os = "wasi")]
fn thread_id() -> usize {
    1
}

/// The buffer of `print` held by the calling thread, when tasks of actions
/// run (D373); at once when it already holds it.
#[inline(always)]
fn hold_out() {
    if ACTIONS.load(core::sync::atomic::Ordering::Relaxed) != 0 {
        hold_out_shared();
    }
}

#[cold]
fn hold_out_shared() {
    use core::sync::atomic::Ordering;
    let me = thread_id();
    if OUT_HOLDER.load(Ordering::Relaxed) == me {
        return;
    }
    let mut spins = 0u32;
    while OUT_HOLDER.compare_exchange_weak(0, me, Ordering::Acquire, Ordering::Relaxed).is_err() {
        if spins < SPIN {
            core::hint::spin_loop();
            spins += 1;
        } else {
            pause_briefly();
        }
    }
}

/// `hold_out` before the end of the program by a panic: after about 0.1 s
/// without the buffer, taken anyway.
fn hold_out_at_end() {
    use core::sync::atomic::Ordering;
    if ACTIONS.load(Ordering::Relaxed) == 0 {
        return;
    }
    let me = thread_id();
    for _ in 0..2048 {
        let h = OUT_HOLDER.load(Ordering::Relaxed);
        if h == me || OUT_HOLDER.compare_exchange(0, me, Ordering::Acquire, Ordering::Relaxed).is_ok() {
            return;
        }
        pause_briefly();
    }
    OUT_HOLDER.store(me, Ordering::Release);
}

/// The buffer of `print` let go, if the calling thread holds it: at the end
/// of a line, of `printBytes`, of a flush.
#[inline(always)]
fn release_out() {
    use core::sync::atomic::Ordering;
    let h = OUT_HOLDER.load(Ordering::Relaxed);
    if h != 0 && h == thread_id() {
        OUT_HOLDER.store(0, Ordering::Release);
    }
}

/// The buffer written out (by the thread that holds it when tasks run).
fn flush_stdout_held() {
    hold_out();
    flush_stdout();
    release_out();
}

fn flush_stdout() {
    unsafe {
        let o = &mut *core::ptr::addr_of_mut!(OUT);
        write_all(1, o.data.as_ptr(), o.len);
        o.len = 0;
    }
}

fn out(bytes: &[u8]) {
    hold_out();
    out_held(bytes)
}

/// `out` by the thread that holds the buffer, or when no task of actions
/// runs.
fn out_held(bytes: &[u8]) {
    unsafe {
        let o = &mut *core::ptr::addr_of_mut!(OUT);
        if o.len + bytes.len() > o.data.len() {
            write_all(1, o.data.as_ptr(), o.len);
            o.len = 0;
            if bytes.len() > o.data.len() {
                write_all(1, bytes.as_ptr(), bytes.len());
                return;
            }
        }
        // a few bytes by moves, not a call of `memcpy` (D498: the digits of a
        // number a line, 15 % of the cycles of `print` there)
        copy_short(bytes.as_ptr(), o.data.as_mut_ptr().add(o.len), bytes.len());
        o.len += bytes.len();
    }
}

struct Stdout;

impl Write for Stdout {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        out(s.as_bytes());
        Ok(())
    }
}

struct Stderr;

impl Write for Stderr {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        write_all(2, s.as_ptr(), s.len());
        Ok(())
    }
}

/// The end of a cleanup of the precompiled `alloc` of Rust (built to
/// unwind): with `panic = "abort"` no panic unwinds, so no cleanup runs. A
/// program linked by the LLD of iscc gets no `libgcc_s` (D248); one linked
/// by `cc` takes this one too.
#[cfg(target_os = "linux")]
#[no_mangle]
pub extern "C" fn _Unwind_Resume() -> ! {
    unsafe { abort() }
}

/// Set by the first thread that panics.
static PANICKING: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

fn die(line: i32, col: i32, msg: &dyn fmt::Display) -> ! {
    // Several tasks may panic at the same time: the first one prints its
    // message and ends the program, the others wait for the end silently.
    if PANICKING.swap(true, core::sync::atomic::Ordering::AcqRel) {
        loop {
            core::hint::spin_loop();
        }
    }
    // the line another task is writing ended first (D373), waited for at
    // most about 0.1 s (that task may have stopped in the middle of it)
    hold_out_at_end();
    flush_stdout();
    let _ = sem::write_panic(&mut Stderr, line as u32, col as u32, msg);
    let _ = Stderr.write_str("\n");
    // under Windows, `exit` would run the end of the thread (the cache of
    // the heap given back) after the other threads were stopped, one of
    // them perhaps holding the lock of the heap: the process ends at once
    // (D246)
    #[cfg(windows)]
    windows::terminate(PANIC_EXIT_CODE);
    #[cfg(not(windows))]
    unsafe {
        exit(PANIC_EXIT_CODE)
    }
}

/// The code of the failed call that gave `r` (negative): its negative
/// result under Linux (D287), the `errno` of the C library elsewhere.
#[cfg(target_os = "linux")]
fn fail(r: isize) -> c_int {
    linux::fail_code(r)
}

#[cfg(target_os = "wasi")]
fn fail(r: isize) -> c_int {
    (-r) as c_int
}

#[cfg(windows)]
fn fail(_r: isize) -> c_int {
    windows::errno()
}

unsafe fn c_str<'a>(p: *const c_char) -> &'a str {
    let n = strlen(p);
    core::str::from_utf8_unchecked(core::slice::from_raw_parts(p as *const u8, n))
}

unsafe fn str_of<'a>(p: *const u8, len: i64) -> &'a str {
    if len <= 0 {
        return "";
    }
    core::str::from_utf8_unchecked(core::slice::from_raw_parts(p, len as usize))
}

// ---------------------------------------------------------------------
// memory

/// Whether the allocator of Isocron (`heap.rs`) is used: always, unless
/// `ISC_ALLOC=glibc` asks for glibc's (DECISIONS D81). Read once, at the
/// first allocation, before the thread pool exists.
// 0 not read yet, 1 glibc, 2 the allocator of Isocron; read by the
// compiled code too, which takes and gives back the cells of nodes itself
// in mode 2 (D176)
#[cfg(not(target_os = "wasi"))]
#[export_name = "isc_heap_mode"]
pub static HEAP_MODE: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

#[cfg(not(target_os = "wasi"))]
#[inline(always)]
fn isc_heap() -> bool {
    match HEAP_MODE.load(Ordering::Relaxed) {
        1 => false,
        2 => true,
        _ => heap_mode(),
    }
}

#[cfg(not(target_os = "wasi"))]
#[cold]
#[inline(never)]
fn heap_mode() -> bool {
    let v = unsafe { getenv(b"ISC_ALLOC\0".as_ptr() as *const c_char) };
    let mut isc = v.is_null() || unsafe { c_str(v) } != "glibc";
    // a program without glibc has only its own (D289)
    #[cfg(target_os = "linux")]
    if process::os().alloc.is_none() {
        isc = true;
    }
    HEAP_MODE.store(if isc { 2 } else { 1 }, Ordering::Relaxed);
    isc
}

#[inline(always)]
fn alloc_bytes(bytes: i64, zero: bool) -> *mut u8 {
    let b = (bytes.max(1) as usize).div_ceil(ARRAY_ALIGN) * ARRAY_ALIGN;
    #[cfg(not(target_os = "wasi"))]
    if isc_heap() {
        let p = unsafe { heap::alloc(b, zero) };
        if p.is_null() {
            die(0, 0, &Panic::OutOfMemory(bytes));
        }
        return p;
    }
    let p = unsafe { aligned_alloc(ARRAY_ALIGN, b) } as *mut u8;
    if p.is_null() {
        die(0, 0, &Panic::OutOfMemory(bytes));
    }
    if zero {
        unsafe { core::ptr::write_bytes(p, 0, bytes.max(0) as usize) };
    }
    p
}

/// Allocates `bytes` bytes aligned on 64 bytes. Never returns null. The
/// common case is a leaf (D165): a block of at most 1 KiB from the cache
/// of the thread; everything else is one tail call, so that the common
/// case saves no register.
#[no_mangle]
pub extern "C" fn isc_alloc(bytes: i64) -> *mut u8 {
    // nothing to hold (an empty array or text): the empty block of the
    // runtime, read only and never freed (D379)
    if bytes <= 0 {
        return empty_ptr();
    }
    #[cfg(not(target_os = "wasi"))]
    if HEAP_MODE.load(Ordering::Relaxed) == 2 && bytes <= 1024 {
        let p = unsafe { heap::alloc_cached((bytes.max(1) as usize).div_ceil(ARRAY_ALIGN) * ARRAY_ALIGN) };
        if !p.is_null() {
            return p;
        }
    }
    alloc_cold(bytes)
}

#[cold]
#[inline(never)]
fn alloc_cold(bytes: i64) -> *mut u8 {
    alloc_bytes(bytes, false)
}

/// A node of a recursive type of `bytes` bytes (D166): a cell of the
/// allocator of nodes, or a block of the general allocator.
#[no_mangle]
pub extern "C" fn isc_node_alloc(bytes: i64) -> *mut u8 {
    #[cfg(not(target_os = "wasi"))]
    if bytes > 0 && bytes as usize <= heap::NODE_MAX && isc_heap() {
        let p = unsafe { heap::node_alloc(bytes as usize) };
        if !p.is_null() {
            return p;
        }
        die(0, 0, &Panic::OutOfMemory(bytes));
    }
    alloc_cold(bytes)
}

/// A region of nodes (D200): the nodes made by a call whose tree dies at
/// once, cut from chunks by a bump (`bump`, `end`: the compiled code bumps
/// itself, `isc_region_refill` when the chunk is full), all freed at once.
/// With `ISC_ALLOC=glibc` each node is a block of its own (`nodes`), so
/// that Valgrind sees each. Native targets only: the compiler makes no
/// region for WebAssembly.
#[cfg(not(target_os = "wasi"))]
#[repr(C)]
pub struct Region {
    bump: *mut u8,
    end: *mut u8,
    /// The chunks, each one's first word the next one, its second its size.
    chunks: *mut u8,
    nodes: *mut alloc::vec::Vec<*mut u8>,
}

#[cfg(not(target_os = "wasi"))]
const REGION_FIRST: usize = 4 << 10;
#[cfg(not(target_os = "wasi"))]
const REGION_CHUNK: usize = 64 << 10;

#[cfg(not(target_os = "wasi"))]
#[no_mangle]
pub extern "C" fn isc_region_new() -> *mut Region {
    let r = isc_alloc(core::mem::size_of::<Region>() as i64) as *mut Region;
    unsafe { *r = Region { bump: core::ptr::null_mut(), end: core::ptr::null_mut(), chunks: core::ptr::null_mut(), nodes: core::ptr::null_mut() } };
    r
}

/// A cell of `bytes` bytes from a new chunk of the region (the first of 4
/// KiB, then 64 KiB); with glibc, a block of its own.
#[cfg(not(target_os = "wasi"))]
#[no_mangle]
pub unsafe extern "C" fn isc_region_refill(r: *mut Region, bytes: i64) -> *mut u8 {
    let n = bytes.max(1) as usize;
    if !isc_heap() {
        let p = isc_alloc(n as i64);
        if (*r).nodes.is_null() {
            (*r).nodes = alloc::boxed::Box::into_raw(alloc::boxed::Box::new(alloc::vec::Vec::new()));
        }
        (*(*r).nodes).push(p);
        return p;
    }
    let base = if (*r).chunks.is_null() { REGION_FIRST } else { REGION_CHUNK };
    let size = base.max(n + 16);
    let c = isc_alloc(size as i64);
    *(c as *mut *mut u8) = (*r).chunks;
    *(c as *mut usize).add(1) = size;
    (*r).chunks = c;
    let start = c.add(16);
    (*r).bump = start.add(n);
    (*r).end = c.add(size);
    start
}

#[cfg(not(target_os = "wasi"))]
#[no_mangle]
pub unsafe extern "C" fn isc_region_free(r: *mut Region) {
    if !(*r).nodes.is_null() {
        let v = alloc::boxed::Box::from_raw((*r).nodes);
        for &p in v.iter() {
            isc_free(p);
        }
    }
    let mut c = (*r).chunks;
    while !c.is_null() {
        let next = *(c as *mut *mut u8);
        isc_free(c);
        c = next;
    }
    isc_free(r as *mut u8);
}

/// Frees a node of `isc_node_alloc` of the same `bytes`.
#[no_mangle]
pub extern "C" fn isc_node_free(p: *mut u8, bytes: i64) {
    #[cfg(not(target_os = "wasi"))]
    if bytes > 0 && bytes as usize <= heap::NODE_MAX && isc_heap() {
        unsafe { heap::node_free(p, bytes as usize) };
        return;
    }
    free_cold(p)
}

#[no_mangle]
pub extern "C" fn isc_alloc_zeroed(bytes: i64) -> *mut u8 {
    if bytes <= 0 {
        return empty_ptr();
    }
    alloc_bytes(bytes, true)
}

#[no_mangle]
pub extern "C" fn isc_free(p: *mut u8) {
    // an empty string made by `new_str`: nothing owned; null, the field of
    // a variant of a sum type that the value is not (its drop frees every
    // field), without the cold path (D445: `parseUrl` 14 % of its cycles
    // in `free_cold` and `heap::free` for them)
    if p == empty_ptr() || p.is_null() {
        return;
    }
    #[cfg(not(target_os = "wasi"))]
    if HEAP_MODE.load(Ordering::Relaxed) == 2 && unsafe { heap::free_cached(p) } {
        return;
    }
    // a literal of the program kept as a string, the empty one too (D361):
    // nothing owned, without the call of the cold path
    if is_literal(p) {
        return;
    }
    free_cold(p)
}

#[cold]
#[inline(never)]
fn free_cold(p: *mut u8) {
    // the standard input mapped from its file (D257)
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    if mapped::owns(p) {
        unsafe { mapped::free(zap_pages) };
        return;
    }
    // a literal of the program kept as a string (D361): nothing owned
    if is_literal(p) {
        return;
    }
    // a line of the block of the lines of a text (D298): 8 bytes past a
    // multiple of 16, where no other block starts
    if p as usize % 16 == 8 {
        // SAFETY: only `lines` gives such strings (see `arena_step`)
        unsafe { arena_free(p) };
        return;
    }
    #[cfg(not(target_os = "wasi"))]
    if isc_heap() {
        unsafe { heap::free(p) };
        return;
    }
    unsafe { free(p as *mut c_void) }
}

// ---------------------------------------------------------------------
// files

/// A NUL-terminated copy of `path` (freed by the caller).
unsafe fn c_path(path: &str) -> *mut c_char {
    let p = isc_alloc(path.len() as i64 + 1);
    core::ptr::copy_nonoverlapping(path.as_ptr(), p, path.len());
    *p.add(path.len()) = 0;
    p as *mut c_char
}

/// The text and the number of an OS error, as the messages show them: the
/// Linux numbering everywhere, so that a program panics with the same message
/// natively, as WebAssembly and in the interpreter (WASI numbers its errors
/// differently: `ENOENT` is 44 there, 2 on Linux).
fn os_error<'a>(code: c_int) -> (&'a str, c_int) {
    #[cfg(target_os = "wasi")]
    {
        // WASI preview 1 numbering -> Linux numbering; `ENOTCAPABLE` (76, a
        // path outside the directories given to the engine) is shown as
        // `EACCES`
        (unsafe { c_str(strerror(code)) }, wasi::linux_code(code))
    }
    #[cfg(not(target_os = "wasi"))]
    (unsafe { c_str(strerror(code)) }, code)
}

/// Reads a whole file into a 64-byte aligned buffer, like `std::fs::read`.
/// Reads a file; an error instead of a panic (D138): the buffer freed and
/// the file closed.
/// The whole standard input (D155), read until end of file.
unsafe fn read_stdin_all(line: i32, col: i32) -> IscArray {
    // what the program wrote first is out (a question before its answer)
    flush_stdout_held();
    // a regular file (`< file`): the bytes left in it, the size of the
    // buffer, read without copies (D179); a pipe: a guess that grows
    let cur = lseek(0, 0, SEEK_CUR);
    let end = if cur >= 0 { lseek(0, 0, SEEK_END) } else { -1 };
    if cur >= 0 {
        lseek(0, cur, SEEK_SET);
    }
    let hint = if cur >= 0 && end > cur && end - cur < (1i64 << 44) { (end - cur) as usize } else { 1 << 16 };
    // a large file: mapped (D257), its pages touched by every thread
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    if cur >= 0 && end - cur >= mapped::MAP_MIN as i64 && end - cur < (1i64 << 44) {
        if let Some(p) = mapped::map_stdin(cur, (end - cur) as usize) {
            touch_pages(p, (end - cur) as usize);
            lseek(0, end, SEEK_SET);
            return IscArray { ptr: p, len: end - cur };
        }
    }
    match read_fd(0, hint, if hint > 1 << 16 || end > cur { cur } else { -1 }) {
        Ok(a) => a,
        Err(e) => {
            let (detail, code) = os_error(e);
            die(line, col, &Panic::Read { path: "<standard input>", detail, code })
        }
    }
}

/// The pages of the `n` bytes from `p` read once, by every thread of the
/// pool, each its part (D257: the pages of a mapped file, given by the
/// kernel by groups of 16 at each fault); nothing when the pool is busy or
/// has one thread (they are then given as they are read).
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
unsafe fn touch_pages(p: *mut u8, n: usize) {
    let threads = thread_count();
    if threads <= 1 || BUSY.load(Ordering::Relaxed) {
        return;
    }
    #[repr(C)]
    struct Ctx {
        p: *mut u8,
        n: usize,
        parts: usize,
    }
    extern "C" fn part(ctx: *mut c_void, k: i64) {
        unsafe {
            let c = &*(ctx as *const Ctx);
            let k = k as usize;
            let (lo, hi) = (c.n * k / c.parts, c.n * (k + 1) / c.parts);
            let mut a = lo & !4095;
            // the pages of the part mapped by the kernel in one call (D285:
            // pages of 4 KiB of the page cache, a tmpfs, one fault each when
            // read: 63631 faults for the 254 MB of reverse-complement); read
            // one by one where the system does not have it (before 5.14)
            const MADV_POPULATE_READ: c_int = 22;
            if a < hi && mapped::advise(c.p.add(a), hi - a, MADV_POPULATE_READ) == 0 {
                return;
            }
            while a < hi {
                core::ptr::read_volatile(c.p.add(a));
                a += 4096;
            }
        }
    }
    let parts = threads.min(64);
    let mut ctx = Ctx { p, n, parts };
    isc_parallel_for(parts as i64, part, &mut ctx as *mut Ctx as *mut c_void, core::mem::size_of::<Ctx>() as i64);
}

/// The pages of the `n` bytes from `p`, a part of the mapped input, given
/// back by every thread of the pool, each its part (D258); by this thread
/// alone when the pool is busy or has one thread.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
unsafe fn zap_pages(p: *mut u8, n: usize) {
    let threads = thread_count();
    if threads <= 1 || BUSY.load(Ordering::Relaxed) {
        mapped::give_pages(p, n);
        return;
    }
    #[repr(C)]
    struct Ctx {
        p: *mut u8,
        n: usize,
        parts: usize,
    }
    extern "C" fn part(ctx: *mut c_void, k: i64) {
        unsafe {
            let c = &*(ctx as *const Ctx);
            let k = k as usize;
            // whole pages: the part of each thread from a page boundary
            let lo = (c.n * k / c.parts) & !4095;
            let hi = if k + 1 == c.parts { c.n } else { (c.n * (k + 1) / c.parts) & !4095 };
            if hi > lo {
                mapped::give_pages(c.p.add(lo), hi - lo);
            }
        }
    }
    let parts = threads.min(64);
    let mut ctx = Ctx { p, n, parts };
    isc_parallel_for(parts as i64, part, &mut ctx as *mut Ctx as *mut c_void, core::mem::size_of::<Ctx>() as i64);
}

/// The end of the program: the mapped input, if it is still there, given
/// back by every thread (D258), where the exit would unmap its pages on one
/// thread.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn release_at_end() {
    if mapped::mapped() {
        unsafe { mapped::free(zap_pages) };
    }
}

/// From this size on, a file is read by several threads (D221).
#[cfg(not(target_os = "wasi"))]
const READ_PARALLEL_MIN: usize = 8 << 20;

/// The first `n` bytes of `fd` from its offset `at` into `buf`, read by
/// every thread of the pool, each its part with `pread` (D221): the copies
/// out of the page cache, and the clearing of the new pages they fill, are
/// shared by the cores, where one `read` makes them on one. The number of
/// bytes read without a gap from `at` (fewer than `n` when the file got
/// shorter); the error code of `pread` otherwise. The offset of `fd` is
/// left as it was. Nothing read when the pool is busy or has one thread.
#[cfg(not(target_os = "wasi"))]
unsafe fn read_parallel(fd: c_int, buf: *mut u8, n: usize, at: i64) -> Result<usize, c_int> {
    let threads = thread_count();
    if threads <= 1 || n < READ_PARALLEL_MIN || BUSY.load(Ordering::Relaxed) {
        return Ok(0);
    }
    const PARTS_MAX: usize = 64;
    let parts = threads.min(PARTS_MAX);
    #[repr(C)]
    struct Ctx {
        fd: c_int,
        buf: *mut u8,
        n: usize,
        at: i64,
        parts: usize,
        got: *mut usize,
        err: *mut c_int,
    }
    extern "C" fn part(ctx: *mut c_void, k: i64) {
        unsafe {
            let c = &*(ctx as *const Ctx);
            let k = k as usize;
            let (lo, hi) = (c.n * k / c.parts, c.n * (k + 1) / c.parts);
            let mut done = lo;
            while done < hi {
                let r = pread(c.fd, c.buf.add(done) as *mut c_void, hi - done, c.at + done as i64);
                if r < 0 {
                    let e = fail(r);
                    if e == EINTR {
                        continue;
                    }
                    *c.err.add(k) = e;
                    break;
                }
                if r == 0 {
                    break;
                }
                done += r as usize;
            }
            *c.got.add(k) = done - lo;
        }
    }
    let mut got = [0usize; PARTS_MAX];
    let mut err = [0 as c_int; PARTS_MAX];
    let mut ctx = Ctx { fd, buf, n, at, parts, got: got.as_mut_ptr(), err: err.as_mut_ptr() };
    isc_parallel_for(parts as i64, part, &mut ctx as *mut Ctx as *mut c_void, core::mem::size_of::<Ctx>() as i64);
    // the bytes without a gap: up to the first part that stopped short
    let mut len = 0usize;
    for k in 0..parts {
        if err[k] != 0 {
            return Err(err[k]);
        }
        let size = n * (k + 1) / parts - n * k / parts;
        len += got[k];
        if got[k] < size {
            break;
        }
    }
    Ok(len)
}

/// Every byte of `fd` until end of file, in a buffer of `hint` bytes first
/// (the size of a file): at the end of the buffer, a probe for end of file
/// before it grows (doubled, copied); the error code of `read` otherwise.
/// `at`: the offset of `fd` when it can be read at any offset (a file:
/// its first `hint` bytes are then read in parallel, D221), -1 otherwise.
unsafe fn read_fd(fd: c_int, hint: usize, at: i64) -> Result<IscArray, c_int> {
    let mut cap: usize = hint.max(1);
    let mut buf = isc_alloc(cap as i64);
    let mut len: usize = 0;
    #[cfg(not(target_os = "wasi"))]
    if at >= 0 {
        match read_parallel(fd, buf, cap, at) {
            Ok(got) => {
                len = got;
                if got > 0 {
                    lseek(fd, at + got as i64, SEEK_SET);
                }
            }
            Err(e) => {
                isc_free(buf);
                return Err(e);
            }
        }
    }
    #[cfg(target_os = "wasi")]
    let _ = at;
    loop {
        if len == cap {
            let mut probe = [0u8; 64];
            let r = read(fd, probe.as_mut_ptr() as *mut c_void, probe.len());
            if r < 0 {
                let e = fail(r);
                if e == EINTR {
                    continue;
                }
                isc_free(buf);
                return Err(e);
            }
            if r == 0 {
                break;
            }
            let nb = isc_alloc((cap * 2) as i64);
            core::ptr::copy_nonoverlapping(buf, nb, len);
            isc_free(buf);
            buf = nb;
            cap *= 2;
            core::ptr::copy_nonoverlapping(probe.as_ptr(), buf.add(len), r as usize);
            len += r as usize;
            continue;
        }
        let r = read(fd, buf.add(len) as *mut c_void, cap - len);
        if r < 0 {
            let e = fail(r);
            if e == EINTR {
                continue;
            }
            isc_free(buf);
            return Err(e);
        }
        if r == 0 {
            break;
        }
        len += r as usize;
    }
    Ok(IscArray { ptr: buf, len: len as i64 })
}

#[no_mangle]
pub unsafe extern "C" fn isc_read_stdin(out: *mut IscArray, line: i32, col: i32) {
    *out = read_stdin_all(line, col);
}

#[no_mangle]
pub unsafe extern "C" fn isc_read_stdin_text(out: *mut IscStr, line: i32, col: i32) {
    let a = read_stdin_all(line, col);
    if let Err(at) = sem::utf8_check(bytes_of(a.ptr, a.len)) {
        die(line, col, &Panic::StdinNotUtf8(at as i64))
    }
    *out = IscStr { ptr: a.ptr, len: a.len };
}

unsafe fn try_read_file(path: &str) -> Result<IscArray, Panic<'_>> {
    let cp = c_path(path);
    let fd = open(cp, O_RDONLY | O_CLOEXEC, 0);
    isc_free(cp as *mut u8);
    let open_error = |code: c_int| {
        let (detail, code) = os_error(code);
        Panic::Read { path, detail, code }
    };
    // WASI engines open a directory, then refuse to read it with `EBADF`:
    // on a descriptor opened just before, that can only be a directory, so
    // it is reported as Linux does (`EISDIR`)
    let read_error = |code: c_int| open_error(if cfg!(target_os = "wasi") && code == 8 { 31 } else { code });
    if fd < 0 {
        return Err(open_error(fail(fd as isize)));
    }
    // the size is a hint; the file is read until end of file
    let end = lseek(fd, 0, SEEK_END);
    lseek(fd, 0, SEEK_SET);
    let hint = if end > 0 && end < (1i64 << 44) { end as usize } else { 1 << 16 };
    let r = read_fd(fd, hint, if end > 0 { 0 } else { -1 });
    close(fd);
    r.map_err(read_error)
}

unsafe fn read_file(path: &str, line: i32, col: i32) -> IscArray {
    match try_read_file(path) {
        Ok(a) => a,
        Err(e) => die(line, col, &e),
    }
}

/// Writes `bytes` (produced chunk by chunk by `fill`) to a file, like
/// `std::fs::write`; an error instead of a panic (D138), the file closed.
unsafe fn try_write_file<'a>(path: &'a str, total: usize, fill: &dyn Fn(usize, &mut [u8]) -> usize) -> Result<(), Panic<'a>> {
    let cp = c_path(path);
    let fd = open(cp, O_WRONLY | O_CREAT | O_TRUNC | O_CLOEXEC, 0o666 as c_int);
    isc_free(cp as *mut u8);
    let error = |code: c_int| {
        let (detail, code) = os_error(code);
        Panic::Write { path, detail, code }
    };
    if fd < 0 {
        return Err(error(fail(fd as isize)));
    }
    let mut chunk = [0u8; 8192];
    let mut done = 0usize;
    while done < total {
        let n = fill(done, &mut chunk);
        let mut off = 0usize;
        while off < n {
            let r = write(fd, chunk.as_ptr().add(off) as *const c_void, n - off);
            if r < 0 {
                let e = fail(r);
                if e == EINTR {
                    continue;
                }
                close(fd);
                return Err(error(e));
            }
            off += r as usize;
        }
        done += n;
    }
    let c = close(fd);
    if c < 0 {
        return Err(error(fail(c as isize)));
    }
    Ok(())
}

unsafe fn write_file(path: &str, total: usize, line: i32, col: i32, fill: &dyn Fn(usize, &mut [u8]) -> usize) {
    if let Err(e) = try_write_file(path, total, fill) {
        die(line, col, &e)
    }
}

// The functions that produce an array write it through `out` instead of
// returning a structure: the generated code and the C ABI of the target then
// cannot disagree on how a structure is returned (wasm32 returns it through
// a hidden pointer).

#[no_mangle]
pub unsafe extern "C" fn isc_read_bytes(out: *mut IscArray, path: *const u8, plen: i64, line: i32, col: i32) {
    *out = read_file(str_of(path, plen), line, col);
}

/// Little-endian `f64` file; x86-64 is little-endian, so the bytes are the
/// array.
#[no_mangle]
pub unsafe extern "C" fn isc_read_f64(out: *mut IscArray, path: *const u8, plen: i64, line: i32, col: i32) {
    let p = str_of(path, plen);
    let a = read_file(p, line, col);
    if a.len % 8 != 0 {
        die(line, col, &Panic::NotMultipleOf { path: p, len: a.len as u64, size: 8 });
    }
    *out = IscArray { ptr: a.ptr, len: a.len / 8 };
}

/// Little-endian `f32` file.
#[no_mangle]
pub unsafe extern "C" fn isc_read_f32(out: *mut IscArray, path: *const u8, plen: i64, line: i32, col: i32) {
    let p = str_of(path, plen);
    let a = read_file(p, line, col);
    if a.len % 4 != 0 {
        die(line, col, &Panic::NotMultipleOf { path: p, len: a.len as u64, size: 4 });
    }
    *out = IscArray { ptr: a.ptr, len: a.len / 4 };
}

#[no_mangle]
pub unsafe extern "C" fn isc_write_f32(path: *const u8, plen: i64, data: *const f32, n: i64, line: i32, col: i32) {
    let n = n.max(0) as usize;
    write_file(str_of(path, plen), n * 4, line, col, &|done, chunk| {
        let first = done / 4;
        let count = (chunk.len() / 4).min(n - first);
        for i in 0..count {
            let bits = sem::canonical_bits32(*data.add(first + i));
            chunk[i * 4..i * 4 + 4].copy_from_slice(&bits.to_le_bytes());
        }
        count * 4
    });
}

#[no_mangle]
pub unsafe extern "C" fn isc_write_f64(path: *const u8, plen: i64, data: *const f64, n: i64, line: i32, col: i32) {
    let n = n.max(0) as usize;
    write_file(str_of(path, plen), n * 8, line, col, &|done, chunk| {
        let first = done / 8;
        let count = (chunk.len() / 8).min(n - first);
        for i in 0..count {
            let bits = sem::canonical_bits(*data.add(first + i));
            chunk[i * 8..i * 8 + 8].copy_from_slice(&bits.to_le_bytes());
        }
        count * 8
    });
}

#[no_mangle]
pub unsafe extern "C" fn isc_write_bytes(path: *const u8, plen: i64, data: *const u8, n: i64, line: i32, col: i32) {
    let n = n.max(0) as usize;
    write_file(str_of(path, plen), n, line, col, &|done, chunk| {
        let count = chunk.len().min(n - done);
        core::ptr::copy_nonoverlapping(data.add(done), chunk.as_mut_ptr(), count);
        count
    });
}

// ---------------------------------------------------------------------
// printing, clock, arguments

#[no_mangle]
pub extern "C" fn isc_print_i64(v: i64) {
    let mut b = [0u8; 20];
    out(signed_decimal(v, &mut b));
}

#[no_mangle]
pub extern "C" fn isc_print_u64(v: u64) {
    let mut b = [0u8; 20];
    out(decimal(v, &mut b));
}

/// `print(v)` of an integer: its digits and the newline written at once,
/// as `isc_print_i64` then `isc_print_newline` (one test of the tasks, the
/// end of the line for a terminal or a pipe) (D498: two calls a line, the
/// buffer taken twice).
#[no_mangle]
pub extern "C" fn isc_print_i64_line(v: i64) {
    let mut b = [0u8; 21];
    let n = signed_decimal(v, (&mut b[..20]).try_into().unwrap()).len();
    // the digits are at the end of the 20 bytes: the newline after them
    b[20] = b'\n';
    print_line(&b[20 - n..]);
}

#[no_mangle]
pub extern "C" fn isc_print_u64_line(v: u64) {
    let mut b = [0u8; 21];
    let n = decimal(v, (&mut b[..20]).try_into().unwrap()).len();
    b[20] = b'\n';
    print_line(&b[20 - n..]);
}

/// The bytes of a whole line (its newline included) to the standard
/// output, as `isc_print_newline` ends one.
fn print_line(bytes: &[u8]) {
    let shared = ACTIONS.load(core::sync::atomic::Ordering::Relaxed) != 0;
    if shared {
        hold_out_shared();
    }
    out_held(bytes);
    if out_by_lines() {
        flush_stdout();
    }
    if shared {
        release_out();
    }
}

/// The pairs of decimal digits "00" to "99".
const DIGIT_PAIRS: [u8; 200] = {
    let mut t = [0u8; 200];
    let mut i = 0;
    while i < 100 {
        t[2 * i] = b'0' + (i / 10) as u8;
        t[2 * i + 1] = b'0' + (i % 10) as u8;
        i += 1;
    }
    t
};

/// The decimal digits of `v`, written at the end of `buf`, two at a time
/// from a table (D300: through `core::fmt`, 22 ns a number of `print`).
fn decimal(mut v: u64, buf: &mut [u8; 20]) -> &[u8] {
    let mut at = buf.len();
    while v >= 100 {
        let r = (v % 100) as usize;
        v /= 100;
        at -= 2;
        buf[at..at + 2].copy_from_slice(&DIGIT_PAIRS[2 * r..2 * r + 2]);
    }
    if v >= 10 {
        let r = v as usize;
        at -= 2;
        buf[at..at + 2].copy_from_slice(&DIGIT_PAIRS[2 * r..2 * r + 2]);
    } else {
        at -= 1;
        buf[at] = b'0' + v as u8;
    }
    &buf[at..]
}

/// `decimal` of `v` with its sign: the digits of its absolute value after a
/// `-` (the 19 digits of `i64::MIN` and its sign hold in 20 bytes).
fn signed_decimal(v: i64, buf: &mut [u8; 20]) -> &[u8] {
    let n = decimal(v.unsigned_abs(), buf).len();
    let mut at = buf.len() - n;
    if v < 0 {
        at -= 1;
        buf[at] = b'-';
    }
    &buf[at..]
}

#[no_mangle]
pub extern "C" fn isc_print_f64(v: f64) {
    let _ = sem::write_f64(&mut Stdout, v);
}

#[no_mangle]
pub extern "C" fn isc_print_f32(v: f32) {
    let _ = sem::write_f32(&mut Stdout, v);
}

#[no_mangle]
pub extern "C" fn isc_print_bool(b: i32) {
    out(if b != 0 { b"true" } else { b"false" })
}

#[no_mangle]
pub unsafe extern "C" fn isc_print_str(p: *const u8, len: i64) {
    if len > 0 {
        out(core::slice::from_raw_parts(p, len as usize));
    }
}

/// The end of `main`: what `printBytes` left in the buffer (D136).
#[no_mangle]
pub extern "C" fn isc_flush() {
    flush_stdout_held();
}

/// `printBytes(x)` (D136): the bytes, in the buffer of `print`.
#[no_mangle]
pub unsafe extern "C" fn isc_print_bytes(p: *const u8, n: i64) {
    if ACTIONS.load(core::sync::atomic::Ordering::Relaxed) == 0 {
        return out_held(bytes_of(p, n));
    }
    hold_out_shared();
    out_held(bytes_of(p, n));
    release_out();
}

#[no_mangle]
pub extern "C" fn isc_print_newline() {
    // one test of the tasks for the newline and the end of the line (D373)
    let shared = ACTIONS.load(core::sync::atomic::Ordering::Relaxed) != 0;
    if shared {
        hold_out_shared();
    }
    out_held(b"\n");
    if out_by_lines() {
        flush_stdout();
    }
    if shared {
        release_out();
    }
}

#[no_mangle]
pub extern "C" fn isc_clock_ns() -> u64 {
    let mut ts = Timespec { tv_sec: 0, tv_nsec: 0 };
    unsafe { sys::monotonic(&mut ts) };
    (ts.tv_sec as u64).wrapping_mul(1_000_000_000).wrapping_add(ts.tv_nsec as u64)
}

#[no_mangle]
pub unsafe extern "C" fn isc_parse_i64(p: *const u8, len: i64, line: i32, col: i32) -> i64 {
    let s = str_of(p, len);
    match sem::parse_i64_short(s) {
        Some(v) => v,
        None => parse_i64_rest(s, line, col),
    }
}

/// `isc_parse_i64` of the texts that are not a sign and 1 to 8 digits,
/// and its panic, out of it (D539: its frame on the stack only for them).
#[inline(never)]
fn parse_i64_rest(s: &str, line: i32, col: i32) -> i64 {
    match sem::parse_i64_rest(s) {
        Some(v) => v,
        None => die(line, col, &Panic::ParseI64(s)),
    }
}

/// The program arguments, without the program name, as `[k]string`: each
/// a copy, owned by the array (a string is freed as an array, D134).
#[no_mangle]
pub unsafe extern "C" fn isc_args(out: *mut IscArray, argc: i32, argv: *const *const u8) {
    // the environment after the arguments (D287), before any allocation
    // (which reads `ISC_ALLOC`)
    #[cfg(target_os = "linux")]
    ENVIRON.store(argv.add(argc.max(0) as usize + 1) as usize, Ordering::Relaxed);
    // under Windows, from the command line in UTF-16 (D541)
    #[cfg(windows)]
    {
        let _ = (argc, argv);
        let mut all = alloc::vec::Vec::new();
        windows::args(&mut |a| all.push(new_str(a)));
        let p = isc_alloc((all.len() * core::mem::size_of::<IscStr>()) as i64) as *mut IscStr;
        for (i, s) in all.iter().enumerate() {
            *p.add(i) = *s;
        }
        *out = IscArray { ptr: p as *mut u8, len: all.len() as i64 };
    }
    #[cfg(not(windows))]
    {
        let n = (argc.max(1) - 1) as usize;
        let p = isc_alloc((n * core::mem::size_of::<IscStr>()) as i64) as *mut IscStr;
        for i in 0..n {
            let s = *argv.add(i + 1);
            let len = strlen(s as *const c_char) as i64;
            *p.add(i) = new_str(core::slice::from_raw_parts(s, len as usize));
        }
        *out = IscArray { ptr: p as *mut u8, len: n as i64 };
    }
}

// ---------------------------------------------------------------------
// strings (D134): a string is `{ptr, len}` of UTF-8 bytes, owned as an
// array; each operation returns a new one

/// A copy of a string (or of an array of `u8`: the same layout).
#[no_mangle]
pub unsafe extern "C" fn isc_str_new(out: *mut IscStr, p: *const u8, n: i64) {
    *out = new_str(bytes_of(p, n));
}

/// The bytes of every empty string made by `new_str`: none owned, read
/// only, never freed (D337: the empty strings of the faults of the readers
/// of JSON were an allocation each).
#[repr(C, align(64))]
struct EmptyBytes([u8; 64]);
static EMPTY_BYTES: EmptyBytes = EmptyBytes([0; 64]);

fn empty_ptr() -> *mut u8 {
    EMPTY_BYTES.0.as_ptr() as *mut u8
}

/// The memory of an empty array of bytes: none owned (as an empty string).
fn empty_bytes() -> *mut u8 {
    empty_ptr()
}

/// The bytes the block of the string `p` can hold (D351): the allocator of
/// Isocron knows it; with glibc's (`ISC_ALLOC=glibc`) or under WASI, none
/// counted (a new block).
/// The usable bytes of the new block `q` of the allocator of Isocron
/// (`heap::usable`); 0 where they are not known (glibc's allocator, WASI).
fn usable_bytes(q: *mut u8) -> usize {
    #[cfg(not(target_os = "wasi"))]
    if HEAP_MODE.load(Ordering::Relaxed) == 2 && q as usize % 64 == 0 {
        // SAFETY: a block just taken from the allocator of Isocron
        return unsafe { heap::usable(q) };
    }
    let _ = q;
    0
}

fn room(p: *mut u8) -> usize {
    // only a block of the heap (aligned on 64 bytes) has room: not a literal
    // of the program (D361), a line of `lines`, the empty string, the null
    // string (the zero of a type, in the slots of the other variants of a
    // value of a sum type)
    if p.is_null() || p == empty_ptr() || p as usize % 64 != 0 {
        return 0;
    }
    // nor the standard input mapped from its file (D257), whose header only
    // holds a class no block has: read as a block, `heap::usable` took it
    // for a block freed twice (D529)
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    if mapped::owns(p) {
        return 0;
    }
    #[cfg(not(target_os = "wasi"))]
    if HEAP_MODE.load(Ordering::Relaxed) == 2 {
        // SAFETY: an owned string of the program, a block of `alloc`
        return unsafe { heap::usable(p) };
    }
    0
}

/// The string `p` of `n` bytes (given: its last use) followed by the parts
/// `parts`: written after its bytes in its block when it has room, else in
/// a new block of at least twice its length (so that the next texts added
/// to it find room), `p` freed (D351).
unsafe fn append_parts(p: *mut u8, n: usize, parts: &[IscStr]) -> IscStr {
    let more: usize = parts.iter().map(|s| s.len as usize).sum();
    let total = n + more;
    if more == 0 {
        return IscStr { ptr: p, len: n as i64 };
    }
    let r = room(p);
    let (q, keep, cap) = if r >= total {
        (p, true, r)
    } else {
        let size = total.max(2 * n);
        let q = isc_alloc(size as i64);
        copy_short(p, q, n);
        (q, false, usable_bytes(q))
    };
    // short parts by moves, not by calls of `memcpy` (D459: 9.5 % of
    // `json.quote` of escaped texts, parts of one or two bytes); a part of
    // at most 16 bytes by one move of 16 when the block of the result has
    // room for them past its place and the 16 bytes read stay in the page
    // of the part (D508: the moves of 1 to 3 bytes, each a branch and a
    // load after another, 31 % of `append_parts`); `cap` 0 (the allocator
    // of glibc, whose blocks are not known) never
    let mut at = n;
    for s in parts {
        let len = s.len as usize;
        if len <= 16 && at + 16 <= cap && (s.ptr as usize) & 4095 <= 4096 - 16 {
            // SAFETY: 16 bytes in the page of the part (mapped), written
            // inside the block of the result (`cap` its usable bytes)
            let v = core::ptr::read_unaligned(s.ptr as *const u128);
            core::ptr::write_unaligned(q.add(at) as *mut u128, v);
        } else {
            copy_short(s.ptr, q.add(at), len);
        }
        at += len;
    }
    if !keep {
        isc_free(p);
    }
    IscStr { ptr: q, len: total as i64 }
}

/// The `n` decimal digits of `v` written before `end`.
unsafe fn write_digits(mut v: u64, end: *mut u8) {
    let mut at = end;
    while v >= 100 {
        let r = (v % 100) as usize;
        v /= 100;
        at = at.sub(2);
        core::ptr::copy_nonoverlapping(DIGIT_PAIRS.as_ptr().add(2 * r), at, 2);
    }
    if v >= 10 {
        at = at.sub(2);
        core::ptr::copy_nonoverlapping(DIGIT_PAIRS.as_ptr().add(2 * v as usize), at, 2);
    } else {
        *at.sub(1) = b'0' + v as u8;
    }
}

/// The block of a template whose first part `p`, `n` bytes, is given (its
/// last use, D351), for `total` bytes (D363): `p` itself when its block has
/// room, else a block of at least twice its length holding its bytes, `p`
/// freed (as `isc_str_append`).
#[no_mangle]
pub unsafe extern "C" fn isc_str_reserve(p: *mut u8, n: i64, total: i64) -> *mut u8 {
    let (n, total) = (n as usize, total as usize);
    if room(p) >= total {
        return p;
    }
    let q = isc_alloc(total.max(2 * n) as i64);
    if n > 0 {
        core::ptr::copy_nonoverlapping(p, q, n);
    }
    isc_free(p);
    q
}

/// The text of an integer of a template written at `dst` (D363): its
/// magnitude `mag`, its sign `neg`, its `n` bytes (the sign counted), in
/// `width` bytes, the rest the fill `fs & 0xff`, before it when `fs` has
/// 0x100 (`padStart`), after it otherwise (`padEnd`).
#[no_mangle]
pub unsafe extern "C" fn isc_put_int(dst: *mut u8, mag: u64, neg: i64, n: i64, width: i64, fs: i64) {
    let (n, w) = (n as usize, width as usize);
    let gap = w - n;
    let (ti, fi) = if fs & 0x100 != 0 { (gap, 0) } else { (0, n) };
    if neg != 0 {
        *dst.add(ti) = b'-';
    }
    write_digits(mag, dst.add(ti + n));
    fill_short(dst.add(fi), (fs & 0xff) as u8, gap);
}

/// `a + b`, `a` given (D351).
#[no_mangle]
pub unsafe extern "C" fn isc_str_append(out: *mut IscStr, p: *mut u8, n: i64, q: *const u8, m: i64) {
    *out = append_parts(p, n as usize, &[IscStr { ptr: q, len: m }]);
}

/// `a + b + c ...` of `k` parts, the first given (D351).
#[no_mangle]
pub unsafe extern "C" fn isc_str_append_n(out: *mut IscStr, parts: *const IscStr, k: i64) {
    let ps = core::slice::from_raw_parts(parts, k as usize);
    *out = append_parts(ps[0].ptr as *mut u8, ps[0].len as usize, &ps[1..]);
}

/// A new string holding these bytes.
fn new_str(b: &[u8]) -> IscStr {
    if b.is_empty() {
        return IscStr { ptr: empty_ptr(), len: 0 };
    }
    let q = isc_alloc(b.len() as i64);
    unsafe { core::ptr::copy_nonoverlapping(b.as_ptr(), q, b.len()) };
    IscStr { ptr: q, len: b.len() as i64 }
}

/// The text of bytes of a string of Isocron, UTF-8 by the language (its
/// strings are made of whole characters, D134).
fn utf8_text(b: &[u8]) -> &str {
    debug_assert!(core::str::from_utf8(b).is_ok());
    // SAFETY: the bytes of a string of Isocron, UTF-8 (see above)
    unsafe { core::str::from_utf8_unchecked(b) }
}

/// A new array of `n` strings, empty (zeros: a null pointer and 0, an
/// empty string), from the allocator of the runtime, for the generated code
/// (which frees it); a new large block is zero already (D79).
fn new_str_array(n: usize) -> &'static mut [IscStr] {
    let p = isc_alloc_zeroed((n * core::mem::size_of::<IscStr>()) as i64) as *mut IscStr;
    // SAFETY: `n` strings of zeros (each a valid empty string), aligned on
    // ARRAY_ALIGN, given to the caller alone
    unsafe { core::slice::from_raw_parts_mut(p, n) }
}

unsafe fn bytes_of<'a>(p: *const u8, n: i64) -> &'a [u8] {
    if n <= 0 {
        &[]
    } else {
        core::slice::from_raw_parts(p, n as usize)
    }
}

/// `a + b + c`, a template literal (D342): the `n` texts at `parts` in one
/// block.
#[no_mangle]
pub unsafe extern "C" fn isc_str_concat_n(out: *mut IscStr, parts: *const IscStr, n: i64) {
    // SAFETY: the code generator gives `n` texts at `parts`
    let parts = core::slice::from_raw_parts(parts, n.max(0) as usize);
    let total: i64 = parts.iter().map(|s| s.len.max(0)).sum();
    if total == 0 {
        *out = new_str(b"");
        return;
    }
    let r = isc_alloc(total);
    let mut at = 0usize;
    for s in parts {
        let len = s.len.max(0) as usize;
        // SAFETY: each text holds `len` bytes, `r` the sum of them
        core::ptr::copy_nonoverlapping(s.ptr, r.add(at), len);
        at += len;
    }
    *out = IscStr { ptr: r, len: total };
}

#[no_mangle]
pub unsafe extern "C" fn isc_str_concat(out: *mut IscStr, p: *const u8, n: i64, q: *const u8, m: i64) {
    let r = isc_alloc(n.max(0) + m.max(0));
    core::ptr::copy_nonoverlapping(p, r, n.max(0) as usize);
    core::ptr::copy_nonoverlapping(q, r.add(n.max(0) as usize), m.max(0) as usize);
    *out = IscStr { ptr: r, len: n.max(0) + m.max(0) };
}

/// -1, 0 or 1: the order of the bytes (lexicographic).
#[no_mangle]
pub unsafe extern "C" fn isc_str_cmp(p: *const u8, n: i64, q: *const u8, m: i64) -> i32 {
    match bytes_of(p, n).cmp(bytes_of(q, m)) {
        core::cmp::Ordering::Less => -1,
        core::cmp::Ordering::Equal => 0,
        core::cmp::Ordering::Greater => 1,
    }
}

/// `stringOf(b)`: a copy of the bytes, a panic if they are not UTF-8.
#[no_mangle]
pub unsafe extern "C" fn isc_str_from_bytes(out: *mut IscStr, p: *const u8, n: i64, line: i32, col: i32) {
    let b = bytes_of(p, n);
    if let Err(at) = sem::utf8_check(b) {
        die(line, col, &Panic::NotUtf8(at as i64))
    }
    *out = new_str(b);
}

/// `stringOf(b)` of bytes at their last use (D399): checked as UTF-8 (a
/// panic otherwise, as `isc_str_from_bytes`), then the text takes their
/// block (a block of the heap, or the empty block): no copy.
#[no_mangle]
pub unsafe extern "C" fn isc_str_adopt_bytes(out: *mut IscStr, p: *const u8, n: i64, line: i32, col: i32) {
    if let Err(at) = sem::utf8_check(bytes_of(p, n)) {
        die(line, col, &Panic::NotUtf8(at as i64))
    }
    *out = IscStr { ptr: p as *mut u8, len: n };
}

/// A writer into a buffer of `N` bytes (the text of a scalar). A float is
/// written without exponent (D108): 5e-324 takes 326 bytes, so the buffer
/// of an `f64` holds 352 (D336: a buffer of 64 cut such texts).
struct Small<const N: usize> {
    buf: [u8; N],
    len: usize,
}

/// The longest text of an `f64` (`-0.` and 323 zeros then a digit; 309
/// digits for the largest) and of an `f32` (48 bytes), with room.
const F64_TEXT: usize = 352;
const F32_TEXT: usize = 64;

impl<const N: usize> Write for Small<N> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let b = s.as_bytes();
        if self.len + b.len() > self.buf.len() {
            return Err(core::fmt::Error);
        }
        self.buf[self.len..self.len + b.len()].copy_from_slice(b);
        self.len += b.len();
        Ok(())
    }
}

#[no_mangle]
pub unsafe extern "C" fn isc_to_string_i64(out: *mut IscStr, v: i64) {
    let mut b = [0u8; 20];
    *out = new_str(signed_decimal(v, &mut b));
}

/// `toString(v).padStart(len, c)` (`start`) or `.padEnd`, `c` one byte,
/// in one block of its size (D348): the text of `v` (signed or not), `c`
/// before or after it up to `len` bytes; the text alone when it has `len`
/// bytes or more, as `padStart` gives it.
#[no_mangle]
pub unsafe extern "C" fn isc_pad_int(out: *mut IscStr, v: i64, signed: i32, len: i64, c: i32, start: i32) {
    let mut b = [0u8; 20];
    let t = if signed != 0 { signed_decimal(v, &mut b) } else { decimal(v as u64, &mut b) };
    if len <= t.len() as i64 {
        *out = new_str(t);
        return;
    }
    let (total, gap) = (len as usize, len as usize - t.len());
    // by moves, as `isc_pad` (D369)
    let r = isc_alloc(total as i64);
    let (at, from) = if start != 0 { (gap, 0) } else { (0, t.len()) };
    copy_short(t.as_ptr(), r.add(at), t.len());
    fill_short(r.add(from), c as u8, gap);
    *out = IscStr { ptr: r, len: total as i64 };
}

#[no_mangle]
pub unsafe extern "C" fn isc_to_string_u64(out: *mut IscStr, v: u64) {
    let mut b = [0u8; 20];
    *out = new_str(decimal(v, &mut b));
}

#[no_mangle]
pub unsafe extern "C" fn isc_to_string_f64(out: *mut IscStr, v: f64) {
    let mut s = Small::<F64_TEXT> { buf: [0; F64_TEXT], len: 0 };
    let _ = sem::write_f64(&mut s, v);
    *out = new_str(&s.buf[..s.len]);
}

#[no_mangle]
pub unsafe extern "C" fn isc_to_string_f32(out: *mut IscStr, v: f32) {
    let mut s = Small::<F32_TEXT> { buf: [0; F32_TEXT], len: 0 };
    let _ = sem::write_f32(&mut s, v);
    *out = new_str(&s.buf[..s.len]);
}

#[no_mangle]
pub unsafe extern "C" fn isc_to_string_bool(out: *mut IscStr, b: i32) {
    *out = new_str(if b != 0 { b"true" } else { b"false" });
}

// ---------------------------------------------------------------------
// text (D135)

/// `readText(path)`: the file, checked to be UTF-8.
#[no_mangle]
pub unsafe extern "C" fn isc_read_text(out: *mut IscStr, path: *const u8, plen: i64, line: i32, col: i32) {
    let p = str_of(path, plen);
    let a = read_file(p, line, col);
    if let Err(at) = sem::utf8_check(bytes_of(a.ptr, a.len)) {
        die(line, col, &Panic::TextNotUtf8 { path: p, at: at as i64 })
    }
    *out = IscStr { ptr: a.ptr, len: a.len };
}

#[no_mangle]
pub unsafe extern "C" fn isc_write_text(path: *const u8, plen: i64, data: *const u8, n: i64, line: i32, col: i32) {
    isc_write_bytes(path, plen, data, n, line, col);
}

/// An array of new strings, one per item.
fn str_array<'a>(items: &mut dyn Iterator<Item = &'a str>, count: usize) -> IscArray {
    let q = new_str_array(count);
    for (slot, s) in q.iter_mut().zip(items) {
        *slot = new_str(s.as_bytes());
    }
    IscArray { ptr: q.as_mut_ptr() as *mut u8, len: count as i64 }
}

/// From this many bytes on, the lines of a text are made by parts in
/// parallel (D252).
const LINES_PARALLEL_MIN: usize = 1 << 20;

/// `lines(text)` (`sem::text_lines`: separated by `\n`, a `\r` before it
/// removed, no empty last line after a final `\n`). A large text by parts
/// in parallel (D252): each part counts its `\n` (a line starts at the
/// start of the text and after each `\n`: the part of a `\n` owns the line
/// after it), the counts give the index of the first line of each part,
/// then each part makes the strings of its lines (its own allocations). The
/// same array for any number of threads.
#[no_mangle]
pub unsafe extern "C" fn isc_lines(out: *mut IscArray, p: *const u8, n: i64) {
    // SAFETY: the `n` bytes of a string of the generated code, alive during
    // the call
    let b = unsafe { bytes_of(p, n) };
    let r = lines(b);
    // SAFETY: the place of the result, the generated code's
    unsafe { out.write(r) };
}

/// `lines` of a text (see `isc_lines`), in safe code (D290).
fn lines(b: &[u8]) -> IscArray {
    let threads = thread_count();
    if b.len() < LINES_PARALLEL_MIN || threads <= 1 {
        return block_strs(&Lines(b));
    }
    // the text without its final `\n`: its lines are `split('\n')`
    let body = &b[..b.len() - (b[b.len() - 1] == b'\n') as usize];
    let parts = (threads * 4).min(256);
    let range = move |k: usize| (body.len() * k / parts, body.len() * (k + 1) / parts);
    // the `\n` of each part, and the bytes its last line takes past its end
    // (to its `\n`), each in its own counter
    let unit: [usize; 257] = core::array::from_fn(|i| i);
    let mut counts = [(0usize, 0usize); 256];
    par::parts_mut(&par::Parts::new(&mut counts[..parts], &unit[..=parts]), par::Order::Interleaved, |k, c| {
        let (a, e) = range(k);
        let past = memchr::memchr(b'\n', &body[e..]).unwrap_or(body.len() - e);
        c[0] = (memchr::memchr_iter(b'\n', &body[a..e]).count(), past);
    });
    // the lines of part k from `first[k]` on: a line starts at the start of
    // the text (part 0's) and after each `\n` (the part of the `\n` has it)
    let mut first = [0usize; 257];
    // the place of part k in the block of the lines from `place[k]` on: a
    // line of `m` bytes takes `ARENA_STEP(m)`, at most `m + 23`, so that a
    // part takes at most its bytes, those of its last line past its end and
    // 23 for each line (D298)
    let mut place = [0usize; 257];
    // part 0: line 0, then the lines after its `\n`
    let mut total = 1 + counts[0].0;
    place[0] = ARENA_HEAD;
    place[1] = place[0] + arena_bound(range(0), counts[0], 1);
    for k in 1..parts {
        first[k] = total;
        total += counts[k].0;
        place[k + 1] = place[k] + arena_bound(range(k), counts[k], 0);
    }
    first[parts] = total;
    let q = new_str_array(total);
    let len = q.len();
    let ptr = q.as_mut_ptr();
    let block = isc_alloc(place[parts] as i64);
    // SAFETY: a block of `place[parts]` bytes of this call alone, not read
    // before it is written (as bytes that may not be initialized)
    let arena = unsafe { core::slice::from_raw_parts_mut(block as *mut core::mem::MaybeUninit<u8>, place[parts]) };
    let base = block as usize;
    let (head, lines_area) = arena.split_at_mut(ARENA_HEAD);
    let bounds: [usize; 257] = core::array::from_fn(|k| place[k.min(parts)] - ARENA_HEAD);
    par::parts_mut2(&par::Parts::new(q, &first[..=parts]), &par::Parts::new(lines_area, &bounds[..=parts]), par::Order::Interleaved, |k, out, area| {
        let (a, e) = range(k);
        let mut idx = 0;
        let mut at = 0;
        // the first line of the part: after its first `\n` (part 0: the
        // start of the text); then each line after the `\n` that ends the
        // one before, while that `\n` is in the part: one search a line
        let mut s = if k == 0 {
            0
        } else {
            match memchr::memchr(b'\n', &body[a..e]) {
                Some(o) => a + o + 1,
                None => return,
            }
        };
        loop {
            let end = memchr::memchr(b'\n', &body[s..]).map(|o| s + o).unwrap_or(body.len());
            let line = &body[s..end];
            let line = if line.last() == Some(&b'\r') { &line[..line.len() - 1] } else { line };
            // the block of the lines, then the bytes (`arena_free`)
            area[at..at + 8].write_copy_of_slice(&(base as u64).to_le_bytes());
            area[at + 8..at + 8 + line.len()].write_copy_of_slice(line);
            let p = area[at + 8..].as_ptr() as *const u8;
            out[idx] = IscStr { ptr: p, len: line.len() as i64 };
            at += arena_step(line.len());
            idx += 1;
            if end >= e {
                break;
            }
            s = end + 1;
        }
    });
    // every line alive: the block freed with the last of them; its size
    // (`isc_free_strs`)
    head[..8].write_copy_of_slice(&(total as u64).to_le_bytes());
    head[8..16].write_copy_of_slice(&(place[parts] as u64).to_le_bytes());
    IscArray { ptr: ptr as *mut u8, len: len as i64 }
}

/// The bytes of the block of the lines of `lines` before the lines: the
/// count of the lines alive, the size of the block (one cache line).
const ARENA_HEAD: usize = 64;

/// The bytes a line of `m` bytes takes in the block of the lines: the
/// address of the block, then its bytes, to the next multiple of 16 (so
/// that each line starts 8 bytes past a multiple of 16, where no block of
/// the heap, of glibc or of the system starts: `free_cold`).
fn arena_step(m: usize) -> usize {
    (8 + m).next_multiple_of(16)
}

/// The bytes the lines starting in a part of the text (`range`, its count
/// of `\n` and the bytes of its last line past its end, `extra` more lines:
/// the first line of the text) take at most (`arena_step`), a multiple of
/// 16.
fn arena_bound(range: (usize, usize), count: (usize, usize), extra: usize) -> usize {
    (range.1 - range.0 + count.1 + 23 * (count.0 + extra)).next_multiple_of(16)
}

/// A line of the block of the lines of `lines` freed (D298): the block
/// freed with the last of them.
///
/// # Safety
/// `p` the bytes of a line made by `lines`, not freed before.
unsafe fn arena_free(p: *mut u8) {
    // SAFETY: the 8 bytes before a line hold the address of its block (see
    // `lines`)
    unsafe { arena_free_many((p.sub(8) as *const u64).read_unaligned() as usize, 1) }
}

/// `k` lines of the block of lines at `block` freed: the block with the
/// last of them.
///
/// # Safety
/// `k` lines of that block, not freed before.
unsafe fn arena_free_many(block: usize, k: u64) {
    // SAFETY: the first 8 bytes of the block count its lines alive
    unsafe {
        let alive = &*(block as *const core::sync::atomic::AtomicU64);
        if alive.fetch_sub(k, Ordering::AcqRel) == k {
            isc_free(block as *mut u8);
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn isc_split(out: *mut IscArray, p: *const u8, n: i64, q: *const u8, m: i64, line: i32, col: i32) {
    let s = core::str::from_utf8_unchecked(bytes_of(p, n));
    let sep = core::str::from_utf8_unchecked(bytes_of(q, m));
    if sep.is_empty() {
        die(line, col, &Panic::SplitEmpty)
    }
    // the separators found by `find_bytes` (`str::split` searched with the
    // general search, prepared at each call), the parts in one block
    // (D305)
    let (b, t) = (s.as_bytes(), sep.as_bytes());
    *out = if t.len() == 1 { block_strs(&ByteParts(b, t[0])) } else { block_strs(&SepParts(b, t)) };
}

/// Each `i` where `b[i] == c`, in order (D305): 32 bytes compared at a
/// time into the bits of a mask (written so that LLVM makes it one vector
/// comparison and `vpmovmskb`), then its bits one by one. No call for each
/// (`memchr` for each part of a `split` cost more than the part: 120.67 ns
/// a line of 6 parts of 5 bytes against 32.02 in C).
#[inline(always)]
fn each_byte(b: &[u8], c: u8, mut f: impl FnMut(usize)) {
    let n = b.len();
    let mut base = 0;
    // `f` called from one place, so that it is compiled into the loop
    while base < n {
        let mut m = if base + 32 <= n {
            byte_mask(b[base..base + 32].try_into().expect("32 bytes"), c)
        } else if n >= 32 {
            // the last bytes: the last 32 bytes of the text read again, those
            // already seen left out (copied into 32 bytes on the stack, then
            // read as one vector, they waited for their stores to be done:
            // 37 % of a `split` of a line of 35 bytes)
            byte_mask(b[n - 32..].try_into().expect("32 bytes"), c) >> (32 - (n - base))
        } else {
            // a text of fewer than 32 bytes, one byte at a time
            b.iter().enumerate().fold(0u32, |m, (j, &x)| m | (((x == c) as u32) << j))
        };
        while m != 0 {
            f(base + m.trailing_zeros() as usize);
            m &= m - 1;
        }
        base += 32;
    }
}

/// The bit j set where `ch[j] == c`: one vector comparison and
/// `vpmovmskb` (LLVM recognizes this loop).
#[inline(always)]
fn byte_mask(ch: &[u8; 32], c: u8) -> u32 {
    let mut m = 0u32;
    for (j, &x) in ch.iter().enumerate() {
        m |= ((x == c) as u32) << j;
    }
    m
}

/// The parts of a text, in order, each given to `part` as its place in the
/// text (start and end), for `block_strs` (D305): a generic function, so
/// that each kind of parts is compiled with its writer (through `dyn`, two
/// calls for each part).
trait Parts {
    fn text(&self) -> &[u8];
    fn each(&self, part: impl FnMut(usize, usize));
}

/// The parts of a text between its bytes `c`, from the left.
struct ByteParts<'a>(&'a [u8], u8);

impl Parts for ByteParts<'_> {
    fn text(&self) -> &[u8] {
        self.0
    }
    #[inline(always)]
    fn each(&self, mut part: impl FnMut(usize, usize)) {
        let mut start = 0;
        each_byte(self.0, self.1, |i| {
            part(start, i);
            start = i + 1;
        });
        part(start, self.0.len());
    }
}

/// The lines of a text, as `sem::text_lines`: separated by `\n`, a `\r`
/// before it removed, none after a final `\n` (D305: `str::split` by a
/// character, 29 ns a line of 60 bytes).
struct Lines<'a>(&'a [u8]);

impl Parts for Lines<'_> {
    fn text(&self) -> &[u8] {
        self.0
    }
    #[inline(always)]
    fn each(&self, mut part: impl FnMut(usize, usize)) {
        let b = self.0;
        if b.is_empty() {
            return;
        }
        let body = &b[..b.len() - (b[b.len() - 1] == b'\n') as usize];
        ByteParts(body, b'\n').each(|s, e| part(s, if e > s && b[e - 1] == b'\r' { e - 1 } else { e }));
    }
}

/// The parts of a text between the occurrences of a text of more than one
/// byte, from the left, as `str::split`: found by `find_bytes`.
struct SepParts<'a>(&'a [u8], &'a [u8]);

impl Parts for SepParts<'_> {
    fn text(&self) -> &[u8] {
        self.0
    }
    #[inline(always)]
    fn each(&self, mut part: impl FnMut(usize, usize)) {
        let (b, sep) = (self.0, self.1);
        let mut start = 0;
        while let Some(i) = sem::find_bytes(&b[start..], sep) {
            part(start, start + i);
            start += i + sep.len();
        }
        part(start, b.len());
    }
}

/// The parts `parts` as strings of one block, as the lines of a large text
/// (D298): one allocation for all of them, each freed with the last (D305:
/// a block of the heap for each, 47.21 ns a line of 60 bytes against 8.51
/// in C). The parts are found twice: to count them and their bytes, then
/// to write them.
fn block_strs(parts: &impl Parts) -> IscArray {
    let (mut count, mut place) = (0, 0);
    parts.each(|s, e| {
        count += 1;
        place += arena_step(e - s);
    });
    let text = parts.text();
    let q = new_str_array(count);
    let ptr = q.as_mut_ptr();
    if count > 0 {
        let size = ARENA_HEAD + place;
        let block = isc_alloc(size as i64);
        // SAFETY: a block of `size` bytes of this call alone, not read before
        // it is written (as bytes that may not be initialized)
        let arena = unsafe { core::slice::from_raw_parts_mut(block as *mut core::mem::MaybeUninit<u8>, size) };
        let base = (block as u64).to_le_bytes();
        let (head, area) = arena.split_at_mut(ARENA_HEAD);
        let (mut at, mut k) = (0, 0);
        parts.each(|s, e| {
            let n = e - s;
            // the address of the block, then the bytes (`arena_free`); a
            // short part by 16 bytes, the bytes past it in the place of the
            // next part, written after it (a call of `memcpy` for each
            // part: 8.69 % of a `split` of parts of 5 bytes)
            area[at..at + 8].write_copy_of_slice(&base);
            if n <= 16 && s + 16 <= text.len() && at + 24 <= area.len() {
                area[at + 8..at + 24].write_copy_of_slice(&text[s..s + 16]);
            } else {
                area[at + 8..at + 8 + n].write_copy_of_slice(&text[s..e]);
            }
            q[k] = IscStr { ptr: area[at + 8..].as_ptr() as *const u8, len: n as i64 };
            at += arena_step(n);
            k += 1;
        });
        head[..8].write_copy_of_slice(&(count as u64).to_le_bytes());
        head[8..16].write_copy_of_slice(&(size as u64).to_le_bytes());
    }
    IscArray { ptr: ptr as *mut u8, len: count as i64 }
}

/// The `n` strings at `p` freed (an array of strings freed by the generated
/// code, then its block): those of one block of strings one after the other
/// (`block_strs`, `lines`) counted down at once (D305: an atomic operation
/// for each).
///
/// # Safety
/// `p` the `n` strings of an array the caller owns, freed only here.
#[no_mangle]
pub unsafe extern "C" fn isc_free_strs(p: *const IscStr, n: i64) {
    if n <= 0 {
        return;
    }
    // SAFETY: the `n` strings of the caller's array
    let strs = unsafe { core::slice::from_raw_parts(p, n as usize) };
    let mut i = 0;
    while i < strs.len() {
        let s = strs[i].ptr as *mut u8;
        if s as usize % 16 != 8 {
            isc_free(s);
            i += 1;
            continue;
        }
        // SAFETY: a string of a block of strings: its block's address in the
        // 8 bytes before it, the size of the block in the second 8 bytes of
        // the block (see `block_strs`)
        let (block, size) = unsafe {
            let b = (s.sub(8) as *const u64).read_unaligned() as usize;
            (b, (b as *const u64).add(1).read())
        };
        // the next strings of the same block: inside it, their bytes not
        // read (a line of cache for each)
        let inside = |t: *const u8| t as usize % 16 == 8 && t as usize > block && t as usize - block < size as usize;
        let mut k = 1;
        while i + k < strs.len() && inside(strs[i + k].ptr) {
            k += 1;
        }
        // SAFETY: `k` strings of that block, not freed before
        unsafe { arena_free_many(block, k as u64) };
        i += k;
    }
}

#[no_mangle]
pub unsafe extern "C" fn isc_trim(out: *mut IscStr, p: *const u8, n: i64) {
    let s = core::str::from_utf8_unchecked(bytes_of(p, n));
    *out = new_str(s.trim().as_bytes());
}

/// A new string of the text that `f` writes (two passes: counted, then
/// written into a block of that size).
/// The text `f` writes, as a string of the program: written once into a
/// `String` that grows (a block of the runtime's heap), whose buffer is the
/// string (`adopt`). It wrote twice, to count the bytes then to fill an
/// exact block: every function through it did its work twice (D190).
unsafe fn written(f: &dyn Fn(&mut dyn Write) -> fmt::Result) -> IscStr {
    let mut out = alloc::string::String::new();
    let _ = f(&mut out);
    adopt(out)
}

unsafe fn text<'a>(p: *const u8, n: i64) -> &'a str {
    core::str::from_utf8_unchecked(bytes_of(p, n))
}

// the functions of strings of TypeScript (D155): the code of
// `isc-runtime-core`, as the interpreter

#[no_mangle]
pub unsafe extern "C" fn isc_index_of(p: *const u8, n: i64, q: *const u8, m: i64) -> i64 {
    sem::index_of(text(p, n), text(q, m))
}

#[no_mangle]
pub unsafe extern "C" fn isc_last_index_of(p: *const u8, n: i64, q: *const u8, m: i64) -> i64 {
    sem::last_index_of(text(p, n), text(q, m))
}

/// 0 `includes`, 1 `startsWith`, 2 `endsWith`.
#[no_mangle]
pub unsafe extern "C" fn isc_str_test(p: *const u8, n: i64, q: *const u8, m: i64, which: i32) -> i32 {
    let (s, t) = (bytes_of(p, n), bytes_of(q, m));
    (match which {
        0 => sem::find_bytes(s, t).is_some(),
        1 => s.starts_with(t),
        _ => s.ends_with(t),
    }) as i32
}

/// `s.replace(from, to)` (`all` 0) and `s.replaceAll(from, to)` of a text,
/// with the `$` patterns of `to` (D157).
#[no_mangle]
pub unsafe extern "C" fn isc_replace_all(out: *mut IscStr, p: *const u8, n: i64, q: *const u8, m: i64, r: *const u8, k: i64, all: i32) {
    let (s, from, to) = (text(p, n), text(q, m), text(r, k));
    // a text without `$` patterns for a non-empty text (D302): its places
    // found by `find_bytes`, the string of their final length written once
    // (through the engine of patterns, 848 ns for a line of 60 bytes)
    if !from.is_empty() && !to.contains('$') {
        let (b, f) = (s.as_bytes(), from.as_bytes());
        let mut at = alloc::vec::Vec::new();
        let mut i = 0;
        while let Some(j) = sem::find_bytes(&b[i..], f) {
            at.push(i + j);
            i += j + f.len();
            if all == 0 {
                break;
            }
        }
        let len = b.len() - at.len() * f.len() + at.len() * to.len();
        *out = made(len, |d| {
            let (mut from_s, mut to_d) = (0, 0);
            for &j in &at {
                d[to_d..to_d + j - from_s].write_copy_of_slice(&b[from_s..j]);
                to_d += j - from_s;
                d[to_d..to_d + to.len()].write_copy_of_slice(to.as_bytes());
                to_d += to.len();
                from_s = j + f.len();
            }
            d[to_d..].write_copy_of_slice(&b[from_s..]);
        });
        return;
    }
    *out = written(&|w| sem::replace_text(w, s, from, to, all != 0));
}

/// A new string of `len` bytes, every one of them written by `fill` (D302).
fn made(len: usize, fill: impl FnOnce(&mut [core::mem::MaybeUninit<u8>])) -> IscStr {
    let p = isc_alloc(len as i64);
    // SAFETY: a block of `len` bytes of this call alone, written by `fill`
    // before it is read (as bytes that may not be initialized)
    let d = unsafe { core::slice::from_raw_parts_mut(p as *mut core::mem::MaybeUninit<u8>, len) };
    fill(d);
    IscStr { ptr: p, len: len as i64 }
}

// regular expressions (D157): the code of isc-runtime-core

#[no_mangle]
pub unsafe extern "C" fn isc_rx_new(out: *mut IscStr, p: *const u8, n: i64, q: *const u8, m: i64, line: i32, col: i32) {
    let (src, flags) = (text(p, n), text(q, m));
    if let Err(e) = sem::rx::compile(src, flags) {
        die(line, col, &e)
    }
    *out = written(&|w| {
        let _ = sem::rx::value(src, flags, w);
        Ok(())
    });
}

/// Parts of a text searched by different tasks (D189), of at least this
/// many bytes.
const RX_PART: usize = 1 << 18;

/// Parts per thread of a search by masks and of a replacement written in
/// parallel, taken in turn (D313): their cost follows their matches, not
/// their bytes (regex-redux: the first quarter of its text has 210332 of
/// the 3.5 million matches of `a[NSt]|BY`, its thread waited for the
/// others), so that the threads that finish early take the remaining parts.
const PARTS_PER_THREAD: usize = 8;

/// The matches of the global search of `re` in `s` (D189): with several
/// threads and a long text, its parts searched by the tasks of the pool,
/// each up to its first match in the next part, then merged into the
/// matches of one search (`merge_bounds`); inside a task, one search.
/// The engine of a global search: the shapes searched by masks (D224), or
/// the automata of regex-automata.
enum RxEngine {
    Simple(sem::rx::simple::Simple),
    Meta(sem::rx::Regex),
}

impl RxEngine {
    fn bounds_from(&self, s: &str, from: usize, until: usize) -> alloc::vec::Vec<(usize, usize)> {
        match self {
            RxEngine::Simple(sm) => sm.bounds_from(s, from, until),
            RxEngine::Meta(re) => sem::rx::bounds_from(re, s, from, until),
        }
    }

    fn merge(&self, s: &str, starts: &[usize], parts: &[alloc::vec::Vec<(usize, usize)>]) -> alloc::vec::Vec<sem::rx::Run> {
        match self {
            RxEngine::Simple(sm) => sem::rx::merge_runs_with(s, starts, parts, |at| sm.find_at(s, at)),
            RxEngine::Meta(re) => sem::rx::merge_runs(re, s, starts, parts),
        }
    }
}

/// The matches of a global search (D281): the matches of its parts, their
/// runs (`merge_runs_with`) as slices of them, those found alone at a cut in
/// `extra`; never copied into one vector unless a caller needs it.
struct Found {
    parts: alloc::vec::Vec<alloc::vec::Vec<(usize, usize)>>,
    /// the bytes of the matches of each part, summed by its search while
    /// they are in its caches (D314); `usize::MAX` when not summed
    removed: alloc::vec::Vec<usize>,
    extra: alloc::vec::Vec<(usize, usize)>,
    /// each run: its part (`usize::MAX` for `extra`), its first and its end
    runs: alloc::vec::Vec<(usize, usize, usize)>,
}

impl Found {
    fn one(v: alloc::vec::Vec<(usize, usize)>) -> Found {
        let n = v.len();
        Found { parts: alloc::vec![v], removed: alloc::vec![usize::MAX], extra: alloc::vec::Vec::new(), runs: alloc::vec![(0, 0, n)] }
    }

    fn of(parts: alloc::vec::Vec<alloc::vec::Vec<(usize, usize)>>, removed: alloc::vec::Vec<usize>, runs: &[sem::rx::Run]) -> Found {
        let mut f = Found { parts, removed, extra: alloc::vec::Vec::new(), runs: alloc::vec::Vec::with_capacity(runs.len()) };
        for r in runs {
            match *r {
                sem::rx::Run::Part(k, lo, hi) => f.runs.push((k, lo, hi)),
                sem::rx::Run::One(a, b) => {
                    f.extra.push((a, b));
                    let n = f.extra.len();
                    f.runs.push((usize::MAX, n - 1, n));
                }
            }
        }
        f
    }

    fn run(&self, r: usize) -> &[(usize, usize)] {
        let (k, lo, hi) = self.runs[r];
        if k == usize::MAX {
            &self.extra[lo..hi]
        } else {
            &self.parts[k][lo..hi]
        }
    }

    fn len(&self) -> usize {
        self.runs.iter().map(|&(_, lo, hi)| hi - lo).sum()
    }

    fn iter(&self) -> impl Iterator<Item = (usize, usize)> + '_ {
        (0..self.runs.len()).flat_map(move |r| self.run(r).iter().copied())
    }

    fn to_vec(&self) -> alloc::vec::Vec<(usize, usize)> {
        let mut out = alloc::vec::Vec::with_capacity(self.len());
        out.extend(self.iter());
        out
    }

    /// The bytes of the matches of the run `r`: the sum of its part less
    /// its matches outside the run (none or one at each end, where the
    /// searches of two parts meet), else counted.
    fn removed_of(&self, r: usize) -> usize {
        let bytes = |m: &[(usize, usize)]| m.iter().map(|&(a, b)| b - a).sum::<usize>();
        let (k, lo, hi) = self.runs[r];
        if k == usize::MAX {
            return bytes(&self.extra[lo..hi]);
        }
        let p = &self.parts[k];
        match self.removed[k] {
            total if total != usize::MAX && lo + (p.len() - hi) < hi - lo => total - bytes(&p[..lo]) - bytes(&p[hi..]),
            _ => bytes(&p[lo..hi]),
        }
    }
}

/// The engine for a global search of `v` in `s`: a simple shape by masks
/// (D224), else the automaton with or without its prefilter (D219).
fn rx_engine(v: &str, s: &str) -> RxEngine {
    match sem::rx::simple::of(v) {
        Some(sm) => RxEngine::Simple(sm),
        None => RxEngine::Meta(rx_for(v, s)),
    }
}

unsafe fn rx_bounds(re: &RxEngine, s: &str) -> Found {
    let threads = thread_count();
    // inside a task (the pool busy) the parts of the automaton would run
    // one after the other, each with a new cache of the lazy DFA to fill
    // again: one search. The masks of a simple shape have no cache: their
    // parts are shared with the threads that have finished their tasks
    // (D201, D277: the last of the 9 counts of regex-redux, alone on one of
    // 4 threads, now on the 4)
    let busy = BUSY.load(Ordering::Relaxed);
    if threads <= 1 || s.len() < 2 * RX_PART || (busy && !matches!(re, RxEngine::Simple(_))) {
        return Found::one(re.bounds_from(s, 0, usize::MAX));
    }
    // the automaton: one part per thread, each one fills one cache; the
    // masks, which have none: several parts per thread, taken in turn
    let per = if matches!(re, RxEngine::Simple(_)) { threads * PARTS_PER_THREAD } else { threads };
    let parts = (s.len() / RX_PART).min(per);
    let mut starts: alloc::vec::Vec<usize> = alloc::vec::Vec::with_capacity(parts);
    for k in 0..parts {
        let mut c = s.len() * k / parts;
        while !s.is_char_boundary(c) {
            c += 1;
        }
        if starts.last() != Some(&c) {
            starts.push(c);
        }
    }
    let mut found: alloc::vec::Vec<alloc::vec::Vec<(usize, usize)>> = (0..starts.len()).map(|_| alloc::vec::Vec::new()).collect();
    #[repr(C)]
    struct Ctx {
        re: *const RxEngine,
        s: *const u8,
        n: usize,
        starts: *const usize,
        k: usize,
        found: *mut alloc::vec::Vec<(usize, usize)>,
        removed: *mut usize,
    }
    extern "C" fn part(ctx: *mut c_void, i: i64) {
        unsafe {
            let c = &*(ctx as *const Ctx);
            let s = core::str::from_utf8_unchecked(core::slice::from_raw_parts(c.s, c.n));
            let starts = core::slice::from_raw_parts(c.starts, c.k);
            let i = i as usize;
            let until = if i + 1 < c.k { starts[i + 1] } else { usize::MAX };
            let m = (*c.re).bounds_from(s, starts[i], until);
            // the bytes of its matches, read where it has just written them
            // (the caches of this thread), for the sizes of a replacement
            // (D314)
            *c.removed.add(i) = m.iter().map(|&(a, b)| b - a).sum();
            *c.found.add(i) = m;
        }
    }
    let mut removed = alloc::vec![0usize; starts.len()];
    let mut ctx = Ctx { re, s: s.as_ptr(), n: s.len(), starts: starts.as_ptr(), k: starts.len(), found: found.as_mut_ptr(), removed: removed.as_mut_ptr() };
    parallel_for(starts.len() as i64, part, &mut ctx as *mut Ctx as *mut c_void, core::mem::size_of::<Ctx>() as i64, true, true);
    let runs = re.merge(s, &starts, &found);
    Found::of(found, removed, &runs)
}

/// `n` bytes from `src` to `dst` (D280): a few bytes, as most texts between
/// the matches of a replacement and its replacements are (regex-redux: 3.5
/// million matches 14 bytes apart), by two moves that overlap, without a
/// call; a call of the copy of the system beyond 64 bytes (D368; from 32
/// before).
#[inline(always)]
unsafe fn copy_short(src: *const u8, dst: *mut u8, n: usize) {
    use core::ptr::{read_unaligned as rd, write_unaligned as wr};
    if n > 64 {
        core::ptr::copy_nonoverlapping(src, dst, n);
    } else if n >= 32 {
        // two moves of 32 bytes that overlap (D368: a line of 60 bytes of a
        // concatenation without the call of `memcpy`)
        let (a, b) = (rd(src as *const [u8; 32]), rd(src.add(n - 32) as *const [u8; 32]));
        wr(dst as *mut [u8; 32], a);
        wr(dst.add(n - 32) as *mut [u8; 32], b);
    } else if n >= 16 {
        let (a, b) = (rd(src as *const u128), rd(src.add(n - 16) as *const u128));
        wr(dst as *mut u128, a);
        wr(dst.add(n - 16) as *mut u128, b);
    } else if n >= 8 {
        let (a, b) = (rd(src as *const u64), rd(src.add(n - 8) as *const u64));
        wr(dst as *mut u64, a);
        wr(dst.add(n - 8) as *mut u64, b);
    } else if n >= 4 {
        let (a, b) = (rd(src as *const u32), rd(src.add(n - 4) as *const u32));
        wr(dst as *mut u32, a);
        wr(dst.add(n - 4) as *mut u32, b);
    } else if n > 0 {
        *dst = *src;
        *dst.add(n / 2) = *src.add(n / 2);
        *dst.add(n - 1) = *src.add(n - 1);
    }
}

/// `n` bytes `c` from `dst`, as `copy_short` copies: by moves that overlap,
/// a call of the fill of the system from 32 bytes on (D359).
#[inline(always)]
unsafe fn fill_short(dst: *mut u8, c: u8, n: usize) {
    use core::ptr::write_unaligned as wr;
    if n >= 32 {
        core::ptr::write_bytes(dst, c, n);
    } else if n >= 8 {
        let v = u64::from_ne_bytes([c; 8]);
        if n >= 16 {
            wr(dst as *mut u64, v);
            wr(dst.add(8) as *mut u64, v);
            wr(dst.add(n - 16) as *mut u64, v);
        }
        wr(dst.add(n - 8) as *mut u64, v);
        wr(dst as *mut u64, v);
    } else if n >= 4 {
        let v = u32::from_ne_bytes([c; 4]);
        wr(dst as *mut u32, v);
        wr(dst.add(n - 4) as *mut u32, v);
    } else if n > 0 {
        *dst = c;
        *dst.add(n / 2) = c;
        *dst.add(n - 1) = c;
    }
}

/// `n` bytes from `src` to `dst` (D308), where `src_room` bytes can be read
/// from `src` and `dst_room` written from `dst`: up to 32 bytes by one move
/// of 32 when both have that room, the bytes past `n` written for nothing
/// (the next writes cover them), so that the length of the text between two
/// matches decides no branch (`copy_short` chose among four moves by it:
/// about one branch mispredicted every two matches of a replacement of
/// regex-redux); else `copy_short`.
#[inline(always)]
unsafe fn copy_room(src: *const u8, dst: *mut u8, n: usize, src_room: usize, dst_room: usize) {
    if n <= 32 && src_room >= 32 && dst_room >= 32 {
        core::ptr::write_unaligned(dst as *mut [u8; 32], core::ptr::read_unaligned(src as *const [u8; 32]));
    } else {
        copy_short(src, dst, n);
    }
}

/// From this many bytes of text on, a replacement writes its result in
/// parallel (D279).
const REPLACE_PARALLEL_MIN: usize = 1 << 20;

/// `s` with each match of `bounds` replaced by `to`, a text without `$`
/// (each replacement the same bytes), written by every thread (D236, D279,
/// D314): the result in pieces, each a run of matches of the search (the
/// matches of one of its parts, D281) with the text before it, or a copy of
/// a text without match longer than a part; the size of each piece known
/// from the search (`Found::removed_of`), the pieces written at their
/// places, each thread its range of them first (D313). The bytes of
/// `replace_bounds`. None with one thread, a short text or a busy pool.
unsafe fn replace_parallel(s: &str, to: &str, found: &Found) -> Option<IscStr> {
    let threads = thread_count();
    if threads <= 1 || s.len() < REPLACE_PARALLEL_MIN || BUSY.load(Ordering::Relaxed) {
        return None;
    }
    // a piece of the result: the text from `start` to `end`, with the
    // matches of the run `run` (`usize::MAX`: none), at `at` in the result
    #[derive(Clone, Copy)]
    struct Piece {
        start: usize,
        end: usize,
        run: usize,
        at: usize,
    }
    // a text without match longer than a part (the first sequence of
    // regex-redux, without any match of `a[NSt]`): copies of a part each,
    // shared like the runs
    let part = (s.len() / (threads * PARTS_PER_THREAD)).max(1);
    let tn = to.len();
    let mut pieces: alloc::vec::Vec<Piece> = alloc::vec::Vec::with_capacity(found.runs.len() + s.len() / part + 2);
    let (mut last, mut at) = (0usize, 0usize);
    for r in 0..found.runs.len() {
        let m = found.run(r);
        let (Some(&(first, _)), Some(&(_, end))) = (m.first(), m.last()) else { continue };
        while first - last > part {
            pieces.push(Piece { start: last, end: last + part, run: usize::MAX, at });
            (last, at) = (last + part, at + part);
        }
        pieces.push(Piece { start: last, end, run: r, at });
        at += (end - last) - found.removed_of(r) + m.len() * tn;
        last = end;
    }
    while last < s.len() {
        let end = (last + part).min(s.len());
        pieces.push(Piece { start: last, end, run: usize::MAX, at });
        (last, at) = (end, at + (end - last));
    }
    let total = at;
    let buf = isc_alloc(total.max(1) as i64);
    #[repr(C)]
    struct Ctx {
        s: *const u8,
        n: usize,
        to: *const u8,
        tn: usize,
        found: *const Found,
        pieces: *const Piece,
        count: usize,
        total: usize,
        out: *mut u8,
    }
    extern "C" fn write(ctx: *mut c_void, k: i64) {
        unsafe {
            let c = &*(ctx as *const Ctx);
            let p = *c.pieces.add(k as usize);
            let mut o = c.out.add(p.at);
            if p.run == usize::MAX {
                core::ptr::copy_nonoverlapping(c.s.add(p.start), o, p.end - p.start);
                return;
            }
            // the end of this piece of the result: past it, the piece of
            // another thread
            let k = k as usize;
            let end_o = c.out.add(if k + 1 < c.count { (*c.pieces.add(k + 1)).at } else { c.total }) as usize;
            let mut last = p.start;
            for &(a, b) in (*c.found).run(p.run) {
                copy_room(c.s.add(last), o, a - last, c.n - last, end_o - o as usize);
                o = o.add(a - last);
                copy_short(c.to, o, c.tn);
                o = o.add(c.tn);
                last = b;
            }
        }
    }
    let mut ctx = Ctx {
        s: s.as_ptr(),
        n: s.len(),
        to: to.as_ptr(),
        tn,
        found: found as *const Found,
        pieces: pieces.as_ptr(),
        count: pieces.len(),
        total,
        out: buf,
    };
    parallel_for(pieces.len() as i64, write, &mut ctx as *mut Ctx as *mut c_void, core::mem::size_of::<Ctx>() as i64, true, true);
    Some(IscStr { ptr: buf, len: total as i64 })
}

/// A string and the code of an error (D322): the tuple `[string, i64]` of a
/// primitive of the system, laid out as the compiled code reads it.
#[repr(C)]
pub struct IscStrCode {
    pub s: IscStr,
    pub code: i64,
}

/// `sysGetenv(name)` (D322): the value of the variable `name` of the
/// environment and 1, or the empty string and 0; a name that is empty or
/// holds `=` or a zero byte names none; the bytes of a value that are not
/// UTF-8 replaced, as the interpreter does.
#[no_mangle]
pub unsafe extern "C" fn isc_sys_getenv(out: *mut IscStrCode, p: *const u8, n: i64) {
    let name = bytes_of(p, n);
    // the name compared where it is (D370: copied with a zero byte into a
    // buffer of 256 bytes cleared at each call, then `strlen`, before)
    let v = if name.is_empty() || name.iter().any(|&c| c == 0 || c == b'=') { None } else { env_value(name) };
    let Some(value) = v else {
        *out = IscStrCode { s: adopt(alloc::string::String::new()), code: 0 };
        return;
    };
    // valid UTF-8 (the common case, ASCII tested first by words) copied
    // once; otherwise each invalid sequence replaced by U+FFFD
    let s = if value.is_ascii() || core::str::from_utf8(value).is_ok() {
        new_str(value)
    } else {
        adopt(alloc::string::String::from_utf8_lossy(value).into_owned())
    };
    *out = IscStrCode { s, code: 1 };
}

/// The value of the variable `name` (not empty, without `=` nor a zero
/// byte) of the environment noted by `isc_args` (D287), or none: each
/// entry rejected at its first byte (D370), the rest compared up to the
/// first difference (an entry shorter than the name differs at its zero
/// byte).
#[cfg(target_os = "linux")]
unsafe fn env_value(name: &[u8]) -> Option<&'static [u8]> {
    let mut e = ENVIRON.load(Ordering::Relaxed) as *const *const u8;
    if e.is_null() {
        return None;
    }
    let (first, n) = (name[0], name.len());
    // SAFETY: the array of the environment, ending with a null, each entry
    // a string ending with a zero byte: read up to its first difference
    // with `name`, which holds no zero byte
    unsafe {
        while !(*e).is_null() {
            let v = *e;
            if *v == first && (1..n).all(|i| *v.add(i) == name[i]) && *v.add(n) == b'=' {
                let at = v.add(n + 1);
                return Some(core::slice::from_raw_parts(at, strlen(at as *const c_char)));
            }
            e = e.add(1);
        }
    }
    None
}

/// The value of the variable `name` of the environment of the system, or
/// none (Windows, WASI: their `getenv`).
#[cfg(not(target_os = "linux"))]
unsafe fn env_value(name: &[u8]) -> Option<&'static [u8]> {
    let mut z = alloc::vec::Vec::with_capacity(name.len() + 1);
    z.extend_from_slice(name);
    z.push(0);
    let v = getenv(z.as_ptr() as *const c_char);
    if v.is_null() {
        return None;
    }
    // SAFETY: `getenv` gives a string ended by a zero byte
    Some(unsafe { core::slice::from_raw_parts(v as *const u8, strlen(v)) })
}

/// A `String` of the runtime as a string of the program: its buffer is a
/// block of the heap of the runtime (the global allocator), no copy.
unsafe fn adopt(s: alloc::string::String) -> IscStr {
    let mut s = core::mem::ManuallyDrop::new(s);
    if s.capacity() == 0 {
        return IscStr { ptr: isc_alloc(0), len: 0 };
    }
    IscStr { ptr: s.as_mut_ptr(), len: s.len() as i64 }
}

/// 0 `re.test(s)` (1 or 0), 1 `s.search(re)`, 2 `countMatches(s, re)`.
#[no_mangle]
pub unsafe extern "C" fn isc_rx_int(v: *const u8, n: i64, s: *const u8, m: i64, which: i32) -> i64 {
    let (re, t) = (text(v, n), text(s, m));
    match which {
        0 => sem::rx::test(re, t) as i64,
        1 => sem::rx::search(re, t),
        _ => rx_bounds(&rx_engine(re, t), t).len() as i64,
    }
}

/// From this size on, a search chooses its engine on a sample (D219).
const RX_CHOOSE_MIN: usize = 4 << 20;
const RX_SAMPLE: usize = 64 << 10;

/// The regular expression `v` for a global search of `s` (D219): with the
/// prefilter of its literals (a fast search of them, the automaton only
/// where they are) or without it (the automaton over every byte), whichever
/// searches the first RX_SAMPLE bytes of `s` faster, each once to fill its
/// cache and bring the sample in, then timed. A prefilter whose literals are
/// everywhere (the patterns of DNA in DNA) costs more than it saves; one
/// whose literals are rare (`<[^>]*>`) saves the scan. The two find the
/// same matches: the choice changes the time only.
fn rx_for(v: &str, s: &str) -> sem::rx::Regex {
    let re = sem::rx::of(v);
    #[cfg(target_arch = "x86_64")]
    if s.len() >= RX_CHOOSE_MIN {
        let plain = sem::rx::of_plain(v);
        let mut end = RX_SAMPLE;
        while !s.is_char_boundary(end) {
            end += 1;
        }
        let sample = &s[..end];
        let (mut c1, mut c2) = (re.create_cache(), plain.create_cache());
        let _ = sem::rx::count_with(&re, &mut c1, sample);
        let _ = sem::rx::count_with(&plain, &mut c2, sample);
        let t0 = unsafe { core::arch::x86_64::_rdtsc() };
        let _ = sem::rx::count_with(&re, &mut c1, sample);
        let t1 = unsafe { core::arch::x86_64::_rdtsc() };
        let _ = sem::rx::count_with(&plain, &mut c2, sample);
        let t2 = unsafe { core::arch::x86_64::_rdtsc() };
        if (t2 - t1) * 10 < (t1 - t0) * 9 {
            return plain;
        }
    }
    re
}

#[no_mangle]
pub unsafe extern "C" fn isc_rx_matches(out: *mut IscArray, s: *const u8, n: i64, v: *const u8, m: i64) {
    let (t, re) = (text(s, n), text(v, m));
    let found = rx_bounds(&rx_engine(re, t), t);
    let count = found.len();
    *out = str_array(&mut found.iter().map(|(a, b)| &t[a..b]), count);
}

/// `to` written over each match of `found` in the bytes at `p` (D315),
/// each run by a task when there are many.
unsafe fn replace_over(p: *mut u8, to: &str, found: &Found) {
    #[repr(C)]
    struct Ctx {
        p: *mut u8,
        to: *const u8,
        tn: usize,
        found: *const Found,
    }
    extern "C" fn over(ctx: *mut c_void, r: i64) {
        unsafe {
            let c = &*(ctx as *const Ctx);
            for &(a, _) in (*c.found).run(r as usize) {
                copy_short(c.to, c.p.add(a), c.tn);
            }
        }
    }
    let mut ctx = Ctx { p, to: to.as_ptr(), tn: to.len(), found: found as *const Found };
    let runs = found.runs.len() as i64;
    if found.len() >= 1 << 16 {
        parallel_for(runs, over, &mut ctx as *mut Ctx as *mut c_void, core::mem::size_of::<Ctx>() as i64, true, true);
    } else {
        for r in 0..runs {
            over(&mut ctx as *mut Ctx as *mut c_void, r);
        }
    }
}

/// The bytes of a string given to the runtime can be written where they
/// are: not those of the standard input mapped from its file, read only
/// (D257).
fn writable(p: *const u8) -> bool {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    if mapped::owns(p) {
        return false;
    }
    // nor a literal of the program (D361), in its read-only data
    !is_literal(p)
}

/// A literal of the program kept as a string without a copy (D361): its
/// bytes in the read-only data of the program, 4 bytes past a multiple of
/// 16, where no block starts (the blocks of the heap aligned on 64 bytes,
/// the lines of `lines` 8 bytes past a multiple of 16, those of `malloc` and
/// `HeapAlloc` on 16): never freed, never written, without room.
#[inline(always)]
fn is_literal(p: *const u8) -> bool {
    p as usize % 16 == 4
}

/// `given`: the text `s` is moved into the replacement at its last use
/// (D315), which frees it, or writes the result in it when each match is
/// replaced by as many bytes.
#[no_mangle]
pub unsafe extern "C" fn isc_rx_replace(out: *mut IscStr, s: *const u8, n: i64, v: *const u8, m: i64, q: *const u8, k: i64, all: i32, given: i32, line: i32, col: i32) {
    let (t, re, to) = (text(s, n), text(v, m), text(q, k));
    let (flags, _) = sem::rx::parts(re);
    if all != 0 && !flags.contains('g') {
        die(line, col, &Panic::ReplaceAllNotGlobal)
    }
    // every match, `to` without groups: the bounds of the global search,
    // in parallel, and the result built once (D189; `written` would search
    // twice, to count the bytes and to write them)
    if (all != 0 || flags.contains('g')) && sem::rx::without_groups(to) {
        let found = rx_bounds(&rx_engine(re, t), t);
        if !to.contains('$') {
            // the text given, each match of the bytes of `to`: the result is
            // the text, `to` over each match, nothing copied (D315)
            if given != 0 && writable(s) && found.iter().all(|(a, b)| b - a == to.len()) {
                replace_over(s as *mut u8, to, &found);
                *out = IscStr { ptr: s as *mut u8, len: n };
                return;
            }
            if let Some(r) = replace_parallel(t, to, &found) {
                *out = r;
                give_back(s, given);
                return;
            }
        }
        *out = adopt(sem::rx::replace_bounds(t, to, &found.to_vec()));
        give_back(s, given);
        return;
    }
    *out = written(&|w| {
        let _ = sem::rx::replace(w, re, t, to, all != 0);
        Ok(())
    });
    give_back(s, given);
}

/// A string given to the runtime (D315) and not kept: freed.
unsafe fn give_back(s: *const u8, given: i32) {
    if given != 0 {
        isc_free(s as *mut u8);
    }
}

#[no_mangle]
pub unsafe extern "C" fn isc_to_case(out: *mut IscStr, p: *const u8, n: i64, upper: i32) {
    let s = text(p, n);
    // an ASCII text (D302): each byte mapped, the string of the same length
    // (a character at a time through `fmt`, 187 ns for 60 bytes)
    if s.is_ascii() {
        let b = s.as_bytes();
        *out = made(b.len(), |d| {
            for (o, &c) in d.iter_mut().zip(b) {
                o.write(if upper != 0 { c.to_ascii_uppercase() } else { c.to_ascii_lowercase() });
            }
        });
        return;
    }
    *out = written(&|w| sem::to_case(w, s, upper != 0));
}

/// `isc_to_case` into the buffer `buf` of `cap` bytes (on the stack of the
/// caller: the text only read in its scope, D501) when the text is ASCII
/// (each byte mapped, the same length) and fits; else a block, as
/// `isc_to_case`.
#[no_mangle]
pub unsafe extern "C" fn isc_to_case_into(out: *mut IscStr, buf: *mut u8, cap: i64, p: *const u8, n: i64, upper: i32) {
    let b = bytes_of(p, n);
    if n <= cap && b.is_ascii() {
        let d = core::slice::from_raw_parts_mut(buf, n as usize);
        for (o, &c) in d.iter_mut().zip(b) {
            *o = if upper != 0 { c.to_ascii_uppercase() } else { c.to_ascii_lowercase() };
        }
        *out = IscStr { ptr: if n == 0 { empty_ptr() } else { buf }, len: n };
        return;
    }
    isc_to_case(out, p, n, upper)
}

#[no_mangle]
pub unsafe extern "C" fn isc_pad(out: *mut IscStr, p: *const u8, n: i64, len: i64, q: *const u8, m: i64, start: i32) {
    let (s, fill) = (text(p, n), text(q, m));
    // `s` itself when it has `len` bytes or more or the fill is empty
    // (`sem::pad`): one copy of its exact size (D348: through the writer of
    // `fmt`, its buffer grown, a third of `pad(2026, 4)`)
    if len <= s.len() as i64 || fill.is_empty() {
        *out = new_str(s.as_bytes());
        return;
    }
    // a fill of one byte (D302): the string of `len` bytes, the fill then
    // the text or the text then the fill (a character at a time through
    // `fmt`, 67 ns for 20 bytes)
    if fill.len() == 1 {
        let (b, c, total) = (s.as_bytes(), fill.as_bytes()[0], len as usize);
        let gap = total - b.len();
        // the text and the fill by moves (`copy_short`, `fill_short`), not
        // by the calls of `memcpy` and `memset` (D369)
        let r = isc_alloc(total as i64);
        let (t, f) = if start != 0 { (gap, 0) } else { (0, b.len()) };
        copy_short(b.as_ptr(), r.add(t), b.len());
        fill_short(r.add(f), c, gap);
        *out = IscStr { ptr: r, len: total as i64 };
        return;
    }
    *out = written(&|w| sem::pad(w, s, len, fill, start != 0));
}

/// `isc_pad` into the buffer `buf` of `cap` bytes on the stack of the
/// caller when the text fits there and its fill is of one byte (or it is
/// the text itself): no block of the heap, the text never freed (D458, as
/// the concatenations of D404); otherwise `isc_pad`.
#[no_mangle]
pub unsafe extern "C" fn isc_pad_into(out: *mut IscStr, buf: *mut u8, cap: i64, p: *const u8, n: i64, len: i64, q: *const u8, m: i64, start: i32) {
    let whole = len <= n || m == 0;
    let total = if whole { n } else { len };
    if total <= cap && (whole || m == 1) {
        if whole {
            copy_short(p, buf, n as usize);
        } else {
            let gap = (len - n) as usize;
            let (t, f) = if start != 0 { (gap, 0) } else { (0, n as usize) };
            copy_short(p, buf.add(t), n as usize);
            fill_short(buf.add(f), *q, gap);
        }
        *out = IscStr { ptr: if total == 0 { empty_ptr() } else { buf }, len: total };
        return;
    }
    isc_pad(out, p, n, len, q, m, start)
}

/// `isc_pad_into` with a fill of one byte known to the compiler, `cs` its
/// byte, plus 256 when the fill goes before the text, into a buffer of
/// 128 bytes (`STACK_STR` of the code generator): six arguments, all in
/// registers (D458).
#[no_mangle]
pub unsafe extern "C" fn isc_pad1_into(out: *mut IscStr, buf: *mut u8, p: *const u8, n: i64, len: i64, cs: i32) {
    let c = cs as u8;
    if len <= n {
        isc_pad_into(out, buf, 128, p, n, len, &c, 1, cs >> 8);
        return;
    }
    if len <= 128 {
        let gap = (len - n) as usize;
        let (t, f) = if cs >> 8 != 0 { (gap, 0) } else { (0, n as usize) };
        copy_short(p, buf.add(t), n as usize);
        fill_short(buf.add(f), c, gap);
        *out = IscStr { ptr: buf, len };
        return;
    }
    isc_pad(out, p, n, len, &c, 1, cs >> 8)
}

#[no_mangle]
pub unsafe extern "C" fn isc_repeat(out: *mut IscStr, p: *const u8, n: i64, count: i64, line: i32, col: i32) {
    let s = text(p, n);
    if count < 0 {
        die(line, col, &Panic::RepeatNegative(count))
    }
    *out = written(&|w| {
        let _ = sem::repeat(w, s, count);
        Ok(())
    });
}

/// 0 `trimStart`, 1 `trimEnd`.
#[no_mangle]
pub unsafe extern "C" fn isc_trim_side(out: *mut IscStr, p: *const u8, n: i64, end: i32) {
    let s = text(p, n);
    *out = new_str(if end != 0 { s.trim_end() } else { s.trim_start() }.as_bytes());
}

#[no_mangle]
pub unsafe extern "C" fn isc_str_slice(out: *mut IscStr, p: *const u8, n: i64, a: i64, b: i64, line: i32, col: i32) {
    match sem::slice_str(text(p, n), a, b) {
        Ok(x) => *out = new_str(x.as_bytes()),
        Err(e) => die(line, col, &e),
    }
}

#[no_mangle]
pub unsafe extern "C" fn isc_to_fixed(out: *mut IscStr, x: f64, d: i64, line: i32, col: i32) {
    if !(0..=100).contains(&d) {
        die(line, col, &Panic::FixedDigits(d))
    }
    *out = written(&|w| {
        let _ = sem::write_fixed(w, x, d);
        Ok(())
    });
}

// `bigint` (D156): the code of isc-runtime-core, as the interpreter

#[no_mangle]
pub unsafe extern "C" fn isc_print_big(p: *const u8, n: i64) {
    let _ = sem::big256::write_decimal(&mut Stdout, bytes_of(p, n));
}

/// `+ - * / % **` and the negation (0 to 6).
#[no_mangle]
pub unsafe extern "C" fn isc_big_op(out: *mut IscStr, p: *const u8, n: i64, q: *const u8, m: i64, op: i32, line: i32, col: i32) {
    match sem::big256::op(op as u8, bytes_of(p, n), bytes_of(q, m)) {
        // its buffer is the value (a block of the runtime's heap, D196)
        Ok(v) => *out = adopt(alloc::string::String::from_utf8_unchecked(v)),
        Err(e) => die(line, col, &e),
    }
}

/// `(a * b + c) / d` (D389): the quotient from the leading words when they
/// decide it, the product and the sum not made.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn isc_big_muladd_div(out: *mut IscStr, a: *const u8, an: i64, b: *const u8, bn: i64, c: *const u8, cn: i64, d: *const u8, dn: i64, line: i32, col: i32) {
    match sem::big256::muladd_div(bytes_of(a, an), bytes_of(b, bn), bytes_of(c, cn), bytes_of(d, dn)) {
        Ok(v) => *out = adopt(alloc::string::String::from_utf8_unchecked(v)),
        Err(e) => die(line, col, &e),
    }
}

/// `z + x * y`, or `z - x * y` (`sub`), D406: in one pass when one of `x`
/// and `y` is of one word and the processor has ADX, the product not made.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn isc_big_muladd(out: *mut IscStr, z: *const u8, zn: i64, x: *const u8, xn: i64, y: *const u8, yn: i64, sub: i32, line: i32, col: i32) {
    match sem::big256::muladd(bytes_of(z, zn), bytes_of(x, xn), bytes_of(y, yn), sub != 0) {
        Ok(v) => *out = adopt(alloc::string::String::from_utf8_unchecked(v)),
        Err(e) => die(line, col, &e),
    }
}

#[no_mangle]
pub unsafe extern "C" fn isc_big_cmp(p: *const u8, n: i64, q: *const u8, m: i64) -> i32 {
    sem::big256::cmp(bytes_of(p, n), bytes_of(q, m)) as i32
}

#[no_mangle]
pub unsafe extern "C" fn isc_big_from_i64(out: *mut IscStr, x: i64) {
    *out = new_str(&sem::big256::from_i64(x));
}

#[no_mangle]
pub unsafe extern "C" fn isc_big_from_u64(out: *mut IscStr, x: u64) {
    *out = new_str(&sem::big256::from_u64(x));
}

#[no_mangle]
pub unsafe extern "C" fn isc_big_parse(out: *mut IscStr, p: *const u8, n: i64, line: i32, col: i32) {
    let s = text(p, n);
    match sem::big256::parse(s) {
        Some(v) => *out = new_str(&v),
        None => die(line, col, &Panic::ParseBigInt(s)),
    }
}

#[no_mangle]
pub unsafe extern "C" fn isc_big_to_i64(p: *const u8, n: i64) -> i64 {
    sem::big256::to_i64(bytes_of(p, n))
}

#[no_mangle]
pub unsafe extern "C" fn isc_big_to_f64(p: *const u8, n: i64) -> f64 {
    sem::big256::to_f64(bytes_of(p, n))
}

#[no_mangle]
pub unsafe extern "C" fn isc_big_to_string(out: *mut IscStr, p: *const u8, n: i64) {
    let b = bytes_of(p, n);
    *out = written(&|w| sem::big256::write_decimal(w, b));
}

/// ECMAScript's `Number(s)` (0), `parseFloat(s)` (1), `parseInt(s, radix)`
/// (2) (D156).
#[no_mangle]
pub unsafe extern "C" fn isc_js_number(p: *const u8, n: i64, which: i32, radix: i64) -> f64 {
    let s = text(p, n);
    match which {
        0 => sem::js_number(s),
        1 => sem::js_parse_float(s),
        _ => sem::js_parse_int(s, radix),
    }
}

/// `argsort(x)` (D155): the indices of the elements in their order, the
/// ties by index (stable). `kind`: 0..=3 `i8`..`i64`, 4..=7 `u8`..`u64`,
/// 8 `f32`, 9 `f64`, 10 `bool` (a byte), 11 `string`.
#[no_mangle]
pub unsafe extern "C" fn isc_argsort(out: *mut IscArray, p: *const u8, n: i64, kind: i32) {
    use core::cmp::Ordering;
    let n = n.max(0) as usize;
    let q = isc_alloc((n * 8) as i64) as *mut i64;
    let idx = core::slice::from_raw_parts_mut(q, n);
    for (i, x) in idx.iter_mut().enumerate() {
        *x = i as i64;
    }
    unsafe fn at<T: Copy>(p: *const u8, i: i64) -> T {
        *(p as *const T).add(i as usize)
    }
    let cmp = |i: i64, j: i64| -> Ordering {
        match kind {
            0 => at::<i8>(p, i).cmp(&at::<i8>(p, j)),
            1 => at::<i16>(p, i).cmp(&at::<i16>(p, j)),
            2 => at::<i32>(p, i).cmp(&at::<i32>(p, j)),
            3 => at::<i64>(p, i).cmp(&at::<i64>(p, j)),
            4 | 10 => at::<u8>(p, i).cmp(&at::<u8>(p, j)),
            5 => at::<u16>(p, i).cmp(&at::<u16>(p, j)),
            6 => at::<u32>(p, i).cmp(&at::<u32>(p, j)),
            7 => at::<u64>(p, i).cmp(&at::<u64>(p, j)),
            8 => sem::order_f64(at::<f32>(p, i) as f64, at::<f32>(p, j) as f64),
            9 => sem::order_f64(at::<f64>(p, i), at::<f64>(p, j)),
            12 => {
                let (a, b) = (at::<IscStr>(p, i), at::<IscStr>(p, j));
                sem::big256::cmp(bytes_of(a.ptr, a.len), bytes_of(b.ptr, b.len))
            }
            _ => {
                let (a, b) = (at::<IscStr>(p, i), at::<IscStr>(p, j));
                text_order(bytes_of(a.ptr, a.len), bytes_of(b.ptr, b.len))
            }
        }
    };
    // numbers and booleans from 64 elements: their keys as unsigned
    // integers in their order, sorted by their bytes (D488: a sort of 4096
    // integers 3.1 times its floor, by a length 17 times)
    if n >= ARGSORT_RADIX_MIN && kind <= 10 {
        let mut keys: alloc::vec::Vec<u64> = (0..n as i64)
            .map(|i| match kind {
                0 => (at::<i8>(p, i) as i64 as u64) ^ (1 << 63),
                1 => (at::<i16>(p, i) as i64 as u64) ^ (1 << 63),
                2 => (at::<i32>(p, i) as i64 as u64) ^ (1 << 63),
                3 => (at::<i64>(p, i) as u64) ^ (1 << 63),
                4 | 10 => at::<u8>(p, i) as u64,
                5 => at::<u16>(p, i) as u64,
                6 => at::<u32>(p, i) as u64,
                7 => at::<u64>(p, i),
                8 => sem::order_key_f64(at::<f32>(p, i) as f64),
                _ => sem::order_key_f64(at::<f64>(p, i)),
            })
            .collect();
        sem::radix_order(&mut keys, idx);
    } else {
        idx.sort_unstable_by(|&i, &j| cmp(i, j).then(i.cmp(&j)));
    }
    *out = IscArray { ptr: q as *mut u8, len: n as i64 };
}

/// The key at position `i` of the keys `p` of a map, of the kind of
/// `group_kind` (integers widened to 64 bits as the interpreter holds them,
/// sign extended when signed; booleans 0 and 1), as an integer (D492).
unsafe fn map_key_word(p: *const u8, i: usize, kind: i32) -> u64 {
    match kind {
        0 => *(p as *const i8).add(i) as u64,
        1 => *(p as *const i16).add(i) as u64,
        2 => *(p as *const i32).add(i) as u64,
        3 | 7 => *(p as *const u64).add(i),
        4 | 10 => *p.add(i) as u64,
        5 => *(p as *const u16).add(i) as u64,
        _ => *(p as *const u32).add(i) as u64,
    }
}

/// `$hashIndex(keys)` (D492): the index of hashes of the `n` distinct keys
/// `p` of a map (`sem::hash_index`), a new array.
#[no_mangle]
pub unsafe extern "C" fn isc_hash_index(out: *mut IscArray, p: *const u8, n: i64, kind: i32) {
    let n = n.max(0) as usize;
    let slots = if kind == 11 {
        sem::hash_index(n, |i| {
            let s = *(p as *const IscStr).add(i);
            sem::hash_bytes(bytes_of(s.ptr, s.len))
        })
    } else {
        sem::hash_index(n, |i| sem::hash_word(map_key_word(p, i, kind)))
    };
    let q = isc_alloc((slots.len() * 8) as i64) as *mut i64;
    core::ptr::copy_nonoverlapping(slots.as_ptr(), q, slots.len());
    *out = IscArray { ptr: q as *mut u8, len: slots.len() as i64 };
}

/// `$hashFind(keys, index, k)` (D492): the position of the key `k` (`kw`,
/// an integer widened as `map_key_word`, or the bytes `kp`, `kn`) among
/// the `n` keys `p` of a map of index `q` (`m` slots), -1 if absent.
#[no_mangle]
pub unsafe extern "C" fn isc_hash_find(p: *const u8, n: i64, q: *const i64, m: i64, kind: i32, kw: i64, kp: *const u8, kn: i64) -> i64 {
    let _ = n;
    let slots = core::slice::from_raw_parts(q, m.max(0) as usize);
    if kind == 11 {
        let key = bytes_of(kp, kn);
        sem::hash_find(slots, sem::hash_bytes(key), |j| {
            let s = *(p as *const IscStr).add(j);
            text_same(bytes_of(s.ptr, s.len), key)
        })
    } else {
        let kw = kw as u64;
        sem::hash_find(slots, sem::hash_word(kw), |j| map_key_word(p, j, kind) == kw)
    }
}

/// The order of the bytes of two texts (D532): by their first 8 bytes read
/// as one word when both have them and they differ, or their first 4,
/// without a call of `memcmp`; else by all their bytes.
#[inline(always)]
fn text_order(x: &[u8], y: &[u8]) -> core::cmp::Ordering {
    if x.len() >= 8 && y.len() >= 8 {
        let u = u64::from_be_bytes(x[..8].try_into().unwrap());
        let v = u64::from_be_bytes(y[..8].try_into().unwrap());
        if u != v {
            return u.cmp(&v);
        }
    } else if x.len() >= 4 && y.len() >= 4 {
        // shorter texts by their first 4 bytes (the texts of numbers of
        // `sorts`, 1 to 8 bytes)
        let u = u32::from_be_bytes(x[..4].try_into().unwrap());
        let v = u32::from_be_bytes(y[..4].try_into().unwrap());
        if u != v {
            return u.cmp(&v);
        }
    }
    x.cmp(y)
}

/// The same bytes (D532): two texts of 4 to 16 bytes by two words each,
/// overlapping, without a call of `memcmp`.
#[inline(always)]
fn text_same(x: &[u8], y: &[u8]) -> bool {
    let n = x.len();
    if n != y.len() {
        return false;
    }
    if (8..=16).contains(&n) {
        let w = |b: &[u8], k: usize| u64::from_le_bytes(b[k..k + 8].try_into().unwrap());
        return w(x, 0) == w(y, 0) && w(x, n - 8) == w(y, n - 8);
    }
    if (4..8).contains(&n) {
        let w = |b: &[u8], k: usize| u32::from_le_bytes(b[k..k + 4].try_into().unwrap());
        return w(x, 0) == w(y, 0) && w(x, n - 4) == w(y, n - 4);
    }
    x == y
}

/// From this many numbers on, `argsort` sorts their keys by their bytes
/// (D488); below, by comparisons.
const ARGSORT_RADIX_MIN: usize = 64;

/// From this many keys on, the groups of integer keys are made in parallel.
const GROUP_PARALLEL_MIN: usize = 1 << 20;
/// The slot of the key this many places ahead is prefetched (D243).
const GROUP_AHEAD: usize = 16;

/// The columns of `group` for integer keys (`key(i)`, its bits), by
/// `WordGroups` (D243): with several threads and many keys, one table per
/// part of consecutive indices, then the tables merged in their order.
unsafe fn group_words(n: usize, key: impl Fn(usize) -> u64 + Sync) -> alloc::vec::Vec<i64> {
    let threads = thread_count();
    if n >= sem::WordGroups::MAX_INDEX {
        return sem::group(n, |i| sem::hash_word(key(i)), |i, j| key(i) == key(j));
    }
    if threads <= 1 || n < GROUP_PARALLEL_MIN {
        let mut g = sem::WordGroups::new(false);
        for i in 0..n {
            if i + GROUP_AHEAD < n {
                g.prefetch(key(i + GROUP_AHEAD));
            }
            g.add(key(i), i);
        }
        return g.columns();
    }
    // inside a task too: its parts shared with the threads that are done
    // (D201)
    let parts = threads.min(64);
    let mut tables: alloc::vec::Vec<sem::WordGroups> = (0..parts).map(|_| sem::WordGroups::new(false)).collect();
    struct Ctx<'a> {
        key: &'a (dyn Fn(usize) -> u64 + Sync),
        n: usize,
        parts: usize,
        tables: *mut sem::WordGroups,
    }
    extern "C" fn part(ctx: *mut c_void, k: i64) {
        unsafe {
            let c = &*(ctx as *const Ctx);
            let k = k as usize;
            let (lo, hi) = (c.n * k / c.parts, c.n * (k + 1) / c.parts);
            let t = &mut *c.tables.add(k);
            for i in lo..hi {
                if i + GROUP_AHEAD < hi {
                    t.prefetch((c.key)(i + GROUP_AHEAD));
                }
                t.add((c.key)(i), i);
            }
        }
    }
    let mut ctx = Ctx { key: &key, n, parts, tables: tables.as_mut_ptr() };
    isc_parallel_for(parts as i64, part, &mut ctx as *mut Ctx as *mut c_void, -1);
    let mut it = tables.into_iter();
    let mut all = it.next().unwrap();
    for t in it {
        all.merge(&t);
    }
    all.columns()
}

/// The number of parts of `$group` of `n` integer keys made as they are
/// grouped (D244): one per thread from GROUP_PARALLEL_MIN keys on, each part
/// of at least GROUP_PART_MIN keys; the columns do not depend on it.
#[no_mangle]
pub extern "C" fn isc_group_parts(n: i64) -> i64 {
    const GROUP_PART_MIN: i64 = 1 << 18;
    if (n as usize) < GROUP_PARALLEL_MIN {
        return 1;
    }
    (thread_count() as i64).min(64).min(n / GROUP_PART_MIN).max(1)
}

/// The tables of the `parts` parts of a `$group` of `n` keys (D244): the
/// small keys counted by their value from GROUP_DIRECT_MIN keys on; for
/// `$groupCounts` (`counts`, D317) tables of counts only.
#[no_mangle]
pub extern "C" fn isc_groups_new(n: i64, parts: i64, counts: i32) -> *mut c_void {
    const GROUP_DIRECT_MIN: i64 = 1 << 16;
    let direct = n >= GROUP_DIRECT_MIN;
    let k = parts.max(1) as usize;
    let st = if counts != 0 {
        // counts by value for the keys below the number of keys (D492)
        GroupsState { tables: alloc::vec::Vec::new(), counts: (0..k).map(|_| sem::CountTable::new(true, n.max(0) as usize)).collect() }
    } else {
        // several tables at once: their large ones scattered (D253)
        GroupsState { tables: (0..k).map(|_| sem::WordGroups::new_for(direct, n.max(0) as usize, parts > 1, k)).collect(), counts: alloc::vec::Vec::new() }
    };
    alloc::boxed::Box::into_raw(alloc::boxed::Box::new(st)) as *mut c_void
}

/// The tables of the parts of a `$group`, or of a `$groupCounts`.
struct GroupsState {
    tables: alloc::vec::Vec<sem::WordGroups>,
    counts: alloc::vec::Vec<sem::CountTable>,
}

/// The `len` keys at `p` (of the integer kind `kind`, as for `isc_group`),
/// of the indices `start ..`, added to the table of the part `part`: its
/// blocks come in the order of their indices.
#[no_mangle]
pub unsafe extern "C" fn isc_groups_add(state: *mut c_void, part: i64, p: *const u8, len: i64, start: i64, kind: i32) {
    // (each part its own table: through the pointer)
    let st = state as *mut GroupsState;
    let counted = !(*st).counts.is_empty();
    let len = len.max(0) as usize;
    let start = start as usize;
    let add = |keys: &[u64], at: usize| {
        if counted {
            (*(*st).counts.as_mut_ptr().add(part as usize)).add_block(keys, at)
        } else {
            (*(*st).tables.as_mut_ptr().add(part as usize)).add_block(keys, at)
        }
    };
    // keys of 64 bits added where they are; narrower ones as their 64 bits
    // (sign extended), by chunks
    if matches!(kind, 3 | 7) {
        add(core::slice::from_raw_parts(p as *const u64, len), start);
        return;
    }
    let mut wide = [0u64; 256];
    let mut done = 0;
    while done < len {
        let m = (len - done).min(wide.len());
        macro_rules! widen {
            ($t:ty) => {{
                let q = (p as *const $t).add(done);
                for i in 0..m {
                    wide[i] = *q.add(i) as u64;
                }
            }};
        }
        match kind {
            0 => widen!(i8),
            1 => widen!(i16),
            2 => widen!(i32),
            4 | 10 => widen!(u8),
            5 => widen!(u16),
            _ => widen!(u32),
        }
        add(&wide[..m], start + done);
        done += m;
    }
}

/// The columns of `isc_group` from the tables of `state`: merged in the
/// order of their parts, or, when some scattered their keys (D253), every
/// group in the order of its first index; `state` freed.
#[no_mangle]
pub unsafe extern "C" fn isc_groups_end(out: *mut IscArray, state: *mut c_void) {
    let st = *alloc::boxed::Box::from_raw(state as *mut GroupsState);
    let GroupsState { mut tables, counts } = st;
    let g = if !counts.is_empty() {
        let mut it = counts.into_iter();
        let mut all = it.next().unwrap();
        for t in it {
            all.merge(&t);
        }
        all.columns()
    } else if tables.iter().any(|t| t.scattered()) {
        scattered_columns(&mut tables)
    } else {
        let mut it = tables.into_iter();
        let mut all = it.next().unwrap();
        for t in it {
            all.merge(&t);
        }
        all.columns()
    };
    let q = isc_alloc((g.len() * 8) as i64) as *mut i64;
    core::ptr::copy_nonoverlapping(g.as_ptr(), q, g.len());
    *out = IscArray { ptr: q as *mut u8, len: g.len() as i64 };
}

/// The flag of `kind` for `isc_group` of `$groupCounts` (D317).
const GROUP_COUNTS: i32 = 64;

/// The columns of `isc_group` from tables of which some scattered their
/// keys (D253): the keys counted by their value merged in the order of the
/// parts; the hashed ones list by list of scatter, in parallel, part after
/// part (its groups in its table, then those of its scattered keys: the
/// order of the indices); every group then in the order of its first index.
unsafe fn scattered_columns(tables: &mut [sem::WordGroups]) -> alloc::vec::Vec<i64> {
    use alloc::vec::Vec;
    type Group = (u32, u64, u32, u32);
    const B: usize = sem::SCATTER_BUCKETS;
    let mut small = sem::WordGroups::new(true);
    let mut pre: Vec<Vec<Vec<Group>>> = (0..tables.len()).map(|_| (0..B).map(|_| Vec::new()).collect()).collect();
    for (p, t) in tables.iter().enumerate() {
        for g in 0..t.len() {
            let e = t.group(g);
            if t.is_small_key(e.1) {
                small.add_aggregate(e.1, e.0, e.2, e.3);
            } else {
                pre[p][sem::scatter_bucket(sem::hash_word(e.1))].push(e);
            }
        }
    }
    // the last round of each part counted
    let lists: Vec<&[sem::ListTable]> = tables.iter_mut().map(|t| &*(t.scattered_tables() as *const [sem::ListTable])).collect();
    struct Ctx<'a> {
        lists: &'a [&'a [sem::ListTable]],
        pre: &'a [Vec<Vec<Group>>],
        out: *mut Vec<Group>,
    }
    extern "C" fn list(ctx: *mut c_void, b: i64) {
        unsafe {
            let c = &*(ctx as *const Ctx);
            let b = b as usize;
            // part after part: its groups of its table, then of its list
            let mut t = sem::ListTable::new();
            for p in 0..c.pre.len() {
                for &(f, k, l, n) in &c.pre[p][b] {
                    t.add(k, sem::hash_word(k), f, l, n);
                }
                if let Some(s) = c.lists[p].get(b) {
                    for (f, k, l, n) in s.groups() {
                        t.add(k, sem::hash_word(k), f, l, n);
                    }
                }
            }
            (*c.out.add(b)).extend(t.groups());
        }
    }
    let mut outs: Vec<Vec<Group>> = (0..B).map(|_| Vec::new()).collect();
    let mut ctx = Ctx { lists: &lists, pre: &pre, out: outs.as_mut_ptr() };
    isc_parallel_for(B as i64, list, &mut ctx as *mut Ctx as *mut c_void, -1);
    let mut all: Vec<Group> = (0..small.len()).map(|g| small.group(g)).collect();
    for o in &outs {
        all.extend_from_slice(o);
    }
    all.sort_unstable_by_key(|e| e.0);
    let mut cols = Vec::with_capacity(3 * all.len());
    cols.extend(all.iter().map(|e| e.0 as i64));
    cols.extend(all.iter().map(|e| e.2 as i64));
    cols.extend(all.iter().map(|e| e.3 as i64));
    cols
}

/// `$group(keys)` (D167): see `isc_runtime_core::group`; `kind` as for
/// `isc_argsort` (strings and `bigint` by their bytes, both canonical).
#[no_mangle]
pub unsafe extern "C" fn isc_group(out: *mut IscArray, p: *const u8, n: i64, kind: i32) {
    // `$groupCounts` (D317): the same groups, their last indices zeros
    let counts = kind & GROUP_COUNTS != 0;
    let kind = kind & !GROUP_COUNTS;
    let n = n.max(0) as usize;
    unsafe fn at<T: Copy>(p: *const u8, i: usize) -> T {
        *(p as *const T).add(i)
    }
    // integer keys in a small range: a table indexed by the key (D194);
    // otherwise each key kept in its slot, the parts in parallel (D243)
    macro_rules! ints {
        ($t:ty, $order:expr) => {
            match sem::group_dense(n, |i| $order(at::<$t>(p, i))) {
                Some(g) => g,
                None => {
                    // (the address as a number: read by the threads)
                    let pa = p as usize;
                    group_words(n, move |i| at::<$t>(pa as *const u8, i) as u64)
                }
            }
        };
        ($t:ty) => {
            ints!($t, |x: $t| x as i64)
        };
    }
    let mut g = match kind {
        0 => ints!(i8),
        1 => ints!(i16),
        2 => ints!(i32),
        3 => ints!(i64),
        4 | 10 => ints!(u8),
        5 => ints!(u16),
        6 => ints!(u32),
        7 => ints!(u64, |x: u64| (x ^ (1 << 63)) as i64),
        _ => sem::group(
            n,
            |i| {
                let a = at::<IscStr>(p, i);
                sem::hash_bytes(bytes_of(a.ptr, a.len))
            },
            |i, j| {
                let (a, b) = (at::<IscStr>(p, i), at::<IscStr>(p, j));
                text_same(bytes_of(a.ptr, a.len), bytes_of(b.ptr, b.len))
            },
        ),
    };
    if counts {
        let m = g.len() / 3;
        g[m..2 * m].fill(0);
    }
    let q = isc_alloc((g.len() * 8) as i64) as *mut i64;
    core::ptr::copy_nonoverlapping(g.as_ptr(), q, g.len());
    *out = IscArray { ptr: q as *mut u8, len: g.len() as i64 };
}

/// `join(xs, sep)`: the strings of `xs` with `sep` between them.
#[no_mangle]
pub unsafe extern "C" fn isc_join(out: *mut IscStr, xs: *const IscStr, n: i64, q: *const u8, m: i64) {
    let n = n.max(0) as usize;
    // SAFETY: the `n` strings of an array of the generated code and the
    // `m` bytes of its separator, alive during the call
    let (parts, sep) = unsafe { (core::slice::from_raw_parts(xs, n), bytes_of(q, m)) };
    let r = join(parts, sep);
    // SAFETY: the place of the result, the generated code's
    unsafe { out.write(r) };
}

/// From this many bytes on, a join is made by parts in parallel (D427).
const JOIN_PARALLEL_MIN: usize = 1 << 20;

/// The strings `parts` joined by `sep`, in safe code (D290): a large result
/// by ranges of the strings in parallel (D427: the 125 MB of the 2 million
/// lines of k-nucleotide copied by one thread took 30 ms), each range
/// about as many bytes, cut where a string starts; each thread writes its
/// own pages. The same bytes for any number of threads.
fn join(parts: &[IscStr], sep: &[u8]) -> IscStr {
    let len = |x: &IscStr| x.len.max(0) as usize;
    let total = parts.iter().map(len).sum::<usize>() + sep.len() * parts.len().saturating_sub(1);
    let r = isc_alloc(total as i64);
    // SAFETY: a block of `total` bytes of this call alone, not read before
    // it is written (as bytes that may not be initialized)
    let dst = unsafe { core::slice::from_raw_parts_mut(r as *mut core::mem::MaybeUninit<u8>, total) };
    // the strings `from .. to` into `out`, each after its separator (but
    // the first of all)
    let write = |out: &mut [core::mem::MaybeUninit<u8>], from: usize, to: usize| {
        let mut at = 0usize;
        for (i, x) in parts[from..to].iter().enumerate() {
            if from + i > 0 {
                out[at..at + sep.len()].write_copy_of_slice(sep);
                at += sep.len();
            }
            // SAFETY: the bytes of a string of the array, alive
            let b = unsafe { bytes_of(x.ptr, x.len.max(0)) };
            out[at..at + b.len()].write_copy_of_slice(b);
            at += b.len();
        }
    };
    if total < JOIN_PARALLEL_MIN || thread_count() <= 1 || parts.len() < 2 {
        write(dst, 0, parts.len());
        return IscStr { ptr: r, len: total as i64 };
    }
    let (first, place, ranges) = join_ranges(parts, sep.len(), total);
    // the separator before the first string of a range (but the first)
    // is the range's: its place starts at it
    par::parts_mut(&par::Parts::new(dst, &place[..=ranges]), par::Order::Contiguous, |c, out| write(out, first[c], first[c + 1]));
    IscStr { ptr: r, len: total as i64 }
}

/// The ranges of the join of `parts` by a separator of `m` bytes (`total`
/// bytes) made in parallel: the first string of each and the place of the
/// separator before it, a range each `total / k` bytes about (`k` four a
/// thread, at most 256), and their count (D427).
fn join_ranges(parts: &[IscStr], m: usize, total: usize) -> ([usize; 257], [usize; 257], usize) {
    let k = (thread_count() * 4).clamp(1, 256);
    let mut first = [parts.len(); 257];
    let mut place = [total; 257];
    first[0] = 0;
    place[0] = 0;
    let mut cut = 1;
    // where the bytes of string `i` start
    let mut at = 0usize;
    for (i, x) in parts.iter().enumerate() {
        if i > 0 && cut < k && at >= total / k * cut {
            first[cut] = i;
            place[cut] = at - m;
            cut += 1;
        }
        at += x.len.max(0) as usize + if i + 1 < parts.len() { m } else { 0 };
    }
    first[cut] = parts.len();
    place[cut] = total;
    (first, place, cut)
}

/// `isc_join_ranges(xs, n, m, first, place)`: the ranges of the join of
/// the `n` strings `xs` by a separator of `m` bytes (`join_ranges`), into
/// `first` and `place` (257 each): their count; `place[count]` is the
/// length of the join. For a join read once in order by a `map` (D428),
/// never made whole.
#[no_mangle]
pub unsafe extern "C" fn isc_join_ranges(xs: *const IscStr, n: i64, m: i64, first: *mut i64, place: *mut i64) -> i64 {
    // SAFETY: the `n` strings of an array of the generated code, alive
    let parts = unsafe { core::slice::from_raw_parts(xs, n.max(0) as usize) };
    let m = m.max(0) as usize;
    let total = parts.iter().map(|x| x.len.max(0) as usize).sum::<usize>() + m * parts.len().saturating_sub(1);
    let (f, p, count) = join_ranges(parts, m, total);
    // SAFETY: the generated code's two arrays of 257 slots
    let (first, place) = unsafe { (core::slice::from_raw_parts_mut(first, 257), core::slice::from_raw_parts_mut(place, 257)) };
    for c in 0..=count {
        first[c] = f[c] as i64;
        place[c] = p[c] as i64;
    }
    count as i64
}

/// Where the reading of a join by `isc_join_next` is: the string `i`, the
/// byte `off` of it with the separator before it (but the first string),
/// the string `end` where its range ends.
#[repr(C)]
pub struct JoinCursor {
    pub i: i64,
    pub off: i64,
    pub end: i64,
}

/// `isc_join_next(cur, xs, q, m, buf, cap)`: the next bytes of the join of
/// the strings `xs` by the `m` bytes `q`, from the cursor to the end of its
/// range, at most `cap` of them, written into `buf`: their count (0 at the
/// end); the cursor moved past them (D428).
#[no_mangle]
pub unsafe extern "C" fn isc_join_next(cur: *mut JoinCursor, xs: *const IscStr, q: *const u8, m: i64, buf: *mut u8, cap: i64) -> i64 {
    // SAFETY: the cursor, the strings of the range (alive), the separator
    // and a buffer of `cap` bytes, the generated code's
    let (c, sep, out) = unsafe { (&mut *cur, bytes_of(q, m), core::slice::from_raw_parts_mut(buf, cap.max(0) as usize)) };
    let mut got = 0usize;
    while got < out.len() && c.i < c.end {
        // SAFETY: a string of the range
        let x = unsafe { *xs.add(c.i as usize) };
        let s = if c.i > 0 { sep } else { &[][..] };
        // SAFETY: the bytes of a string of the array, alive
        let b = unsafe { bytes_of(x.ptr, x.len.max(0)) };
        let off = c.off as usize;
        let (from, at) = if off < s.len() { (s, off) } else { (b, off - s.len()) };
        let k = (from.len() - at).min(out.len() - got);
        out[got..got + k].copy_from_slice(&from[at..at + k]);
        got += k;
        c.off += k as i64;
        if c.off as usize == s.len() + b.len() {
            c.i += 1;
            c.off = 0;
        }
    }
    got as i64
}

/// An array of arrays of `f64`: its block, rows, columns and stride (the
/// layout of the code generator).
#[repr(C)]
pub struct IscNested {
    pub ptr: *mut u8,
    pub rows: i64,
    pub cols: i64,
    pub stride: i64,
}

/// `parseCsvF64(text, skip)`: checked and counted, then read into one
/// block, rows contiguous.
#[no_mangle]
pub unsafe extern "C" fn isc_parse_csv_f64(out: *mut IscNested, p: *const u8, n: i64, skip: i64, line: i32, col: i32) {
    let s = core::str::from_utf8_unchecked(bytes_of(p, n));
    let (rows, cols) = match sem::parse_csv_f64(s, skip, &mut |_, _| {}) {
        Ok(rc) => rc,
        Err(e) => die(line, col, &e),
    };
    let q = isc_alloc(rows * cols * 8) as *mut f64;
    let _ = sem::parse_csv_f64(s, skip, &mut |i, v| *q.add(i) = v);
    *out = IscNested { ptr: q as *mut u8, rows, cols, stride: cols };
}

/// `readCsvF64(path, skip)`.
#[no_mangle]
pub unsafe extern "C" fn isc_read_csv_f64(out: *mut IscNested, path: *const u8, plen: i64, skip: i64, line: i32, col: i32) {
    let mut t = IscStr { ptr: core::ptr::null(), len: 0 };
    isc_read_text(&mut t, path, plen, line, col);
    isc_parse_csv_f64(out, t.ptr, t.len, skip, line, col);
    isc_free(t.ptr as *mut u8);
}

// ---------------------------------------------------------------------
// the fallible forms of the actions (D138): the value, and an error
// message instead of a panic ("" when the action succeeds; on an error the
// value is empty or zero)

/// The message of an error, as a new string.
unsafe fn message(p: &Panic) -> IscStr {
    struct Count(usize);
    impl Write for Count {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            self.0 += s.len();
            Ok(())
        }
    }
    struct Fill(*mut u8, usize);
    impl Write for Fill {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            unsafe { core::ptr::copy_nonoverlapping(s.as_ptr(), self.0.add(self.1), s.len()) };
            self.1 += s.len();
            Ok(())
        }
    }
    let mut c = Count(0);
    let _ = write!(c, "{}", p);
    let q = isc_alloc(c.0 as i64);
    let mut f = Fill(q, 0);
    let _ = write!(f, "{}", p);
    IscStr { ptr: q, len: c.0 as i64 }
}

fn no_error() -> IscStr {
    new_str(b"")
}

fn empty_array() -> IscArray {
    IscArray { ptr: isc_alloc(0), len: 0 }
}

unsafe fn try_read_f64s(p: &str) -> Result<IscArray, Panic<'_>> {
    let a = try_read_file(p)?;
    if a.len % 8 != 0 {
        isc_free(a.ptr);
        return Err(Panic::NotMultipleOf { path: p, len: a.len as u64, size: 8 });
    }
    Ok(IscArray { ptr: a.ptr, len: a.len / 8 })
}

unsafe fn try_read_text(p: &str) -> Result<IscStr, Panic<'_>> {
    let a = try_read_file(p)?;
    if let Err(at) = sem::utf8_check(bytes_of(a.ptr, a.len)) {
        isc_free(a.ptr);
        return Err(Panic::TextNotUtf8 { path: p, at: at as i64 });
    }
    Ok(IscStr { ptr: a.ptr, len: a.len })
}

unsafe fn try_csv(s: &str, skip: i64) -> Result<IscNested, Panic<'_>> {
    let (rows, cols) = sem::parse_csv_f64(s, skip, &mut |_, _| {})?;
    let q = isc_alloc(rows * cols * 8) as *mut f64;
    let _ = sem::parse_csv_f64(s, skip, &mut |i, v| *q.add(i) = v);
    Ok(IscNested { ptr: q as *mut u8, rows, cols, stride: cols })
}

#[no_mangle]
pub unsafe extern "C" fn isc_try_parse_i64(v: *mut i64, e: *mut IscStr, p: *const u8, n: i64) {
    let s = str_of(p, n);
    match sem::parse_i64(s) {
        Some(x) => (*v, *e) = (x, no_error()),
        None => (*v, *e) = (0, message(&Panic::ParseI64(s))),
    }
}

#[no_mangle]
pub unsafe extern "C" fn isc_try_parse_f64(v: *mut f64, e: *mut IscStr, p: *const u8, n: i64) {
    let s = str_of(p, n);
    match sem::parse_f64(s) {
        Some(x) => (*v, *e) = (x, no_error()),
        None => (*v, *e) = (0.0, message(&Panic::ParseF64(s))),
    }
}

#[no_mangle]
pub unsafe extern "C" fn isc_try_read_bytes(out: *mut IscArray, e: *mut IscStr, path: *const u8, plen: i64) {
    match try_read_file(str_of(path, plen)) {
        Ok(a) => (*out, *e) = (a, no_error()),
        Err(p) => (*out, *e) = (empty_array(), message(&p)),
    }
}

#[no_mangle]
pub unsafe extern "C" fn isc_try_read_f64(out: *mut IscArray, e: *mut IscStr, path: *const u8, plen: i64) {
    match try_read_f64s(str_of(path, plen)) {
        Ok(a) => (*out, *e) = (a, no_error()),
        Err(p) => (*out, *e) = (empty_array(), message(&p)),
    }
}

#[no_mangle]
pub unsafe extern "C" fn isc_try_read_text(out: *mut IscStr, e: *mut IscStr, path: *const u8, plen: i64) {
    match try_read_text(str_of(path, plen)) {
        Ok(t) => (*out, *e) = (t, no_error()),
        Err(p) => (*out, *e) = (no_error(), message(&p)),
    }
}

#[no_mangle]
pub unsafe extern "C" fn isc_try_read_csv_f64(out: *mut IscNested, e: *mut IscStr, path: *const u8, plen: i64, skip: i64) {
    let empty = || IscNested { ptr: isc_alloc(0), rows: 0, cols: 0, stride: 0 };
    match try_read_text(str_of(path, plen)) {
        Ok(t) => {
            match try_csv(core::str::from_utf8_unchecked(bytes_of(t.ptr, t.len)), skip) {
                Ok(m) => (*out, *e) = (m, no_error()),
                Err(p) => (*out, *e) = (empty(), message(&p)),
            }
            isc_free(t.ptr as *mut u8);
        }
        Err(p) => (*out, *e) = (empty(), message(&p)),
    }
}

#[no_mangle]
pub unsafe extern "C" fn isc_try_write_bytes(e: *mut IscStr, path: *const u8, plen: i64, data: *const u8, n: i64) {
    let n = n.max(0) as usize;
    let r = try_write_file(str_of(path, plen), n, &|done, chunk| {
        let count = chunk.len().min(n - done);
        core::ptr::copy_nonoverlapping(data.add(done), chunk.as_mut_ptr(), count);
        count
    });
    *e = match r {
        Ok(()) => no_error(),
        Err(p) => message(&p),
    };
}

#[no_mangle]
pub unsafe extern "C" fn isc_parse_f64(p: *const u8, len: i64, line: i32, col: i32) -> f64 {
    let s = str_of(p, len);
    match sem::parse_f64(s) {
        Some(v) => v,
        None => die(line, col, &Panic::ParseF64(s)),
    }
}

// ---------------------------------------------------------------------
// parallel loops (SPEC section 15, stage 2 phase 6: "parallélisme par blocs,
// pool de threads")
//
// `isc_parallel_for(n, task, ctx)` runs `task(ctx, i)` for every `i` in
// `0..n`. The generated code makes each task compute whole blocks of the fixed
// form and store its result in its own slot, so the result never depends on
// the number of threads. Tasks are distributed statically: thread `k` of `T`
// runs `k, k + T, k + 2T, ...`; no task is ever claimed twice. Workers spin
// for a short while, then sleep on a futex. The number of threads is
// `$ISC_THREADS`, or else the number of CPUs the process may run on.

use core::sync::atomic::{AtomicI64, AtomicU32, AtomicU64, AtomicUsize, Ordering};

type Task = extern "C" fn(ctx: *mut c_void, i: i64);

/// The environment of the process (D287): the strings after the arguments
/// on the stack the kernel made (`argv[argc + 1 ..]`, until a null),
/// noted by `isc_args`, the first call of a program.
#[cfg(target_os = "linux")]
static ENVIRON: AtomicUsize = AtomicUsize::new(0);

/// The value of the variable `name` (a string ending with a zero byte), a
/// string ending with a zero byte, or null.
#[cfg(target_os = "linux")]
unsafe fn getenv(name: *const c_char) -> *const c_char {
    let mut e = ENVIRON.load(Ordering::Relaxed) as *const *const c_char;
    if e.is_null() {
        return core::ptr::null();
    }
    // SAFETY: `name` ends with a zero byte (see above); `e` the array of the
    // environment, ending with a null, each a string ending with a zero byte
    unsafe {
        let n = strlen(name);
        while !(*e).is_null() {
            let v = *e;
            let mut i = 0;
            while i < n && *v.add(i) == *name.add(i) {
                i += 1;
            }
            if i == n && *v.add(n) == b'=' as c_char {
                return v.add(n + 1);
            }
            e = e.add(1);
        }
    }
    core::ptr::null()
}

// `pthread_t` is a pointer in wasi-libc (the threads of Linux: `process`)
#[cfg(all(target_os = "wasi", isc_wasm_threads))]
extern "C" {
    fn pthread_create(tid: *mut usize, attr: *const c_void, f: extern "C" fn(*mut c_void) -> *mut c_void, arg: *mut c_void) -> c_int;
}

#[cfg(all(not(target_os = "wasi"), not(windows)))]
use linux::{futex, nanosleep, sched_getaffinity};


/// The stack of the threads that run the body of `main` and the tasks: a
/// recursion inside a task reaches the same depth on any thread (D102,
/// D201). Reserved, not committed: only the pages touched are memory.
#[cfg(not(target_os = "wasi"))]
const BIG_STACK: usize = 1 << 30;

/// Starts a worker of the pool on a stack of BIG_STACK bytes, or of the
/// default size if that cannot be had.
#[cfg(windows)]
fn start_worker(k: usize) {
    match windows::spawn(BIG_STACK, worker, k as *mut c_void).or_else(|| windows::spawn(0, worker, k as *mut c_void)) {
        Some(h) => windows::detach(h),
        None => {}
    }
}

/// (Linux: the workers are never joined; they end with the process.)
#[cfg(target_os = "linux")]
fn start_worker(k: usize) {
    (process::os().spawn)(BIG_STACK, worker, k as *mut c_void);
}

#[cfg(all(target_os = "wasi", isc_wasm_threads))]
fn start_worker(k: usize) {
    let mut tid = 0usize;
    unsafe { pthread_create(&mut tid, core::ptr::null(), worker, k as *mut c_void) };
}

#[cfg(windows)]
fn wait_gen(seen: u32) {
    windows::wait_word(GEN.gen.as_ptr(), seen)
}

#[cfg(windows)]
fn wake_all() {
    windows::wake_word(GEN.gen.as_ptr())
}

/// Sleeps while `GEN` is `seen` (spurious wake-ups are allowed).
#[cfg(all(not(target_os = "wasi"), not(windows)))]
fn wait_gen(seen: u32) {
    futex(&GEN.gen, FUTEX_WAIT_PRIVATE, seen);
}

/// Wakes every thread sleeping in `wait_gen`.
#[cfg(all(not(target_os = "wasi"), not(windows)))]
fn wake_all() {
    futex(&GEN.gen, FUTEX_WAKE_PRIVATE, i32::MAX as u32);
}

// `memory.atomic.wait32` and `memory.atomic.notify` are unstable in Rust
// (`stdarch_wasm_atomic_wait`): the generated module defines these two
// functions with the LLVM intrinsics instead (`iscc`, target
// wasm32-wasi-threads).
#[cfg(all(target_os = "wasi", isc_wasm_threads))]
extern "C" {
    fn isc_wasm_wait32(p: *mut i32, expected: i32);
    fn isc_wasm_notify(p: *mut i32);
}

#[cfg(all(target_os = "wasi", isc_wasm_threads))]
fn wait_gen(seen: u32) {
    unsafe { isc_wasm_wait32(GEN.gen.as_ptr() as *mut i32, seen as i32) };
}

#[cfg(all(target_os = "wasi", isc_wasm_threads))]
fn wake_all() {
    unsafe { isc_wasm_notify(GEN.gen.as_ptr() as *mut i32) };
}

/// Number of CPUs when `$ISC_THREADS` is not set: the CPUs the process may
/// run on (Linux); WASI has no way to know it, so 4 there (DECISIONS D68).
#[cfg(windows)]
fn default_threads() -> usize {
    windows::cpus()
}

#[cfg(all(not(target_os = "wasi"), not(windows)))]
fn default_threads() -> usize {
    let mut mask = [0u8; 128];
    // the bytes of the mask written (the system call, not 0 as in glibc)
    if unsafe { sched_getaffinity(0, mask.len(), mask.as_mut_ptr()) } > 0 {
        mask.iter().map(|b| b.count_ones() as usize).sum()
    } else {
        1
    }
}

#[cfg(all(target_os = "wasi", isc_wasm_threads))]
fn default_threads() -> usize {
    4
}

// WASI without threads (wasm32-wasip1): one thread, whatever `$ISC_THREADS`.
// The result is the same as with any number of threads (SPEC section 10).
#[cfg(all(target_os = "wasi", not(isc_wasm_threads)))]
fn thread_count() -> usize {
    1
}

#[cfg(not(windows))]
#[cfg(not(windows))]
const FUTEX_WAIT_PRIVATE: c_int = 128;
#[cfg(not(windows))]
const FUTEX_WAKE_PRIVATE: c_int = 129;
const SPIN: u32 = 1 << 10;

// The variables the threads of the pool share, in three lines of 128 bytes
// (an aligned pair of cache lines, the unit of the L2 prefetcher), grouped
// by who writes them and when (DECISIONS D81). The workers spin on `gen`,
// alone in its line: the caller writes that line once per job. A worker
// that sees the new generation reads the job in one other line, where it
// writes `finished` at the end. `sleepers` changes only when a worker goes
// to sleep. Exported names: LLVM may not split them into separate globals,
// which would keep the alignment for the first field only and let other
// data share the lines (the output buffer, the free lists of the allocator).

/// Generation of the current job: the futex word the workers wait on.
#[repr(C, align(128))]
pub struct GenLine {
    gen: AtomicU32,
}

/// Bytes of a task context copied into the job line.
const CTX_INLINE: usize = 64;

/// The job, written by the caller before `gen`; `finished` counts the
/// workers that have finished it. A context of at most CTX_INLINE bytes is
/// copied first, at offset 0 (aligned like any value of a context): a worker
/// reads the job and the context in one aligned pair of lines, and the
/// caller does not have to take back from the workers the line of its stack
/// that held the context (DECISIONS D81).
#[repr(C, align(128))]
pub struct JobLine {
    inline: core::cell::UnsafeCell<[u64; CTX_INLINE / 8]>,
    task: AtomicUsize,
    ctx: AtomicUsize,
    n: AtomicI64,
    /// Number of threads (0: not yet known).
    threads: AtomicUsize,
    finished: AtomicUsize,
}

/// The next task of a dynamic job (`isc_parallel_for_dynamic`, D145), in a
/// line of its own: the threads take it in turn while they work. With
/// `contiguous`, each thread its range of the tasks (D254). With both, each
/// thread its range first, then the tasks left in the ranges of the others,
/// from their ends (D313: `RANGES`).
#[repr(C, align(128))]
pub struct NextLine {
    next: AtomicI64,
    dynamic: core::sync::atomic::AtomicBool,
    contiguous: core::sync::atomic::AtomicBool,
}

static NEXT: NextLine = NextLine { next: AtomicI64::new(0), dynamic: core::sync::atomic::AtomicBool::new(false), contiguous: core::sync::atomic::AtomicBool::new(false) };

/// Threads whose ranges `RANGES` can hold; a job of more threads, or of
/// more than `u32::MAX` tasks, is only dynamic.
const RANGE_THREADS: usize = 64;

/// The tasks left in the range of each thread for a job both dynamic and
/// contiguous (D313): the first in the low 32 bits, the end in the high
/// ones. Its thread takes them from the first, the threads that have
/// finished their own from the end, each by one exchange of the whole
/// word: no task is taken twice. Each thread writes first the pages of its
/// own range (the threads do not fault the same huge page of a new array
/// together, D254), and a part costlier than the others (more matches of a
/// search) is finished by them, from its other end.
static RANGES: [AtomicU64; RANGE_THREADS] = [const { AtomicU64::new(0) }; RANGE_THREADS];

/// The next task of the range `r`: from its first (`front`) or its end.
fn take_range(r: &AtomicU64, front: bool) -> Option<i64> {
    let mut v = r.load(Ordering::Acquire);
    loop {
        let (first, end) = (v & 0xFFFF_FFFF, v >> 32);
        if first >= end {
            return None;
        }
        let (next, task) = if front { (v + 1, first) } else { (v - (1 << 32), end - 1) };
        match r.compare_exchange_weak(v, next, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => return Some(task as i64),
            Err(now) => v = now,
        }
    }
}

#[repr(C, align(128))]
pub struct SleepLine {
    /// Workers asleep in `wait_gen`, or about to be: a new job wakes them
    /// only if there are some, so that a job given to spinning workers costs
    /// no system call (DECISIONS D78).
    sleepers: AtomicU32,
    /// Generation when the workers were created: they wait for the next one,
    /// even if they start running after it was published.
    start_gen: AtomicU32,
}

#[export_name = "isc_pool_gen"]
static GEN: GenLine = GenLine { gen: AtomicU32::new(0) };
// The inline context is written by the caller before it publishes `gen`
// (release) and read by the workers after they see it (acquire).
unsafe impl Sync for JobLine {}

#[export_name = "isc_pool_job"]
static JOB: JobLine = JobLine {
    inline: core::cell::UnsafeCell::new([0; CTX_INLINE / 8]),
    task: AtomicUsize::new(0),
    ctx: AtomicUsize::new(0),
    n: AtomicI64::new(0),
    threads: AtomicUsize::new(0),
    finished: AtomicUsize::new(0),
};
#[export_name = "isc_pool_sleep"]
static SLEEP: SleepLine = SleepLine { sleepers: AtomicU32::new(0), start_gen: AtomicU32::new(0) };

#[cfg(any(not(target_os = "wasi"), isc_wasm_threads))]
fn thread_count() -> usize {
    let t = JOB.threads.load(Ordering::Acquire);
    if t != 0 {
        return t;
    }
    let mut n = 0usize;
    unsafe {
        let v = getenv(b"ISC_THREADS\0".as_ptr() as *const c_char);
        if !v.is_null() {
            for &b in c_str(v).as_bytes() {
                if b.is_ascii_digit() {
                    n = n * 10 + (b - b'0') as usize;
                }
            }
        }
    }
    if n == 0 {
        n = default_threads();
    }
    let n = n.clamp(1, 256);
    SLEEP.start_gen.store(GEN.gen.load(Ordering::Acquire), Ordering::Release);
    for k in 1..n {
        start_worker(k);
    }
    JOB.threads.store(n, Ordering::Release);
    n
}

fn run_share(k: usize, threads: usize) {
    let task: Task = unsafe { core::mem::transmute(JOB.task.load(Ordering::Acquire)) };
    let ctx = JOB.ctx.load(Ordering::Acquire) as *mut c_void;
    let n = JOB.n.load(Ordering::Acquire);
    // each thread its range, then the ends of the others' (D313)
    if NEXT.dynamic.load(Ordering::Acquire) && NEXT.contiguous.load(Ordering::Acquire) {
        while let Some(i) = take_range(&RANGES[k], true) {
            task(ctx, i);
        }
        for j in 1..threads {
            while let Some(i) = take_range(&RANGES[(k + j) % threads], false) {
                task(ctx, i);
            }
        }
        return;
    }
    // a dynamic job: the next task not taken yet, until there is none
    if NEXT.dynamic.load(Ordering::Acquire) {
        loop {
            let i = NEXT.next.fetch_add(1, Ordering::Relaxed);
            if i >= n {
                return;
            }
            task(ctx, i);
        }
    }
    // the tasks that fill an array: its contiguous range (D254)
    if NEXT.contiguous.load(Ordering::Acquire) {
        for i in sem::static_share(n, threads as i64, k as i64) {
            task(ctx, i);
        }
        return;
    }
    // every `threads`-th task: the threads read close places of memory
    // (D65, D254)
    let mut i = k as i64;
    while i < n {
        task(ctx, i);
        i += threads as i64;
    }
}

#[cfg(any(not(target_os = "wasi"), isc_wasm_threads))]
extern "C" fn worker(arg: *mut c_void) -> *mut c_void {
    let k = arg as usize;
    let mut seen = SLEEP.start_gen.load(Ordering::Acquire);
    loop {
        let mut spins = 0u32;
        while GEN.gen.load(Ordering::Acquire) == seen {
            if spins < SPIN {
                core::hint::spin_loop();
                spins += 1;
            } else {
                // Announced before the last check of `GEN`, and `GEN` is
                // written before `SLEEPERS` is read, with a full fence
                // between the write and the read on both sides: either the
                // caller sees the sleeper and wakes it, or the sleeper sees
                // the new generation and does not sleep.
                SLEEP.sleepers.fetch_add(1, Ordering::SeqCst);
                core::sync::atomic::fence(Ordering::SeqCst);
                if GEN.gen.load(Ordering::SeqCst) == seen {
                    wait_gen(seen);
                }
                SLEEP.sleepers.fetch_sub(1, Ordering::SeqCst);
            }
        }
        seen = seen.wrapping_add(1);
        let threads = JOB.threads.load(Ordering::Acquire);
        run_share(k, threads);
        JOB.finished.fetch_add(1, Ordering::AcqRel);
        help_until_done(seen, threads, true);
    }
}

// Parallel loops inside tasks (D201). A loop called by a task, while the
// pool runs a job, puts its tasks in a slot; its caller takes them in turn,
// and so do the threads that have finished their share of the job, until
// the job is done. Each task writes only its own results (as for
// `isc_parallel_for_dynamic`): they do not depend on which thread runs it.

/// The tasks of a loop called by a task, on the stack of its caller.
#[repr(C)]
struct Nested {
    task: Task,
    ctx: *mut c_void,
    n: i64,
    next: AtomicI64,
}

/// `job`: 0 free, 1 being closed, else a `*const Nested`. `users`: the
/// helpers that may read it. The caller closes it (1), then waits for no
/// user: a helper counts itself before it reads `job` (sequentially
/// consistent on both sides), so either it sees 1 or the caller sees it,
/// and the `Nested` is never read once its frame is gone.
#[repr(C, align(128))]
pub struct NestedSlot {
    job: AtomicUsize,
    users: AtomicUsize,
}

const NESTED_SLOTS: usize = 64;

#[export_name = "isc_pool_nested"]
static NESTED: [NestedSlot; NESTED_SLOTS] = [const { NestedSlot { job: AtomicUsize::new(0), users: AtomicUsize::new(0) } }; NESTED_SLOTS];

/// One more than the highest slot ever used, in a line of its own: written
/// only when it grows (D410: the count of the slots in use, written twice
/// by each loop opened inside a task, was a line taken from one core to the
/// other at each loop of a server; the threads with nothing left to do read
/// the slots below it instead).
#[repr(C, align(128))]
pub struct NestedCount {
    high: AtomicUsize,
}

#[export_name = "isc_pool_nested_count"]
static NESTED_COUNT: NestedCount = NestedCount { high: AtomicUsize::new(0) };

/// Runs the tasks of `j` not taken yet; true if it ran one.
unsafe fn run_nested(j: &Nested) -> bool {
    let mut ran = false;
    loop {
        let i = j.next.fetch_add(1, Ordering::Relaxed);
        if i >= j.n {
            return ran;
        }
        (j.task)(j.ctx, i);
        ran = true;
    }
}

/// Runs tasks of the loops open in the slots; true if it ran one.
fn help_nested() -> bool {
    let high = NESTED_COUNT.high.load(Ordering::Acquire).min(NESTED_SLOTS);
    if high == 0 {
        return false;
    }
    let mut ran = false;
    for s in &NESTED[..high] {
        if s.job.load(Ordering::Acquire) <= 1 {
            continue;
        }
        s.users.fetch_add(1, Ordering::SeqCst);
        let j = s.job.load(Ordering::SeqCst);
        if j > 1 {
            ran |= unsafe { run_nested(&*(j as *const Nested)) };
        }
        s.users.fetch_sub(1, Ordering::Release);
    }
    ran
}

/// After its share: the tasks of the loops opened by the tasks still
/// running, until every thread has finished its share of the job of
/// generation `gen`. A worker spins SPIN times without work, then sleeps by
/// 50 µs; the caller of the job (`main`, which waits for it) only spins.
fn help_until_done(gen: u32, threads: usize, sleep: bool) {
    let mut idle = 0u32;
    while JOB.finished.load(Ordering::Acquire) < threads && GEN.gen.load(Ordering::Acquire) == gen {
        if help_nested() {
            idle = 0;
            continue;
        }
        if idle < SPIN || !sleep {
            idle = idle.saturating_add(1);
            core::hint::spin_loop();
        } else {
            pause_briefly();
        }
    }
}

#[cfg(windows)]
fn pause_briefly() {
    windows::pause_briefly()
}

#[cfg(all(not(target_os = "wasi"), not(windows)))]
fn pause_briefly() {
    let t = [0i64, 50_000];
    unsafe { nanosleep(&t, core::ptr::null_mut()) };
}

#[cfg(target_os = "wasi")]
fn pause_briefly() {
    core::hint::spin_loop();
}

/// A loop called by a task while the pool runs a job: its tasks in a slot,
/// taken by its caller and by the threads with nothing left to do; returns
/// when all are done. In place, in order, when no slot is free.
fn nested_for(n: i64, task: Task, ctx: *mut c_void) {
    let j = Nested { task, ctx, n, next: AtomicI64::new(0) };
    let addr = &j as *const Nested as usize;
    // the first slot tried depends on the stack (of the thread, then the
    // depth): the threads do not all try the same slots in turn
    // (in 64 bits: `usize` has 32 on WebAssembly)
    let a = addr as u64;
    let first = (((a >> 12) ^ (a >> 30)).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 58) as usize;
    let mut slot = None;
    for d in 0..NESTED_SLOTS {
        let k = (first + d) % NESTED_SLOTS;
        let s = &NESTED[k];
        if s.job.load(Ordering::Relaxed) == 0 && s.job.compare_exchange(0, addr, Ordering::SeqCst, Ordering::Relaxed).is_ok() {
            if NESTED_COUNT.high.load(Ordering::Relaxed) < k + 1 {
                NESTED_COUNT.high.fetch_max(k + 1, Ordering::AcqRel);
            }
            slot = Some(s);
            break;
        }
    }
    let Some(s) = slot else {
        for i in 0..n {
            task(ctx, i);
        }
        return;
    };
    unsafe { run_nested(&j) };
    s.job.store(1, Ordering::SeqCst);
    while s.users.load(Ordering::SeqCst) != 0 {
        core::hint::spin_loop();
    }
    s.job.store(0, Ordering::Release);
}

/// `isc_parallel_for` whose tasks are taken in turn by the threads as they
/// finish the one before (D145): for tasks of very different costs. Each
/// task writes only its own results, so they do not depend on which thread
/// runs it; no task is taken twice.
#[no_mangle]
pub extern "C" fn isc_parallel_for_dynamic(n: i64, task: Task, ctx: *mut c_void, ctx_bytes: i64) {
    parallel_for(n, task, ctx, ctx_bytes, true, false)
}

/// The cycles of the time stamp counter (D357); 0 where there is none.
#[inline(always)]
fn stamp() -> u64 {
    #[cfg(target_arch = "x86_64")]
    // SAFETY: `rdtsc` reads a counter, nothing else
    unsafe {
        return core::arch::x86_64::_rdtsc();
    }
    #[cfg(not(target_arch = "x86_64"))]
    0
}

/// Cycles above which a loop is never run on the calling thread alone: its
/// run on the pool takes 20 µs, far above what giving it to the pool costs
/// (about 1 µs, D357).
const SEQ_NEVER: u64 = 64000;
/// The runs between two tries of the mode that was slower.
const RETRY: u64 = 256;
/// The runs of a loop on the pool before it is tried on the calling thread:
/// a try costs up to the work of all the threads on one (four runs), which
/// a loop run a few hundred times never gains back (D357: fasta 3.3 %
/// slower with tries after 16 runs, its time back without them); after 256
/// runs a try costs at most 1.6 % of them.
const RUNS_BEFORE_TRY: u64 = 256;

/// A parallel loop at a place of the program (D357), `mode` that of
/// `isc_parallel_for` (0), `isc_parallel_for_dynamic` (1) or
/// `isc_parallel_for_fill` (2):
/// `site` keeps the cycles of its runs on the pool and of its runs on the
/// calling thread (averages), its runs, and whether its last run was alone. The faster mode is taken; the other
/// tried again once every RETRY runs; the calling thread never tried when a
/// run on the pool took more than SEQ_NEVER cycles (a job given to the pool
/// and waited for costs about 1 µs: 16 elements of 10 steps 96 ns on one
/// thread, 1088 ns on four; 16 of 1000 steps 2247 against 1264). The tasks
/// write each its own results, whatever the thread (D121): the same results
/// either way.
#[no_mangle]
pub extern "C" fn isc_parallel_for_site(n: i64, task: Task, ctx: *mut c_void, ctx_bytes: i64, site: *const [AtomicU64; 4], mode: i32) {
    // SAFETY: four words of the program, for this loop
    let [par, seq, runs, last] = unsafe { &*site };
    let (dynamic, contiguous) = (mode == 1, mode == 2);
    // a loop inside a task (the pool busy): neither the pool nor the
    // calling thread to choose, no clock read, nothing written to the words
    // of the site (D410: written at each loop by each thread of a server,
    // their line taken from one core to the other, 12.01 % of its cycles)
    if BUSY.load(Ordering::Relaxed) {
        return parallel_for(n, task, ctx, ctx_bytes, dynamic, contiguous);
    }
    // one element, or one thread: nothing to choose, no clock read, nothing
    // written to the words of the site (D460: two `rdtsc` and an atomic add
    // at each loop, 3.6 % of `json.parse` on one thread)
    if n <= 1 || thread_count() == 1 {
        for i in 0..n {
            task(ctx, i);
        }
        return;
    }
    let t0 = stamp();
    if t0 == 0 {
        return parallel_for(n, task, ctx, ctx_bytes, dynamic, contiguous);
    }
    let (tp, ts) = (par.load(Ordering::Relaxed), seq.load(Ordering::Relaxed));
    let k = runs.fetch_add(1, Ordering::Relaxed);
    let small = tp != 0 && tp < SEQ_NEVER && k >= RUNS_BEFORE_TRY;
    // the calling thread alone unless the pool is faster by an eighth: at
    // the same time it keeps the other cores free (a server whose turns are
    // small); the other mode tried by two runs in turn
    let alone = if !small {
        false
    } else if ts == 0 {
        true
    } else {
        let better = ts <= tp + tp / 8;
        if k % RETRY >= RETRY - 2 { !better } else { better }
    };
    // each time an average of the runs, seven eighths the one before: one
    // run slowed by the system does not turn the loop over for RETRY runs
    let mean = |old: u64, t: u64| if old == 0 { t.max(1) } else { (old * 7 + t) / 8 }.max(1);
    if alone {
        for i in 0..n {
            task(ctx, i);
        }
        seq.store(mean(ts, stamp().wrapping_sub(t0)), Ordering::Relaxed);
        last.store(1, Ordering::Relaxed);
    } else {
        parallel_for(n, task, ctx, ctx_bytes, dynamic, contiguous);
        // a run of the pool counted only when the run before was one too:
        // after runs alone the threads sleep, and the first run pays for
        // waking them (D357: 12209 cycles for 16 elements of 10 steps)
        if last.swap(0, Ordering::Relaxed) == 0 {
            par.store(mean(tp, stamp().wrapping_sub(t0)), Ordering::Relaxed);
        }
    }
}

/// `isc_parallel_for` whose tasks fill an array, each its own part (D254):
/// each thread takes a contiguous range of the tasks. Its writes go to its
/// own pages, where every `threads`-th task has the threads write the same
/// pages at once: a new array has them fault the same huge page together,
/// the others waiting for the one that clears it (reverse-complement, its
/// filter of 250 MB with 4 threads: 43 ms -> 31 ms); its reads, of the
/// sources of the elements, are one stream per thread (the reversed bases
/// of reverse-complement). Loops that only read keep every `threads`-th
/// task, close places of memory for all threads (D65: the large `dot` 1.3 %
/// slower by ranges with 4 threads).
#[no_mangle]
pub extern "C" fn isc_parallel_for_fill(n: i64, task: Task, ctx: *mut c_void, ctx_bytes: i64) {
    parallel_for(n, task, ctx, ctx_bytes, false, true)
}

/// Runs `task(ctx, i)` for `i` in `0..n` on the thread pool; returns when all
/// tasks are done. `ctx_bytes` is the size of the context. Called from
/// inside a task, its tasks are shared with the idle threads (D201).
#[no_mangle]
pub extern "C" fn isc_parallel_for(n: i64, task: Task, ctx: *mut c_void, ctx_bytes: i64) {
    parallel_for(n, task, ctx, ctx_bytes, false, false)
}

fn parallel_for(n: i64, task: Task, ctx: *mut c_void, ctx_bytes: i64, dynamic: bool, contiguous: bool) {
    parallel_job(n, task, ctx, ctx_bytes, dynamic, contiguous, false)
}

/// `parallel_for`; `sleep`: the caller, once its share is done, sleeps as
/// the workers do while it waits for the others (the tasks of actions,
/// which may wait for the system for long, D373), instead of spinning.
fn parallel_job(n: i64, task: Task, ctx: *mut c_void, ctx_bytes: i64, dynamic: bool, contiguous: bool, sleep: bool) {
    if n <= 0 {
        return;
    }
    let threads = if n == 1 { 1 } else { thread_count() };
    if threads == 1 {
        for i in 0..n {
            task(ctx, i);
        }
        return;
    }
    // a parallel loop inside a task: the job line is taken, its tasks are
    // shared with the threads that have finished their share (D201); the
    // results do not depend on the number of threads (D102). `BUSY` read
    // before it is written (D410: a swap writes its line even when it finds
    // it set, taken from the other cores at each loop of a server)
    if BUSY.load(Ordering::Relaxed) || BUSY.swap(true, Ordering::Acquire) {
        nested_for(n, task, ctx);
        return;
    }
    // the context in the job line when it fits (see JobLine); the workers
    // are all waiting, nobody reads it
    let ctx = if ctx_bytes >= 0 && ctx_bytes as usize <= CTX_INLINE {
        let inline = JOB.inline.get() as *mut c_void;
        unsafe { core::ptr::copy_nonoverlapping(ctx as *const u8, inline as *mut u8, ctx_bytes as usize) };
        inline
    } else {
        ctx
    };
    JOB.task.store(task as usize, Ordering::Relaxed);
    JOB.ctx.store(ctx as usize, Ordering::Relaxed);
    JOB.n.store(n, Ordering::Relaxed);
    JOB.finished.store(0, Ordering::Relaxed);
    NEXT.next.store(0, Ordering::Relaxed);
    // the ranges of a job both dynamic and contiguous; too many threads or
    // tasks for them: only dynamic
    let ranged = dynamic && contiguous && threads <= RANGE_THREADS && n <= u32::MAX as i64;
    if ranged {
        for (k, r) in RANGES.iter().enumerate().take(threads) {
            let share = sem::static_share(n, threads as i64, k as i64);
            r.store(share.start as u64 | (share.end as u64) << 32, Ordering::Relaxed);
        }
    }
    NEXT.dynamic.store(dynamic, Ordering::Relaxed);
    NEXT.contiguous.store(ranged || (contiguous && !dynamic), Ordering::Relaxed);
    // Published by a plain write: only the caller writes `gen`. A locked
    // add would ask for its line only once every earlier write is done; a
    // write lets the processor fetch it while the line of the job is still
    // on its way (DECISIONS D81). The fence keeps the order with the read of
    // `sleepers` (see `worker`).
    GEN.gen.store(GEN.gen.load(Ordering::Relaxed).wrapping_add(1), Ordering::Release);
    core::sync::atomic::fence(Ordering::SeqCst);
    #[cfg(any(not(target_os = "wasi"), isc_wasm_threads))]
    if SLEEP.sleepers.load(Ordering::SeqCst) != 0 {
        wake_all();
    }
    run_share(0, threads);
    JOB.finished.fetch_add(1, Ordering::AcqRel);
    let gen = GEN.gen.load(Ordering::Relaxed);
    help_until_done(gen, threads, sleep);
    BUSY.store(false, Ordering::Release);
}

/// `mapParallel(x, f)` (D373): `task(ctx, i)` for `i` in `0..n`, at the same
/// time on the threads of the pool, each task taken by the next thread
/// free (the caller's too), never on the calling thread alone for being
/// faster (an action waits for the system: two that wait for each other
/// must run at the same time). While tasks run, a line of `print` is
/// written whole (`hold_out`).
#[no_mangle]
pub extern "C" fn isc_parallel_actions(n: i64, task: Task, ctx: *mut c_void, ctx_bytes: i64) {
    ACTIONS.fetch_add(1, Ordering::SeqCst);
    parallel_job(n, task, ctx, ctx_bytes, true, false, true);
    ACTIONS.fetch_sub(1, Ordering::SeqCst);
}

/// A parallel loop is running (`isc_parallel_for`).
static BUSY: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// The lowest address of the stack of the calling thread that a recursive
/// function may reach, with a margin for the frames that do not test it
/// (the scratch of a `reduce`, the runtime, the panic itself); 0 when it is
/// not known (no test). The generated code keeps it in a thread-local
/// variable and stops on `isc_panic_stack` below it (D102).
#[cfg(windows)]
#[no_mangle]
pub extern "C" fn isc_stack_floor() -> i64 {
    const MARGIN: usize = 256 << 10;
    let (low, high) = windows::stack_limits();
    if high <= low + 2 * MARGIN {
        return 0;
    }
    (low + MARGIN) as i64
}

#[cfg(target_os = "linux")]
#[no_mangle]
pub extern "C" fn isc_stack_floor() -> i64 {
    const MARGIN: usize = 256 << 10;
    let (low, size) = (process::os().stack)();
    if size <= 2 * MARGIN {
        return 0;
    }
    (low + MARGIN) as i64
}

/// Runs `f(argc, argv)` on a thread with a stack of `BIG_STACK` bytes and
/// returns its result: the body of `main` of a program with recursive
/// functions (D102), whose depth is then limited by this stack rather than
/// by the 8 MiB of the main thread. Runs it here if the thread cannot be
/// created.
#[cfg(windows)]
#[no_mangle]
pub extern "C" fn isc_run_big_stack(f: extern "C" fn(c_int, *mut c_void) -> c_int, argc: c_int, argv: *mut c_void) -> c_int {
    struct Start {
        f: extern "C" fn(c_int, *mut c_void) -> c_int,
        argc: c_int,
        argv: *mut c_void,
        ret: c_int,
    }
    extern "C" fn run(arg: *mut c_void) -> *mut c_void {
        let s = unsafe { &mut *(arg as *mut Start) };
        s.ret = (s.f)(s.argc, s.argv);
        core::ptr::null_mut()
    }
    let mut s = Start { f, argc, argv, ret: 0 };
    match windows::spawn(BIG_STACK, run, &mut s as *mut Start as *mut c_void) {
        Some(h) => {
            windows::join(h);
            s.ret
        }
        None => f(argc, argv),
    }
}

#[cfg(target_os = "linux")]
#[no_mangle]
pub extern "C" fn isc_run_big_stack(f: extern "C" fn(c_int, *mut c_void) -> c_int, argc: c_int, argv: *mut c_void) -> c_int {
    struct Start {
        f: extern "C" fn(c_int, *mut c_void) -> c_int,
        argc: c_int,
        argv: *mut c_void,
        ret: c_int,
    }
    extern "C" fn run(arg: *mut c_void) -> *mut c_void {
        let s = unsafe { &mut *(arg as *mut Start) };
        s.ret = (s.f)(s.argc, s.argv);
        core::ptr::null_mut()
    }
    let mut s = Start { f, argc, argv, ret: 0 };
    let t = (process::os().spawn)(BIG_STACK, run, &mut s as *mut Start as *mut c_void);
    if t == 0 {
        let r = f(argc, argv);
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        release_at_end();
        return r;
    }
    (process::os().join)(t);
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    release_at_end();
    s.ret
}

#[no_mangle]
pub extern "C" fn isc_panic_stack(name: *const u8, len: i64, line: i32, col: i32) -> ! {
    let bytes = unsafe { core::slice::from_raw_parts(name, len as usize) };
    let name = core::str::from_utf8(bytes).unwrap_or("?");
    die(line, col, &Panic::StackOverflow(name))
}

/// A NUL-terminated copy of a `string` for an `extern` function (D125),
/// freed by the caller after the call; a panic when it contains a NUL.
#[no_mangle]
pub extern "C" fn isc_cstr(p: *const u8, len: i64, name: *const u8, nlen: i64, line: i32, col: i32) -> *mut u8 {
    let bytes = unsafe { core::slice::from_raw_parts(p, len.max(0) as usize) };
    if bytes.contains(&0) {
        let n = unsafe { core::slice::from_raw_parts(name, nlen as usize) };
        die(line, col, &Panic::ExternNul(core::str::from_utf8(n).unwrap_or("?")))
    }
    let q = isc_alloc(len.max(0) + 1);
    unsafe {
        core::ptr::copy_nonoverlapping(p, q, bytes.len());
        *q.add(bytes.len()) = 0;
    }
    q
}

/// Number of threads of the pool (for tests and reports).
#[no_mangle]
pub extern "C" fn isc_thread_count() -> i64 {
    thread_count() as i64
}

/// The parallel loops of the program would run here one task after the
/// other: one thread, or a parallel loop already running (`BUSY`, as
/// `isc_parallel_for_site` finds it). A `map` of a few elements that are
/// loops of their own is then made where it is, without a task (D520).
#[no_mangle]
pub extern "C" fn isc_loops_alone() -> bool {
    BUSY.load(Ordering::Relaxed) || thread_count() <= 1
}

// ---------------------------------------------------------------------
// panics of the generated code

#[no_mangle]
pub extern "C" fn isc_panic_bounds(idx: i64, len: i64, unsigned: i32, line: i32, col: i32) -> ! {
    let index = if unsigned != 0 { idx as u64 as i128 } else { idx as i128 };
    die(line, col, &Panic::Bounds { index, len })
}

#[no_mangle]
pub extern "C" fn isc_panic_window(k: i64, len: i64, line: i32, col: i32) -> ! {
    die(line, col, &Panic::Window { k, len })
}

#[no_mangle]
pub extern "C" fn isc_panic_slice(a: i64, b: i64, len: i64, line: i32, col: i32) -> ! {
    die(line, col, &Panic::Slice { a, b, len })
}

#[no_mangle]
pub extern "C" fn isc_panic_size(a: i64, b: i64, line: i32, col: i32) -> ! {
    die(line, col, &Panic::Size { a, b })
}

#[no_mangle]
pub extern "C" fn isc_panic_negsize(k: i64, line: i32, col: i32) -> ! {
    die(line, col, &Panic::NegativeSize(k))
}

#[no_mangle]
pub extern "C" fn isc_panic_div0(line: i32, col: i32) -> ! {
    die(line, col, &Panic::DivByZero)
}

#[no_mangle]
pub extern "C" fn isc_panic_negexp(k: i64, line: i32, col: i32) -> ! {
    die(line, col, &Panic::NegativeExponent(k))
}

#[no_mangle]
pub extern "C" fn isc_panic_bench(r: i64, line: i32, col: i32) -> ! {
    die(line, col, &Panic::BenchReps(r))
}

// ---------------------------------------------------------------------
// mathematical functions (D108): those of `isc_runtime_core`, called by the
// generated code

#[no_mangle]
pub extern "C" fn isc_tan(x: f64) -> f64 {
    sem::tan_f64(x)
}
#[no_mangle]
pub extern "C" fn isc_tanf(x: f32) -> f32 {
    sem::tan_f32(x)
}
#[no_mangle]
pub extern "C" fn isc_asin(x: f64) -> f64 {
    sem::asin_f64(x)
}
#[no_mangle]
pub extern "C" fn isc_asinf(x: f32) -> f32 {
    sem::asin_f32(x)
}
#[no_mangle]
pub extern "C" fn isc_acos(x: f64) -> f64 {
    sem::acos_f64(x)
}
#[no_mangle]
pub extern "C" fn isc_acosf(x: f32) -> f32 {
    sem::acos_f32(x)
}
#[no_mangle]
pub extern "C" fn isc_atan(x: f64) -> f64 {
    sem::atan_f64(x)
}
#[no_mangle]
pub extern "C" fn isc_atanf(x: f32) -> f32 {
    sem::atan_f32(x)
}
#[no_mangle]
pub extern "C" fn isc_floor(x: f64) -> f64 {
    sem::floor_f64(x)
}
#[no_mangle]
pub extern "C" fn isc_floorf(x: f32) -> f32 {
    sem::floor_f32(x)
}
#[no_mangle]
pub extern "C" fn isc_ceil(x: f64) -> f64 {
    sem::ceil_f64(x)
}
#[no_mangle]
pub extern "C" fn isc_ceilf(x: f32) -> f32 {
    sem::ceil_f32(x)
}
#[no_mangle]
pub extern "C" fn isc_trunc(x: f64) -> f64 {
    sem::trunc_f64(x)
}
#[no_mangle]
pub extern "C" fn isc_truncf(x: f32) -> f32 {
    sem::trunc_f32(x)
}
#[no_mangle]
pub extern "C" fn isc_round(x: f64) -> f64 {
    sem::round_f64(x)
}
#[no_mangle]
pub extern "C" fn isc_roundf(x: f32) -> f32 {
    sem::round_f32(x)
}
#[no_mangle]
pub extern "C" fn isc_abs(x: f64) -> f64 {
    sem::abs_f64(x)
}
#[no_mangle]
pub extern "C" fn isc_absf(x: f32) -> f32 {
    sem::abs_f32(x)
}
#[no_mangle]
pub extern "C" fn isc_atan2(y: f64, x: f64) -> f64 {
    sem::atan2_f64(y, x)
}
#[no_mangle]
pub extern "C" fn isc_atan2f(y: f32, x: f32) -> f32 {
    sem::atan2_f32(y, x)
}
#[no_mangle]
pub extern "C" fn isc_exp(x: f64) -> f64 {
    sem::exp_f64(x)
}
#[no_mangle]
pub extern "C" fn isc_log(x: f64) -> f64 {
    sem::log_f64(x)
}
#[no_mangle]
pub extern "C" fn isc_sin(x: f64) -> f64 {
    sem::sin_f64(x)
}
#[no_mangle]
pub extern "C" fn isc_cos(x: f64) -> f64 {
    sem::cos_f64(x)
}
#[no_mangle]
pub extern "C" fn isc_pow(x: f64, y: f64) -> f64 {
    sem::pow_f64(x, y)
}
#[no_mangle]
pub extern "C" fn isc_expf(x: f32) -> f32 {
    sem::exp_f32(x)
}
#[no_mangle]
pub extern "C" fn isc_logf(x: f32) -> f32 {
    sem::log_f32(x)
}
#[no_mangle]
pub extern "C" fn isc_sinf(x: f32) -> f32 {
    sem::sin_f32(x)
}
#[no_mangle]
pub extern "C" fn isc_cosf(x: f32) -> f32 {
    sem::cos_f32(x)
}
#[no_mangle]
pub extern "C" fn isc_powf(x: f32, y: f32) -> f32 {
    sem::pow_f32(x, y)
}
