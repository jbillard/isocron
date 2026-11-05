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
        17 => 3,
        39 => 4,
        111 => 5,
        110 => 6,
        e => -e,
    }
}

/// The result of a call of the kernel as a code (0 when it succeeded).
#[cfg(target_os = "linux")]
fn code_of_call(r: isize) -> i64 {
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
