//! The primitives of the network of the module `net` (SPEC section 18,
//! D339): connections and listeners of TCP, a datagram of UDP for the
//! resolver of names, each returning its value and the code of its error as
//! the other primitives of the system (`system.rs`): 0 none, 5 refused, 6
//! timed out, otherwise the error of the system negated (-22 an address that
//! is not numeric). A connection or a listener is the number of its socket,
//! closed exactly once by `sysClose` or `sysCloseListener` (the checker
//! proves it: linear values).
//! Under Linux, the system called directly (D287); under Windows, Winsock;
//! WASI has no sockets (the compiler refuses the programs that reach them,
//! D324), only `sysPlatform`.

use super::*;
#[cfg(not(target_os = "wasi"))]
use crate::system::IscArrayCode;

/// `[resource, code]`.
#[repr(C)]
pub struct HandleCode {
    pub h: i64,
    pub code: i64,
}

/// `[conn, peer, code]`.
#[repr(C)]
pub struct AcceptOut {
    pub h: i64,
    pub peer: IscStr,
    pub code: i64,
}

/// `sysThreads()`: the threads of the pool (D373), 1 under WASI.
#[no_mangle]
pub unsafe extern "C" fn isc_sys_threads(out: *mut i64) {
    *out = crate::isc_thread_count();
}

/// `sysPlatform()`: 0 Linux, 1 Windows, 2 WASI.
#[no_mangle]
pub unsafe extern "C" fn isc_sys_platform(out: *mut i64) {
    *out = if cfg!(target_os = "wasi") {
        2
    } else if cfg!(windows) {
        1
    } else {
        0
    };
}

/// The numeric address `a` and the port: the bytes of its `sockaddr_in` or
/// `sockaddr_in6`, and its family.
#[cfg(not(target_os = "wasi"))]
fn sockaddr(a: &[u8], port: i64, inet6: u16) -> Option<([u8; 28], usize, u16)> {
    let text = core::str::from_utf8(a).ok()?;
    let ip: core::net::IpAddr = text.parse().ok()?;
    let port = u16::try_from(port).ok()?;
    let mut b = [0u8; 28];
    match ip {
        core::net::IpAddr::V4(v) => {
            b[0..2].copy_from_slice(&2u16.to_ne_bytes());
            b[2..4].copy_from_slice(&port.to_be_bytes());
            b[4..8].copy_from_slice(&v.octets());
            Some((b, 16, 2))
        }
        core::net::IpAddr::V6(v) => {
            b[0..2].copy_from_slice(&inet6.to_ne_bytes());
            b[2..4].copy_from_slice(&port.to_be_bytes());
            b[8..24].copy_from_slice(&v.octets());
            Some((b, 28, inet6))
        }
    }
}

/// The address of a `sockaddr` as text: `a.b.c.d:port`, `[v6]:port`.
#[cfg(not(target_os = "wasi"))]
fn address_text(b: &[u8; 28], inet6: u16) -> alloc::string::String {
    let family = u16::from_ne_bytes([b[0], b[1]]);
    let port = u16::from_be_bytes([b[2], b[3]]);
    if family == inet6 {
        let mut o = [0u8; 16];
        o.copy_from_slice(&b[8..24]);
        alloc::format!("{}", core::net::SocketAddr::new(core::net::IpAddr::V6(core::net::Ipv6Addr::from(o)), port))
    } else {
        alloc::format!("{}", core::net::SocketAddr::new(core::net::IpAddr::V4(core::net::Ipv4Addr::new(b[4], b[5], b[6], b[7])), port))
    }
}

/// An array of bytes holding the first `n` bytes of `buf` (of `max`
/// allocated): `buf` itself when they fill most of it, else a copy.
#[cfg(not(target_os = "wasi"))]
unsafe fn bytes_read(buf: *mut u8, n: usize, max: usize) -> IscArray {
    if n * 2 >= max {
        return IscArray { ptr: buf, len: n as i64 };
    }
    let q = isc_alloc(n.max(1) as i64);
    core::ptr::copy_nonoverlapping(buf, q, n);
    isc_free(buf);
    IscArray { ptr: q, len: n as i64 }
}

/// A text and a code (`[string, i64]`).
#[repr(C)]
pub struct TextCode {
    pub text: IscStr,
    pub code: i64,
}

/// A handle, a text and a code (`[Server, string, i64]`).
#[repr(C)]
pub struct HandleTextCode {
    pub h: i64,
    pub text: IscStr,
    pub code: i64,
}

/// The result of `sysServerWait` (D342): the numbers of the connections
/// read, the bytes read from each (0: the end of its stream, or an error
/// that ends it), and those bytes one after the other.
#[repr(C)]
pub struct WaitOut {
    pub ids: IscArray,
    pub lens: IscArray,
    pub data: IscArray,
    pub code: i64,
}

/// An array of the program holding a copy of `xs`.
#[cfg(not(target_os = "wasi"))]
unsafe fn array_of<T: Copy>(xs: &[T]) -> IscArray {
    if xs.is_empty() {
        return IscArray { ptr: empty_bytes(), len: 0 };
    }
    let bytes = core::mem::size_of_val(xs);
    let p = isc_alloc(bytes as i64);
    // SAFETY: `p` holds `bytes` bytes, `xs` as many
    core::ptr::copy_nonoverlapping(xs.as_ptr() as *const u8, p, bytes);
    IscArray { ptr: p, len: xs.len() as i64 }
}

/// The state of a server behind its handle (D342): its listener, its
/// connections by number (given in order, never twice: a number dropped
/// stays unknown, so a stale number never reaches another connection), and
/// what the system needs to wait on them. `F` is a socket of the system.
#[cfg(not(target_os = "wasi"))]
struct ServerState<F: Copy> {
    listener: F,
    next: i64,
    conns: alloc::vec::Vec<(i64, F)>,
    /// The set of `epoll` under Linux (unused under Windows).
    ep: F,
    /// The context of TLS of the server (0: none, D343), and the session of
    /// each connection with whether its handshake is done.
    tls: usize,
    sessions: alloc::vec::Vec<(i64, usize, bool)>,
    /// The vectors of a wait (the numbers, the lengths and the bytes read),
    /// kept from one wait to the next with their room (D376): a wait reads
    /// into the room it reserved for each connection, then copies.
    scratch: (alloc::vec::Vec<i64>, alloc::vec::Vec<i64>, alloc::vec::Vec<u8>),
}

/// The room of the bytes of a wait kept for the next one, at most.
#[cfg(not(target_os = "wasi"))]
const SCRATCH_KEPT: usize = 1 << 20;

#[cfg(not(target_os = "wasi"))]
impl<F: Copy> ServerState<F> {
    /// The socket of the connection `id`.
    fn fd(&self, id: i64) -> Option<F> {
        self.conns.binary_search_by_key(&id, |c| c.0).ok().map(|i| self.conns[i].1)
    }

    /// A connection added: its number.
    fn add(&mut self, fd: F) -> i64 {
        let id = self.next;
        self.next += 1;
        self.conns.push((id, fd));
        id
    }

    /// The vectors of a wait, empty, with the room of the waits before.
    fn take_scratch(&mut self) -> (alloc::vec::Vec<i64>, alloc::vec::Vec<i64>, alloc::vec::Vec<u8>) {
        let (mut a, mut b, mut c) = core::mem::take(&mut self.scratch);
        a.clear();
        b.clear();
        c.clear();
        (a, b, c)
    }

    /// The vectors of a wait kept for the next one (the bytes not, past
    /// SCRATCH_KEPT of room: a large body read once).
    fn keep_scratch(&mut self, v: (alloc::vec::Vec<i64>, alloc::vec::Vec<i64>, alloc::vec::Vec<u8>)) {
        let (a, b, c) = v;
        self.scratch = (a, b, if c.capacity() > SCRATCH_KEPT { alloc::vec::Vec::new() } else { c });
    }

    /// The connection `id` taken out: its socket.
    fn take(&mut self, id: i64) -> Option<F> {
        self.conns.binary_search_by_key(&id, |c| c.0).ok().map(|i| self.conns.remove(i).1)
    }
}

