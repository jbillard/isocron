//! The resources of the system in the interpreter (D339): a connection or
//! a listener of TCP, held in a cell that `close` empties; the resource is
//! given back to the system when it is closed or when its last owner goes,
//! as the compiled program does when it frees the value.

use std::cell::RefCell;
use std::net::{TcpListener, TcpStream};

#[derive(Debug)]
pub enum Resource {
    Conn(TcpStream),
    Listener(TcpListener),
    /// A listener and the connections it accepted (D342), all non-blocking,
    /// by number in the order they came (a number never given twice); a
    /// server of TLS (D343): its context and the session of each connection
    /// with whether its handshake is done.
    Server { listener: TcpListener, next: i64, conns: Vec<(i64, TcpStream)>, tls: usize, sessions: Vec<(i64, usize, bool)> },
}

/// TLS in the interpreter (D343): the code of the runtime
/// (`isc_runtime_core::tls`), with `dlopen` and `dlsym` of the C library
/// that `iscc` is linked with; the session of each connection by its
/// socket, taken out when the connection closes or its value goes.
pub mod tls {
    use isc_runtime_core::tls as core_tls;
    use std::cell::RefCell;

    #[cfg(target_os = "linux")]
    extern "C" {
        fn dlopen(name: *const std::ffi::c_char, flags: std::ffi::c_int) -> *mut std::ffi::c_void;
        fn dlsym(h: *mut std::ffi::c_void, name: *const std::ffi::c_char) -> *mut std::ffi::c_void;
    }

    /// `dlopen` and `dlsym` of the C library (none under Windows yet).
    pub fn dl() -> Option<(core_tls::DlOpen, core_tls::DlSym)> {
        #[cfg(target_os = "linux")]
        {
            Some((dlopen as core_tls::DlOpen, dlsym as core_tls::DlSym))
        }
        #[cfg(not(target_os = "linux"))]
        {
            None
        }
    }

    thread_local! {
        static SESSIONS: RefCell<Vec<(i64, core_tls::Session)>> = const { RefCell::new(Vec::new()) };
    }

    /// The socket of a stream, as the number the library of TLS reads.
    pub fn fd_of(s: &std::net::TcpStream) -> i64 {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            s.as_raw_fd() as i64
        }
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawSocket;
            s.as_raw_socket() as i64
        }
    }

    pub fn add(fd: i64, s: core_tls::Session) {
        SESSIONS.with(|v| v.borrow_mut().push((fd, s)));
    }

    pub fn get(fd: i64) -> Option<core_tls::Session> {
        SESSIONS.with(|v| v.borrow().iter().find(|x| x.0 == fd).map(|x| x.1))
    }

    /// The session of `fd` ended and freed, if it has one.
    pub fn end(fd: i64) {
        if let Some(s) = SESSIONS.with(|v| {
            let mut v = v.borrow_mut();
            v.iter().position(|x| x.0 == fd).map(|i| v.remove(i).1)
        }) {
            if let Some(d) = dl() {
                core_tls::close(d, s);
            }
        }
    }
}

impl Drop for Cell {
    /// A connection whose value goes: its session of TLS ended before its
    /// socket closes (another socket may get its number); a server: its
    /// sessions and its context.
    fn drop(&mut self) {
        match self.0.get_mut().take() {
            Some(Resource::Conn(s)) => tls::end(tls::fd_of(&s)),
            Some(Resource::Server { tls: ctx, sessions, .. }) => {
                if let Some(d) = tls::dl() {
                    for (_, sess, _) in sessions {
                        isc_runtime_core::tls::close(d, isc_runtime_core::tls::Session(sess));
                    }
                    if ctx != 0 {
                        isc_runtime_core::tls::free_server_ctx(d, isc_runtime_core::tls::ServerCtx(ctx));
                    }
                }
            }
            _ => {}
        }
    }
}

#[derive(Debug, Default)]
pub struct Cell(pub RefCell<Option<Resource>>);

impl PartialEq for Cell {
    fn eq(&self, other: &Cell) -> bool {
        std::ptr::eq(self, other)
    }
}
