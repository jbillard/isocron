//! The primitives of the system of the standard library (SPEC section 18,
//! D322): `sysStat`, `sysReadDir`, ... called by the modules shipped inside
//! `iscc`, each returning its value and the code of its error, as the
//! interpreter (`isc_interp::rt::sys`) does: 0 none, 1 not found, 2
//! permission denied, 3 already exists, 4 not empty, 5 refused, 6 timed out,
//! otherwise the error of the system negated (-1 for a path holding a zero
//! byte). Under Linux, the system called directly (D287).

use super::*;

/// An array and the code of an error: the tuple `[T[k], i64]` of a
/// primitive, laid out as the compiled code reads it.
#[repr(C)]
pub struct IscArrayCode {
    pub a: IscArray,
    pub code: i64,
}

/// The code of an error of the kernel of Linux (its `errno`).
#[cfg(target_os = "linux")]
fn code_of(errno: i64) -> i64 {
    match errno {
        2 => 1,
        1 | 13 => 2,
        // an address already in use (`EADDRINUSE`) as Windows says it
        17 | 98 => 3,
        39 => 4,
        111 => 5,
        110 => 6,
        e => -e,
    }
}

/// The result of a call of the kernel as a code (0 when it succeeded).
#[cfg(target_os = "linux")]
pub(crate) fn code_of_call(r: isize) -> i64 {
    if r < 0 {
        code_of(-(r as i64))
    } else {
        0
    }
}

/// The path of `n` bytes at `p` with a zero byte after it; None when it
/// holds one (no file has such a name).
///
/// # Safety
/// `p` and `n` the bytes of a string of the generated code, alive during
/// the call.
#[deny(unsafe_op_in_unsafe_fn)]
unsafe fn zpath(p: *const u8, n: i64) -> Option<alloc::vec::Vec<u8>> {
    // SAFETY: the contract of this function
    let b = unsafe { bytes_of(p, n) };
    if b.contains(&0) {
        return None;
    }
    let mut z = alloc::vec::Vec::with_capacity(b.len() + 1);
    z.extend_from_slice(b);
    z.push(0);
    Some(z)
}

/// A new array of bytes holding `b`.
fn byte_array(b: &[u8]) -> IscArray {
    let q = isc_alloc(b.len().max(1) as i64);
    // SAFETY: `q` a new block of at least `b.len()` bytes, apart from `b`
    unsafe { core::ptr::copy_nonoverlapping(b.as_ptr(), q, b.len()) };
    IscArray { ptr: q, len: b.len() as i64 }
}

/// A new array of the strings `names`, in the order of their bytes, the
/// bytes that are not UTF-8 replaced: each checked by `from_utf8` (its ASCII
/// by words) and copied once, `from_utf8_lossy` (byte by byte) only for one
/// that is not UTF-8 (D400).
fn sorted_strings<B: AsRef<[u8]> + Ord>(mut names: alloc::vec::Vec<B>) -> IscArray {
    // equal names are the same bytes: the order of the stable sort
    names.sort_unstable();
    strings_of(names)
}

/// The entries `name=value` of the environment in the order of the bytes of
/// their names (D423: by the whole entries before, `VAR1=` came before
/// `VAR=`, a byte below `=` after the name), as `strings_of`.
fn sorted_entries<B: AsRef<[u8]> + Ord>(all: alloc::vec::Vec<B>) -> IscArray {
    // the length of each name measured once, not at each comparison (D435:
    // a scan of both names a comparison made `envAll` 1684 -> 2376 ns)
    let mut keyed: alloc::vec::Vec<(usize, B)> = all
        .into_iter()
        .map(|e| {
            let n = e.as_ref().iter().position(|&c| c == b'=').unwrap_or(e.as_ref().len());
            (n, e)
        })
        .collect();
    keyed.sort_unstable_by(|(na, a), (nb, b)| a.as_ref()[..*na].cmp(&b.as_ref()[..*nb]).then_with(|| a.as_ref().cmp(b.as_ref())));
    strings_of(keyed.into_iter().map(|(_, e)| e).collect())
}

/// A new array of the strings `names` in their order, the bytes that are
/// not UTF-8 replaced (D400).
fn strings_of<B: AsRef<[u8]>>(names: alloc::vec::Vec<B>) -> IscArray {
    let q = new_str_array(names.len());
    for (slot, b) in q.iter_mut().zip(names.iter()) {
        let b = b.as_ref();
        *slot = match core::str::from_utf8(b) {
            Ok(_) => new_str(b),
            Err(_) => new_str(alloc::string::String::from_utf8_lossy(b).as_bytes()),
        };
    }
    IscArray { ptr: q.as_mut_ptr() as *mut u8, len: names.len() as i64 }
}

#[cfg(target_os = "linux")]
mod imp {
    // every unsafe operation in a block of its own, each justified (D564):
    // the pointers of the generated code read once at each entry, the
    // system's calls of paths behind `ZPath`
    #![deny(unsafe_op_in_unsafe_fn)]
    #![deny(clippy::undocumented_unsafe_blocks)]
    use super::*;

    const O_WRONLY: c_int = 0o1;
    const O_CREAT: c_int = 0o100;
    const O_TRUNC: c_int = 0o1000;
    const O_APPEND: c_int = 0o2000;
    const O_DIRECTORY: c_int = 0o200000;
    const O_CLOEXEC: c_int = 0o2000000;

