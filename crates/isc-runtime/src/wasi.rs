//! The system under WASI (preview 1) without wasi-libc (D543): the
//! functions of the C library that the runtime called, with the same names
//! and meanings as those of `linux`, over the functions of WASI itself (the
//! imports of `wasi_snapshot_preview1`). A failed call gives the negative
//! of its code, numbered as WASI numbers them (`ENOENT` is 44); paths are
//! resolved against the directories the engine opened for the program (its
//! preopens), as wasi-libc resolves them; the blocks of memory come from a
//! heap of the runtime over `memory.grow`. A program that declares C
//! functions keeps wasi-libc for its C code (its start then is wasi-libc's).

use core::ffi::{c_char, c_int, c_void};
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[repr(C)]
struct Iovec {
    buf: *const u8,
    len: usize,
}

/// `filestat` of WASI preview 1.
#[repr(C)]
#[derive(Default)]
pub struct Filestat {
    pub dev: u64,
    pub ino: u64,
    pub filetype: u8,
    pub nlink: u64,
    pub size: u64,
    pub atim: u64,
    pub mtim: u64,
    pub ctim: u64,
}

/// `fdstat` of WASI preview 1.
#[repr(C)]
#[derive(Default)]
struct Fdstat {
    filetype: u8,
    flags: u16,
    rights_base: u64,
    rights_inheriting: u64,
}

#[repr(C)]
struct Prestat {
    tag: u8,
    len: usize,
}

#[link(wasm_import_module = "wasi_snapshot_preview1")]
extern "C" {
    fn args_sizes_get(argc: *mut usize, size: *mut usize) -> u16;
    fn args_get(argv: *mut *mut u8, buf: *mut u8) -> u16;
    fn environ_sizes_get(n: *mut usize, size: *mut usize) -> u16;
    fn environ_get(env: *mut *mut u8, buf: *mut u8) -> u16;
    fn clock_time_get(id: u32, precision: u64, time: *mut u64) -> u16;
    fn fd_close(fd: u32) -> u16;
    fn fd_read(fd: u32, iovs: *const Iovec, n: usize, done: *mut usize) -> u16;
    fn fd_write(fd: u32, iovs: *const Iovec, n: usize, done: *mut usize) -> u16;
    fn fd_seek(fd: u32, offset: i64, whence: u8, new: *mut u64) -> u16;
    fn fd_fdstat_get(fd: u32, out: *mut Fdstat) -> u16;
    fn fd_prestat_get(fd: u32, out: *mut Prestat) -> u16;
    fn fd_prestat_dir_name(fd: u32, path: *mut u8, len: usize) -> u16;
    fn fd_readdir(fd: u32, buf: *mut u8, len: usize, cookie: u64, used: *mut usize) -> u16;
    fn path_open(fd: u32, dirflags: u32, path: *const u8, len: usize, oflags: u16, base: u64, inheriting: u64, fdflags: u16, opened: *mut u32) -> u16;
    fn path_filestat_get(fd: u32, flags: u32, path: *const u8, len: usize, out: *mut Filestat) -> u16;
    fn path_create_directory(fd: u32, path: *const u8, len: usize) -> u16;
    fn path_remove_directory(fd: u32, path: *const u8, len: usize) -> u16;
    fn path_unlink_file(fd: u32, path: *const u8, len: usize) -> u16;
    fn path_rename(fd: u32, old: *const u8, old_len: usize, new_fd: u32, new: *const u8, new_len: usize) -> u16;
    fn proc_exit(code: u32) -> !;
}

/// The code of WASI of a path outside the directories given to the engine.
const ENOTCAPABLE: u16 = 76;

// ---------------------------------------------------------------------
// the functions of memory that LLVM and `core` call

/// # Safety
/// `s` is a string ending with a zero byte.
#[no_mangle]
pub unsafe extern "C" fn strlen(s: *const c_char) -> usize {
    let mut n = 0;
    // volatile reads: LLVM does not make the loop a call of `strlen` itself
    // SAFETY: the bytes up to the zero one are readable (the contract)
    while unsafe { (s as *const u8).add(n).read_volatile() } != 0 {
        n += 1;
    }
    n
}

