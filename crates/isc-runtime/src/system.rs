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
unsafe fn zpath(p: *const u8, n: i64) -> Option<alloc::vec::Vec<u8>> {
    let b = bytes_of(p, n);
    if b.contains(&0) {
        return None;
    }
    let mut z = alloc::vec::Vec::with_capacity(b.len() + 1);
    z.extend_from_slice(b);
    z.push(0);
    Some(z)
}

/// A new array of bytes holding `b`.
unsafe fn byte_array(b: &[u8]) -> IscArray {
    let q = isc_alloc(b.len().max(1) as i64);
    core::ptr::copy_nonoverlapping(b.as_ptr(), q, b.len());
    IscArray { ptr: q, len: b.len() as i64 }
}

/// A new array of the strings `names`, in the order of their bytes, the
/// bytes that are not UTF-8 replaced.
unsafe fn sorted_strings(mut names: alloc::vec::Vec<alloc::vec::Vec<u8>>) -> IscArray {
    names.sort();
    let texts: alloc::vec::Vec<alloc::string::String> = names.iter().map(|b| alloc::string::String::from_utf8_lossy(b).into_owned()).collect();
    str_array(&mut texts.iter().map(|t| t.as_str()), texts.len())
}

#[cfg(target_os = "linux")]
mod imp {
    use super::*;

    const O_WRONLY: c_int = 0o1;
    const O_CREAT: c_int = 0o100;
    const O_TRUNC: c_int = 0o1000;
    const O_APPEND: c_int = 0o2000;
    const O_DIRECTORY: c_int = 0o200000;
    const O_CLOEXEC: c_int = 0o2000000;

    /// `sysEnviron()`: the entries `name=value` of the environment.
    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_environ(out: *mut IscArray) {
        let mut all = alloc::vec::Vec::new();
        let mut e = ENVIRON.load(Ordering::Relaxed) as *const *const u8;
        if !e.is_null() {
            while !(*e).is_null() {
                let v = *e;
                let mut n = 0;
                while *v.add(n) != 0 {
                    n += 1;
                }
                all.push(core::slice::from_raw_parts(v, n).to_vec());
                e = e.add(1);
            }
        }
        *out = sorted_strings(all);
    }

    /// `sysNow()`: CLOCK_REALTIME in nanoseconds since 1970.
    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_now(out: *mut i64) {
        let mut ts = Timespec { tv_sec: 0, tv_nsec: 0 };
        linux::clock_gettime(0, &mut ts);
        *out = ts.tv_sec.wrapping_mul(1_000_000_000).wrapping_add(ts.tv_nsec as i64);
    }

    /// `sysStat(path)`: [kind, size, modified, code].
    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_stat(out: *mut [i64; 4], p: *const u8, n: i64) {
        *out = match zpath(p, n) {
            None => [0, 0, 0, -1],
            Some(z) => match linux::stat(z.as_ptr() as *const c_char) {
                Ok((kind, size, at)) => [kind, size, at, 0],
                Err(r) => [0, 0, 0, code_of_call(r)],
            },
        };
    }