    /// A path with a zero byte after it and none in it (`zpath`): what the
    /// system's calls of paths read; they are made only by its methods.
    struct ZPath(alloc::vec::Vec<u8>);

    impl ZPath {
        /// The path of the string `p`, `n` of the generated code; None when
        /// it holds a zero byte.
        ///
        /// # Safety
        /// As `zpath`.
        unsafe fn of(p: *const u8, n: i64) -> Option<ZPath> {
            // SAFETY: the contract of this function
            unsafe { zpath(p, n) }.map(ZPath)
        }

        fn ptr(&self) -> *const c_char {
            self.0.as_ptr() as *const c_char
        }

        fn stat(&self) -> Result<(i64, i64, i64), isize> {
            // SAFETY: a path ending with its zero byte, alive during the call
            unsafe { linux::stat(self.ptr()) }
        }

        fn open(&self, flags: c_int, mode: c_int) -> c_int {
            // SAFETY: as above
            unsafe { linux::open(self.ptr(), flags, mode) }
        }

        fn mkdir(&self) -> isize {
            // SAFETY: as above
            unsafe { linux::mkdir(self.ptr()) }
        }

        fn unlink(&self, folder: bool) -> isize {
            // SAFETY: as above
            unsafe { linux::unlink(self.ptr(), folder) }
        }

        fn rename(&self, to: &ZPath) -> isize {
            // SAFETY: two paths ending with their zero bytes, alive during
            // the call
            unsafe { linux::rename(self.ptr(), to.ptr()) }
        }
    }

    /// `read` of `fd` into `buf`.
    fn read_into(fd: c_int, buf: &mut [u8]) -> isize {
        // SAFETY: `buf` writable for its length
        unsafe { linux::read(fd, buf.as_mut_ptr() as *mut c_void, buf.len()) }
    }

    /// `sysEnviron()`: the entries `name=value` of the environment.
    ///
    /// # Safety
    /// `out` the place of the result, the generated code's.
    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_environ(out: *mut IscArray) {
        let mut all = alloc::vec::Vec::new();
        let mut e = ENVIRON.load(Ordering::Relaxed) as *const *const u8;
        if !e.is_null() {
            // SAFETY: `environ` (the start of the process, D289): pointers to
            // texts each ending with a zero byte, then a null one; never
            // changed by the program (no `setenv`)
            unsafe {
                while !(*e).is_null() {
                    let v = *e;
                    // its length by aligned vectors (`strlen`, D400)
                    let n = strlen(v as *const c_char);
                    // the bytes of the entry, copied once into its string
                    all.push(core::slice::from_raw_parts(v, n));
                    e = e.add(1);
                }
            }
        }
        // SAFETY: the contract of this function
        unsafe { out.write(sorted_entries(all)) };
    }

    /// `sysNow()`: CLOCK_REALTIME in nanoseconds since 1970.
    ///
    /// # Safety
    /// `out` the place of the result, the generated code's.
    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_now(out: *mut i64) {
        let mut ts = Timespec { tv_sec: 0, tv_nsec: 0 };
        // SAFETY: `ts` a place of this frame
        unsafe { linux::clock_gettime(0, &mut ts) };
        // SAFETY: the contract of this function
        unsafe { out.write(ts.tv_sec.wrapping_mul(1_000_000_000).wrapping_add(ts.tv_nsec as i64)) };
    }

    /// `sysStat(path)`: [kind, size, modified, code].
    ///
    /// # Safety
    /// `out` the place of the result, `p` and `n` a string, the generated
    /// code's.
    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_stat(out: *mut [i64; 4], p: *const u8, n: i64) {
        // SAFETY: the contract of this function
        let r = match unsafe { ZPath::of(p, n) } {
            None => [0, 0, 0, -1],
            Some(z) => match z.stat() {
                Ok((kind, size, at)) => [kind, size, at, 0],
                Err(r) => [0, 0, 0, code_of_call(r)],
            },
        };
        // SAFETY: the contract of this function
        unsafe { out.write(r) };
    }

    /// `sysReadDir(path)`: [names, code], without `.` and `..`.
    ///
    /// # Safety
    /// As `isc_sys_stat`.
    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_readdir(out: *mut IscArrayCode, p: *const u8, n: i64) {
        // SAFETY: the contract of this function
        let r = match unsafe { ZPath::of(p, n) } {
            None => IscArrayCode { a: sorted_strings(alloc::vec::Vec::<&[u8]>::new()), code: -1 },
            Some(z) => read_dir(&z),
        };
        // SAFETY: the contract of this function
        unsafe { out.write(r) };
    }

    fn read_dir(z: &ZPath) -> IscArrayCode {
        let fd = z.open(O_DIRECTORY | O_CLOEXEC, 0);
        if fd < 0 {
            return IscArrayCode { a: sorted_strings(alloc::vec::Vec::<&[u8]>::new()), code: code_of_call(fd as isize) };
        }
        let mut names = alloc::vec::Vec::new();
        let mut buf = alloc::vec![0u8; 32 << 10];
        let code = loop {
            // SAFETY: `buf` writable for its length
            let r = unsafe { linux::getdents(fd, buf.as_mut_ptr(), buf.len()) };
            if r <= 0 {
                break code_of_call(r);
            }
            // each entry: inode (8), offset (8), its length (2), its type
            // (1), its name ending with a zero byte
            let mut at = 0usize;
            while at < r as usize {
                let len = u16::from_ne_bytes([buf[at + 16], buf[at + 17]]) as usize;
                let name = &buf[at + 19..at + len];
                let name = &name[..name.iter().position(|&c| c == 0).unwrap_or(name.len())];
                if name != b"." && name != b".." {
                    names.push(name.to_vec());
                }
                at += len;
            }
        };
        linux::close(fd);
        IscArrayCode { a: sorted_strings(if code == 0 { names } else { alloc::vec::Vec::new() }), code }
    }