/// The state behind the handle `s` of a server (none for the handle of a
/// failure, -1).
///
/// # Safety
/// `s` is a handle that `sysServerOpen` made and `sysServerClose` has not
/// freed: the linear check of the program makes sure of it (D339).
#[cfg(not(target_os = "wasi"))]
unsafe fn server<'a, F: Copy>(s: i64) -> Option<&'a mut ServerState<F>> {
    if s <= 0 {
        None
    } else {
        // SAFETY: see above
        Some(unsafe { &mut *(s as *mut ServerState<F>) })
    }
}

/// The sessions of TLS of the connections, by socket: few, behind a lock
/// that spins (the actions run on one thread; the lock keeps it sound).
#[cfg(not(target_os = "wasi"))]
struct Sessions {
    lock: core::sync::atomic::AtomicBool,
    count: core::sync::atomic::AtomicUsize,
    list: core::cell::UnsafeCell<alloc::vec::Vec<(i64, isc_runtime_core::tls::Session)>>,
}

// SAFETY: `list` is read and written only while `lock` is held
#[cfg(not(target_os = "wasi"))]
unsafe impl Sync for Sessions {}

#[cfg(not(target_os = "wasi"))]
static SESSIONS: Sessions = Sessions { lock: core::sync::atomic::AtomicBool::new(false), count: core::sync::atomic::AtomicUsize::new(0), list: core::cell::UnsafeCell::new(alloc::vec::Vec::new()) };

#[cfg(not(target_os = "wasi"))]
fn with_sessions<T>(f: impl FnOnce(&mut alloc::vec::Vec<(i64, isc_runtime_core::tls::Session)>) -> T) -> T {
    while SESSIONS.lock.compare_exchange_weak(false, true, core::sync::atomic::Ordering::Acquire, core::sync::atomic::Ordering::Relaxed).is_err() {
        core::hint::spin_loop();
    }
    // SAFETY: the lock is held
    let r = f(unsafe { &mut *SESSIONS.list.get() });
    SESSIONS.count.store(unsafe { (*SESSIONS.list.get()).len() }, core::sync::atomic::Ordering::Release);
    SESSIONS.lock.store(false, core::sync::atomic::Ordering::Release);
    r
}

/// The session of TLS of the socket `fd`, if it has one.
#[cfg(not(target_os = "wasi"))]
fn session(fd: i64) -> Option<isc_runtime_core::tls::Session> {
    if SESSIONS.count.load(core::sync::atomic::Ordering::Acquire) == 0 {
        return None;
    }
    with_sessions(|v| v.iter().find(|x| x.0 == fd).map(|x| x.1))
}

#[cfg(not(target_os = "wasi"))]
fn take_session(fd: i64) -> Option<isc_runtime_core::tls::Session> {
    if SESSIONS.count.load(core::sync::atomic::Ordering::Acquire) == 0 {
        return None;
    }
    with_sessions(|v| v.iter().position(|x| x.0 == fd).map(|i| v.remove(i).1))
}


#[cfg(target_os = "linux")]
mod imp {
    use super::*;
    use crate::linux::syscall6;

    const SYS_SOCKET: usize = 41;
    const SYS_CONNECT: usize = 42;
    const SYS_SENDTO: usize = 44;
    const SYS_RECVFROM: usize = 45;
    const SYS_BIND: usize = 49;
    const SYS_LISTEN: usize = 50;
    const SYS_GETSOCKNAME: usize = 51;
    const SYS_SETSOCKOPT: usize = 54;
    const SYS_ACCEPT4: usize = 288;
    const AF_INET6: u16 = 10;
    const SOCK_STREAM: usize = 1;
    const SOCK_DGRAM: usize = 2;
    const SOCK_CLOEXEC: usize = 0o2000000;
    const SOL_SOCKET: usize = 1;
    const SO_REUSEADDR: usize = 2;
    const SO_REUSEPORT: usize = 15;
    const SO_RCVTIMEO: usize = 20;
    const SO_SNDTIMEO: usize = 21;
    const MSG_NOSIGNAL: usize = 0x4000;
    const EINTR: isize = -4;

    /// The code of the result of a call on a socket: a timeout (`EAGAIN`
    /// past `SO_RCVTIMEO`) as one.
    fn net_code(r: isize) -> i64 {
        if r == -11 {
            6
        } else {
            crate::system::code_of_call(r)
        }
    }

    /// A socket of `kind` for the family `family`, or a negative code.
    unsafe fn socket(family: u16, kind: usize) -> isize {
        syscall6(SYS_SOCKET, family as usize, kind | SOCK_CLOEXEC, 0, 0, 0, 0)
    }

    /// The call `f` again while a signal interrupts it.
    fn again(mut f: impl FnMut() -> isize) -> isize {
        loop {
            let r = f();
            if r != EINTR {
                return r;
            }
        }
    }

    /// The socket `h` closed (none: -1).
    unsafe fn close_socket(h: i64) {
        if h >= 0 {
            linux::close(h as c_int);
        }
    }

    /// `sysConnect(address, port)`.
    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_connect(out: *mut HandleCode, p: *const u8, n: i64, port: i64) {
        let Some((addr, len, family)) = sockaddr(bytes_of(p, n), port, AF_INET6) else {
            *out = HandleCode { h: -1, code: -22 };
            return;
        };
        let fd = socket(family, SOCK_STREAM);
        if fd < 0 {
            *out = HandleCode { h: -1, code: net_code(fd) };
            return;
        }
        // SAFETY: the address of `len` bytes, read by the kernel
        let r = again(|| unsafe { syscall6(SYS_CONNECT, fd as usize, addr.as_ptr() as usize, len, 0, 0, 0) });
        if r < 0 {
            linux::close(fd as c_int);
            *out = HandleCode { h: -1, code: net_code(r) };
            return;
        }
        *out = HandleCode { h: fd as i64, code: 0 };
    }

    /// `sysListen(address, port)`: the address reusable at once.
    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_listen(out: *mut HandleCode, p: *const u8, n: i64, port: i64) {
        listen_on(out, p, n, port, false)
    }

    /// `sysListenShared(address, port)` (D374): a listener whose port the
    /// other listeners of the program with this option share
    /// (`SO_REUSEPORT`): the kernel shares the new connections out among
    /// them (`socket(7)`: the option set on each socket before `bind`, all
    /// of the same effective user).
    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_listenshared(out: *mut HandleCode, p: *const u8, n: i64, port: i64) {
        listen_on(out, p, n, port, true)
    }

    unsafe fn listen_on(out: *mut HandleCode, p: *const u8, n: i64, port: i64, shared: bool) {
        let Some((addr, len, family)) = sockaddr(bytes_of(p, n), port, AF_INET6) else {
            *out = HandleCode { h: -1, code: -22 };
            return;
        };
        let fd = socket(family, SOCK_STREAM);
        if fd < 0 {
            *out = HandleCode { h: -1, code: net_code(fd) };
            return;
        }
        let one: c_int = 1;
        syscall6(SYS_SETSOCKOPT, fd as usize, SOL_SOCKET, SO_REUSEADDR, &one as *const c_int as usize, 4, 0);
        if shared {
            let r = syscall6(SYS_SETSOCKOPT, fd as usize, SOL_SOCKET, SO_REUSEPORT, &one as *const c_int as usize, 4, 0);
            if r < 0 {
                linux::close(fd as c_int);
                *out = HandleCode { h: -1, code: net_code(r) };
                return;
            }
        }
        let mut r = syscall6(SYS_BIND, fd as usize, addr.as_ptr() as usize, len, 0, 0, 0);
        if r >= 0 {
            r = syscall6(SYS_LISTEN, fd as usize, 128, 0, 0, 0, 0);
        }
        if r < 0 {
            linux::close(fd as c_int);
            *out = HandleCode { h: -1, code: net_code(r) };
            return;
        }
        *out = HandleCode { h: fd as i64, code: 0 };
    }

