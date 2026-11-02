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
// without threads the pool is not compiled: its constants and statics are unused
#![cfg_attr(all(target_os = "wasi", not(isc_wasm_threads)), allow(dead_code))]

use core::ffi::{c_char, c_int, c_void};
use core::fmt::{self, Write};
use isc_runtime_core::{self as sem, Panic, ARRAY_ALIGN, PANIC_EXIT_CODE};

#[cfg(not(target_os = "wasi"))]
mod heap;

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

#[repr(C)]
struct Timespec {
    tv_sec: i64,
    /// `long`: 64 bits on x86-64, 32 bits on wasm32.
    tv_nsec: core::ffi::c_long,
}

extern "C" {
    fn aligned_alloc(align: usize, size: usize) -> *mut c_void;
    fn free(p: *mut c_void);
    fn strlen(s: *const c_char) -> usize;
    fn strerror(code: c_int) -> *const c_char;
    fn __errno_location() -> *mut c_int;
    fn open(path: *const c_char, flags: c_int, ...) -> c_int;
    fn read(fd: c_int, buf: *mut c_void, n: usize) -> isize;
    fn write(fd: c_int, buf: *const c_void, n: usize) -> isize;
    fn close(fd: c_int) -> c_int;
    fn lseek(fd: c_int, off: i64, whence: c_int) -> i64;
    fn exit(code: c_int) -> !;
    fn abort() -> !;
}

// The constants of the C library: Linux (glibc, musl) or WASI (wasi-libc).
#[cfg(not(target_os = "wasi"))]
mod sys {
    use core::ffi::c_int;
    pub const O_RDONLY: c_int = 0;
    pub const O_WRONLY: c_int = 0o1;
    pub const O_CREAT: c_int = 0o100;
    pub const O_TRUNC: c_int = 0o1000;
    pub const O_CLOEXEC: c_int = 0o2000000;
    extern "C" {
        fn clock_gettime(clk: c_int, ts: *mut super::Timespec) -> c_int;
    }
    pub unsafe fn monotonic(ts: *mut super::Timespec) {
        clock_gettime(1, ts);
    }
}

#[cfg(target_os = "wasi")]
mod sys {
    use core::ffi::c_int;
    pub const O_RDONLY: c_int = 0x0400_0000;
    pub const O_WRONLY: c_int = 0x1000_0000;
    pub const O_CREAT: c_int = 1 << 12;
    pub const O_TRUNC: c_int = 8 << 12;
    pub const O_CLOEXEC: c_int = 0;
    // `clockid_t` is a pointer in wasi-libc: `CLOCK_MONOTONIC` is `&_CLOCK_MONOTONIC`
    extern "C" {
        static _CLOCK_MONOTONIC: u8;
        fn clock_gettime(clk: *const u8, ts: *mut super::Timespec) -> c_int;
    }
    pub unsafe fn monotonic(ts: *mut super::Timespec) {
        clock_gettime(&_CLOCK_MONOTONIC, ts);
    }
}

use sys::{O_CLOEXEC, O_CREAT, O_RDONLY, O_TRUNC, O_WRONLY};
const SEEK_SET: c_int = 0;
const SEEK_END: c_int = 2;
const EINTR: c_int = 4;

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

/// Standard output, buffered and flushed at each newline (the generated code
/// ends every `print` with `isc_print_newline`) and before a panic.
struct OutBuf {
    data: [u8; 8192],
    len: usize,
}

static mut OUT: OutBuf = OutBuf { data: [0; 8192], len: 0 };

fn write_all(fd: c_int, mut p: *const u8, mut n: usize) {
    while n > 0 {
        let r = unsafe { write(fd, p as *const c_void, n) };
        if r < 0 {
            if errno() == EINTR {
                continue;
            }
            return;
        }
        p = unsafe { p.add(r as usize) };
        n -= r as usize;
    }
}

fn flush_stdout() {
    unsafe {
        let o = &mut *core::ptr::addr_of_mut!(OUT);
        write_all(1, o.data.as_ptr(), o.len);
        o.len = 0;
    }
}

fn out(bytes: &[u8]) {
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
        o.data[o.len..o.len + bytes.len()].copy_from_slice(bytes);
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
    flush_stdout();
    let _ = sem::write_panic(&mut Stderr, line as u32, col as u32, msg);
    let _ = Stderr.write_str("\n");
    unsafe { exit(PANIC_EXIT_CODE) }
}