    /// `sysMkdir(path)`: its code.
    ///
    /// # Safety
    /// As `isc_sys_stat`.
    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_mkdir(out: *mut i64, p: *const u8, n: i64) {
        // SAFETY: the contract of this function
        let r = match unsafe { ZPath::of(p, n) } {
            None => -1,
            Some(z) => code_of_call(z.mkdir()),
        };
        // SAFETY: the contract of this function
        unsafe { out.write(r) };
    }

    /// `sysRemoveFile(path)`: its code.
    ///
    /// # Safety
    /// As `isc_sys_stat`.
    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_removefile(out: *mut i64, p: *const u8, n: i64) {
        // SAFETY: the contract of this function
        let r = match unsafe { ZPath::of(p, n) } {
            None => -1,
            Some(z) => code_of_call(z.unlink(false)),
        };
        // SAFETY: the contract of this function
        unsafe { out.write(r) };
    }

    /// `sysRemoveDir(path)`: its code (an empty folder).
    ///
    /// # Safety
    /// As `isc_sys_stat`.
    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_removedir(out: *mut i64, p: *const u8, n: i64) {
        // SAFETY: the contract of this function
        let r = match unsafe { ZPath::of(p, n) } {
            None => -1,
            Some(z) => code_of_call(z.unlink(true)),
        };
        // SAFETY: the contract of this function
        unsafe { out.write(r) };
    }

    /// `sysRename(from, to)`: its code.
    ///
    /// # Safety
    /// `out` the place of the result, `p` and `n`, `q` and `m` two strings,
    /// the generated code's.
    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_rename(out: *mut i64, p: *const u8, n: i64, q: *const u8, m: i64) {
        // SAFETY: the contract of this function
        let (a, b) = unsafe { (ZPath::of(p, n), ZPath::of(q, m)) };
        let r = match (a, b) {
            (Some(a), Some(b)) => code_of_call(a.rename(&b)),
            _ => -1,
        };
        // SAFETY: the contract of this function
        unsafe { out.write(r) };
    }

    /// `sysCwd()`: [path, code].
    ///
    /// # Safety
    /// `out` the place of the result, the generated code's.
    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_cwd(out: *mut IscStrCode) {
        let mut buf = alloc::vec![0u8; 4096];
        let r = loop {
            // SAFETY: `buf` writable for its length
            let r = unsafe { linux::getcwd(buf.as_mut_ptr(), buf.len()) };
            // the path too long for the buffer (ERANGE): a larger one
            if r == -34 && buf.len() < 1 << 20 {
                buf = alloc::vec![0u8; buf.len() * 4];
                continue;
            }
            if r < 0 {
                break IscStrCode { s: adopt(alloc::string::String::new()), code: code_of_call(r) };
            }
            let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
            break IscStrCode { s: adopt(alloc::string::String::from_utf8_lossy(&buf[..len]).into_owned()), code: 0 };
        };
        // SAFETY: the contract of this function
        unsafe { out.write(r) };
    }

    /// `sysReadFile(path)`: [bytes, code].
    ///
    /// # Safety
    /// As `isc_sys_stat`.
    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_readfile(out: *mut IscArrayCode, p: *const u8, n: i64) {
        // SAFETY: the contract of this function
        let r = match unsafe { ZPath::of(p, n) } {
            None => IscArrayCode { a: byte_array(&[]), code: -1 },
            Some(z) => read_file(&z),
        };
        // SAFETY: the contract of this function
        unsafe { out.write(r) };
    }

    fn read_file(z: &ZPath) -> IscArrayCode {
        let fd = z.open(O_CLOEXEC, 0);
        if fd < 0 {
            return IscArrayCode { a: byte_array(&[]), code: code_of_call(fd as isize) };
        }
        let mut data = alloc::vec::Vec::new();
        let mut buf = alloc::vec![0u8; 1 << 16];
        let code = loop {
            let r = read_into(fd, &mut buf);
            if r <= 0 {
                break code_of_call(r);
            }
            data.extend_from_slice(&buf[..r as usize]);
        };
        linux::close(fd);
        IscArrayCode { a: byte_array(if code == 0 { &data } else { &[] }), code }
    }

    /// `sysWriteFile(path, bytes, append)`: its code; the file made if it
    /// is not there, emptied first unless `append`.
    ///
    /// # Safety
    /// `out` the place of the result, `p` and `n` a string, `d` and `m` an
    /// array of bytes, the generated code's.
    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_writefile(out: *mut i64, p: *const u8, n: i64, d: *const u8, m: i64, append: i32) {
        // SAFETY: the contract of this function
        let (z, data) = unsafe { (ZPath::of(p, n), bytes_of(d, m)) };
        let r = match z {
            None => -1,
            Some(z) => write_file(&z, data, append != 0),
        };
        // SAFETY: the contract of this function
        unsafe { out.write(r) };
    }