    /// `sysAccept(listener)`: the connection and the address of its peer.
    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_accept(out: *mut AcceptOut, l: i64) {
        let mut addr = [0u8; 28];
        let mut len: u32 = 28;
        // SAFETY: the kernel writes at most `len` bytes at `addr`, and its
        // length at `len`
        let r = again(|| unsafe { syscall6(SYS_ACCEPT4, l as usize, addr.as_mut_ptr() as usize, &mut len as *mut u32 as usize, SOCK_CLOEXEC, 0, 0) });
        if r < 0 {
            *out = AcceptOut { h: -1, peer: new_str(b""), code: net_code(r) };
            return;
        }
        let peer = address_text(&addr, AF_INET6);
        *out = AcceptOut { h: r as i64, peer: new_str(peer.as_bytes()), code: 0 };
    }

    // ----- TLS of the connections (D343) --------------------------------

    use isc_runtime_core::tls;

    /// `dlopen` and `dlsym` of glibc (a static program has none: it never
    /// reaches TLS, `iscc` links the programs that do with glibc).
    fn dl() -> Option<(tls::DlOpen, tls::DlSym)> {
        crate::process::os().dl
    }

    /// `SIGPIPE` ignored: OpenSSL writes to its sockets without
    /// `MSG_NOSIGNAL`, a peer gone is then an error, not the end of the
    /// program (the runtime's own writes never raised it).
    unsafe fn ignore_sigpipe() {
        // struct sigaction of the kernel: handler SIG_IGN, flags, restorer, mask
        let act: [u64; 4] = [1, 0, 0, 0];
        syscall6(13, 13, act.as_ptr() as usize, 0, 8, 0, 0);
    }

    fn text_of(t: &str) -> IscStr {
        new_str(t.as_bytes())
    }

    /// `sysTlsStart(conn, host, ca)`: the client of TLS on `conn`, its
    /// reads and writes encrypted from now on.
    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_tlsstart(out: *mut TextCode, c: i64, hp: *const u8, hn: i64, cp: *const u8, cn: i64) {
        let Some(d) = dl() else {
            *out = TextCode { text: text_of("TLS needs the C library of the system (the program is static)"), code: tls::NO_LIBRARY };
            return;
        };
        ignore_sigpipe();
        match tls::connect(d, c, bytes_of(hp, hn), bytes_of(cp, cn)) {
            Ok(sess) => {
                with_sessions(|v| v.push((c, sess)));
                *out = TextCode { text: text_of(""), code: 0 };
            }
            Err((code, t)) => *out = TextCode { text: text_of(&t), code },
        }
    }

    /// The code of a step of TLS that did not end well.
    fn tls_code(st: tls::Step) -> i64 {
        match st {
            tls::Step::WantRead | tls::Step::WantWrite => 6,
            _ => tls::TLS_FAULT,
        }
    }

    /// `sysRecv(conn, max)`: the bytes of one read, none at the end.
    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_recv(out: *mut IscArrayCode, c: i64, max: i64) {
        let max = max.clamp(1, 1 << 24) as usize;
        let buf = isc_alloc(max as i64);
        if let (Some(sess), Some(d)) = (session(c), dl()) {
            // SAFETY: `buf` holds `max` bytes
            let b = unsafe { core::slice::from_raw_parts_mut(buf, max) };
            *out = match tls::read(d, sess, b) {
                tls::Step::Done(n) => IscArrayCode { a: bytes_read(buf, n, max), code: 0 },
                tls::Step::End => IscArrayCode { a: bytes_read(buf, 0, max), code: 0 },
                st => {
                    isc_free(buf);
                    IscArrayCode { a: IscArray { ptr: empty_bytes(), len: 0 }, code: tls_code(st) }
                }
            };
            return;
        }
        // SAFETY: the kernel writes at most `max` bytes at `buf`
        let r = again(|| unsafe { linux::read(c as c_int, buf as *mut c_void, max) });
        if r < 0 {
            isc_free(buf);
            *out = IscArrayCode { a: IscArray { ptr: empty_bytes(), len: 0 }, code: net_code(r) };
            return;
        }
        *out = IscArrayCode { a: bytes_read(buf, r as usize, max), code: 0 };
    }

    /// `sysSend(conn, bytes)`: every byte, or an error (never `SIGPIPE`).
    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_send(out: *mut i64, c: i64, p: *const u8, n: i64) {
        let mut at = 0usize;
        let n = n.max(0) as usize;
        if let (Some(sess), Some(d)) = (session(c), dl()) {
            // SAFETY: `p` holds `n` bytes
            let all = unsafe { core::slice::from_raw_parts(p, n) };
            while at < n {
                match tls::write(d, sess, &all[at..]) {
                    tls::Step::Done(k) => at += k,
                    st => {
                        *out = tls_code(st);
                        return;
                    }
                }
            }
            *out = 0;
            return;
        }
        while at < n {
            // SAFETY: the kernel reads `n - at` bytes from `p + at`
            let r = again(|| unsafe { syscall6(SYS_SENDTO, c as usize, p.add(at) as usize, n - at, MSG_NOSIGNAL, 0, 0) });
            if r < 0 {
                *out = net_code(r);
                return;
            }
            at += r as usize;
        }
        *out = 0;
    }

    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_close(out: *mut i64, c: i64) {
        if let (Some(sess), Some(d)) = (take_session(c), dl()) {
            tls::close(d, sess);
        }
        close_socket(c);
        *out = 0;
    }

    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_closelistener(out: *mut i64, l: i64) {
        close_socket(l);
        *out = 0;
    }

    /// `sysShutdown(conn)`: `shutdown(fd, SHUT_WR)` (after the end of the
    /// stream of TLS, `close_notify`, on a connection of TLS).
    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_shutdown(out: *mut i64, c: i64) {
        if let (Some(sess), Some(d)) = (session(c), dl()) {
            tls::notify_close(d, sess);
        }
        *out = net_code(syscall6(48, c as usize, 1, 0, 0, 0, 0).min(0));
    }

    // ----- servers (D342): `epoll`, level-triggered ----------------------

    const SYS_POLL: usize = 7;
    const SYS_FCNTL: usize = 72;
    const SYS_EPOLL_WAIT: usize = 232;
    const SYS_EPOLL_CTL: usize = 233;
    const SYS_EPOLL_CREATE1: usize = 291;
    const SOCK_NONBLOCK: usize = 0o4000;
    const F_GETFL: usize = 3;
    const F_SETFL: usize = 4;
    const EPOLL_CTL_ADD: usize = 1;
    const EPOLLIN: u32 = 1;
    const POLLOUT: i16 = 4;
    const IPPROTO_TCP: usize = 6;
    const TCP_NODELAY: usize = 1;
    const EAGAIN: isize = -11;
    /// The number of the listener in the set (no connection has it).
    const LISTENER: u64 = u64::MAX;
    /// How long a response waits for a peer that does not read.
    const SEND_WAIT_MS: isize = 30_000;

    /// `struct epoll_event` of x86-64 (packed).
    #[repr(C, packed)]
    #[derive(Clone, Copy)]
    struct EpollEvent {
        events: u32,
        data: u64,
    }

    #[repr(C)]
    struct PollFd {
        fd: c_int,
        events: i16,
        revents: i16,
    }

    /// `fd` watched by the set `ep` for reading, with the number `data`.
    unsafe fn watch(ep: c_int, fd: c_int, data: u64) -> isize {
        let ev = EpollEvent { events: EPOLLIN, data };
        // SAFETY: the kernel reads the event of 12 bytes
        syscall6(SYS_EPOLL_CTL, ep as usize, EPOLL_CTL_ADD, fd as usize, &ev as *const EpollEvent as usize, 0, 0)
    }

    /// `sysServerOpen(listener)`: the listener taken by a new server (closed
    /// with it, or now when this fails).
    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_serveropen(out: *mut HandleCode, l: i64) {
        let ep = syscall6(SYS_EPOLL_CREATE1, SOCK_CLOEXEC, 0, 0, 0, 0, 0);
        let flags = if ep >= 0 { syscall6(SYS_FCNTL, l as usize, F_GETFL, 0, 0, 0, 0) } else { ep };
        let mut r = if flags >= 0 { syscall6(SYS_FCNTL, l as usize, F_SETFL, flags as usize | SOCK_NONBLOCK, 0, 0, 0) } else { flags };
        if r >= 0 {
            r = watch(ep as c_int, l as c_int, LISTENER);
        }
        if r < 0 {
            if ep >= 0 {
                linux::close(ep as c_int);
            }
            close_socket(l);
            *out = HandleCode { h: -1, code: net_code(r) };
            return;
        }
        let st = alloc::boxed::Box::new(ServerState { listener: l as c_int, next: 1, conns: alloc::vec::Vec::new(), ep: ep as c_int, tls: 0, sessions: alloc::vec::Vec::new(), scratch: Default::default() });
        *out = HandleCode { h: alloc::boxed::Box::into_raw(st) as i64, code: 0 };
    }