/// # Safety
/// `n` bytes readable at `a` and at `b`.
#[no_mangle]
pub unsafe extern "C" fn memcmp(a: *const u8, b: *const u8, n: usize) -> c_int {
    let mut i = 0;
    // 8 bytes at a time while they are the same, then the first that differs
    // SAFETY: the reads stay within the `n` bytes (the contract)
    unsafe {
        while i + 8 <= n && (a.add(i) as *const u64).read_unaligned() == (b.add(i) as *const u64).read_unaligned() {
            i += 8;
        }
        while i < n {
            let (x, y) = (a.add(i).read_volatile(), b.add(i).read_volatile());
            if x != y {
                return x as c_int - y as c_int;
            }
            i += 1;
        }
    }
    0
}

/// # Safety
/// `n` bytes readable at `a` and at `b`.
#[no_mangle]
pub unsafe extern "C" fn bcmp(a: *const u8, b: *const u8, n: usize) -> c_int {
    // SAFETY: the same contract
    unsafe { memcmp(a, b, n) }
}

// ---------------------------------------------------------------------
// the heap: blocks of the classes of sizes of the runtime, each 64 bytes
// after the start of its cell (the class in the 8 bytes before the block),
// from the end of the memory, which grows by pages of 64 KiB; a freed block
// kept for its class (the memory of WebAssembly never shrinks)

const PAGE: usize = 64 << 10;
static HEAP_LOCK: AtomicBool = AtomicBool::new(false);
/// The next free byte of the memory (0: not known yet) and its end.
static TOP: AtomicUsize = AtomicUsize::new(0);
static END: AtomicUsize = AtomicUsize::new(0);
static FREE: [AtomicUsize; isc_runtime_core::ALLOC_CLASSES] = [const { AtomicUsize::new(0) }; isc_runtime_core::ALLOC_CLASSES];

fn lock() {
    while HEAP_LOCK.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
        core::hint::spin_loop();
    }
}

fn unlock() {
    HEAP_LOCK.store(false, Ordering::Release);
}

/// The program started at `isc_wasi_start`: the memory after the data
/// (`__heap_base`) is the runtime's; under wasi-libc's start it is that of
/// wasi-libc's allocator, and the runtime's blocks come from new pages only.
static OWN_START: AtomicBool = AtomicBool::new(false);

/// `n` bytes at a multiple of 64, from the free end of the runtime's
/// memory, which grows if needed (from the new pages when the memory grew
/// for another allocator in between); 0 when it cannot.
fn bump(n: usize) -> usize {
    extern "C" {
        static __heap_base: u8;
    }
    let mut top = TOP.load(Ordering::Relaxed);
    let mut end = END.load(Ordering::Relaxed);
    if top == 0 && OWN_START.load(Ordering::Relaxed) {
        // SAFETY: the address of the symbol that the linker puts after the
        // data and the stack
        top = unsafe { core::ptr::addr_of!(__heap_base) } as usize;
        end = core::arch::wasm32::memory_size::<0>() * PAGE;
    }
    let mut start = top.next_multiple_of(64);
    if top == 0 || start.checked_add(n).map(|stop| stop > end).unwrap_or(true) {
        let pages = (n + 64).div_ceil(PAGE);
        let old = core::arch::wasm32::memory_grow::<0>(pages);
        if old == usize::MAX {
            return 0;
        }
        let base = old * PAGE;
        if top == 0 || base != end {
            start = base;
        }
        end = base + pages * PAGE;
    }
    TOP.store(start + n, Ordering::Relaxed);
    END.store(end, Ordering::Relaxed);
    start
}

/// A block of `size` bytes at a multiple of 64 (`align` at most 64), or null.
pub unsafe fn aligned_alloc(align: usize, size: usize) -> *mut c_void {
    let Some((class, bytes)) = isc_runtime_core::alloc_class(size.max(1)) else { return core::ptr::null_mut() };
    if align > 64 {
        return core::ptr::null_mut();
    }
    lock();
    let free = FREE[class].load(Ordering::Relaxed);
    let p = if free != 0 {
        // SAFETY: a freed block of this class, its first word the next one
        FREE[class].store(unsafe { *(free as *const usize) }, Ordering::Relaxed);
        free
    } else {
        let cell = bump(64 + bytes);
        if cell == 0 {
            0
        } else {
            // SAFETY: the 8 bytes before the block, in its cell
            unsafe { ((cell + 56) as *mut usize).write(class) };
            cell + 64
        }
    };
    unlock();
    p as *mut c_void
}