    fn write_file(z: &ZPath, data: &[u8], append: bool) -> i64 {
        let flags = O_WRONLY | O_CREAT | O_CLOEXEC | if append { O_APPEND } else { O_TRUNC };
        let fd = z.open(flags, 0o666);
        if fd < 0 {
            return code_of_call(fd as isize);
        }
        let mut at = 0;
        let mut code = 0;
        while at < data.len() {
            // SAFETY: the bytes of `data` from `at`, readable
            let r = unsafe { linux::write(fd, data[at..].as_ptr() as *const c_void, data.len() - at) };
            if r <= 0 {
                code = if r < 0 { code_of_call(r) } else { -5 };
                break;
            }
            at += r as usize;
        }
        linux::close(fd);
        code
    }
}

#[cfg(windows)]
mod imp {
    // every unsafe operation in a block with its reason (D569)
    #![deny(unsafe_op_in_unsafe_fn)]
    #![deny(clippy::undocumented_unsafe_blocks)]
    use super::*;

    type Handle = *mut c_void;
    const INVALID: Handle = !0usize as Handle;
    const CP_UTF8: u32 = 65001;

    #[repr(C)]
    struct AttributeData {
        attributes: u32,
        created: [u32; 2],
        accessed: [u32; 2],
        written: [u32; 2],
        size_high: u32,
        size_low: u32,
    }

    #[repr(C)]
    struct FindData {
        attributes: u32,
        times: [u32; 6],
        size_high: u32,
        size_low: u32,
        reserved: [u32; 2],
        name: [u16; 260],
        short: [u16; 14],
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetLastError() -> u32;
        fn MultiByteToWideChar(cp: u32, flags: u32, s: *const u8, n: i32, w: *mut u16, wn: i32) -> i32;
        fn WideCharToMultiByte(cp: u32, flags: u32, w: *const u16, wn: i32, s: *mut u8, n: i32, default: *const u8, used: *mut i32) -> i32;
        fn GetFileAttributesExW(name: *const u16, level: i32, info: *mut AttributeData) -> i32;
        fn FindFirstFileW(name: *const u16, data: *mut FindData) -> Handle;
        fn FindNextFileW(h: Handle, data: *mut FindData) -> i32;
        fn FindClose(h: Handle) -> i32;
        fn CreateDirectoryW(name: *const u16, sa: *mut c_void) -> i32;
        fn DeleteFileW(name: *const u16) -> i32;
        fn RemoveDirectoryW(name: *const u16) -> i32;
        fn MoveFileExW(from: *const u16, to: *const u16, flags: u32) -> i32;
        fn GetCurrentDirectoryW(n: u32, buf: *mut u16) -> u32;
        fn GetEnvironmentStringsW() -> *mut u16;
        fn FreeEnvironmentStringsW(p: *mut u16) -> i32;
        fn CreateFileW(name: *const u16, access: u32, share: u32, sa: *mut c_void, disposition: u32, flags: u32, template: Handle) -> Handle;
        fn ReadFile(h: Handle, buf: *mut c_void, n: u32, done: *mut u32, ov: *mut c_void) -> i32;
        fn WriteFile(h: Handle, buf: *const c_void, n: u32, done: *mut u32, ov: *mut c_void) -> i32;
        fn SetFilePointerEx(h: Handle, distance: i64, new: *mut i64, method: u32) -> i32;
        fn CloseHandle(h: Handle) -> i32;
    }

    /// The code of the last error of Windows, as the standard library of
    /// Rust names its kinds (D322).
    fn last_code() -> i64 {
        // SAFETY: no argument
        match unsafe { GetLastError() } {
            2 | 3 => 1,
            5 => 2,
            80 | 183 => 3,
            145 => 4,
            10061 => 5,
            1460 | 10060 => 6,
            e => -(e as i64),
        }
    }

    /// `ok` (a result of Windows not 0) as a code.
    fn code_if(ok: bool) -> i64 {
        if ok {
            0
        } else {
            last_code()
        }
    }

    /// The path of `n` bytes at `p` in UTF-16 with a zero after it; None when
    /// it holds a zero byte or is not UTF-8.
    ///
    /// # Safety
    /// `p` and `n` the bytes of a string of the generated code (C1 of
    /// `lib.rs`).
    unsafe fn wide(p: *const u8, n: i64) -> Option<alloc::vec::Vec<u16>> {
        // SAFETY: the contract: the `n` bytes at `p` read; `w` of `wn + 1`
        // units, the conversion writes `wn`
        unsafe {
            let b = bytes_of(p, n);
            if b.contains(&0) || core::str::from_utf8(b).is_err() {
                return None;
            }
            let wn = if b.is_empty() { 0 } else { MultiByteToWideChar(CP_UTF8, 0, b.as_ptr(), b.len() as i32, core::ptr::null_mut(), 0) };
            let mut w = alloc::vec![0u16; wn as usize + 1];
            if wn > 0 {
                MultiByteToWideChar(CP_UTF8, 0, b.as_ptr(), b.len() as i32, w.as_mut_ptr(), wn);
            }
            Some(w)
    
        }
    }