    /// `sysServerOpenTls(listener, cert, key)`: a server whose connections
    /// speak TLS with the certificate and the key of the PEM texts (D343).
    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_serveropentls(out: *mut HandleTextCode, l: i64, cp: *const u8, cn: i64, kp: *const u8, kn: i64) {
        let Some(d) = dl() else {
            close_socket(l);
            *out = HandleTextCode { h: -1, text: text_of("TLS needs the C library of the system (the program is static)"), code: tls::NO_LIBRARY };
            return;
        };
        ignore_sigpipe();
        let ctx = match tls::server_ctx(d, bytes_of(cp, cn), bytes_of(kp, kn)) {
            Ok(c) => c,
            Err((code, t)) => {
                close_socket(l);
                *out = HandleTextCode { h: -1, text: text_of(&t), code };
                return;
            }
        };
        let mut plain = HandleCode { h: -1, code: 0 };
        isc_sys_serveropen(&mut plain, l);
        if plain.code != 0 {
            tls::free_server_ctx(d, ctx);
            *out = HandleTextCode { h: -1, text: text_of("the server"), code: plain.code };
            return;
        }
        if let Some(st) = server::<c_int>(plain.h) {
            st.tls = ctx.0;
        }
        *out = HandleTextCode { h: plain.h, text: text_of(""), code: 0 };
    }

    /// The session of TLS of the connection `id` of a server, and whether
    /// its handshake is done.
    fn server_session(st: &ServerState<c_int>, id: i64) -> Option<(tls::Session, bool)> {
        st.sessions.iter().find(|x| x.0 == id).map(|x| (tls::Session(x.1), x.2))
    }

    /// `sysServerWait(server, max, ms)`: the connections waiting accepted,
    /// then at most `max` bytes read from each connection that has some
    /// (or has ended), waiting at most `ms` milliseconds (-1: no limit).
    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_serverwait(out: *mut WaitOut, s: i64, max: i64, ms: i64) {
        let empty = || IscArray { ptr: empty_bytes(), len: 0 };
        let Some(st) = server::<c_int>(s) else {
            *out = WaitOut { ids: empty(), lens: empty(), data: empty(), code: -9 };
            return;
        };
        let max = max.clamp(1, 1 << 24) as usize;
        let mut evs = [EpollEvent { events: 0, data: 0 }; 64];
        let (mut ids, mut lens, mut data) = st.take_scratch();
        // again while only connections were accepted (their bytes come with
        // the next wait), within the delay: as the interpreter, which gives
        // the first bytes or ends, or nothing at the end of the delay
        let deadline = (ms >= 0).then(|| crate::isc_clock_ns().saturating_add(ms as u64 * 1_000_000));
        if st.tls != 0 {
            read_pending(st, max, &mut ids, &mut lens, &mut data);
            if !ids.is_empty() {
                *out = WaitOut { ids: array_of(&ids), lens: array_of(&lens), data: array_of(&data), code: 0 };
                st.keep_scratch((ids, lens, data));
                return;
            }
        }
        loop {
            let left = match deadline {
                Some(d) => (d.saturating_sub(crate::isc_clock_ns()).div_ceil(1_000_000)).min(i32::MAX as u64) as isize,
                None => -1,
            };
            // SAFETY: the kernel writes at most 64 events at `evs`
            let n = again(|| unsafe { syscall6(SYS_EPOLL_WAIT, st.ep as usize, evs.as_mut_ptr() as usize, 64, left as usize, 0, 0) });
            if n < 0 {
                *out = WaitOut { ids: empty(), lens: empty(), data: empty(), code: net_code(n) };
                st.keep_scratch((ids, lens, data));
                return;
            }
            read_ready(st, &evs[..n as usize], max, &mut ids, &mut lens, &mut data);
            if !ids.is_empty() || n == 0 || left == 0 {
                break;
            }
        }
        *out = WaitOut { ids: array_of(&ids), lens: array_of(&lens), data: array_of(&data), code: 0 };
        st.keep_scratch((ids, lens, data));
    }

    /// The events `evs` of a wait: the connections waiting accepted, the
    /// bytes of the others read.
    unsafe fn read_ready(st: &mut ServerState<c_int>, evs: &[EpollEvent], max: usize, ids: &mut alloc::vec::Vec<i64>, lens: &mut alloc::vec::Vec<i64>, data: &mut alloc::vec::Vec<u8>) {
        for ev in evs {
            let tag = { ev.data };
            if tag == LISTENER {
                loop {
                    let c = again(|| unsafe { syscall6(SYS_ACCEPT4, st.listener as usize, 0, 0, SOCK_NONBLOCK | SOCK_CLOEXEC, 0, 0) });
                    if c < 0 {
                        break;
                    }
                    let one: c_int = 1;
                    syscall6(SYS_SETSOCKOPT, c as usize, IPPROTO_TCP, TCP_NODELAY, &one as *const c_int as usize, 4, 0);
                    let id = st.add(c as c_int);
                    if watch(st.ep, c as c_int, id as u64) < 0 {
                        st.take(id);
                        linux::close(c as c_int);
                        continue;
                    }
                    // a server of TLS: the session of the connection, its
                    // handshake made as its bytes come
                    if st.tls != 0 {
                        match dl().and_then(|d| tls::accept(d, tls::ServerCtx(st.tls), c as i64)) {
                            Some(sess) => st.sessions.push((id, sess.0, false)),
                            None => {
                                st.take(id);
                                linux::close(c as c_int);
                            }
                        }
                    }
                }
                continue;
            }
            let id = tag as i64;
            let Some(fd) = st.fd(id) else { continue };
            if st.tls != 0 {
                read_tls(st, id, max, ids, lens, data);
                continue;
            }
            data.reserve(max);
            let at = data.len();
            // SAFETY: the kernel writes at most `max` bytes at the room
            // reserved after the `at` bytes of `data`
            let r = again(|| unsafe { syscall6(SYS_RECVFROM, fd as usize, data.as_mut_ptr().add(at) as usize, max, 0, 0, 0) });
            if r == EAGAIN {
                continue;
            }
            let got = r.max(0) as usize;
            // SAFETY: the kernel wrote `got` bytes there
            data.set_len(at + got);
            ids.push(id);
            lens.push(got as i64);
        }
    }

    /// The connection `id` of a server of TLS read: its handshake a step
    /// further, or at most `max` bytes of its stream (an end or a fault: 0).
    fn read_tls(st: &mut ServerState<c_int>, id: i64, max: usize, ids: &mut alloc::vec::Vec<i64>, lens: &mut alloc::vec::Vec<i64>, data: &mut alloc::vec::Vec<u8>) {
        let (Some((sess, done)), Some(d)) = (server_session(st, id), dl()) else { return };
        if !done {
            match tls::handshake(d, sess) {
                tls::Step::Done(_) => {
                    if let Some(x) = st.sessions.iter_mut().find(|x| x.0 == id) {
                        x.2 = true;
                    }
                }
                tls::Step::WantRead | tls::Step::WantWrite => return,
                _ => {
                    ids.push(id);
                    lens.push(0);
                    return;
                }
            }
        }
        let at = data.len();
        data.resize(at + max, 0);
        match tls::read(d, sess, &mut data[at..]) {
            tls::Step::Done(n) => {
                data.truncate(at + n);
                ids.push(id);
                lens.push(n as i64);
            }
            tls::Step::WantRead | tls::Step::WantWrite => data.truncate(at),
            _ => {
                data.truncate(at);
                ids.push(id);
                lens.push(0);
            }
        }
    }