/// Frees a block of `aligned_alloc` (null: nothing).
pub unsafe fn free(p: *mut c_void) {
    if p.is_null() {
        return;
    }
    // SAFETY: a block of `aligned_alloc`, its class before it
    let class = unsafe { ((p as usize - 8) as *const usize).read() };
    lock();
    // SAFETY: the first word of the block, which is no longer used
    unsafe { (p as *mut usize).write(FREE[class].load(Ordering::Relaxed)) };
    FREE[class].store(p as usize, Ordering::Relaxed);
    unlock();
}

// ---------------------------------------------------------------------
// errors

/// The number of Linux of a code of WASI preview 1, for its text (that of
/// glibc: the same messages on every target); `ENOTCAPABLE` (a path outside
/// the directories given to the engine) as `EACCES`.
pub fn linux_code(code: c_int) -> c_int {
    match code {
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
        55 => 39,
        63 => 1,
        69 => 30,
        70 => 29,
        c => c,
    }
}

/// The text of the code of WASI `code`.
pub unsafe fn strerror(code: c_int) -> *const c_char {
    isc_runtime_core::oserr::text_c(linux_code(code)).as_ptr() as *const c_char
}

// ---------------------------------------------------------------------
// the start, the arguments, the environment, the end

/// The start of a command of WASI without wasi-libc: the constructors the
/// linker gathers, the generated `main` with the arguments, then the end of
/// the process with its code.
#[no_mangle]
pub extern "C" fn isc_wasi_start() {
    extern "C" {
        fn __wasm_call_ctors();
        fn __main_argc_argv(argc: c_int, argv: *const *const u8) -> c_int;
    }
    OWN_START.store(true, Ordering::Relaxed);
    // SAFETY: the constructors, once, before anything else
    unsafe { __wasm_call_ctors() };
    let (argc, argv) = strings(args_sizes_get, args_get);
    // SAFETY: the `main` of the generated code and its arguments
    exit(unsafe { __main_argc_argv(argc as c_int, argv) })
}

/// The strings that `sizes` and `get` of WASI give (the arguments or the
/// environment): their count and an array of them ending with a null, in
/// one block (never freed).
fn strings(sizes: unsafe extern "C" fn(*mut usize, *mut usize) -> u16, get: unsafe extern "C" fn(*mut *mut u8, *mut u8) -> u16) -> (usize, *const *const u8) {
    let (mut n, mut size) = (0usize, 0usize);
    // SAFETY: two places for the counts; then a block for the array and
    // the bytes, of the sizes WASI gave
    unsafe {
        if sizes(&mut n, &mut size) != 0 {
            n = 0;
            size = 0;
        }
        let ptrs = (n + 1) * core::mem::size_of::<usize>();
        let block = aligned_alloc(64, ptrs + size) as *mut u8;
        if block.is_null() {
            exit(70);
        }
        let array = block as *mut *mut u8;
        if n > 0 && get(array, block.add(ptrs)) != 0 {
            n = 0;
        }
        *array.add(n) = core::ptr::null_mut();
        (n, array as *const *const u8)
    }
}

/// The environment (`NAME=value` strings ending with a zero byte, the array
/// ending with a null), read once.
static ENVIRON: AtomicUsize = AtomicUsize::new(0);

pub fn environ() -> *const *const u8 {
    let e = ENVIRON.load(Ordering::Acquire);
    if e != 0 {
        return e as *const *const u8;
    }
    let (_, e) = strings(environ_sizes_get, environ_get);
    // another thread may have read it first: its array is kept
    match ENVIRON.compare_exchange(0, e as usize, Ordering::AcqRel, Ordering::Acquire) {
        Ok(_) => e,
        Err(other) => other as *const *const u8,
    }
}