fn errno() -> c_int {
    unsafe { *__errno_location() }
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
#[cfg(not(target_os = "wasi"))]
fn isc_heap() -> bool {
    use core::sync::atomic::AtomicU8;
    static MODE: AtomicU8 = AtomicU8::new(0);
    match MODE.load(Ordering::Relaxed) {
        1 => false,
        2 => true,
        _ => {
            let v = unsafe { getenv(b"ISC_ALLOC\0".as_ptr() as *const c_char) };
            let isc = v.is_null() || unsafe { c_str(v) } != "glibc";
            MODE.store(if isc { 2 } else { 1 }, Ordering::Relaxed);
            isc
        }
    }
}

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

/// Allocates `bytes` bytes aligned on 64 bytes. Never returns null.
#[no_mangle]
pub extern "C" fn isc_alloc(bytes: i64) -> *mut u8 {
    alloc_bytes(bytes, false)
}

#[no_mangle]
pub extern "C" fn isc_alloc_zeroed(bytes: i64) -> *mut u8 {
    alloc_bytes(bytes, true)
}

#[no_mangle]
pub extern "C" fn isc_free(p: *mut u8) {
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
        // WASI preview 1 numbering -> Linux numbering, for the errors a read
        // or a write can give; `ENOTCAPABLE` (76, a path outside the
        // directories given to the engine) is shown as `EACCES`
        let linux = match code {
            2 | 76 => 13,
            8 => 9,
            20 => 17,
            22 => 27,
            27 => 4,
            28 => 22,
            29 => 5,
            31 => 21,
            32 => 40,
            33 => 24,
            37 => 36,
            41 => 23,
            44 => 2,
            48 => 12,
            51 => 28,
            54 => 20,
            63 => 1,
            69 => 30,
            70 => 29,
            c => c,
        };
        let text = if code == 76 { unsafe { c_str(strerror(2)) } } else { unsafe { c_str(strerror(code)) } };
        (text, linux)
    }
    #[cfg(not(target_os = "wasi"))]
    (unsafe { c_str(strerror(code)) }, code)
}