    /// The connections of a server of TLS whose session holds bytes already
    /// decrypted, read without waiting (the socket has nothing left for
    /// `epoll` to tell).
    fn read_pending(st: &mut ServerState<c_int>, max: usize, ids: &mut alloc::vec::Vec<i64>, lens: &mut alloc::vec::Vec<i64>, data: &mut alloc::vec::Vec<u8>) {
        let Some(d) = dl() else { return };
        let ready: alloc::vec::Vec<i64> = st.sessions.iter().filter(|x| x.2 && tls::pending(d, tls::Session(x.1)) > 0).map(|x| x.0).collect();
        for id in ready {
            read_tls(st, id, max, ids, lens, data);
        }
    }

    /// `sysServerSend(server, id, bytes)`: all of them, waiting for a peer
    /// that reads slowly (at most SEND_WAIT_MS without progress: `TimedOut`).
    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_serversend(out: *mut i64, s: i64, id: i64, p: *const u8, n: i64) {
        let Some(st) = server::<c_int>(s) else {
            *out = -9;
            return;
        };
        let Some(fd) = st.fd(id) else {
            *out = -9;
            return;
        };
        let n = n.max(0) as usize;
        let mut at = 0usize;
        if st.tls != 0 {
            let (Some((sess, _)), Some(d)) = (server_session(st, id), dl()) else {
                *out = -9;
                return;
            };
            // SAFETY: `p` holds `n` bytes
            let all = unsafe { core::slice::from_raw_parts(p, n) };
            while at < n {
                match tls::write(d, sess, &all[at..]) {
                    tls::Step::Done(k) => at += k,
                    st @ (tls::Step::WantWrite | tls::Step::WantRead) => {
                        let events = if st == tls::Step::WantWrite { POLLOUT } else { 1 };
                        let mut pfd = PollFd { fd, events, revents: 0 };
                        // SAFETY: one `struct pollfd`, written by the kernel
                        let w = again(|| unsafe { syscall6(SYS_POLL, &mut pfd as *mut PollFd as usize, 1, SEND_WAIT_MS as usize, 0, 0, 0) });
                        if w == 0 {
                            *out = 6;
                            return;
                        }
                    }
                    _ => {
                        *out = tls::TLS_FAULT;
                        return;
                    }
                }
            }
            *out = 0;
            return;
        }
        while at < n {
            // SAFETY: the kernel reads `n - at` bytes from `p + at`
            let r = again(|| unsafe { syscall6(SYS_SENDTO, fd as usize, p.add(at) as usize, n - at, MSG_NOSIGNAL, 0, 0) });
            if r == EAGAIN {
                let mut pfd = PollFd { fd, events: POLLOUT, revents: 0 };
                // SAFETY: one `struct pollfd`, written by the kernel
                let w = again(|| unsafe { syscall6(SYS_POLL, &mut pfd as *mut PollFd as usize, 1, SEND_WAIT_MS as usize, 0, 0, 0) });
                if w == 0 {
                    *out = 6;
                    return;
                }
                continue;
            }
            if r < 0 {
                *out = net_code(r);
                return;
            }
            at += r as usize;
        }
        *out = 0;
    }

    /// `sysServerDrop(server, id)`: the connection closed.
    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_serverdrop(out: *mut i64, s: i64, id: i64) {
        let Some(st) = server::<c_int>(s) else {
            *out = -9;
            return;
        };
        if let Some(i) = st.sessions.iter().position(|x| x.0 == id) {
            let (_, sess, _) = st.sessions.remove(i);
            if let Some(d) = dl() {
                tls::close(d, tls::Session(sess));
            }
        }
        *out = match st.take(id) {
            Some(fd) => {
                linux::close(fd);
                0
            }
            None => -9,
        };
    }

    /// `sysServerClose(server)`: its connections, its listener and its set
    /// closed, its state freed.
    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_serverclose(out: *mut i64, s: i64) {
        if s > 0 {
            // SAFETY: the state that `isc_sys_serveropen` made, freed once
            // (the handle is consumed, D339)
            let st = unsafe { alloc::boxed::Box::from_raw(s as *mut ServerState<c_int>) };
            if let Some(d) = dl() {
                for &(_, sess, _) in &st.sessions {
                    tls::close(d, tls::Session(sess));
                }
                if st.tls != 0 {
                    tls::free_server_ctx(d, tls::ServerCtx(st.tls));
                }
            }
            for &(_, fd) in &st.conns {
                linux::close(fd);
            }
            linux::close(st.listener);
            linux::close(st.ep);
        }
        *out = 0;
    }

    /// The delay of `ms` milliseconds of each read (`SO_RCVTIMEO`) and write
    /// of the socket `fd`: 0 for none.
    unsafe fn timeouts(fd: i64, ms: i64) -> isize {
        let tv = [ms.max(0) / 1000, (ms.max(0) % 1000) * 1000];
        let r = syscall6(SYS_SETSOCKOPT, fd as usize, SOL_SOCKET, SO_RCVTIMEO, tv.as_ptr() as usize, 16, 0);
        if r < 0 {
            return r;
        }
        syscall6(SYS_SETSOCKOPT, fd as usize, SOL_SOCKET, SO_SNDTIMEO, tv.as_ptr() as usize, 16, 0)
    }

    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_settimeout(out: *mut i64, c: i64, ms: i64) {
        *out = net_code(timeouts(c, ms).min(0));
    }

    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_localport(out: *mut i64, l: i64) {
        let mut addr = [0u8; 28];
        let mut len: u32 = 28;
        let r = syscall6(SYS_GETSOCKNAME, l as usize, addr.as_mut_ptr() as usize, &mut len as *mut u32 as usize, 0, 0, 0);
        *out = if r < 0 { -1 } else { u16::from_be_bytes([addr[2], addr[3]]) as i64 };
    }

    /// `sysUdpQuery(address, port, packet, ms)`: the packet sent, the first
    /// datagram from that address within `ms` milliseconds.
    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_udpquery(out: *mut IscArrayCode, a: *const u8, an: i64, port: i64, p: *const u8, n: i64, ms: i64) {
        let fail = |code: i64| IscArrayCode { a: IscArray { ptr: empty_bytes(), len: 0 }, code };
        let Some((addr, len, family)) = sockaddr(bytes_of(a, an), port, AF_INET6) else {
            *out = fail(-22);
            return;
        };
        let fd = socket(family, SOCK_DGRAM);
        if fd < 0 {
            *out = fail(net_code(fd));
            return;
        }
        let r = timeouts(fd as i64, ms.max(1));
        let r = if r < 0 { r } else { syscall6(SYS_SENDTO, fd as usize, p as usize, n.max(0) as usize, 0, addr.as_ptr() as usize, len) };
        if r < 0 {
            linux::close(fd as c_int);
            *out = fail(net_code(r));
            return;
        }
        let max = 65536usize;
        let buf = isc_alloc(max as i64);
        loop {
            let mut from = [0u8; 28];
            let mut flen: u32 = 28;
            // SAFETY: the kernel writes at most `max` bytes at `buf` and its
            // sender at `from`
            let r = again(|| unsafe { syscall6(SYS_RECVFROM, fd as usize, buf as usize, max, 0, from.as_mut_ptr() as usize, &mut flen as *mut u32 as usize) });
            if r < 0 {
                isc_free(buf);
                linux::close(fd as c_int);
                *out = fail(net_code(r));
                return;
            }
            // only the answer of the server asked
            if from[..len] == addr[..len] {
                linux::close(fd as c_int);
                *out = IscArrayCode { a: bytes_read(buf, r as usize, max), code: 0 };
                return;
            }
        }
    }

    /// `sysResolve(host)`: the module resolves the names under Linux.
    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_resolve(out: *mut IscArrayCode, _p: *const u8, _n: i64) {
        *out = IscArrayCode { a: str_array(&mut core::iter::empty(), 0), code: 7 };
    }
}

#[cfg(windows)]
mod imp {
    use super::*;
    use core::sync::atomic::AtomicBool;

    const AF_INET6: u16 = 23;
    const SOCK_STREAM: c_int = 1;
    const SOCK_DGRAM: c_int = 2;
    const SOL_SOCKET: c_int = 0xffff;
    // `~SO_REUSEADDR`: no other socket binds the port of this one (with
    // `SO_REUSEADDR` Windows let a second listener take the same port)
    const SO_EXCLUSIVEADDRUSE: c_int = !4;
    const SO_RCVTIMEO: c_int = 0x1006;
    const SO_SNDTIMEO: c_int = 0x1005;
    const INVALID_SOCKET: usize = !0;