/// The value of the variable `name` (a string ending with a zero byte), a
/// string ending with a zero byte, or null.
pub unsafe fn getenv(name: *const c_char) -> *const c_char {
    let mut e = environ();
    // SAFETY: `name` ends with a zero byte; `e` the array of the
    // environment, ending with a null, each a string ending with a zero byte
    unsafe {
        let n = strlen(name);
        while !(*e).is_null() {
            let v = *e;
            let mut i = 0;
            while i < n && *v.add(i) == *(name as *const u8).add(i) {
                i += 1;
            }
            if i == n && *v.add(n) == b'=' {
                return v.add(n + 1) as *const c_char;
            }
            e = e.add(1);
        }
    }
    core::ptr::null()
}

/// Ends the process with `code`.
pub fn exit(code: c_int) -> ! {
    // SAFETY: the end of the process
    unsafe { proc_exit(code as u32) }
}

/// Ends the process at once, by a trap (as `abort` of wasi-libc).
pub fn abort() -> ! {
    core::arch::wasm32::unreachable()
}

/// The clock `id` of WASI (0 the real time, 1 the monotonic one), in
/// nanoseconds.
pub fn clock(id: u32) -> u64 {
    let mut t = 0u64;
    // SAFETY: a place for the time
    unsafe { clock_time_get(id, 1, &mut t) };
    t
}

/// The monotonic clock, in seconds and nanoseconds.
pub unsafe fn monotonic(ts: *mut super::Timespec) {
    let t = clock(1);
    // SAFETY: a `Timespec` (the caller's)
    unsafe {
        (*ts).tv_sec = (t / 1_000_000_000) as i64;
        (*ts).tv_nsec = (t % 1_000_000_000) as core::ffi::c_long;
    }
}

// ---------------------------------------------------------------------
// paths

/// The directories the engine opened for the program: each one's handle
/// and its name without its leading `/` and `./` (as wasi-libc keeps them),
/// read at the first path, in one block: `[count, (fd, address, length)...]`.
static PREOPENS: AtomicUsize = AtomicUsize::new(0);

fn preopens() -> &'static [[usize; 3]] {
    let p = PREOPENS.load(Ordering::Acquire);
    if p != 0 {
        // SAFETY: a block made below, never freed nor changed
        return unsafe { core::slice::from_raw_parts((p + 16) as *const [usize; 3], *(p as *const usize)) };
    }
    let mut v: alloc::vec::Vec<[usize; 3]> = alloc::vec::Vec::new();
    let mut fd = 3u32;
    loop {
        let mut st = Prestat { tag: 1, len: 0 };
        // SAFETY: a place for the description of the handle `fd`
        if unsafe { fd_prestat_get(fd, &mut st) } != 0 {
            break;
        }
        if st.tag == 0 {
            // SAFETY: a block of the length WASI gave, then its name in it
            let name = unsafe { aligned_alloc(64, st.len.max(1)) } as *mut u8;
            if !name.is_null() && unsafe { fd_prestat_dir_name(fd, name, st.len) } == 0 {
                // SAFETY: the bytes just written
                let full = unsafe { core::slice::from_raw_parts(name, st.len) };
                let full = &full[..full.iter().position(|&b| b == 0).unwrap_or(full.len())];
                let mut s = full;
                loop {
                    if let Some(r) = s.strip_prefix(b"/") {
                        s = r;
                    } else if let Some(r) = s.strip_prefix(b"./") {
                        s = r;
                    } else if s == b"." {
                        s = b"";
                    } else {
                        break;
                    }
                }
                v.push([fd as usize, s.as_ptr() as usize, s.len()]);
            }
        }
        fd += 1;
    }
    let bytes = 16 + v.len() * core::mem::size_of::<[usize; 3]>();
    // SAFETY: a block for the count and the table, written once
    unsafe {
        let b = aligned_alloc(64, bytes) as usize;
        if b == 0 {
            return &[];
        }
        *(b as *mut usize) = v.len();
        core::ptr::copy_nonoverlapping(v.as_ptr(), (b + 16) as *mut [usize; 3], v.len());
        match PREOPENS.compare_exchange(0, b, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => core::slice::from_raw_parts((b + 16) as *const [usize; 3], v.len()),
            Err(other) => {
                free(b as *mut c_void);
                core::slice::from_raw_parts((other + 16) as *const [usize; 3], *(other as *const usize))
            }
        }
    }
}