    /// `sysReadDir(path)`: [names, code], without `.` and `..`.
    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_readdir(out: *mut IscArrayCode, p: *const u8, n: i64) {
        let Some(z) = zpath(p, n) else {
            *out = IscArrayCode { a: sorted_strings(alloc::vec::Vec::new()), code: -1 };
            return;
        };
        let fd = linux::open(z.as_ptr() as *const c_char, O_DIRECTORY | O_CLOEXEC, 0);
        if fd < 0 {
            *out = IscArrayCode { a: sorted_strings(alloc::vec::Vec::new()), code: code_of_call(fd as isize) };
            return;
        }
        let mut names = alloc::vec::Vec::new();
        let mut buf = alloc::vec![0u8; 32 << 10];
        let mut code = 0;
        loop {
            let r = linux::getdents(fd, buf.as_mut_ptr(), buf.len());
            if r <= 0 {
                code = code_of_call(r);
                break;
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
        }
        linux::close(fd);
        *out = IscArrayCode { a: sorted_strings(if code == 0 { names } else { alloc::vec::Vec::new() }), code };
    }

    /// `sysMkdir(path)`: its code.
    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_mkdir(out: *mut i64, p: *const u8, n: i64) {
        *out = match zpath(p, n) {
            None => -1,
            Some(z) => code_of_call(linux::mkdir(z.as_ptr() as *const c_char)),
        };
    }

    /// `sysRemoveFile(path)`: its code.
    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_removefile(out: *mut i64, p: *const u8, n: i64) {
        *out = match zpath(p, n) {
            None => -1,
            Some(z) => code_of_call(linux::unlink(z.as_ptr() as *const c_char, false)),
        };
    }

    /// `sysRemoveDir(path)`: its code (an empty folder).
    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_removedir(out: *mut i64, p: *const u8, n: i64) {
        *out = match zpath(p, n) {
            None => -1,
            Some(z) => code_of_call(linux::unlink(z.as_ptr() as *const c_char, true)),
        };
    }

    /// `sysRename(from, to)`: its code.
    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_rename(out: *mut i64, p: *const u8, n: i64, q: *const u8, m: i64) {
        *out = match (zpath(p, n), zpath(q, m)) {
            (Some(a), Some(b)) => code_of_call(linux::rename(a.as_ptr() as *const c_char, b.as_ptr() as *const c_char)),
            _ => -1,
        };
    }

    /// `sysCwd()`: [path, code].
    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_cwd(out: *mut IscStrCode) {
        let mut buf = alloc::vec![0u8; 4096];
        loop {
            let r = linux::getcwd(buf.as_mut_ptr(), buf.len());
            // the path too long for the buffer (ERANGE): a larger one
            if r == -34 && buf.len() < 1 << 20 {
                buf = alloc::vec![0u8; buf.len() * 4];
                continue;
            }
            if r < 0 {
                *out = IscStrCode { s: adopt(alloc::string::String::new()), code: code_of_call(r) };
                return;
            }
            let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
            *out = IscStrCode { s: adopt(alloc::string::String::from_utf8_lossy(&buf[..len]).into_owned()), code: 0 };
            return;
        }
    }

    /// `sysReadFile(path)`: [bytes, code].
    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_readfile(out: *mut IscArrayCode, p: *const u8, n: i64) {
        let Some(z) = zpath(p, n) else {
            *out = IscArrayCode { a: byte_array(&[]), code: -1 };
            return;
        };
        let fd = linux::open(z.as_ptr() as *const c_char, O_CLOEXEC, 0);
        if fd < 0 {
            *out = IscArrayCode { a: byte_array(&[]), code: code_of_call(fd as isize) };
            return;
        }
        let mut data = alloc::vec::Vec::new();
        let mut buf = alloc::vec![0u8; 1 << 16];
        let mut code = 0;
        loop {
            let r = linux::read(fd, buf.as_mut_ptr() as *mut c_void, buf.len());
            if r <= 0 {
                code = code_of_call(r);
                break;
            }
            data.extend_from_slice(&buf[..r as usize]);
        }
        linux::close(fd);
        *out = IscArrayCode { a: byte_array(if code == 0 { &data } else { &[] }), code };
    }

    /// `sysWriteFile(path, bytes, append)`: its code; the file made if it
    /// is not there, emptied first unless `append`.
    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_writefile(out: *mut i64, p: *const u8, n: i64, d: *const u8, m: i64, append: i32) {
        let Some(z) = zpath(p, n) else {
            *out = -1;
            return;
        };
        let flags = O_WRONLY | O_CREAT | O_CLOEXEC | if append != 0 { O_APPEND } else { O_TRUNC };
        let fd = linux::open(z.as_ptr() as *const c_char, flags, 0o666);
        if fd < 0 {
            *out = code_of_call(fd as isize);
            return;
        }
        let data = bytes_of(d, m);
        let mut at = 0;
        let mut code = 0;
        while at < data.len() {
            let r = linux::write(fd, data[at..].as_ptr() as *const c_void, data.len() - at);
            if r <= 0 {
                code = if r < 0 { code_of_call(r) } else { -5 };
                break;
            }
            at += r as usize;
        }
        linux::close(fd);
        *out = code;
    }
}

#[cfg(windows)]
mod imp {
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
    unsafe fn wide(p: *const u8, n: i64) -> Option<alloc::vec::Vec<u16>> {
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

    /// The UTF-8 of the `wn` units of UTF-16 at `w`.
    unsafe fn narrow(w: *const u16, wn: usize) -> alloc::vec::Vec<u8> {
        if wn == 0 {
            return alloc::vec::Vec::new();
        }
        let n = WideCharToMultiByte(CP_UTF8, 0, w, wn as i32, core::ptr::null_mut(), 0, core::ptr::null(), core::ptr::null_mut());
        let mut b = alloc::vec![0u8; n.max(0) as usize];
        WideCharToMultiByte(CP_UTF8, 0, w, wn as i32, b.as_mut_ptr(), n, core::ptr::null(), core::ptr::null_mut());
        b
    }

    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_environ(out: *mut IscArray) {
        let mut all = alloc::vec::Vec::new();
        let base = GetEnvironmentStringsW();
        if !base.is_null() {
            let mut e = base;
            while *e != 0 {
                let mut n = 0;
                while *e.add(n) != 0 {
                    n += 1;
                }
                // the entries of the drives (`=C:=C:\...`) are not variables
                if *e != b'=' as u16 {
                    all.push(narrow(e, n));
                }
                e = e.add(n + 1);
            }
            FreeEnvironmentStringsW(base);
        }
        *out = sorted_strings(all);
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetSystemTimePreciseAsFileTime(ft: *mut [u32; 2]);
    }

    /// `sysNow()`: 100 ns since 1601 -> ns since 1970.
    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_now(out: *mut i64) {
        let mut ft = [0u32; 2];
        GetSystemTimePreciseAsFileTime(&mut ft);
        let t = ((ft[1] as i64) << 32) | ft[0] as i64;
        *out = (t - 116_444_736_000_000_000).wrapping_mul(100);
    }

    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_stat(out: *mut [i64; 4], p: *const u8, n: i64) {
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

    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_readdir(out: *mut IscArrayCode, p: *const u8, n: i64) {
        let Some(mut w) = wide(p, n) else {
            *out = IscArrayCode { a: sorted_strings(alloc::vec::Vec::new()), code: -1 };
            return;
        };
        w.pop();
        w.extend_from_slice(&[b'\\' as u16, b'*' as u16, 0]);
        let mut d: FindData = core::mem::zeroed();
        let h = FindFirstFileW(w.as_ptr(), &mut d);
        if h == INVALID {
            *out = IscArrayCode { a: sorted_strings(alloc::vec::Vec::new()), code: last_code() };
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

    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_mkdir(out: *mut i64, p: *const u8, n: i64) {
        *out = match wide(p, n) {
            None => -1,
            Some(w) => code_if(CreateDirectoryW(w.as_ptr(), core::ptr::null_mut()) != 0),
        };
    }

    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_removefile(out: *mut i64, p: *const u8, n: i64) {
        *out = match wide(p, n) {
            None => -1,
            Some(w) => code_if(DeleteFileW(w.as_ptr()) != 0),
        };
    }

    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_removedir(out: *mut i64, p: *const u8, n: i64) {
        *out = match wide(p, n) {
            None => -1,
            Some(w) => code_if(RemoveDirectoryW(w.as_ptr()) != 0),
        };
    }

    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_rename(out: *mut i64, p: *const u8, n: i64, q: *const u8, m: i64) {
        *out = match (wide(p, n), wide(q, m)) {
            // MOVEFILE_REPLACE_EXISTING, as the standard library of Rust
            (Some(a), Some(b)) => code_if(MoveFileExW(a.as_ptr(), b.as_ptr(), 1) != 0),
            _ => -1,
        };
    }

    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_cwd(out: *mut IscStrCode) {
        let need = GetCurrentDirectoryW(0, core::ptr::null_mut());
        let mut w = alloc::vec![0u16; need as usize + 1];
        let got = GetCurrentDirectoryW(w.len() as u32, w.as_mut_ptr());
        if got == 0 {
            *out = IscStrCode { s: adopt(alloc::string::String::new()), code: last_code() };
            return;
        }
        *out = IscStrCode { s: adopt(alloc::string::String::from_utf8_lossy(&narrow(w.as_ptr(), got as usize)).into_owned()), code: 0 };
    }

    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_readfile(out: *mut IscArrayCode, p: *const u8, n: i64) {
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

    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_writefile(out: *mut i64, p: *const u8, n: i64, d: *const u8, m: i64, append: i32) {
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

#[cfg(target_os = "wasi")]
mod imp {
    use super::*;

    // `struct stat` of wasi-libc for wasm32 (measured with `offsetof`): 144
    // bytes, the mode at 24, the size at 48, the time of the last change at
    // 88 (seconds, 64 bits) and 96 (nanoseconds, 32 bits); a `dirent` its
    // name at 9
    const STAT_BYTES: usize = 144;
    const O_APPEND: c_int = 1;

    extern "C" {
        fn stat(path: *const c_char, st: *mut u8) -> c_int;
        fn opendir(path: *const c_char) -> *mut c_void;
        fn readdir(d: *mut c_void) -> *const u8;
        fn closedir(d: *mut c_void) -> c_int;
        fn mkdir(path: *const c_char, mode: u32) -> c_int;
        fn unlink(path: *const c_char) -> c_int;
        fn rmdir(path: *const c_char) -> c_int;
        fn rename(from: *const c_char, to: *const c_char) -> c_int;
        fn getcwd(buf: *mut c_char, n: usize) -> *mut c_char;
        static environ: *const *const u8;
    }

    /// The code of the `errno` of WASI.
    fn last_code() -> i64 {
        // SAFETY: the `errno` of this thread
        match unsafe { *__errno_location() } {
            44 => 1,
            2 | 63 => 2,
            20 => 3,
            55 => 4,
            14 => 5,
            73 => 6,
            e => -(e as i64),
        }
    }

    fn code_if(r: c_int) -> i64 {
        if r == 0 {
            0
        } else {
            last_code()
        }
    }

    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_environ(out: *mut IscArray) {
        let mut all = alloc::vec::Vec::new();
        let mut e = environ;
        if !e.is_null() {
            while !(*e).is_null() {
                let v = *e;
                let mut n = 0;
                while *v.add(n) != 0 {
                    n += 1;
                }
                all.push(core::slice::from_raw_parts(v, n).to_vec());
                e = e.add(1);
            }
        }
        *out = sorted_strings(all);
    }

    extern "C" {
        // `clockid_t` is a pointer in wasi-libc
        static _CLOCK_REALTIME: u8;
        fn clock_gettime(clk: *const u8, ts: *mut Timespec) -> c_int;
    }

    /// `sysNow()`: CLOCK_REALTIME in nanoseconds since 1970.
    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_now(out: *mut i64) {
        let mut ts = Timespec { tv_sec: 0, tv_nsec: 0 };
        clock_gettime(&_CLOCK_REALTIME, &mut ts);
        *out = ts.tv_sec.wrapping_mul(1_000_000_000).wrapping_add(ts.tv_nsec as i64);
    }

    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_stat(out: *mut [i64; 4], p: *const u8, n: i64) {
        let Some(z) = zpath(p, n) else {
            *out = [0, 0, 0, -1];
            return;
        };
        let mut st = [0u8; STAT_BYTES];
        if stat(z.as_ptr() as *const c_char, st.as_mut_ptr()) != 0 {
            *out = [0, 0, 0, last_code()];
            return;
        }
        let word = |at: usize| i64::from_le_bytes(st[at..at + 8].try_into().unwrap());
        let mode = u32::from_le_bytes(st[24..28].try_into().unwrap());
        let kind = match mode & 0xF000 {
            0x8000 => 1,
            0x4000 => 2,
            _ => 3,
        };
        let nsec = i32::from_le_bytes(st[96..100].try_into().unwrap()) as i64;
        *out = [kind, word(48), word(88).wrapping_mul(1_000_000_000).wrapping_add(nsec), 0];
    }

    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_readdir(out: *mut IscArrayCode, p: *const u8, n: i64) {
        let Some(z) = zpath(p, n) else {
            *out = IscArrayCode { a: sorted_strings(alloc::vec::Vec::new()), code: -1 };
            return;
        };
        let d = opendir(z.as_ptr() as *const c_char);
        if d.is_null() {
            *out = IscArrayCode { a: sorted_strings(alloc::vec::Vec::new()), code: last_code() };
            return;
        }
        let mut names = alloc::vec::Vec::new();
        loop {
            let e = readdir(d);
            if e.is_null() {
                break;
            }
            let name = e.add(9);
            let mut len = 0;
            while *name.add(len) != 0 {
                len += 1;
            }
            let b = core::slice::from_raw_parts(name, len);
            if b != b"." && b != b".." {
                names.push(b.to_vec());
            }
        }
        closedir(d);
        *out = IscArrayCode { a: sorted_strings(names), code: 0 };
    }

    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_mkdir(out: *mut i64, p: *const u8, n: i64) {
        *out = match zpath(p, n) {
            None => -1,
            Some(z) => code_if(mkdir(z.as_ptr() as *const c_char, 0o777)),
        };
    }

    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_removefile(out: *mut i64, p: *const u8, n: i64) {
        *out = match zpath(p, n) {
            None => -1,
            Some(z) => code_if(unlink(z.as_ptr() as *const c_char)),
        };
    }

    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_removedir(out: *mut i64, p: *const u8, n: i64) {
        *out = match zpath(p, n) {
            None => -1,
            Some(z) => code_if(rmdir(z.as_ptr() as *const c_char)),
        };
    }

    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_rename(out: *mut i64, p: *const u8, n: i64, q: *const u8, m: i64) {
        *out = match (zpath(p, n), zpath(q, m)) {
            (Some(a), Some(b)) => code_if(rename(a.as_ptr() as *const c_char, b.as_ptr() as *const c_char)),
            _ => -1,
        };
    }

    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_cwd(out: *mut IscStrCode) {
        let mut buf = alloc::vec![0u8; 4096];
        if getcwd(buf.as_mut_ptr() as *mut c_char, buf.len()).is_null() {
            *out = IscStrCode { s: adopt(alloc::string::String::new()), code: last_code() };
            return;
        }
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        *out = IscStrCode { s: adopt(alloc::string::String::from_utf8_lossy(&buf[..len]).into_owned()), code: 0 };
    }

    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_readfile(out: *mut IscArrayCode, p: *const u8, n: i64) {
        let Some(z) = zpath(p, n) else {
            *out = IscArrayCode { a: byte_array(&[]), code: -1 };
            return;
        };
        let fd = open(z.as_ptr() as *const c_char, O_RDONLY);
        if fd < 0 {
            *out = IscArrayCode { a: byte_array(&[]), code: last_code() };
            return;
        }
        let mut data = alloc::vec::Vec::new();
        let mut buf = alloc::vec![0u8; 1 << 16];
        let mut code = 0;
        loop {
            let r = read(fd, buf.as_mut_ptr() as *mut c_void, buf.len());
            if r < 0 {
                code = last_code();
                break;
            }
            if r == 0 {
                break;
            }
            data.extend_from_slice(&buf[..r as usize]);
        }
        close(fd);
        *out = IscArrayCode { a: byte_array(if code == 0 { &data } else { &[] }), code };
    }

    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_writefile(out: *mut i64, p: *const u8, n: i64, d: *const u8, m: i64, append: i32) {
        let Some(z) = zpath(p, n) else {
            *out = -1;
            return;
        };
        let flags = O_WRONLY | O_CREAT | if append != 0 { O_APPEND } else { O_TRUNC };
        let fd = open(z.as_ptr() as *const c_char, flags, 0o666 as c_int);
        if fd < 0 {
            *out = last_code();
            return;
        }
        let data = bytes_of(d, m);
        let mut at = 0;
        let mut code = 0;
        while at < data.len() {
            let r = write(fd, data[at..].as_ptr() as *const c_void, data.len() - at);
            if r <= 0 {
                code = if r < 0 { last_code() } else { -29 };
                break;
            }
            at += r as usize;
        }
        close(fd);
        *out = code;
    }
}