    isc_runtime_core::system_dll! {
        "ws2_32.dll";
        fn WSAStartup(version: u16, data: *mut u8) -> c_int;
        fn WSAGetLastError() -> c_int;
        fn socket(af: c_int, kind: c_int, protocol: c_int) -> usize;
        fn connect(s: usize, name: *const u8, len: c_int) -> c_int;
        fn bind(s: usize, name: *const u8, len: c_int) -> c_int;
        fn listen(s: usize, backlog: c_int) -> c_int;
        fn accept(s: usize, addr: *mut u8, len: *mut c_int) -> usize;
        fn recv(s: usize, buf: *mut u8, len: c_int, flags: c_int) -> c_int;
        fn send(s: usize, buf: *const u8, len: c_int, flags: c_int) -> c_int;
        fn sendto(s: usize, buf: *const u8, len: c_int, flags: c_int, to: *const u8, tolen: c_int) -> c_int;
        fn recvfrom(s: usize, buf: *mut u8, len: c_int, flags: c_int, from: *mut u8, fromlen: *mut c_int) -> c_int;
        fn closesocket(s: usize) -> c_int;
        fn shutdown(s: usize, how: c_int) -> c_int;
        fn WSAPoll(fds: *mut WsaPollFd, n: u32, timeout: c_int) -> c_int;
        fn ioctlsocket(s: usize, cmd: i32, arg: *mut u32) -> c_int;
        fn setsockopt(s: usize, level: c_int, name: c_int, value: *const u8, len: c_int) -> c_int;
        fn getsockname(s: usize, name: *mut u8, len: *mut c_int) -> c_int;
        fn GetAddrInfoW(node: *const u16, service: *const u16, hints: *const c_void, result: *mut *mut AddrInfoW) -> c_int;
        fn FreeAddrInfoW(info: *mut AddrInfoW);
    }

    #[repr(C)]
    struct AddrInfoW {
        flags: c_int,
        family: c_int,
        socktype: c_int,
        protocol: c_int,
        addrlen: usize,
        canonname: *mut u16,
        addr: *const u8,
        next: *mut AddrInfoW,
    }

    static STARTED: AtomicBool = AtomicBool::new(false);

    /// Winsock started once, before the first socket.
    fn start() {
        if !STARTED.swap(true, Ordering::AcqRel) {
            let mut data = [0u8; 512];
            // SAFETY: WSADATA fits the 512 bytes given
            unsafe { WSAStartup(0x0202, data.as_mut_ptr()) };
        }
    }

    /// The code of the last error of Winsock (the codes of `system.rs`).
    fn last_code() -> i64 {
        // SAFETY: no argument
        match unsafe { WSAGetLastError() } {
            10061 => 5,
            10060 => 6,
            10013 => 2,
            10048 => 3,
            e => -(e as i64),
        }
    }

    /// The socket `h` closed (none: -1).
    unsafe fn close_socket(h: i64) {
        if h >= 0 {
            closesocket(h as usize);
        }
    }

    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_connect(out: *mut HandleCode, p: *const u8, n: i64, port: i64) {
        start();
        let Some((addr, len, family)) = sockaddr(bytes_of(p, n), port, AF_INET6) else {
            *out = HandleCode { h: -1, code: -22 };
            return;
        };
        let s = socket(family as c_int, SOCK_STREAM, 0);
        if s == INVALID_SOCKET {
            *out = HandleCode { h: -1, code: last_code() };
            return;
        }
        if connect(s, addr.as_ptr(), len as c_int) != 0 {
            let code = last_code();
            closesocket(s);
            *out = HandleCode { h: -1, code };
            return;
        }
        *out = HandleCode { h: s as i64, code: 0 };
    }

    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_listen(out: *mut HandleCode, p: *const u8, n: i64, port: i64) {
        start();
        let Some((addr, len, family)) = sockaddr(bytes_of(p, n), port, AF_INET6) else {
            *out = HandleCode { h: -1, code: -22 };
            return;
        };
        let s = socket(family as c_int, SOCK_STREAM, 0);
        if s == INVALID_SOCKET {
            *out = HandleCode { h: -1, code: last_code() };
            return;
        }
        let one: c_int = 1;
        // a port already listened to: `WSAEADDRINUSE`, `AlreadyExists` as
        // under Linux and in the interpreter
        setsockopt(s, SOL_SOCKET, SO_EXCLUSIVEADDRUSE, &one as *const c_int as *const u8, 4);
        if bind(s, addr.as_ptr(), len as c_int) != 0 || listen(s, 128) != 0 {
            let code = last_code();
            closesocket(s);
            *out = HandleCode { h: -1, code };
            return;
        }
        *out = HandleCode { h: s as i64, code: 0 };
    }

    /// `sysListenShared`: not under Windows (its headers have no
    /// `SO_REUSEPORT`, only `SO_REUSEADDR` and `SO_EXCLUSIVEADDRUSE`, D374;
    /// whether `SO_REUSEADDR` would share the connections out is not
    /// known here): the code of `EOPNOTSUPP`; `serve` keeps one loop there.
    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_listenshared(out: *mut HandleCode, _p: *const u8, _n: i64, _port: i64) {
        *out = HandleCode { h: -1, code: -95 };
    }

    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_accept(out: *mut AcceptOut, l: i64) {
        let mut addr = [0u8; 28];
        let mut len: c_int = 28;
        let s = accept(l as usize, addr.as_mut_ptr(), &mut len);
        if s == INVALID_SOCKET {
            *out = AcceptOut { h: -1, peer: new_str(b""), code: last_code() };
            return;
        }
        let peer = address_text(&addr, AF_INET6);
        *out = AcceptOut { h: s as i64, peer: new_str(peer.as_bytes()), code: 0 };
    }

    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_recv(out: *mut IscArrayCode, c: i64, max: i64) {
        let max = max.clamp(1, 1 << 24) as usize;
        let buf = isc_alloc(max as i64);
        if let Some(sess) = session(c) {
            // SAFETY: `buf` holds `max` bytes
            let b = unsafe { core::slice::from_raw_parts_mut(buf, max) };
            *out = match tls::read(tls::no_dl(), sess, b) {
                tls::Step::Done(n) => IscArrayCode { a: bytes_read(buf, n, max), code: 0 },
                tls::Step::End => IscArrayCode { a: bytes_read(buf, 0, max), code: 0 },
                st => {
                    isc_free(buf);
                    IscArrayCode { a: IscArray { ptr: empty_bytes(), len: 0 }, code: tls_code(st) }
                }
            };
            return;
        }
        let r = recv(c as usize, buf, max as c_int, 0);
        if r < 0 {
            isc_free(buf);
            *out = IscArrayCode { a: IscArray { ptr: empty_bytes(), len: 0 }, code: last_code() };
            return;
        }
        *out = IscArrayCode { a: bytes_read(buf, r as usize, max), code: 0 };
    }

    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_send(out: *mut i64, c: i64, p: *const u8, n: i64) {
        let mut at = 0usize;
        let n = n.max(0) as usize;
        if let Some(sess) = session(c) {
            // SAFETY: `p` holds `n` bytes
            let all = unsafe { core::slice::from_raw_parts(p, n) };
            while at < n {
                match tls::write(tls::no_dl(), sess, &all[at..]) {
                    tls::Step::Done(k) => at += k,
                    st => {
                        *out = tls_code(st);
                        return;
                    }
                }
            }
            *out = 0;
            return;
        }
        while at < n {
            let r = send(c as usize, p.add(at), (n - at).min(1 << 30) as c_int, 0);
            if r < 0 {
                *out = last_code();
                return;
            }
            at += r as usize;
        }
        *out = 0;
    }

    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_close(out: *mut i64, c: i64) {
        if let Some(sess) = take_session(c) {
            tls::close(tls::no_dl(), sess);
        }
        close_socket(c);
        *out = 0;
    }

    // ----- TLS of the connections (D343): SChannel ---------------------

    use isc_runtime_core::tls;