/// The directory handle and the path relative to it of `path`, by the rules
/// of wasi-libc: the current directory is `/`, so that a relative path is
/// read from it; the leading `/` set aside, the longest name of a preopen
/// that starts the path at a boundary of its parts (an empty name any
/// path), the rest after it without its leading `/`, `.` when nothing is
/// left. `ENOTCAPABLE` when no preopen holds it.
pub fn resolve(path: &[u8]) -> Result<(u32, &[u8]), u16> {
    let mut p = path;
    while let Some(r) = p.strip_prefix(b"/") {
        p = r;
    }
    let mut best: Option<(u32, usize)> = None;
    for &[fd, at, len] in preopens().iter().rev() {
        // SAFETY: a name of the table of the preopens
        let name = unsafe { core::slice::from_raw_parts(at as *const u8, len) };
        let fits = len == 0 || (p.starts_with(name) && (p.len() == len || p[len] == b'/' || name.ends_with(b"/")));
        if fits && best.map(|(_, l)| len > l).unwrap_or(true) {
            best = Some((fd as u32, len));
        }
    }
    let (fd, len) = best.ok_or(ENOTCAPABLE)?;
    let mut rest = &p[len..];
    while let Some(r) = rest.strip_prefix(b"/") {
        rest = r;
    }
    Ok((fd, if rest.is_empty() { b"." } else { rest }))
}

/// The bytes of the path at `path` (ending with a zero byte).
///
/// # Safety
/// `path` ends with a zero byte.
unsafe fn c_path<'a>(path: *const c_char) -> &'a [u8] {
    // SAFETY: see above
    unsafe { core::slice::from_raw_parts(path as *const u8, strlen(path)) }
}

// ---------------------------------------------------------------------
// files

pub const O_RDONLY: c_int = 0;
pub const O_WRONLY: c_int = 1;
pub const O_CREAT: c_int = 0o100;
pub const O_TRUNC: c_int = 0o1000;
pub const O_CLOEXEC: c_int = 0;
pub const O_APPEND: c_int = 0o2000;
pub const O_DIRECTORY: c_int = 0o200000;

const OFLAGS_CREAT: u16 = 1;
const OFLAGS_DIRECTORY: u16 = 2;
const OFLAGS_TRUNC: u16 = 8;
const FDFLAGS_APPEND: u16 = 1;
const LOOKUP_SYMLINK_FOLLOW: u32 = 1;
/// Every right of WASI preview 1 (30 bits, from `fd_datasync` to
/// `sock_accept`; an engine refuses others).
const RIGHTS: u64 = (1 << 30) - 1;
/// The rights to read (`fd_read`, `fd_readdir`) and to write
/// (`fd_datasync`, `fd_write`, `fd_allocate`, `fd_filestat_set_size`).
const RIGHTS_READ: u64 = 1 << 1 | 1 << 14;
const RIGHTS_WRITE: u64 = 1 | 1 << 6 | 1 << 8 | 1 << 22;

/// Opens the file named by the UTF-8 bytes `path` (ending with a zero byte)
/// with the flags of Linux above: its handle, or the negative of a code.
pub unsafe fn open(path: *const c_char, flags: c_int, _mode: c_int) -> c_int {
    // SAFETY: the contract
    let path = unsafe { c_path(path) };
    match open_at(path, flags) {
        Ok(fd) => fd as c_int,
        Err(e) => -(e as c_int),
    }
}

fn open_at(path: &[u8], flags: c_int) -> Result<u32, u16> {
    let (dir, rel) = resolve(path)?;
    let mut oflags = 0;
    if flags & O_CREAT != 0 {
        oflags |= OFLAGS_CREAT;
    }
    if flags & O_TRUNC != 0 {
        oflags |= OFLAGS_TRUNC;
    }
    if flags & O_DIRECTORY != 0 {
        oflags |= OFLAGS_DIRECTORY;
    }
    let fdflags = if flags & O_APPEND != 0 { FDFLAGS_APPEND } else { 0 };
    // the rights as wasi-libc asks them: all but those that contradict the
    // way the file is opened (an engine refuses to open a directory with
    // those of writing), within those its directory passes on
    let mode = if flags & O_WRONLY != 0 { RIGHTS_WRITE } else { RIGHTS_READ };
    let mut st = Fdstat::default();
    // SAFETY: a place for the status of the directory
    let inheriting = if unsafe { fd_fdstat_get(dir, &mut st) } == 0 { st.rights_inheriting } else { RIGHTS };
    let base = (RIGHTS & !(RIGHTS_READ | RIGHTS_WRITE) | mode) & inheriting;
    let mut fd = 0u32;
    // SAFETY: a path and its length, a place for the handle
    match unsafe { path_open(dir, LOOKUP_SYMLINK_FOLLOW, rel.as_ptr(), rel.len(), oflags, base, inheriting, fdflags, &mut fd) } {
        0 => Ok(fd),
        e => Err(e),
    }
}