    /// The UTF-8 of the `wn` units of UTF-16 at `w`.
    ///
    /// # Safety
    /// `wn` units of UTF-16 readable at `w`.
    unsafe fn narrow(w: *const u16, wn: usize) -> alloc::vec::Vec<u8> {
        // SAFETY: the contract: the `wn` units at `w` read; `b` of `n` bytes,
        // the conversion writes `n`
        unsafe {
            if wn == 0 {
                return alloc::vec::Vec::new();
            }
            let n = WideCharToMultiByte(CP_UTF8, 0, w, wn as i32, core::ptr::null_mut(), 0, core::ptr::null(), core::ptr::null_mut());
            let mut b = alloc::vec![0u8; n.max(0) as usize];
            WideCharToMultiByte(CP_UTF8, 0, w, wn as i32, b.as_mut_ptr(), n, core::ptr::null(), core::ptr::null_mut());
            b
    
        }
    }

    #[no_mangle]
    ///
    /// # Safety
    /// The pointers as the generated code gives them (C1 of `lib.rs`).
    pub unsafe extern "C" fn isc_sys_environ(out: *mut IscArray) {
        // SAFETY: the block of the environment that the system gives (strings of
        // UTF-16 each ended by a zero, the last one empty), read entry by entry
        // within it, then given back once; `out` the place of the result (C1 of
        // `lib.rs`)
        unsafe {
            let mut all = alloc::vec::Vec::new();
            let base = GetEnvironmentStringsW();
            if !base.is_null() {
                let mut e = base;
                while *e != 0 {
                    let n = isc_runtime_core::winargs::wide_len(e);
                    // the entries of the drives (`=C:=C:\...`) are not variables
                    if *e != b'=' as u16 {
                        all.push(narrow(e, n));
                    }
                    e = e.add(n + 1);
                }
                FreeEnvironmentStringsW(base);
            }
            *out = sorted_entries(all);
    
        }
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetSystemTimePreciseAsFileTime(ft: *mut [u32; 2]);
    }

    /// `sysNow()`: 100 ns since 1601 -> ns since 1970.
    #[no_mangle]
    ///
    /// # Safety
    /// The pointers as the generated code gives them (C1 of `lib.rs`).
    pub unsafe extern "C" fn isc_sys_now(out: *mut i64) {
        // SAFETY: a place for the time; `out` the place of the result (C1 of
        // `lib.rs`)
        unsafe {
            let mut ft = [0u32; 2];
            GetSystemTimePreciseAsFileTime(&mut ft);
            let t = ((ft[1] as i64) << 32) | ft[0] as i64;
            *out = (t - 116_444_736_000_000_000).wrapping_mul(100);
    
        }
    }

    #[no_mangle]
    ///
    /// # Safety
    /// The pointers as the generated code gives them (C1 of `lib.rs`).
    pub unsafe extern "C" fn isc_sys_stat(out: *mut [i64; 4], p: *const u8, n: i64) {
        // SAFETY: the path of the generated code (C1 of `lib.rs`), `w` its
        // NUL-terminated UTF-16; `d` a place of its size; `out` the place of the
        // result
        unsafe {
            let Some(w) = wide(p, n) else {
                *out = [0, 0, 0, -1];
                return;
            };
            let mut d: AttributeData = core::mem::zeroed();
            if GetFileAttributesExW(w.as_ptr(), 0, &mut d) == 0 {
                *out = [0, 0, 0, last_code()];
                return;
            }
            let kind = if d.attributes & 0x10 != 0 { 2 } else { 1 };
            let size = ((d.size_high as i64) << 32) | d.size_low as i64;
            // 100 ns since 1601 -> ns since 1970
            let ft = ((d.written[1] as i64) << 32) | d.written[0] as i64;
            *out = [kind, size, (ft - 116_444_736_000_000_000).wrapping_mul(100), 0];
    
        }
    }

    #[no_mangle]
    ///
    /// # Safety
    /// The pointers as the generated code gives them (C1 of `lib.rs`).
    pub unsafe extern "C" fn isc_sys_readdir(out: *mut IscArrayCode, p: *const u8, n: i64) {
        // SAFETY: the path of the generated code (C1 of `lib.rs`), `w` its
        // pattern in NUL-terminated UTF-16; `d` a place of its size, its name
        // read up to its zero within it; the search closed once; `out` the place
        // of the result
        unsafe {
            let Some(mut w) = wide(p, n) else {
                *out = IscArrayCode { a: sorted_strings(alloc::vec::Vec::<&[u8]>::new()), code: -1 };
                return;
            };
            w.pop();
            w.extend_from_slice(&[b'\\' as u16, b'*' as u16, 0]);
            let mut d: FindData = core::mem::zeroed();
            let h = FindFirstFileW(w.as_ptr(), &mut d);
            if h == INVALID {
                *out = IscArrayCode { a: sorted_strings(alloc::vec::Vec::<&[u8]>::new()), code: last_code() };
                return;
            }
            let mut names = alloc::vec::Vec::new();
            loop {
                let len = d.name.iter().position(|&c| c == 0).unwrap_or(d.name.len());
                let name = narrow(d.name.as_ptr(), len);
                if name != b"." && name != b".." {
                    names.push(name);
                }
                if FindNextFileW(h, &mut d) == 0 {
                    break;
                }
            }
            FindClose(h);
            *out = IscArrayCode { a: sorted_strings(names), code: 0 };
    
        }
    }