    fn tls_code(st: tls::Step) -> i64 {
        match st {
            tls::Step::WantRead | tls::Step::WantWrite => 6,
            _ => tls::TLS_FAULT,
        }
    }

    /// `sysTlsStart(conn, host, ca)`: the client of TLS on `conn`.
    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_tlsstart(out: *mut TextCode, c: i64, hp: *const u8, hn: i64, cp: *const u8, cn: i64) {
        match tls::connect(tls::no_dl(), c, bytes_of(hp, hn), bytes_of(cp, cn)) {
            Ok(sess) => {
                with_sessions(|v| v.push((c, sess)));
                *out = TextCode { text: new_str(b""), code: 0 };
            }
            Err((code, t)) => *out = TextCode { text: new_str(t.as_bytes()), code },
        }
    }

    /// `sysServerOpenTls(listener, cert, key)`: a server whose connections
    /// speak TLS (D343).
    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_serveropentls(out: *mut HandleTextCode, l: i64, cp: *const u8, cn: i64, kp: *const u8, kn: i64) {
        let ctx = match tls::server_ctx(tls::no_dl(), bytes_of(cp, cn), bytes_of(kp, kn)) {
            Ok(c) => c,
            Err((code, t)) => {
                close_socket(l);
                *out = HandleTextCode { h: -1, text: new_str(t.as_bytes()), code };
                return;
            }
        };
        let mut plain = HandleCode { h: -1, code: 0 };
        isc_sys_serveropen(&mut plain, l);
        if plain.code != 0 {
            tls::free_server_ctx(tls::no_dl(), ctx);
            *out = HandleTextCode { h: -1, text: new_str(b"the server"), code: plain.code };
            return;
        }
        if let Some(st) = server::<usize>(plain.h) {
            st.tls = ctx.0;
        }
        *out = HandleTextCode { h: plain.h, text: new_str(b""), code: 0 };
    }

    /// The connection `id` of a server of TLS read: its handshake a step
    /// further, or at most `max` bytes of its stream (an end or a fault: 0).
    fn read_tls(st: &mut ServerState<usize>, id: i64, max: usize, ids: &mut alloc::vec::Vec<i64>, lens: &mut alloc::vec::Vec<i64>, data: &mut alloc::vec::Vec<u8>) {
        let Some(x) = st.sessions.iter().find(|x| x.0 == id).copied() else { return };
        let sess = tls::Session(x.1);
        if !x.2 {
            match tls::handshake(tls::no_dl(), sess) {
                tls::Step::Done(_) => {
                    if let Some(y) = st.sessions.iter_mut().find(|y| y.0 == id) {
                        y.2 = true;
                    }
                }
                tls::Step::WantRead | tls::Step::WantWrite => return,
                _ => {
                    ids.push(id);
                    lens.push(0);
                    return;
                }
            }
        }
        let at = data.len();
        data.resize(at + max, 0);
        match tls::read(tls::no_dl(), sess, &mut data[at..]) {
            tls::Step::Done(n) => {
                data.truncate(at + n);
                ids.push(id);
                lens.push(n as i64);
            }
            tls::Step::WantRead | tls::Step::WantWrite => data.truncate(at),
            _ => {
                data.truncate(at);
                ids.push(id);
                lens.push(0);
            }
        }
    }

    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_closelistener(out: *mut i64, l: i64) {
        close_socket(l);
        *out = 0;
    }

    /// `sysShutdown(conn)`: `shutdown(s, SD_SEND)`.
    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_shutdown(out: *mut i64, c: i64) {
        if let Some(sess) = session(c) {
            tls::notify_close(tls::no_dl(), sess);
        }
        *out = if shutdown(c as usize, 1) == 0 { 0 } else { last_code() };
    }

    // ----- servers (D342): `WSAPoll` --------------------------------------

    #[repr(C)]
    struct WsaPollFd {
        fd: usize,
        events: i16,
        revents: i16,
    }

    const POLLRDNORM: i16 = 0x0100;
    const POLLWRNORM: i16 = 0x0010;
    const POLLERR: i16 = 0x0001;
    const POLLHUP: i16 = 0x0002;
    const FIONBIO: i32 = 0x8004667Eu32 as i32;
    const IPPROTO_TCP: c_int = 6;
    const TCP_NODELAY: c_int = 1;
    const WSAEWOULDBLOCK: c_int = 10035;
    const SEND_WAIT_MS: c_int = 30_000;

    /// `s` made non-blocking.
    unsafe fn nonblocking(s: usize) -> c_int {
        let mut on: u32 = 1;
        ioctlsocket(s, FIONBIO, &mut on)
    }

    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_serveropen(out: *mut HandleCode, l: i64) {
        if nonblocking(l as usize) != 0 {
            let code = last_code();
            close_socket(l);
            *out = HandleCode { h: -1, code };
            return;
        }
        let st = alloc::boxed::Box::new(ServerState { listener: l as usize, next: 1, conns: alloc::vec::Vec::new(), ep: 0usize, tls: 0, sessions: alloc::vec::Vec::new(), scratch: Default::default() });
        *out = HandleCode { h: alloc::boxed::Box::into_raw(st) as i64, code: 0 };
    }

    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_serverwait(out: *mut WaitOut, s: i64, max: i64, ms: i64) {
        let empty = || IscArray { ptr: empty_bytes(), len: 0 };
        let Some(st) = server::<usize>(s) else {
            *out = WaitOut { ids: empty(), lens: empty(), data: empty(), code: -9 };
            return;
        };
        let max = max.clamp(1, 1 << 24) as usize;
        let (mut ids, mut lens, mut data) = st.take_scratch();
        // the sessions of TLS that hold bytes already received: read first
        if st.tls != 0 {
            let ready: alloc::vec::Vec<i64> = st.sessions.iter().filter(|x| x.2 && tls::pending(tls::no_dl(), tls::Session(x.1)) > 0).map(|x| x.0).collect();
            for id in ready {
                read_tls(st, id, max, &mut ids, &mut lens, &mut data);
            }
            if !ids.is_empty() {
                *out = WaitOut { ids: array_of(&ids), lens: array_of(&lens), data: array_of(&data), code: 0 };
                st.keep_scratch((ids, lens, data));
                return;
            }
        }
        // again while only connections were accepted, within the delay (as
        // under Linux and in the interpreter)
        let deadline = (ms >= 0).then(|| crate::isc_clock_ns().saturating_add(ms as u64 * 1_000_000));
        loop {
            let left = match deadline {
                Some(d) => (d.saturating_sub(crate::isc_clock_ns()).div_ceil(1_000_000)).min(i32::MAX as u64) as c_int,
                None => -1,
            };
            let mut fds: alloc::vec::Vec<WsaPollFd> = core::iter::once(st.listener).chain(st.conns.iter().map(|c| c.1)).map(|fd| WsaPollFd { fd, events: POLLRDNORM, revents: 0 }).collect();
            let r = WSAPoll(fds.as_mut_ptr(), fds.len() as u32, left);
            if r < 0 {
                *out = WaitOut { ids: empty(), lens: empty(), data: empty(), code: last_code() };
                st.keep_scratch((ids, lens, data));
                return;
            }
            read_ready(st, &fds, max, &mut ids, &mut lens, &mut data);
            if !ids.is_empty() || r == 0 || left == 0 {
                break;
            }
        }
        *out = WaitOut { ids: array_of(&ids), lens: array_of(&lens), data: array_of(&data), code: 0 };
        st.keep_scratch((ids, lens, data));
    }