pub unsafe fn read(fd: c_int, buf: *mut c_void, n: usize) -> isize {
    let v = Iovec { buf: buf as *const u8, len: n };
    let mut done = 0usize;
    // SAFETY: `n` bytes to write at `buf` (the caller's), a place for the count
    match unsafe { fd_read(fd as u32, &v, 1, &mut done) } {
        0 => done as isize,
        e => -(e as isize),
    }
}

pub unsafe fn write(fd: c_int, buf: *const c_void, n: usize) -> isize {
    let v = Iovec { buf: buf as *const u8, len: n };
    let mut done = 0usize;
    // SAFETY: `n` bytes to read at `buf` (the caller's), a place for the count
    match unsafe { fd_write(fd as u32, &v, 1, &mut done) } {
        0 => done as isize,
        e => -(e as isize),
    }
}

pub unsafe fn close(fd: c_int) -> c_int {
    // SAFETY: a handle of the process
    -(unsafe { fd_close(fd as u32) } as c_int)
}

pub unsafe fn lseek(fd: c_int, off: i64, whence: c_int) -> i64 {
    let mut at = 0u64;
    // SAFETY: a place for the new offset
    match unsafe { fd_seek(fd as u32, off, whence as u8, &mut at) } {
        0 => at as i64,
        e => -(e as i64),
    }
}

// ---------------------------------------------------------------------
// the module `sys` (the folders, the files by their paths)

/// The status of the file at `path` (its links followed).
pub fn stat(path: &[u8]) -> Result<Filestat, u16> {
    let (dir, rel) = resolve(path)?;
    let mut st = Filestat::default();
    // SAFETY: a path and its length, a place for the status
    match unsafe { path_filestat_get(dir, LOOKUP_SYMLINK_FOLLOW, rel.as_ptr(), rel.len(), &mut st) } {
        0 => Ok(st),
        e => Err(e),
    }
}

/// The names of the entries of the folder at `path`, `.` and `..` among them.
pub fn read_dir(path: &[u8]) -> Result<alloc::vec::Vec<alloc::vec::Vec<u8>>, u16> {
    let fd = open_at(path, O_DIRECTORY)?;
    let mut names = alloc::vec::Vec::new();
    let mut buf = alloc::vec![0u8; 4096];
    let mut cookie = 0u64;
    let r = loop {
        let mut used = 0usize;
        // SAFETY: a buffer and its length, a place for the bytes written
        let e = unsafe { fd_readdir(fd, buf.as_mut_ptr(), buf.len(), cookie, &mut used) };
        if e != 0 {
            break Err(e);
        }
        let mut at = 0;
        let mut whole = true;
        // each entry: the cookie of the next (8), its inode (8), the length
        // of its name (4), its type (1), 3 bytes, then its name
        while at + 24 <= used {
            let next = u64::from_le_bytes(buf[at..at + 8].try_into().unwrap());
            let len = u32::from_le_bytes(buf[at + 16..at + 20].try_into().unwrap()) as usize;
            if at + 24 + len > used {
                // a name cut by the end of the buffer: read again from it,
                // in a buffer that holds it
                whole = false;
                if at == 0 {
                    buf.resize((24 + len).max(2 * buf.len()), 0);
                }
                break;
            }
            names.push(buf[at + 24..at + 24 + len].to_vec());
            cookie = next;
            at += 24 + len;
        }
        if whole && used < buf.len() {
            break Ok(());
        }
    };
    // SAFETY: the handle opened above
    unsafe { fd_close(fd) };
    r.map(|_| names)
}