    #[no_mangle]
    ///
    /// # Safety
    /// The pointers as the generated code gives them (C1 of `lib.rs`).
    pub unsafe extern "C" fn isc_sys_mkdir(out: *mut i64, p: *const u8, n: i64) {
        // SAFETY: the path or paths of the generated code (C1 of `lib.rs`) in
        // NUL-terminated UTF-16 (`wide`); `out` the place of the result
        unsafe {
            *out = match wide(p, n) {
                None => -1,
                Some(w) => code_if(CreateDirectoryW(w.as_ptr(), core::ptr::null_mut()) != 0),
            };
    
        }
    }

    #[no_mangle]
    ///
    /// # Safety
    /// The pointers as the generated code gives them (C1 of `lib.rs`).
    pub unsafe extern "C" fn isc_sys_removefile(out: *mut i64, p: *const u8, n: i64) {
        // SAFETY: the path or paths of the generated code (C1 of `lib.rs`) in
        // NUL-terminated UTF-16 (`wide`); `out` the place of the result
        unsafe {
            *out = match wide(p, n) {
                None => -1,
                Some(w) => code_if(DeleteFileW(w.as_ptr()) != 0),
            };
    
        }
    }

    #[no_mangle]
    ///
    /// # Safety
    /// The pointers as the generated code gives them (C1 of `lib.rs`).
    pub unsafe extern "C" fn isc_sys_removedir(out: *mut i64, p: *const u8, n: i64) {
        // SAFETY: the path or paths of the generated code (C1 of `lib.rs`) in
        // NUL-terminated UTF-16 (`wide`); `out` the place of the result
        unsafe {
            *out = match wide(p, n) {
                None => -1,
                Some(w) => code_if(RemoveDirectoryW(w.as_ptr()) != 0),
            };
    
        }
    }

    #[no_mangle]
    ///
    /// # Safety
    /// The pointers as the generated code gives them (C1 of `lib.rs`).
    pub unsafe extern "C" fn isc_sys_rename(out: *mut i64, p: *const u8, n: i64, q: *const u8, m: i64) {
        // SAFETY: the path or paths of the generated code (C1 of `lib.rs`) in
        // NUL-terminated UTF-16 (`wide`); `out` the place of the result
        unsafe {
            *out = match (wide(p, n), wide(q, m)) {
                // MOVEFILE_REPLACE_EXISTING, as the standard library of Rust
                (Some(a), Some(b)) => code_if(MoveFileExW(a.as_ptr(), b.as_ptr(), 1) != 0),
                _ => -1,
            };
    
        }
    }

    #[no_mangle]
    ///
    /// # Safety
    /// The pointers as the generated code gives them (C1 of `lib.rs`).
    pub unsafe extern "C" fn isc_sys_cwd(out: *mut IscStrCode) {
        // SAFETY: `w` of `need + 1` units, the system writes at most its length;
        // `out` the place of the result (C1 of `lib.rs`)
        unsafe {
            let need = GetCurrentDirectoryW(0, core::ptr::null_mut());
            let mut w = alloc::vec![0u16; need as usize + 1];
            let got = GetCurrentDirectoryW(w.len() as u32, w.as_mut_ptr());
            if got == 0 {
                *out = IscStrCode { s: adopt(alloc::string::String::new()), code: last_code() };
                return;
            }
            *out = IscStrCode { s: adopt(alloc::string::String::from_utf8_lossy(&narrow(w.as_ptr(), got as usize)).into_owned()), code: 0 };
    
        }
    }

    #[no_mangle]
    ///
    /// # Safety
    /// The pointers as the generated code gives them (C1 of `lib.rs`).
    pub unsafe extern "C" fn isc_sys_readfile(out: *mut IscArrayCode, p: *const u8, n: i64) {
        // SAFETY: the path of the generated code (C1 of `lib.rs`), `w` its
        // NUL-terminated UTF-16; each read into `buf`, of its length; the handle
        // closed once; `out` the place of the result
        unsafe {
            let Some(w) = wide(p, n) else {
                *out = IscArrayCode { a: byte_array(&[]), code: -1 };
                return;
            };
            // GENERIC_READ, FILE_SHARE_READ | FILE_SHARE_WRITE, OPEN_EXISTING
            let h = CreateFileW(w.as_ptr(), 0x8000_0000, 3, core::ptr::null_mut(), 3, 0x80, core::ptr::null_mut());
            if h == INVALID {
                *out = IscArrayCode { a: byte_array(&[]), code: last_code() };
                return;
            }
            let mut data = alloc::vec::Vec::new();
            let mut buf = alloc::vec![0u8; 1 << 16];
            let mut code = 0;
            loop {
                let mut got = 0u32;
                if ReadFile(h, buf.as_mut_ptr() as *mut c_void, buf.len() as u32, &mut got, core::ptr::null_mut()) == 0 {
                    code = last_code();
                    break;
                }
                if got == 0 {
                    break;
                }
                data.extend_from_slice(&buf[..got as usize]);
            }
            CloseHandle(h);
            *out = IscArrayCode { a: byte_array(if code == 0 { &data } else { &[] }), code };
    
        }
    }

