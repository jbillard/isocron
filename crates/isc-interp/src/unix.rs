//! The module `unix` in the interpreter (D632): the calls of the system that
//! the compiled program makes itself (`isc_codegen::emit::unix`), with the
//! same codes. A connection of a Unix socket is held as the connections of
//! TCP are (its descriptor in a `TcpStream`, whose reads, writes, time
//! limits and closing are those of any socket), so that the primitives of
//! `net` work on it as in the compiled program.
//!
//! What the standard library of Rust does not offer (descriptors passed
//! with bytes, `poll`, `memfd_create`, `mmap`) calls the C library that
//! `iscc` is linked with, each call justified where it is made.

use crate::handles::{Cell, Resource};
use crate::{Array, Value};
use isc_ir::SysOp as S;
use std::rc::Rc;

/// The code of an error of the system (`rt::io_code`).
fn code(e: &std::io::Error) -> i64 {
    crate::rt::io_code(e)
}

fn tuple(xs: Vec<Value>) -> Value {
    Value::Tuple(Rc::new(xs))
}

fn handle(r: Option<Resource>) -> Value {
    Value::Handle(Rc::new(Cell(std::cell::RefCell::new(r))))
}

fn int(v: &Value) -> i64 {
    match v {
        Value::Int(x) => *x,
        v => panic!("an integer expected, got {:?}", v),
    }
}

fn ints(v: &Value) -> Vec<i64> {
    match v {
        Value::Array(a) => match &**a {
            Array::Int(x) => x.clone(),
            a => panic!("integers expected, got {:?}", a),
        },
        v => panic!("an array expected, got {:?}", v),
    }
}

fn int_array(x: Vec<i64>) -> Value {
    Value::Array(Rc::new(Array::Int(x)))
}

fn cell(v: &Value) -> Rc<Cell> {
    match v {
        Value::Handle(c) => c.clone(),
        v => panic!("a resource expected, got {:?}", v),
    }
}

/// A primitive of the module `unix`; not Linux: the code of `ENOSYS`, as no
/// compiled program for another system has them (D324).
#[cfg(not(target_os = "linux"))]
pub fn sys(o: S, _vs: &[Value]) -> Value {
    match o {
        S::UnixConnect | S::UnixListen | S::UnixAccept | S::SharedOpen | S::SharedMap => tuple(vec![handle(None), Value::Int(-38)]),
        S::RecvFds => tuple(vec![int_array(Vec::new()), int_array(Vec::new()), Value::Int(-38)]),
        S::Poll => tuple(vec![int_array(Vec::new()), Value::Int(-38)]),
        S::SharedRead => tuple(vec![int_array(Vec::new()), Value::Int(-38)]),
        _ => Value::Int(-38),
    }
}

#[cfg(target_os = "linux")]
pub use linux::{sys, Mapping};