fn status(e: u16) -> Result<(), u16> {
    if e == 0 {
        Ok(())
    } else {
        Err(e)
    }
}

pub fn mkdir(path: &[u8]) -> Result<(), u16> {
    let (dir, rel) = resolve(path)?;
    // SAFETY: a path and its length
    status(unsafe { path_create_directory(dir, rel.as_ptr(), rel.len()) })
}

pub fn rmdir(path: &[u8]) -> Result<(), u16> {
    let (dir, rel) = resolve(path)?;
    // SAFETY: a path and its length
    status(unsafe { path_remove_directory(dir, rel.as_ptr(), rel.len()) })
}

pub fn unlink(path: &[u8]) -> Result<(), u16> {
    let (dir, rel) = resolve(path)?;
    // SAFETY: a path and its length
    status(unsafe { path_unlink_file(dir, rel.as_ptr(), rel.len()) })
}

pub fn rename(from: &[u8], to: &[u8]) -> Result<(), u16> {
    let (d1, r1) = resolve(from)?;
    let (d2, r2) = resolve(to)?;
    // SAFETY: two paths and their lengths
    status(unsafe { path_rename(d1, r1.as_ptr(), r1.len(), d2, r2.as_ptr(), r2.len()) })
}

// ---------------------------------------------------------------------
// the threads of wasi-threads (D543)

#[cfg(isc_wasm_threads)]
#[link(wasm_import_module = "wasi")]
extern "C" {
    #[link_name = "thread-spawn"]
    fn thread_spawn(start: *mut c_void) -> i32;
}

#[cfg(isc_wasm_threads)]
extern "C" {
    /// The thread-local storage of the calling thread at `p` (made by the
    /// linker: the first values copied, `__tls_base` set).
    fn __wasm_init_tls(p: *mut c_void);
    /// Its size and alignment (the generated module: intrinsics of LLVM).
    fn isc_wasm_tls_size() -> i32;
    fn isc_wasm_tls_align() -> i32;
}

/// What a new thread starts with: the top of its stack first (read by
/// `wasi_thread_start` of the generated module before anything else), its
/// thread-local storage, its function.
#[cfg(isc_wasm_threads)]
#[repr(C)]
struct ThreadStart {
    stack_top: usize,
    tls: *mut c_void,
    f: extern "C" fn(*mut c_void) -> *mut c_void,
    arg: *mut c_void,
}

/// A thread of wasi-threads running `f(arg)` on a stack of `stack` bytes;
/// false when it cannot be had. Its blocks are kept (the threads of the
/// runtime run as long as the process).
#[cfg(isc_wasm_threads)]
pub fn spawn(stack: usize, f: extern "C" fn(*mut c_void) -> *mut c_void, arg: *mut c_void) -> bool {
    // SAFETY: blocks of the heap for the stack, the storage and the start
    unsafe {
        let size = isc_wasm_tls_size().max(1) as usize;
        if isc_wasm_tls_align() > 64 {
            return false;
        }
        let low = aligned_alloc(64, stack) as usize;
        let tls = aligned_alloc(64, size);
        let start = aligned_alloc(64, core::mem::size_of::<ThreadStart>()) as *mut ThreadStart;
        if low == 0 || tls.is_null() || start.is_null() {
            free(low as *mut c_void);
            free(tls);
            free(start as *mut c_void);
            return false;
        }
        start.write(ThreadStart { stack_top: (low + stack) & !15, tls, f, arg });
        if thread_spawn(start as *mut c_void) < 0 {
            free(low as *mut c_void);
            free(tls);
            free(start as *mut c_void);
            return false;
        }
    }
    true
}

/// The start of a thread of `spawn`, called by `wasi_thread_start` once its
/// stack is set: its thread-local storage, then its function.
#[cfg(isc_wasm_threads)]
#[no_mangle]
pub extern "C" fn isc_wasi_thread(_tid: i32, start: *mut c_void) {
    // SAFETY: the block of `spawn`, never freed
    let s = unsafe { &*(start as *const ThreadStart) };
    // SAFETY: the storage of this thread, before any thread-local variable
    unsafe { __wasm_init_tls(s.tls) };
    (s.f)(s.arg);
}