/// Reads a whole file into a 64-byte aligned buffer, like `std::fs::read`.
/// Reads a file; an error instead of a panic (D138): the buffer freed and
/// the file closed.
unsafe fn try_read_file(path: &str) -> Result<IscArray, Panic<'_>> {
    let cp = c_path(path);
    let fd = open(cp, O_RDONLY | O_CLOEXEC);
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
        return Err(open_error(errno()));
    }
    // the size is a hint; the file is read until end of file
    let end = lseek(fd, 0, SEEK_END);
    lseek(fd, 0, SEEK_SET);
    let mut cap: usize = if end > 0 && end < (1i64 << 44) { end as usize } else { 1 << 16 };
    let mut buf = isc_alloc(cap as i64);
    let mut len: usize = 0;
    loop {
        if len == cap {
            // probe for end of file before growing
            let mut probe = [0u8; 64];
            let r = read(fd, probe.as_mut_ptr() as *mut c_void, probe.len());
            if r < 0 {
                let e = errno();
                if e == EINTR {
                    continue;
                }
                isc_free(buf);
                close(fd);
                return Err(read_error(e));
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
            let e = errno();
            if e == EINTR {
                continue;
            }
            isc_free(buf);
            close(fd);
            return Err(read_error(e));
        }
        if r == 0 {
            break;
        }
        len += r as usize;
    }
    close(fd);
    Ok(IscArray { ptr: buf, len: len as i64 })
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
        return Err(error(errno()));
    }
    let mut chunk = [0u8; 8192];
    let mut done = 0usize;
    while done < total {
        let n = fill(done, &mut chunk);
        let mut off = 0usize;
        while off < n {
            let r = write(fd, chunk.as_ptr().add(off) as *const c_void, n - off);
            if r < 0 {
                let e = errno();
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
    if close(fd) < 0 {
        return Err(error(errno()));
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
    let _ = write!(Stdout, "{}", v);
}

#[no_mangle]
pub extern "C" fn isc_print_u64(v: u64) {
    let _ = write!(Stdout, "{}", v);
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
    flush_stdout();
}

/// `printBytes(x)` (D136): the bytes, in the buffer of `print`.
#[no_mangle]
pub unsafe extern "C" fn isc_print_bytes(p: *const u8, n: i64) {
    out(bytes_of(p, n));
}

#[no_mangle]
pub extern "C" fn isc_print_newline() {
    out(b"\n");
    flush_stdout();
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
    match sem::parse_i64(s) {
        Some(v) => v,
        None => die(line, col, &Panic::ParseI64(s)),
    }
}

/// The program arguments, without the program name, as `[k]string`: each
/// a copy, owned by the array (a string is freed as an array, D134).
#[no_mangle]
pub unsafe extern "C" fn isc_args(out: *mut IscArray, argc: i32, argv: *const *const u8) {
    let n = (argc.max(1) - 1) as usize;
    let p = isc_alloc((n * core::mem::size_of::<IscStr>()) as i64) as *mut IscStr;
    for i in 0..n {
        let s = *argv.add(i + 1);
        let len = strlen(s as *const c_char) as i64;
        *p.add(i) = new_str(core::slice::from_raw_parts(s, len as usize));
    }
    *out = IscArray { ptr: p as *mut u8, len: n as i64 };
}

// ---------------------------------------------------------------------
// strings (D134): a string is `{ptr, len}` of UTF-8 bytes, owned as an
// array; each operation returns a new one

/// A copy of a string (or of an array of `u8`: the same layout).
#[no_mangle]
pub unsafe extern "C" fn isc_str_new(out: *mut IscStr, p: *const u8, n: i64) {
    *out = new_str(bytes_of(p, n));
}

/// A new string holding these bytes.
fn new_str(b: &[u8]) -> IscStr {
    let q = isc_alloc(b.len() as i64);
    unsafe { core::ptr::copy_nonoverlapping(b.as_ptr(), q, b.len()) };
    IscStr { ptr: q, len: b.len() as i64 }
}

unsafe fn bytes_of<'a>(p: *const u8, n: i64) -> &'a [u8] {
    if n <= 0 {
        &[]
    } else {
        core::slice::from_raw_parts(p, n as usize)
    }
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
    if let Err(e) = core::str::from_utf8(b) {
        die(line, col, &Panic::NotUtf8(e.valid_up_to() as i64))
    }
    *out = new_str(b);
}

/// A writer into a small buffer (the text of a scalar).
struct Small {
    buf: [u8; 64],
    len: usize,
}

impl Write for Small {
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
    let mut s = Small { buf: [0; 64], len: 0 };
    let _ = write!(s, "{}", v);
    *out = new_str(&s.buf[..s.len]);
}

#[no_mangle]
pub unsafe extern "C" fn isc_to_string_u64(out: *mut IscStr, v: u64) {
    let mut s = Small { buf: [0; 64], len: 0 };
    let _ = write!(s, "{}", v);
    *out = new_str(&s.buf[..s.len]);
}

#[no_mangle]
pub unsafe extern "C" fn isc_to_string_f64(out: *mut IscStr, v: f64) {
    let mut s = Small { buf: [0; 64], len: 0 };
    let _ = sem::write_f64(&mut s, v);
    *out = new_str(&s.buf[..s.len]);
}

#[no_mangle]
pub unsafe extern "C" fn isc_to_string_f32(out: *mut IscStr, v: f32) {
    let mut s = Small { buf: [0; 64], len: 0 };
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
    if let Err(e) = core::str::from_utf8(bytes_of(a.ptr, a.len)) {
        die(line, col, &Panic::TextNotUtf8 { path: p, at: e.valid_up_to() as i64 })
    }
    *out = IscStr { ptr: a.ptr, len: a.len };
}

#[no_mangle]
pub unsafe extern "C" fn isc_write_text(path: *const u8, plen: i64, data: *const u8, n: i64, line: i32, col: i32) {
    isc_write_bytes(path, plen, data, n, line, col);
}

/// An array of new strings, one per item.
unsafe fn str_array<'a>(items: &mut dyn Iterator<Item = &'a str>, count: usize) -> IscArray {
    let q = isc_alloc((count * core::mem::size_of::<IscStr>()) as i64) as *mut IscStr;
    for (i, s) in items.enumerate().take(count) {
        *q.add(i) = new_str(s.as_bytes());
    }
    IscArray { ptr: q as *mut u8, len: count as i64 }
}

#[no_mangle]
pub unsafe extern "C" fn isc_lines(out: *mut IscArray, p: *const u8, n: i64) {
    let s = core::str::from_utf8_unchecked(bytes_of(p, n));
    let count = sem::text_lines(s).count();
    *out = str_array(&mut sem::text_lines(s), count);
}

#[no_mangle]
pub unsafe extern "C" fn isc_split(out: *mut IscArray, p: *const u8, n: i64, q: *const u8, m: i64, line: i32, col: i32) {
    let s = core::str::from_utf8_unchecked(bytes_of(p, n));
    let sep = core::str::from_utf8_unchecked(bytes_of(q, m));
    if sep.is_empty() {
        die(line, col, &Panic::SplitEmpty)
    }
    let count = s.split(sep).count();
    *out = str_array(&mut s.split(sep), count);
}

#[no_mangle]
pub unsafe extern "C" fn isc_trim(out: *mut IscStr, p: *const u8, n: i64) {
    let s = core::str::from_utf8_unchecked(bytes_of(p, n));
    *out = new_str(s.trim().as_bytes());
}

/// `join(xs, sep)`: the strings of `xs` with `sep` between them.
#[no_mangle]
pub unsafe extern "C" fn isc_join(out: *mut IscStr, xs: *const IscStr, n: i64, q: *const u8, m: i64) {
    let n = n.max(0) as usize;
    let m = m.max(0) as usize;
    let mut total = 0usize;
    for i in 0..n {
        total += (*xs.add(i)).len.max(0) as usize;
    }
    total += m * n.saturating_sub(1);
    let r = isc_alloc(total as i64);
    let mut at = 0usize;
    for i in 0..n {
        if i > 0 {
            core::ptr::copy_nonoverlapping(q, r.add(at), m);
            at += m;
        }
        let x = *xs.add(i);
        let l = x.len.max(0) as usize;
        core::ptr::copy_nonoverlapping(x.ptr, r.add(at), l);
        at += l;
    }
    *out = IscStr { ptr: r, len: total as i64 };
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
    if let Err(e) = core::str::from_utf8(bytes_of(a.ptr, a.len)) {
        isc_free(a.ptr);
        return Err(Panic::TextNotUtf8 { path: p, at: e.valid_up_to() as i64 });
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

use core::sync::atomic::{AtomicI64, AtomicU32, AtomicUsize, Ordering};

type Task = extern "C" fn(ctx: *mut c_void, i: i64);

extern "C" {
    fn getenv(name: *const c_char) -> *const c_char;
}

// `pthread_t` is an `unsigned long` on Linux, a pointer in wasi-libc: one
// machine word in both cases.
#[cfg(any(not(target_os = "wasi"), isc_wasm_threads))]
extern "C" {
    fn pthread_create(tid: *mut usize, attr: *const c_void, f: extern "C" fn(*mut c_void) -> *mut c_void, arg: *mut c_void) -> c_int;
}

#[cfg(not(target_os = "wasi"))]
extern "C" {
    fn syscall(n: i64, ...) -> i64;
    fn sched_getaffinity(pid: c_int, size: usize, mask: *mut u8) -> c_int;
}

/// Sleeps while `GEN` is `seen` (spurious wake-ups are allowed).
#[cfg(not(target_os = "wasi"))]
fn wait_gen(seen: u32) {
    unsafe { syscall(SYS_FUTEX, GEN.gen.as_ptr(), FUTEX_WAIT_PRIVATE, seen, core::ptr::null::<c_void>()) };
}

/// Wakes every thread sleeping in `wait_gen`.
#[cfg(not(target_os = "wasi"))]
fn wake_all() {
    unsafe { syscall(SYS_FUTEX, GEN.gen.as_ptr(), FUTEX_WAKE_PRIVATE, i32::MAX) };
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
#[cfg(not(target_os = "wasi"))]
fn default_threads() -> usize {
    let mut mask = [0u8; 128];
    if unsafe { sched_getaffinity(0, mask.len(), mask.as_mut_ptr()) } == 0 {
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

const SYS_FUTEX: i64 = 202;
const FUTEX_WAIT_PRIVATE: c_int = 128;
const FUTEX_WAKE_PRIVATE: c_int = 129;
const SPIN: u32 = 1 << 16;

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
/// line of its own: the threads take it in turn while they work.
#[repr(C, align(128))]
pub struct NextLine {
    next: AtomicI64,
    dynamic: core::sync::atomic::AtomicBool,
}

static NEXT: NextLine = NextLine { next: AtomicI64::new(0), dynamic: core::sync::atomic::AtomicBool::new(false) };

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
        let mut tid = 0usize;
        unsafe { pthread_create(&mut tid, core::ptr::null(), worker, k as *mut c_void) };
    }
    JOB.threads.store(n, Ordering::Release);
    n
}

fn run_share(k: usize, threads: usize) {
    let task: Task = unsafe { core::mem::transmute(JOB.task.load(Ordering::Acquire)) };
    let ctx = JOB.ctx.load(Ordering::Acquire) as *mut c_void;
    let n = JOB.n.load(Ordering::Acquire);
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
        run_share(k, JOB.threads.load(Ordering::Acquire));
        JOB.finished.fetch_add(1, Ordering::AcqRel);
    }
}

/// `isc_parallel_for` whose tasks are taken in turn by the threads as they
/// finish the one before (D145): for tasks of very different costs. Each
/// task writes only its own results, so they do not depend on which thread
/// runs it; no task is taken twice.
#[no_mangle]
pub extern "C" fn isc_parallel_for_dynamic(n: i64, task: Task, ctx: *mut c_void, ctx_bytes: i64) {
    parallel_for(n, task, ctx, ctx_bytes, true)
}

/// Runs `task(ctx, i)` for `i` in `0..n` on the thread pool; returns when all
/// tasks are done. `ctx_bytes` is the size of the context. Must not be called
/// from inside a task.
#[no_mangle]
pub extern "C" fn isc_parallel_for(n: i64, task: Task, ctx: *mut c_void, ctx_bytes: i64) {
    parallel_for(n, task, ctx, ctx_bytes, false)
}

fn parallel_for(n: i64, task: Task, ctx: *mut c_void, ctx_bytes: i64, dynamic: bool) {
    if n <= 0 {
        return;
    }
    let threads = if n == 1 { 1 } else { thread_count() };
    // a parallel loop inside a task (a recursive function called by a
    // task): the job line is taken, so its tasks run here, in order; the
    // results do not depend on the number of threads (D102)
    if threads == 1 || BUSY.swap(true, Ordering::Acquire) {
        for i in 0..n {
            task(ctx, i);
        }
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
    NEXT.dynamic.store(dynamic, Ordering::Relaxed);
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
    while JOB.finished.load(Ordering::Acquire) != threads - 1 {
        core::hint::spin_loop();
    }
    BUSY.store(false, Ordering::Release);
}

/// A parallel loop is running (`isc_parallel_for`).
static BUSY: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// The lowest address of the stack of the calling thread that a recursive
/// function may reach, with a margin for the frames that do not test it
/// (the scratch of a `reduce`, the runtime, the panic itself); 0 when it is
/// not known (no test). The generated code keeps it in a thread-local
/// variable and stops on `isc_panic_stack` below it (D102).
#[cfg(not(target_os = "wasi"))]
#[no_mangle]
pub extern "C" fn isc_stack_floor() -> i64 {
    extern "C" {
        fn pthread_self() -> usize;
        fn pthread_getattr_np(t: usize, attr: *mut c_void) -> c_int;
        fn pthread_attr_getstack(attr: *const c_void, addr: *mut *mut c_void, size: *mut usize) -> c_int;
        fn pthread_attr_destroy(attr: *mut c_void) -> c_int;
    }
    const MARGIN: usize = 256 << 10;
    // pthread_attr_t: 56 bytes with glibc on x86-64
    let mut attr = [0u64; 16];
    let a = attr.as_mut_ptr() as *mut c_void;
    let mut addr: *mut c_void = core::ptr::null_mut();
    let mut size = 0usize;
    unsafe {
        if pthread_getattr_np(pthread_self(), a) != 0 {
            return 0;
        }
        let ok = pthread_attr_getstack(a, &mut addr, &mut size) == 0;
        pthread_attr_destroy(a);
        if !ok || size <= 2 * MARGIN {
            return 0;
        }
    }
    (addr as usize + MARGIN) as i64
}

/// Runs `f(argc, argv)` on a thread with a stack of `BIG_STACK` bytes and
/// returns its result: the body of `main` of a program with recursive
/// functions (D102), whose depth is then limited by this stack rather than
/// by the 8 MiB of the main thread. Runs it here if the thread cannot be
/// created.
#[cfg(not(target_os = "wasi"))]
#[no_mangle]
pub extern "C" fn isc_run_big_stack(f: extern "C" fn(c_int, *mut c_void) -> c_int, argc: c_int, argv: *mut c_void) -> c_int {
    const BIG_STACK: usize = 1 << 30;
    extern "C" {
        fn pthread_attr_init(attr: *mut c_void) -> c_int;
        fn pthread_attr_setstacksize(attr: *mut c_void, size: usize) -> c_int;
        fn pthread_attr_destroy(attr: *mut c_void) -> c_int;
        fn pthread_join(t: usize, ret: *mut *mut c_void) -> c_int;
    }
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
    let mut attr = [0u64; 16];
    let a = attr.as_mut_ptr() as *mut c_void;
    let mut tid = 0usize;
    let started = unsafe {
        pthread_attr_init(a) == 0
            && pthread_attr_setstacksize(a, BIG_STACK) == 0
            && pthread_create(&mut tid, a, run, &mut s as *mut Start as *mut c_void) == 0
    };
    unsafe { pthread_attr_destroy(a) };
    if !started {
        return f(argc, argv);
    }
    unsafe { pthread_join(tid, core::ptr::null_mut()) };
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