    #[no_mangle]
    ///
    /// # Safety
    /// The pointers as the generated code gives them (C1 of `lib.rs`).
    pub unsafe extern "C" fn isc_sys_writefile(out: *mut i64, p: *const u8, n: i64, d: *const u8, m: i64, append: i32) {
        // SAFETY: the path and the bytes of the generated code (C1 of `lib.rs`),
        // `w` the path in NUL-terminated UTF-16; each write of at most the bytes
        // of `data` from `at`; the handle closed once; `out` the place of the
        // result
        unsafe {
            let Some(w) = wide(p, n) else {
                *out = -1;
                return;
            };
            // GENERIC_WRITE, FILE_SHARE_READ; OPEN_ALWAYS to append,
            // CREATE_ALWAYS otherwise
            let disposition = if append != 0 { 4 } else { 2 };
            let h = CreateFileW(w.as_ptr(), 0x4000_0000, 1, core::ptr::null_mut(), disposition, 0x80, core::ptr::null_mut());
            if h == INVALID {
                *out = last_code();
                return;
            }
            if append != 0 {
                // FILE_END
                SetFilePointerEx(h, 0, core::ptr::null_mut(), 2);
            }
            let data = bytes_of(d, m);
            let mut at = 0;
            let mut code = 0;
            while at < data.len() {
                let mut done = 0u32;
                let chunk = (data.len() - at).min(1 << 30) as u32;
                if WriteFile(h, data[at..].as_ptr() as *const c_void, chunk, &mut done, core::ptr::null_mut()) == 0 {
                    code = last_code();
                    break;
                }
                at += done as usize;
            }
            CloseHandle(h);
            *out = code;
    
        }
    }
}

#[cfg(target_os = "wasi")]
mod imp {
    // every unsafe operation in a block with its reason (D569)
    #![deny(unsafe_op_in_unsafe_fn)]
    #![deny(clippy::undocumented_unsafe_blocks)]
    use super::*;
    use crate::wasi;

    /// The code of `sys` of a code of WASI.
    fn kind(e: u16) -> i64 {
        match e {
            44 => 1,
            2 | 63 => 2,
            20 => 3,
            55 => 4,
            14 => 5,
            73 => 6,
            e => -(e as i64),
        }
    }

    fn code_of(r: Result<(), u16>) -> i64 {
        match r {
            Ok(()) => 0,
            Err(e) => kind(e),
        }
    }

    /// The bytes of a path of `zpath` without its zero byte.
    fn bytes(z: &[u8]) -> &[u8] {
        &z[..z.len() - 1]
    }

    #[no_mangle]
    ///
    /// # Safety
    /// The pointers as the generated code gives them (C1 of `lib.rs`).
    pub unsafe extern "C" fn isc_sys_environ(out: *mut IscArray) {
        // SAFETY: the environment of WASI (`environ`, a list of NUL-terminated
        // strings ended by null), read once; `out` the place of the result (C1
        // of `lib.rs`)
        unsafe {
            let mut all = alloc::vec::Vec::new();
            let mut e = wasi::environ();
            while !(*e).is_null() {
                let v = *e;
                // the bytes of the entry, copied once into its string
                all.push(core::slice::from_raw_parts(v, strlen(v as *const c_char)));
                e = e.add(1);
            }
            *out = sorted_entries(all);
    
        }
    }

    /// `sysNow()`: the real time in nanoseconds since 1970.
    #[no_mangle]
    ///
    /// # Safety
    /// The pointers as the generated code gives them (C1 of `lib.rs`).
    pub unsafe extern "C" fn isc_sys_now(out: *mut i64) {
        // SAFETY: `out` the place of the result (C1 of `lib.rs`)
        unsafe {
            *out = wasi::clock(0) as i64;
    
        }
    }

    #[no_mangle]
    ///
    /// # Safety
    /// The pointers as the generated code gives them (C1 of `lib.rs`).
    pub unsafe extern "C" fn isc_sys_stat(out: *mut [i64; 4], p: *const u8, n: i64) {
        // SAFETY: the path or paths of the generated code (C1 of `lib.rs`), read
        // by `zpath`; `out` the place of the result
        unsafe {
            let Some(z) = zpath(p, n) else {
                *out = [0, 0, 0, -1];
                return;
            };
            match wasi::stat(bytes(&z)) {
                Err(e) => *out = [0, 0, 0, kind(e)],
                Ok(st) => {
                    // the types of WASI: 3 a directory, 4 a regular file
                    let k = match st.filetype {
                        4 => 1,
                        3 => 2,
                        _ => 3,
                    };
                    *out = [k, st.size as i64, st.mtim as i64, 0];
                }
            }
    
        }
    }

    #[no_mangle]
    ///
    /// # Safety
    /// The pointers as the generated code gives them (C1 of `lib.rs`).
    pub unsafe extern "C" fn isc_sys_readdir(out: *mut IscArrayCode, p: *const u8, n: i64) {
        // SAFETY: the path or paths of the generated code (C1 of `lib.rs`), read
        // by `zpath`; `out` the place of the result
        unsafe {
            let Some(z) = zpath(p, n) else {
                *out = IscArrayCode { a: sorted_strings(alloc::vec::Vec::<&[u8]>::new()), code: -1 };
                return;
            };
            match wasi::read_dir(bytes(&z)) {
                Err(e) => *out = IscArrayCode { a: sorted_strings(alloc::vec::Vec::<&[u8]>::new()), code: kind(e) },
                Ok(all) => {
                    let names: alloc::vec::Vec<alloc::vec::Vec<u8>> = all.into_iter().filter(|b| b != b"." && b != b"..").collect();
                    *out = IscArrayCode { a: sorted_strings(names), code: 0 };
                }
            }
    
        }
    }