/// A primitive of the module `win32` (D636): not run by the interpreter yet
/// (a program reaches it in a branch for Windows, `sys.platform()`, which
/// the interpreter does not take on Linux): the code of `ENOSYS`.
pub fn win32(o: S, _vs: &[Value]) -> Value {
    match o {
        S::WinOpen => tuple(vec![handle(None), Value::Int(-38)]),
        S::WinEvents | S::WinCapture | S::WinProgram | S::WinClipGet => tuple(vec![int_array(Vec::new()), Value::Int(-38)]),
        S::WinSize => tuple(vec![Value::Int(0), Value::Int(0)]),
        S::WinClose => Value::Int(0),
        S::WinDpi => Value::Int(96),
        _ => Value::Int(-38),
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use std::ffi::{c_int, c_long, c_uint, c_void};
    use std::net::{TcpListener, TcpStream};
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::linux::net::SocketAddrExt;
    use std::os::unix::net::{SocketAddr, UnixListener, UnixStream};

    extern "C" {
        fn sendmsg(fd: c_int, msg: *const MsgHdr, flags: c_int) -> isize;
        fn recvmsg(fd: c_int, msg: *mut MsgHdr, flags: c_int) -> isize;
        fn poll(fds: *mut PollFd, n: u64, ms: c_int) -> c_int;
        fn close(fd: c_int) -> c_int;
        fn memfd_create(name: *const u8, flags: c_uint) -> c_int;
        fn mmap(addr: *mut c_void, len: usize, prot: c_int, flags: c_int, fd: c_int, off: c_long) -> *mut c_void;
        fn munmap(addr: *mut c_void, len: usize) -> c_int;
    }

    #[repr(C)]
    struct IoVec {
        base: *mut u8,
        len: usize,
    }

    #[repr(C)]
    struct MsgHdr {
        name: *mut c_void,
        namelen: u32,
        iov: *mut IoVec,
        iovlen: usize,
        control: *mut u8,
        controllen: usize,
        flags: c_int,
    }

    #[repr(C)]
    struct PollFd {
        fd: c_int,
        events: i16,
        revents: i16,
    }

    const MSG_NOSIGNAL: c_int = 0x4000;
    const MSG_CMSG_CLOEXEC: c_int = 0x4000_0000;
    const MSG_CTRUNC: c_int = 8;
    /// `SCM_MAX_FD`: the descriptors of one message at most.
    const MAX_FDS: usize = 253;
    /// Bytes of one `recvmsg` at most (as the compiled program).
    const MAX_BYTES: i64 = 1 << 24;

    /// A projection of a memory shared: undone when its value goes; its
    /// file (`sysSharedOpen`) closed then.
    #[derive(Debug)]
    pub struct Mapping {
        at: *mut u8,
        len: usize,
        file: Option<OwnedFd>,
        writable: bool,
    }

    impl Drop for Mapping {
        fn drop(&mut self) {
            // SAFETY: the projection made by `mmap` of `len` bytes at `at`,
            // undone once (this value is its only owner)
            unsafe { munmap(self.at as *mut c_void, self.len) };
        }
    }

    /// The address of a Unix socket for `path`: a file, or `@` and a name of
    /// the abstract space (at most 107 bytes either way, no zero byte in a
    /// path), as the compiled program reads it.
    fn address(path: &str) -> Result<SocketAddr, i64> {
        let b = path.as_bytes();
        match b.first() {
            None => Err(-22),
            Some(b'@') if b.len() - 1 <= 107 => SocketAddr::from_abstract_name(&b[1..]).map_err(|_| -22),
            Some(b'@') => Err(-22),
            Some(_) if b.len() <= 107 && !b.contains(&0) => SocketAddr::from_pathname(path).map_err(|_| -22),
            Some(_) => Err(-22),
        }
    }

    /// The descriptor of a connection.
    fn conn_fd(c: &Cell) -> Option<c_int> {
        match &*c.0.borrow() {
            Some(Resource::Conn(s)) => Some(s.as_raw_fd()),
            _ => None,
        }
    }

    /// The mapping of a memory shared, read by `f`.
    fn with_mapping<T>(v: &Value, f: impl FnOnce(&Mapping) -> T) -> Option<T> {
        match &*cell(v).0.borrow() {
            Some(Resource::Shared(m)) => Some(f(m)),
            _ => None,
        }
    }

    /// `n` bytes from `at` inside `len` bytes.
    fn inside(at: i64, n: i64, len: usize) -> bool {
        at >= 0 && n >= 0 && (n as u64) <= len as u64 && (at as u64) <= len as u64 - n as u64
    }

    pub fn sys(o: S, vs: &[Value]) -> Value {
        let text = |v: &Value| match v {
            Value::Str(s) => s.to_string(),
            v => panic!("a string expected, got {:?}", v),
        };
        match o {
            S::UnixConnect => match address(&text(&vs[0])).map(|a| UnixStream::connect_addr(&a).map_err(|e| code(&e))) {
                Ok(Ok(s)) => tuple(vec![handle(Some(Resource::Conn(TcpStream::from(OwnedFd::from(s))))), Value::Int(0)]),
                Ok(Err(e)) | Err(e) => tuple(vec![handle(None), Value::Int(e)]),
            },
            S::UnixListen => match address(&text(&vs[0])).map(|a| UnixListener::bind_addr(&a).map_err(|e| code(&e))) {
                Ok(Ok(l)) => tuple(vec![handle(Some(Resource::Listener(TcpListener::from(OwnedFd::from(l))))), Value::Int(0)]),
                Ok(Err(e)) | Err(e) => tuple(vec![handle(None), Value::Int(e)]),
            },
            S::UnixAccept => {
                // the listener seen as a Unix one through a copy of its
                // descriptor (its peer is not an address of the network)
                let l = match &*cell(&vs[0]).0.borrow() {
                    Some(Resource::Listener(l)) => l.try_clone().map(|l| UnixListener::from(OwnedFd::from(l))).map_err(|e| code(&e)),
                    _ => Err(-9),
                };
                match l.and_then(|l| l.accept().map_err(|e| code(&e))) {
                    Ok((s, _)) => tuple(vec![handle(Some(Resource::Conn(TcpStream::from(OwnedFd::from(s))))), Value::Int(0)]),
                    Err(e) => tuple(vec![handle(None), Value::Int(e)]),
                }
            }
            S::ConnFd => Value::Int(conn_fd(&cell(&vs[0])).map_or(-1, |fd| fd as i64)),
            S::SendFds => {
                let data: Vec<u8> = ints(&vs[1]).into_iter().map(|x| x as u8).collect();
                let fds: Vec<c_int> = ints(&vs[2]).into_iter().map(|x| x as c_int).collect();
                let Some(fd) = conn_fd(&cell(&vs[0])) else {
                    return Value::Int(-9);
                };
                Value::Int(send_fds(fd, &data, &fds))
            }
            S::RecvFds => {
                let Some(fd) = conn_fd(&cell(&vs[0])) else {
                    return tuple(vec![int_array(Vec::new()), int_array(Vec::new()), Value::Int(-9)]);
                };
                let (bytes, fds, c) = recv_fds(fd, int(&vs[1]).clamp(1, MAX_BYTES) as usize, int(&vs[2]).clamp(0, MAX_FDS as i64) as usize);
                tuple(vec![int_array(bytes.into_iter().map(i64::from).collect()), int_array(fds.into_iter().map(i64::from).collect()), Value::Int(c)])
            }
            S::Poll => {
                let mut fds: Vec<PollFd> = ints(&vs[0]).into_iter().map(|fd| PollFd { fd: fd as c_int, events: 1, revents: 0 }).collect();
                let ms = int(&vs[1]);
                let t = if ms < 0 { -1 } else { ms.min(i32::MAX as i64) as c_int };
                let r = loop {
                    // SAFETY: `fds` the descriptors to wait for, written by
                    // the kernel in place (their `revents`), alive during
                    // the call
                    let r = unsafe { poll(fds.as_mut_ptr(), fds.len() as u64, t) };
                    let e = std::io::Error::last_os_error();
                    if r >= 0 {
                        break 0;
                    }
                    if e.raw_os_error() != Some(4) {
                        break code(&e);
                    }
                };
                tuple(vec![int_array(fds.iter().map(|p| p.revents as u16 as i64).collect()), Value::Int(r)])
            }
            S::CloseFd => {
                // SAFETY: a descriptor named by the program, closed once; a
                // number that is not one is the error of the kernel
                let r = unsafe { close(int(&vs[0]) as c_int) };
                Value::Int(if r < 0 { code(&std::io::Error::last_os_error()) } else { 0 })
            }
            S::SharedOpen => {
                let bytes = int(&vs[0]);
                if bytes < 1 {
                    return tuple(vec![handle(None), Value::Int(-22)]);
                }
                // SAFETY: a name ending with its zero byte
                let fd = unsafe { memfd_create(b"isocron\0".as_ptr(), 1) };
                if fd < 0 {
                    return tuple(vec![handle(None), Value::Int(code(&std::io::Error::last_os_error()))]);
                }
                // SAFETY: the descriptor just made, owned from here on
                let file = std::fs::File::from(unsafe { OwnedFd::from_raw_fd(fd) });
                if let Err(e) = file.set_len(bytes as u64) {
                    return tuple(vec![handle(None), Value::Int(code(&e))]);
                }
                match map(file.as_raw_fd(), bytes as usize, true) {
                    Ok(at) => tuple(vec![handle(Some(Resource::Shared(Mapping { at, len: bytes as usize, file: Some(OwnedFd::from(file)), writable: true }))), Value::Int(0)]),
                    Err(e) => tuple(vec![handle(None), Value::Int(e)]),
                }
            }
            S::SharedMap => {
                let (fd, bytes, w) = (int(&vs[0]) as c_int, int(&vs[1]), matches!(vs[2], Value::Bool(true)));
                if bytes < 1 {
                    return tuple(vec![handle(None), Value::Int(-22)]);
                }
                match map(fd, bytes as usize, w) {
                    Ok(at) => tuple(vec![handle(Some(Resource::Shared(Mapping { at, len: bytes as usize, file: None, writable: w }))), Value::Int(0)]),
                    Err(e) => tuple(vec![handle(None), Value::Int(e)]),
                }
            }
            S::SharedWrite | S::SharedWrite32 => {
                let (at, xs) = (int(&vs[1]), ints(&vs[2]));
                let data: Vec<u8> = if o == S::SharedWrite { xs.iter().map(|&x| x as u8).collect() } else { xs.iter().flat_map(|&x| (x as u32).to_le_bytes()).collect() };
                let r = with_mapping(&vs[0], |m| {
                    if !m.writable {
                        2
                    } else if !inside(at, data.len() as i64, m.len) {
                        -22
                    } else {
                        // SAFETY: `data.len()` bytes from `at` inside the
                        // projection (tested), writable, apart from `data`
                        unsafe { std::ptr::copy_nonoverlapping(data.as_ptr(), m.at.add(at as usize), data.len()) };
                        0
                    }
                });
                Value::Int(r.unwrap_or(-9))
            }
            S::SharedRead => {
                let (at, n) = (int(&vs[1]), int(&vs[2]));
                let r = with_mapping(&vs[0], |m| {
                    if !inside(at, n, m.len) {
                        return Err(-22);
                    }
                    let mut out = vec![0u8; n as usize];
                    // SAFETY: `n` bytes from `at` inside the projection
                    // (tested), copied into a vector of their length
                    unsafe { std::ptr::copy_nonoverlapping(m.at.add(at as usize), out.as_mut_ptr(), n as usize) };
                    Ok(out)
                });
                match r.unwrap_or(Err(-9)) {
                    Ok(b) => tuple(vec![int_array(b.into_iter().map(i64::from).collect()), Value::Int(0)]),
                    Err(e) => tuple(vec![int_array(Vec::new()), Value::Int(e)]),
                }
            }
            S::SharedFd => Value::Int(with_mapping(&vs[0], |m| m.file.as_ref().map_or(-1, |f| f.as_raw_fd() as i64)).unwrap_or(-1)),
            S::SharedSize => Value::Int(with_mapping(&vs[0], |m| m.len as i64).unwrap_or(0)),
            S::SharedClose => {
                // the projection undone and its file closed as the value
                // goes (`Mapping::drop`), now
                drop(cell(&vs[0]).0.borrow_mut().take());
                Value::Int(0)
            }
            _ => unreachable!("not a primitive of the module unix: {:?}", o),
        }
    }

    /// `mmap` of `len` bytes of `fd`: shared and writable, or private and
    /// read only.
    fn map(fd: c_int, len: usize, writable: bool) -> Result<*mut u8, i64> {
        let (prot, flags) = if writable { (3, 1) } else { (1, 2) };
        // SAFETY: a new projection at a place the kernel chooses: no memory
        // of the interpreter changed
        let p = unsafe { mmap(std::ptr::null_mut(), len, prot, flags, fd, 0) };
        if p as isize == -1 {
            return Err(code(&std::io::Error::last_os_error()));
        }
        Ok(p as *mut u8)
    }

    /// `sysSendFds`: as the compiled program, `sendmsg` with the
    /// descriptors and the first part of the bytes, the rest by `send`.
    fn send_fds(fd: c_int, data: &[u8], fds: &[c_int]) -> i64 {
        if !fds.is_empty() && (fds.len() > MAX_FDS || data.is_empty()) {
            return -22;
        }
        let mut at = 0usize;
        if !fds.is_empty() {
            let mut ctl = vec![0u64; 130];
            let clen = 16 + 4 * fds.len();
            let cbytes = ctl.as_mut_ptr() as *mut u8;
            // SAFETY: the header of the message of control (its length,
            // `SOL_SOCKET`, `SCM_RIGHTS`) and the descriptors after it,
            // inside the 1040 bytes of `ctl`
            unsafe {
                *(cbytes as *mut u64) = clen as u64;
                *(cbytes.add(8) as *mut i32) = 1;
                *(cbytes.add(12) as *mut i32) = 1;
                std::ptr::copy_nonoverlapping(fds.as_ptr() as *const u8, cbytes.add(16), 4 * fds.len());
            }
            let mut iov = IoVec { base: data.as_ptr() as *mut u8, len: data.len() };
            let msg = MsgHdr { name: std::ptr::null_mut(), namelen: 0, iov: &mut iov, iovlen: 1, control: cbytes, controllen: (clen + 7) & !7, flags: 0 };
            loop {
                // SAFETY: the message and what it points to alive during the
                // call, only read by the kernel
                let r = unsafe { sendmsg(fd, &msg, MSG_NOSIGNAL) };
                if r >= 0 {
                    at = r as usize;
                    break;
                }
                let e = std::io::Error::last_os_error();
                if e.raw_os_error() != Some(4) {
                    return code(&e);
                }
            }
        }
        while at < data.len() {
            let mut iov = IoVec { base: data[at..].as_ptr() as *mut u8, len: data.len() - at };
            let msg = MsgHdr { name: std::ptr::null_mut(), namelen: 0, iov: &mut iov, iovlen: 1, control: std::ptr::null_mut(), controllen: 0, flags: 0 };
            // SAFETY: as above
            let r = unsafe { sendmsg(fd, &msg, MSG_NOSIGNAL) };
            if r < 0 {
                let e = std::io::Error::last_os_error();
                if e.raw_os_error() == Some(4) {
                    continue;
                }
                return code(&e);
            }
            at += r as usize;
        }
        0
    }

    /// `sysRecvFds`: one `recvmsg` of at most `max` bytes and `max_fds`
    /// descriptors; more descriptors than that: those received closed, the
    /// bytes dropped, the code of `EMSGSIZE` (as the compiled program).
    fn recv_fds(fd: c_int, max: usize, max_fds: usize) -> (Vec<u8>, Vec<c_int>, i64) {
        let mut buf = vec![0u8; max];
        let mut ctl = vec![0u64; 130];
        let space = if max_fds == 0 { 0 } else { (16 + 4 * max_fds + 7) & !7 };
        let mut iov = IoVec { base: buf.as_mut_ptr(), len: max };
        let mut msg = MsgHdr { name: std::ptr::null_mut(), namelen: 0, iov: &mut iov, iovlen: 1, control: ctl.as_mut_ptr() as *mut u8, controllen: space, flags: 0 };
        let r = loop {
            // SAFETY: the buffers of the message alive during the call,
            // written by the kernel within their lengths
            let r = unsafe { recvmsg(fd, &mut msg, MSG_CMSG_CLOEXEC) };
            if r >= 0 {
                break r as usize;
            }
            let e = std::io::Error::last_os_error();
            if e.raw_os_error() != Some(4) {
                return (Vec::new(), Vec::new(), code(&e));
            }
        };
        let cbytes = ctl.as_ptr() as *const u8;
        let mut fds = Vec::new();
        let mut off = 0usize;
        while off + 16 <= msg.controllen {
            // SAFETY: a header of a message of control inside the bytes the
            // kernel wrote (`controllen`, within `ctl`)
            let (len, level, ty) = unsafe { (*(cbytes.add(off) as *const u64) as usize, *(cbytes.add(off + 8) as *const i32), *(cbytes.add(off + 12) as *const i32)) };
            if len < 16 {
                break;
            }
            if level == 1 && ty == 1 {
                let count = ((len - 16) / 4).min(max_fds - fds.len());
                for i in 0..count {
                    // SAFETY: the descriptors after the header, inside the
                    // length of this message of control
                    fds.push(unsafe { *(cbytes.add(off + 16 + 4 * i) as *const i32) });
                }
            }
            off += (len + 7) & !7;
        }
        if msg.flags & MSG_CTRUNC != 0 {
            for f in fds {
                // SAFETY: descriptors the kernel just gave this process,
                // closed once
                unsafe { close(f) };
            }
            return (Vec::new(), Vec::new(), -90);
        }
        buf.truncate(r);
        (buf, fds, 0)
    }
}