    /// The results `fds` of a poll: the connections waiting accepted, the
    /// bytes of the others read.
    unsafe fn read_ready(st: &mut ServerState<usize>, fds: &[WsaPollFd], max: usize, ids: &mut alloc::vec::Vec<i64>, lens: &mut alloc::vec::Vec<i64>, data: &mut alloc::vec::Vec<u8>) {
        // the connections ready (their numbers taken before any is added)
        let ready: alloc::vec::Vec<i64> = st.conns.iter().zip(&fds[1..]).filter(|(_, f)| f.revents & (POLLRDNORM | POLLHUP | POLLERR) != 0).map(|(c, _)| c.0).collect();
        if fds[0].revents & POLLRDNORM != 0 {
            loop {
                let c = accept(st.listener, core::ptr::null_mut(), core::ptr::null_mut());
                if c == INVALID_SOCKET {
                    break;
                }
                let one: c_int = 1;
                setsockopt(c, IPPROTO_TCP, TCP_NODELAY, &one as *const c_int as *const u8, 4);
                if nonblocking(c) != 0 {
                    closesocket(c);
                    continue;
                }
                let id = st.add(c);
                if st.tls != 0 {
                    match tls::accept(tls::no_dl(), tls::ServerCtx(st.tls), c as i64) {
                        Some(sess) => st.sessions.push((id, sess.0, false)),
                        None => {
                            st.take(id);
                            closesocket(c);
                        }
                    }
                }
            }
        }
        for id in ready {
            let Some(fd) = st.fd(id) else { continue };
            if st.tls != 0 {
                read_tls(st, id, max, ids, lens, data);
                continue;
            }
            data.reserve(max);
            let at = data.len();
            // SAFETY: Winsock writes at most `max` bytes at the room
            // reserved after the `at` bytes of `data`
            let r = recv(fd, data.as_mut_ptr().add(at), max as c_int, 0);
            if r < 0 && WSAGetLastError() == WSAEWOULDBLOCK {
                continue;
            }
            let got = r.max(0) as usize;
            // SAFETY: Winsock wrote `got` bytes there
            data.set_len(at + got);
            ids.push(id);
            lens.push(got as i64);
        }
    }

    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_serversend(out: *mut i64, s: i64, id: i64, p: *const u8, n: i64) {
        let Some(st) = server::<usize>(s) else {
            *out = -9;
            return;
        };
        let Some(fd) = st.fd(id) else {
            *out = -9;
            return;
        };
        let n = n.max(0) as usize;
        let mut at = 0usize;
        if st.tls != 0 {
            let Some(x) = st.sessions.iter().find(|x| x.0 == id) else {
                *out = -9;
                return;
            };
            // SAFETY: `p` holds `n` bytes
            let all = unsafe { core::slice::from_raw_parts(p, n) };
            while at < n {
                match tls::write(tls::no_dl(), tls::Session(x.1), &all[at..]) {
                    tls::Step::Done(k) => at += k,
                    st => {
                        *out = tls_code(st);
                        return;
                    }
                }
            }
            *out = 0;
            return;
        }
        while at < n {
            let r = send(fd, p.add(at), (n - at).min(1 << 30) as c_int, 0);
            if r < 0 {
                if WSAGetLastError() != WSAEWOULDBLOCK {
                    *out = last_code();
                    return;
                }
                let mut pfd = WsaPollFd { fd, events: POLLWRNORM, revents: 0 };
                if WSAPoll(&mut pfd, 1, SEND_WAIT_MS) == 0 {
                    *out = 6;
                    return;
                }
                continue;
            }
            at += r as usize;
        }
        *out = 0;
    }

    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_serverdrop(out: *mut i64, s: i64, id: i64) {
        let Some(st) = server::<usize>(s) else {
            *out = -9;
            return;
        };
        if let Some(i) = st.sessions.iter().position(|x| x.0 == id) {
            let (_, sess, _) = st.sessions.remove(i);
            tls::close(tls::no_dl(), tls::Session(sess));
        }
        *out = match st.take(id) {
            Some(fd) => {
                closesocket(fd);
                0
            }
            None => -9,
        };
    }

    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_serverclose(out: *mut i64, s: i64) {
        if s > 0 {
            // SAFETY: the state that `isc_sys_serveropen` made, freed once
            let st = alloc::boxed::Box::from_raw(s as *mut ServerState<usize>);
            for &(_, sess, _) in &st.sessions {
                tls::close(tls::no_dl(), tls::Session(sess));
            }
            if st.tls != 0 {
                tls::free_server_ctx(tls::no_dl(), tls::ServerCtx(st.tls));
            }
            for &(_, fd) in &st.conns {
                closesocket(fd);
            }
            closesocket(st.listener);
        }
        *out = 0;
    }

    /// The delay of each read and write of `s`, in milliseconds (a DWORD).
    unsafe fn timeouts(s: usize, ms: i64) -> c_int {
        let v: u32 = ms.clamp(0, u32::MAX as i64) as u32;
        let r = setsockopt(s, SOL_SOCKET, SO_RCVTIMEO, &v as *const u32 as *const u8, 4);
        if r != 0 {
            return r;
        }
        setsockopt(s, SOL_SOCKET, SO_SNDTIMEO, &v as *const u32 as *const u8, 4)
    }

    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_settimeout(out: *mut i64, c: i64, ms: i64) {
        *out = if timeouts(c as usize, ms) == 0 { 0 } else { last_code() };
    }

    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_localport(out: *mut i64, l: i64) {
        let mut addr = [0u8; 28];
        let mut len: c_int = 28;
        *out = if getsockname(l as usize, addr.as_mut_ptr(), &mut len) != 0 { -1 } else { u16::from_be_bytes([addr[2], addr[3]]) as i64 };
    }

    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_udpquery(out: *mut IscArrayCode, a: *const u8, an: i64, port: i64, p: *const u8, n: i64, ms: i64) {
        start();
        let fail = |code: i64| IscArrayCode { a: IscArray { ptr: empty_bytes(), len: 0 }, code };
        let Some((addr, len, family)) = sockaddr(bytes_of(a, an), port, AF_INET6) else {
            *out = fail(-22);
            return;
        };
        let s = socket(family as c_int, SOCK_DGRAM, 0);
        if s == INVALID_SOCKET {
            *out = fail(last_code());
            return;
        }
        if timeouts(s, ms.max(1)) != 0 || sendto(s, p, n.max(0) as c_int, 0, addr.as_ptr(), len as c_int) < 0 {
            let code = last_code();
            closesocket(s);
            *out = fail(code);
            return;
        }
        let max = 65536usize;
        let buf = isc_alloc(max as i64);
        loop {
            let mut from = [0u8; 28];
            let mut flen: c_int = 28;
            let r = recvfrom(s, buf, max as c_int, 0, from.as_mut_ptr(), &mut flen);
            if r < 0 {
                let code = last_code();
                isc_free(buf);
                closesocket(s);
                *out = fail(code);
                return;
            }
            if from[..len] == addr[..len] {
                closesocket(s);
                *out = IscArrayCode { a: bytes_read(buf, r as usize, max), code: 0 };
                return;
            }
        }
    }

    /// `sysResolve(host)`: the addresses of `GetAddrInfoW` (D326), as text,
    /// each once.
    #[no_mangle]
    pub unsafe extern "C" fn isc_sys_resolve(out: *mut IscArrayCode, p: *const u8, n: i64) {
        start();
        let host = alloc::string::String::from_utf8_lossy(bytes_of(p, n)).into_owned();
        let wide: alloc::vec::Vec<u16> = host.encode_utf16().chain(core::iter::once(0)).collect();
        let mut info: *mut AddrInfoW = core::ptr::null_mut();
        let r = GetAddrInfoW(wide.as_ptr(), core::ptr::null(), core::ptr::null(), &mut info);
        if r != 0 {
            let code = match r {
                11001 => 1,
                e => -(e as i64),
            };
            *out = IscArrayCode { a: str_array(&mut core::iter::empty(), 0), code };
            return;
        }
        let mut texts: alloc::vec::Vec<alloc::string::String> = alloc::vec::Vec::new();
        let mut it = info;
        while !it.is_null() {
            let a = &*it;
            if !a.addr.is_null() && a.addrlen <= 28 {
                let mut b = [0u8; 28];
                core::ptr::copy_nonoverlapping(a.addr, b.as_mut_ptr(), a.addrlen);
                // the address alone, without its port
                let t = address_text(&b, AF_INET6);
                let ip = match t.rfind(':') {
                    Some(k) => t[..k].trim_start_matches('[').trim_end_matches(']').into(),
                    None => t,
                };
                if !texts.contains(&ip) {
                    texts.push(ip);
                }
            }
            it = a.next;
        }
        FreeAddrInfoW(info);
        *out = IscArrayCode { a: str_array(&mut texts.iter().map(|t| t.as_str()), texts.len()), code: 0 };
    }
}