    #[no_mangle]
    ///
    /// # Safety
    /// The pointers as the generated code gives them (C1 of `lib.rs`).
    pub unsafe extern "C" fn isc_sys_mkdir(out: *mut i64, p: *const u8, n: i64) {
        // SAFETY: the path or paths of the generated code (C1 of `lib.rs`), read
        // by `zpath`; `out` the place of the result
        unsafe {
            *out = match zpath(p, n) {
                None => -1,
                Some(z) => code_of(wasi::mkdir(bytes(&z))),
            };
    
        }
    }

    #[no_mangle]
    ///
    /// # Safety
    /// The pointers as the generated code gives them (C1 of `lib.rs`).
    pub unsafe extern "C" fn isc_sys_removefile(out: *mut i64, p: *const u8, n: i64) {
        // SAFETY: the path or paths of the generated code (C1 of `lib.rs`), read
        // by `zpath`; `out` the place of the result
        unsafe {
            *out = match zpath(p, n) {
                None => -1,
                Some(z) => code_of(wasi::unlink(bytes(&z))),
            };
    
        }
    }

    #[no_mangle]
    ///
    /// # Safety
    /// The pointers as the generated code gives them (C1 of `lib.rs`).
    pub unsafe extern "C" fn isc_sys_removedir(out: *mut i64, p: *const u8, n: i64) {
        // SAFETY: the path or paths of the generated code (C1 of `lib.rs`), read
        // by `zpath`; `out` the place of the result
        unsafe {
            *out = match zpath(p, n) {
                None => -1,
                Some(z) => code_of(wasi::rmdir(bytes(&z))),
            };
    
        }
    }

    #[no_mangle]
    ///
    /// # Safety
    /// The pointers as the generated code gives them (C1 of `lib.rs`).
    pub unsafe extern "C" fn isc_sys_rename(out: *mut i64, p: *const u8, n: i64, q: *const u8, m: i64) {
        // SAFETY: the path or paths of the generated code (C1 of `lib.rs`), read
        // by `zpath`; `out` the place of the result
        unsafe {
            *out = match (zpath(p, n), zpath(q, m)) {
                (Some(a), Some(b)) => code_of(wasi::rename(bytes(&a), bytes(&b))),
                _ => -1,
            };
    
        }
    }

    /// The current directory: `/` under WASI (as wasi-libc: a program
    /// cannot change it, its relative paths are read from it).
    #[no_mangle]
    ///
    /// # Safety
    /// The pointers as the generated code gives them (C1 of `lib.rs`).
    pub unsafe extern "C" fn isc_sys_cwd(out: *mut IscStrCode) {
        // SAFETY: `out` the place of the result (C1 of `lib.rs`)
        unsafe {
            *out = IscStrCode { s: adopt(alloc::string::String::from("/")), code: 0 };
    
        }
    }

    #[no_mangle]
    ///
    /// # Safety
    /// The pointers as the generated code gives them (C1 of `lib.rs`).
    pub unsafe extern "C" fn isc_sys_readfile(out: *mut IscArrayCode, p: *const u8, n: i64) {
        // SAFETY: the path of the generated code (C1 of `lib.rs`), `z` its
        // NUL-terminated copy; each read into `buf`, of its length; the
        // descriptor closed once; `out` the place of the result
        unsafe {
            let Some(z) = zpath(p, n) else {
                *out = IscArrayCode { a: byte_array(&[]), code: -1 };
                return;
            };
            let fd = wasi::open(z.as_ptr() as *const c_char, wasi::O_RDONLY, 0);
            if fd < 0 {
                *out = IscArrayCode { a: byte_array(&[]), code: kind(-fd as u16) };
                return;
            }
            let mut data = alloc::vec::Vec::new();
            let mut buf = alloc::vec![0u8; 1 << 16];
            let mut code = 0;
            loop {
                let r = wasi::read(fd, buf.as_mut_ptr() as *mut c_void, buf.len());
                if r < 0 {
                    code = kind(-r as u16);
                    break;
                }
                if r == 0 {
                    break;
                }
                data.extend_from_slice(&buf[..r as usize]);
            }
            wasi::close(fd);
            *out = IscArrayCode { a: byte_array(if code == 0 { &data } else { &[] }), code };
    
        }
    }

    #[no_mangle]
    ///
    /// # Safety
    /// The pointers as the generated code gives them (C1 of `lib.rs`).
    pub unsafe extern "C" fn isc_sys_writefile(out: *mut i64, p: *const u8, n: i64, d: *const u8, m: i64, append: i32) {
        // SAFETY: the path and the bytes of the generated code (C1 of `lib.rs`),
        // `z` its NUL-terminated copy; each write of the bytes of `data` from
        // `at`; the descriptor closed once; `out` the place of the result
        unsafe {
            let Some(z) = zpath(p, n) else {
                *out = -1;
                return;
            };
            let flags = wasi::O_WRONLY | wasi::O_CREAT | if append != 0 { wasi::O_APPEND } else { wasi::O_TRUNC };
            let fd = wasi::open(z.as_ptr() as *const c_char, flags, 0o666);
            if fd < 0 {
                *out = kind(-fd as u16);
                return;
            }
            let data = bytes_of(d, m);
            let mut at = 0;
            let mut code = 0;
            while at < data.len() {
                let r = wasi::write(fd, data[at..].as_ptr() as *const c_void, data.len() - at);
                if r <= 0 {
                    code = if r < 0 { kind(-r as u16) } else { -29 };
                    break;
                }
                at += r as usize;
            }
            wasi::close(fd);
            *out = code;
    
        }
    }
}
