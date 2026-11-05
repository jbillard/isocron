//! TLS by the library of the system (D328, D343), the one code of the
//! runtime and of the interpreter: under Linux, OpenSSL 3 (`libssl.so.3`)
//! loaded at the first use by `dlopen`, never linked, so that a program
//! that does not use TLS has none of it; its functions called through the
//! pointers that `dlsym` gives. Only a program linked with the C library of
//! the system reaches this (the TLS primitives make `iscc` link it, D289):
//! `dlopen` and `dlsym` are given by the caller.
//!
//! Every `unsafe` here is a call of a function of OpenSSL with the
//! arguments its manual names: objects that it made and that are not yet
//! freed, buffers of the lengths given, texts ended by a zero byte.

use alloc::string::String;
use alloc::vec::Vec;
use core::ffi::{c_char, c_int, c_void};

/// `dlopen(name, flags)` of the C library.
pub type DlOpen = unsafe extern "C" fn(*const c_char, c_int) -> *mut c_void;
/// `dlsym(handle, name)` of the C library.
pub type DlSym = unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void;

/// The code of an error of TLS for the module (`sys.ioError`): the
/// certificate, the handshake or the library; its text says which.
pub const TLS_FAULT: i64 = 20;
/// No `libssl.so.3` on the machine.
pub const NO_LIBRARY: i64 = 21;

/// A session of TLS on a socket: its state in the library (an `SSL` of
/// OpenSSL, the state of SChannel's context).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Session(pub usize);

/// The context of a server of TLS: the certificate (and the chain after
/// it) and the private key of the PEM texts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ServerCtx(pub usize);

/// What a step of a session on a non-blocking socket gives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Done (`n` bytes for a read or a write).
    Done(usize),
    /// The socket must be read again first.
    WantRead,
    /// The socket must be written again first.
    WantWrite,
    /// The end of the stream (closed by the peer).
    End,
    /// A fault: the session cannot go on.
    Fault,
}

/// `dlopen` and `dlsym` that find nothing: under Windows, where the
/// library of TLS is SChannel, linked as the other libraries of the system.
pub fn no_dl() -> (DlOpen, DlSym) {
    unsafe extern "C" fn none_open(_: *const c_char, _: c_int) -> *mut c_void {
        core::ptr::null_mut()
    }
    unsafe extern "C" fn none_sym(_: *mut c_void, _: *const c_char) -> *mut c_void {
        core::ptr::null_mut()
    }
    (none_open, none_sym)
}

/// The bytes of each block `-----BEGIN <label>-----` of the PEM text
/// `pem` (base64 decoded), in order.
pub fn pem_blocks(pem: &[u8], label: &str) -> Vec<Vec<u8>> {
    let text = String::from_utf8_lossy(pem);
    let begin = alloc::format!("-----BEGIN {}-----", label);
    let end = alloc::format!("-----END {}-----", label);
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(b) = text[at..].find(&begin) {
        let start = at + b + begin.len();
        let Some(e) = text[start..].find(&end) else { break };
        if let Some(der) = base64(text[start..start + e].as_bytes()) {
            out.push(der);
        }
        at = start + e + end.len();
    }
    out
}

/// The bytes of the base64 text `t` (spaces and line ends apart), or none.
fn base64(t: &[u8]) -> Option<Vec<u8>> {
    let val = |c: u8| match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    };
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0);
    for &c in t {
        if c == b'=' {
            break;
        }
        if c.is_ascii_whitespace() {
            continue;
        }
        acc = (acc << 6) | val(c)? as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

#[cfg(not(windows))]
pub use openssl::*;

/// OpenSSL 3, loaded at run time (Linux).
#[cfg(not(windows))]
mod openssl {
    use super::*;
    use core::ffi::{c_long, c_ulong};
    use core::sync::atomic::{AtomicPtr, Ordering};

    type P = *mut c_void;

    /// The functions of OpenSSL that are used.
    struct Api {
        client_method: unsafe extern "C" fn() -> *const c_void,
        server_method: unsafe extern "C" fn() -> *const c_void,
        ctx_new: unsafe extern "C" fn(*const c_void) -> P,
        ctx_free: unsafe extern "C" fn(P),
        ctx_up_ref: unsafe extern "C" fn(P) -> c_int,
        ctx_ctrl: unsafe extern "C" fn(P, c_int, c_long, P) -> c_long,
        ctx_set_options: unsafe extern "C" fn(P, u64) -> u64,
        ctx_set_verify: unsafe extern "C" fn(P, c_int, *const c_void),
        ctx_default_paths: unsafe extern "C" fn(P) -> c_int,
        ctx_cert_store: unsafe extern "C" fn(P) -> P,
        store_add_cert: unsafe extern "C" fn(P, P) -> c_int,
        ctx_use_certificate: unsafe extern "C" fn(P, P) -> c_int,
        ctx_use_private_key: unsafe extern "C" fn(P, P) -> c_int,
        ctx_check_private_key: unsafe extern "C" fn(P) -> c_int,
        bio_new_mem_buf: unsafe extern "C" fn(*const c_void, c_int) -> P,
        bio_free: unsafe extern "C" fn(P) -> c_int,
        pem_read_x509: unsafe extern "C" fn(P, P, P, P) -> P,
        pem_read_key: unsafe extern "C" fn(P, P, P, P) -> P,
        x509_free: unsafe extern "C" fn(P),
        pkey_free: unsafe extern "C" fn(P),
        ssl_new: unsafe extern "C" fn(P) -> P,
        ssl_free: unsafe extern "C" fn(P),
        ssl_set_fd: unsafe extern "C" fn(P, c_int) -> c_int,
        ssl_ctrl: unsafe extern "C" fn(P, c_int, c_long, P) -> c_long,
        ssl_set1_host: unsafe extern "C" fn(P, *const c_char) -> c_int,
        ssl_connect: unsafe extern "C" fn(P) -> c_int,
        ssl_accept_state: unsafe extern "C" fn(P),
        ssl_do_handshake: unsafe extern "C" fn(P) -> c_int,
        ssl_read: unsafe extern "C" fn(P, P, c_int) -> c_int,
        ssl_write: unsafe extern "C" fn(P, *const c_void, c_int) -> c_int,
        ssl_pending: unsafe extern "C" fn(P) -> c_int,
        ssl_shutdown: unsafe extern "C" fn(P) -> c_int,
        ssl_get_error: unsafe extern "C" fn(P, c_int) -> c_int,
        ssl_verify_result: unsafe extern "C" fn(P) -> c_long,
        verify_text: unsafe extern "C" fn(c_long) -> *const c_char,
        err_get: unsafe extern "C" fn() -> c_ulong,
        err_text: unsafe extern "C" fn(c_ulong, *mut c_char, usize),
        err_clear: unsafe extern "C" fn(),
    }

    static API: AtomicPtr<Api> = AtomicPtr::new(core::ptr::null_mut());

    const RTLD_NOW: c_int = 2;
    const SSL_VERIFY_PEER: c_int = 1;
    const SSL_CTRL_EXTRA_CHAIN_CERT: c_int = 14;
    const SSL_CTRL_MODE: c_int = 33;
    const SSL_CTRL_SET_TLSEXT_HOSTNAME: c_int = 55;
    const SSL_CTRL_SET_MIN_PROTO_VERSION: c_int = 123;
    const TLS1_2_VERSION: c_long = 0x0303;
    const SSL_MODE_ACCEPT_MOVING_WRITE_BUFFER: c_long = 2;
    const SSL_OP_IGNORE_UNEXPECTED_EOF: u64 = 1 << 7;
    const SSL_ERROR_WANT_READ: c_int = 2;
    const SSL_ERROR_WANT_WRITE: c_int = 3;
    const SSL_ERROR_ZERO_RETURN: c_int = 6;

    /// OpenSSL, loaded once (`dlopen` of `libssl.so.3`; its `libcrypto`
    /// comes with it), or none.
    fn api(dl: (DlOpen, DlSym)) -> Option<&'static Api> {
        let p = API.load(Ordering::Acquire);
        if !p.is_null() {
            // SAFETY: made below, never freed
            return Some(unsafe { &*p });
        }
        let (dlopen, dlsym) = dl;
        // SAFETY: a text ended by a zero byte; the library stays loaded
        let h = unsafe { dlopen(c"libssl.so.3".as_ptr(), RTLD_NOW) };
        if h.is_null() {
            return None;
        }
        macro_rules! f {
            ($name:literal) => {{
                // SAFETY: a name ended by a zero byte, looked up in the library
                // loaded and its dependencies; the type is the one of its manual
                let s = unsafe { dlsym(h, concat!($name, "\0").as_ptr() as *const c_char) };
                if s.is_null() {
                    return None;
                }
                // SAFETY: the function of OpenSSL of that name, of that type
                unsafe { core::mem::transmute::<*mut c_void, _>(s) }
            }};
        }
        let a = Api {
            client_method: f!("TLS_client_method"),
            server_method: f!("TLS_server_method"),
            ctx_new: f!("SSL_CTX_new"),
            ctx_free: f!("SSL_CTX_free"),
            ctx_up_ref: f!("SSL_CTX_up_ref"),
            ctx_ctrl: f!("SSL_CTX_ctrl"),
            ctx_set_options: f!("SSL_CTX_set_options"),
            ctx_set_verify: f!("SSL_CTX_set_verify"),
            ctx_default_paths: f!("SSL_CTX_set_default_verify_paths"),
            ctx_cert_store: f!("SSL_CTX_get_cert_store"),
            store_add_cert: f!("X509_STORE_add_cert"),
            ctx_use_certificate: f!("SSL_CTX_use_certificate"),
            ctx_use_private_key: f!("SSL_CTX_use_PrivateKey"),
            ctx_check_private_key: f!("SSL_CTX_check_private_key"),
            bio_new_mem_buf: f!("BIO_new_mem_buf"),
            bio_free: f!("BIO_free"),
            pem_read_x509: f!("PEM_read_bio_X509"),
            pem_read_key: f!("PEM_read_bio_PrivateKey"),
            x509_free: f!("X509_free"),
            pkey_free: f!("EVP_PKEY_free"),
            ssl_new: f!("SSL_new"),
            ssl_free: f!("SSL_free"),
            ssl_set_fd: f!("SSL_set_fd"),
            ssl_ctrl: f!("SSL_ctrl"),
            ssl_set1_host: f!("SSL_set1_host"),
            ssl_connect: f!("SSL_connect"),
            ssl_accept_state: f!("SSL_set_accept_state"),
            ssl_do_handshake: f!("SSL_do_handshake"),
            ssl_read: f!("SSL_read"),
            ssl_write: f!("SSL_write"),
            ssl_pending: f!("SSL_pending"),
            ssl_shutdown: f!("SSL_shutdown"),
            ssl_get_error: f!("SSL_get_error"),
            ssl_verify_result: f!("SSL_get_verify_result"),
            verify_text: f!("X509_verify_cert_error_string"),
            err_get: f!("ERR_get_error"),
            err_text: f!("ERR_error_string_n"),
            err_clear: f!("ERR_clear_error"),
        };
        let b = alloc::boxed::Box::into_raw(alloc::boxed::Box::new(a));
        match API.compare_exchange(core::ptr::null_mut(), b, Ordering::AcqRel, Ordering::Acquire) {
            // SAFETY: just stored, never freed
            Ok(_) => Some(unsafe { &*b }),
            Err(other) => {
                // SAFETY: `b` was not published; `other` was, never freed
                drop(unsafe { alloc::boxed::Box::from_raw(b) });
                Some(unsafe { &*other })
            }
        }
    }

    /// The text of the last error of OpenSSL, or `what`.
    fn last_error(a: &Api, what: &str) -> String {
        // SAFETY: the queue of errors of this thread
        let e = unsafe { (a.err_get)() };
        // SAFETY: the queue emptied
        unsafe { (a.err_clear)() };
        if e == 0 {
            return String::from(what);
        }
        let mut buf = [0u8; 256];
        // SAFETY: at most 256 bytes written, ended by a zero byte
        unsafe { (a.err_text)(e, buf.as_mut_ptr() as *mut c_char, buf.len()) };
        let n = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        alloc::format!("{}: {}", what, String::from_utf8_lossy(&buf[..n]))
    }

    /// The text of a zero-ended C string.
    fn c_text(p: *const c_char) -> String {
        if p.is_null() {
            return String::new();
        }
        // SAFETY: a text of OpenSSL ended by a zero byte
        let n = (0..).take_while(|&i| unsafe { *p.add(i) } != 0).count();
        // SAFETY: the `n` bytes before its zero
        String::from_utf8_lossy(unsafe { core::slice::from_raw_parts(p as *const u8, n) }).into_owned()
    }

    /// `s` with a zero byte after it.
    fn zero_ended(s: &[u8]) -> Vec<u8> {
        let mut v = s.to_vec();
        v.push(0);
        v
    }

    /// The certificates of the PEM text `pem` (one or more), or an error.
    fn certificates(a: &Api, pem: &[u8]) -> Result<Vec<P>, String> {
        // SAFETY: the bytes of `pem`, read while the BIO lives
        let bio = unsafe { (a.bio_new_mem_buf)(pem.as_ptr() as *const c_void, pem.len() as c_int) };
        if bio.is_null() {
            return Err(last_error(a, "the certificates"));
        }
        let mut out = Vec::new();
        loop {
            // SAFETY: the next certificate of the BIO, or null at its end
            let x = unsafe { (a.pem_read_x509)(bio, core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null_mut()) };
            if x.is_null() {
                break;
            }
            out.push(x);
        }
        // SAFETY: the end of the text is an error in the queue, not one of ours
        unsafe {
            (a.err_clear)();
            (a.bio_free)(bio);
        }
        if out.is_empty() {
            return Err(String::from("no certificate in the PEM text"));
        }
        Ok(out)
    }


    /// Is `host` an address (IPv4 or IPv6) rather than a name?
    fn is_address(host: &[u8]) -> bool {
        host.contains(&b':') || (!host.is_empty() && host.iter().all(|c| c.is_ascii_digit() || *c == b'.'))
    }

    /// The contexts of the clients, by the authorities they trust (the PEM
    /// text, empty for those of the system): made once, kept for the life of
    /// the program (an `SSL_CTX` of OpenSSL 3 costs a part of a millisecond to
    /// make and to load; each session holds its own reference).
    struct Contexts {
        lock: core::sync::atomic::AtomicBool,
        list: core::cell::UnsafeCell<Vec<(Vec<u8>, usize)>>,
    }

    // SAFETY: `list` is read and written only while `lock` is held
    unsafe impl Sync for Contexts {}

    static CLIENTS: Contexts = Contexts { lock: core::sync::atomic::AtomicBool::new(false), list: core::cell::UnsafeCell::new(Vec::new()) };

    /// At most this many contexts kept (a program trusts few authorities).
    const CONTEXTS_KEPT: usize = 8;

    /// The context of a client that trusts the authorities of `ca`.
    fn client_ctx(a: &Api, ca: &[u8]) -> Result<P, String> {
        while CLIENTS.lock.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
            core::hint::spin_loop();
        }
        // SAFETY: the lock is held
        let list = unsafe { &mut *CLIENTS.list.get() };
        let r = match list.iter().find(|x| x.0 == ca) {
            Some(x) => Ok(x.1 as P),
            None => new_client_ctx(a, ca).inspect(|&ctx| {
                if list.len() < CONTEXTS_KEPT {
                    list.push((ca.to_vec(), ctx as usize));
                }
            }),
        };
        // a context not kept: given to the session alone (`connect` frees its
        // own reference) — taken here once more so that both paths match
        if let Ok(ctx) = r {
            if !list.iter().any(|x| x.1 == ctx as usize) {
                CLIENTS.lock.store(false, Ordering::Release);
                return Ok(ctx);
            }
            // SAFETY: a context of OpenSSL; its count of references goes up
            unsafe { (a.ctx_up_ref)(ctx) };
        }
        CLIENTS.lock.store(false, Ordering::Release);
        r
    }

    fn new_client_ctx(a: &Api, ca: &[u8]) -> Result<P, String> {
        // SAFETY: each call of OpenSSL on the objects it made just before
        unsafe {
            let ctx = (a.ctx_new)((a.client_method)());
            if ctx.is_null() {
                return Err(last_error(a, "SSL_CTX_new"));
            }
            (a.ctx_ctrl)(ctx, SSL_CTRL_SET_MIN_PROTO_VERSION, TLS1_2_VERSION, core::ptr::null_mut());
            (a.ctx_set_options)(ctx, SSL_OP_IGNORE_UNEXPECTED_EOF);
            (a.ctx_set_verify)(ctx, SSL_VERIFY_PEER, core::ptr::null());
            let trusted = if ca.is_empty() {
                if (a.ctx_default_paths)(ctx) == 1 {
                    Ok(())
                } else {
                    Err(last_error(a, "the authorities of the system"))
                }
            } else {
                certificates(a, ca).map(|xs| {
                    let store = (a.ctx_cert_store)(ctx);
                    for x in xs {
                        (a.store_add_cert)(store, x);
                        (a.x509_free)(x);
                    }
                })
            };
            if let Err(t) = trusted {
                (a.ctx_free)(ctx);
                return Err(t);
            }
            Ok(ctx)
        }
    }

    /// The client of TLS on the connected socket `fd` (blocking): the
    /// certificate of the server checked against the authorities of the PEM
    /// text `ca` (empty: those of the system) and the name `host`; TLS 1.2 at
    /// least. The session, or the code and the text of the fault.
    pub fn connect(dl: (DlOpen, DlSym), fd: i64, host: &[u8], ca: &[u8]) -> Result<Session, (i64, String)> {
        let a = api(dl).ok_or((NO_LIBRARY, String::from("no TLS library (libssl.so.3) on this machine")))?;
        let ctx = client_ctx(a, ca).map_err(|t| (TLS_FAULT, t))?;
        // SAFETY: each call of OpenSSL on the objects it made just before
        unsafe {
            let ssl = (a.ssl_new)(ctx);
            // the session holds its own reference to the context: ours given
            // back (the list of the kept contexts holds one more)
            (a.ctx_free)(ctx);
            if ssl.is_null() {
                return Err((TLS_FAULT, last_error(a, "SSL_new")));
            }
            let name = zero_ended(host);
            if !is_address(host) {
                (a.ssl_ctrl)(ssl, SSL_CTRL_SET_TLSEXT_HOSTNAME, 0, name.as_ptr() as P);
            }
            (a.ssl_set1_host)(ssl, name.as_ptr() as *const c_char);
            (a.ssl_set_fd)(ssl, fd as c_int);
            if (a.ssl_connect)(ssl) != 1 {
                let v = (a.ssl_verify_result)(ssl);
                let text = if v != 0 {
                    alloc::format!("the certificate of {}: {}", String::from_utf8_lossy(host), c_text((a.verify_text)(v)))
                } else {
                    last_error(a, "the handshake")
                };
                (a.err_clear)();
                (a.ssl_free)(ssl);
                return Err((TLS_FAULT, text));
            }
            Ok(Session(ssl as usize))
        }
    }


    pub fn server_ctx(dl: (DlOpen, DlSym), cert: &[u8], key: &[u8]) -> Result<ServerCtx, (i64, String)> {
        let a = api(dl).ok_or((NO_LIBRARY, String::from("no TLS library (libssl.so.3) on this machine")))?;
        let xs = certificates(a, cert).map_err(|t| (TLS_FAULT, t))?;
        // SAFETY: each call of OpenSSL on the objects it made just before
        unsafe {
            let ctx = (a.ctx_new)((a.server_method)());
            if ctx.is_null() {
                for x in xs {
                    (a.x509_free)(x);
                }
                return Err((TLS_FAULT, last_error(a, "SSL_CTX_new")));
            }
            (a.ctx_ctrl)(ctx, SSL_CTRL_SET_MIN_PROTO_VERSION, TLS1_2_VERSION, core::ptr::null_mut());
            (a.ctx_set_options)(ctx, SSL_OP_IGNORE_UNEXPECTED_EOF);
            (a.ctx_ctrl)(ctx, SSL_CTRL_MODE, SSL_MODE_ACCEPT_MOVING_WRITE_BUFFER, core::ptr::null_mut());
            let mut ok = (a.ctx_use_certificate)(ctx, xs[0]) == 1;
            (a.x509_free)(xs[0]);
            // the rest of the chain, given to the context
            for &x in &xs[1..] {
                if (a.ctx_ctrl)(ctx, SSL_CTRL_EXTRA_CHAIN_CERT, 0, x) != 1 {
                    (a.x509_free)(x);
                    ok = false;
                }
            }
            let bio = (a.bio_new_mem_buf)(key.as_ptr() as *const c_void, key.len() as c_int);
            let pkey = if bio.is_null() { core::ptr::null_mut() } else { (a.pem_read_key)(bio, core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null_mut()) };
            if !bio.is_null() {
                (a.bio_free)(bio);
            }
            ok = ok && !pkey.is_null() && (a.ctx_use_private_key)(ctx, pkey) == 1 && (a.ctx_check_private_key)(ctx) == 1;
            if !pkey.is_null() {
                (a.pkey_free)(pkey);
            }
            if !ok {
                let t = last_error(a, "the certificate or the key of the server");
                (a.ctx_free)(ctx);
                return Err((TLS_FAULT, t));
            }
            Ok(ServerCtx(ctx as usize))
        }
    }

    pub fn free_server_ctx(dl: (DlOpen, DlSym), c: ServerCtx) {
        if let Some(a) = api(dl) {
            // SAFETY: a context of `server_ctx`, freed once
            unsafe { (a.ctx_free)(c.0 as P) };
        }
    }

    /// A session of the server on the accepted socket `fd` (non-blocking): its
    /// handshake made by `step`.
    pub fn accept(dl: (DlOpen, DlSym), c: ServerCtx, fd: i64) -> Option<Session> {
        let a = api(dl)?;
        // SAFETY: a context of `server_ctx` and a socket of the caller
        unsafe {
            let ssl = (a.ssl_new)(c.0 as P);
            if ssl.is_null() {
                return None;
            }
            (a.ssl_set_fd)(ssl, fd as c_int);
            (a.ssl_accept_state)(ssl);
            Some(Session(ssl as usize))
        }
    }

    fn step_of(a: &Api, ssl: P, r: c_int) -> Step {
        if r > 0 {
            return Step::Done(r as usize);
        }
        // SAFETY: the result of the last call on `ssl`
        let e = unsafe { (a.ssl_get_error)(ssl, r) };
        // SAFETY: the queue of errors of this thread emptied
        unsafe { (a.err_clear)() };
        match e {
            SSL_ERROR_WANT_READ => Step::WantRead,
            SSL_ERROR_WANT_WRITE => Step::WantWrite,
            SSL_ERROR_ZERO_RETURN => Step::End,
            _ => Step::Fault,
        }
    }

    /// The handshake of a session of the server, one step.
    pub fn handshake(dl: (DlOpen, DlSym), s: Session) -> Step {
        let Some(a) = api(dl) else { return Step::Fault };
        // SAFETY: a session of `accept`
        let r = unsafe { (a.ssl_do_handshake)(s.0 as P) };
        if r == 1 {
            Step::Done(0)
        } else {
            step_of(a, s.0 as P, r)
        }
    }

    /// At most `buf.len()` bytes of the stream of `s`.
    pub fn read(dl: (DlOpen, DlSym), s: Session, buf: &mut [u8]) -> Step {
        let Some(a) = api(dl) else { return Step::Fault };
        // SAFETY: a session, a buffer of that length
        let r = unsafe { (a.ssl_read)(s.0 as P, buf.as_mut_ptr() as P, buf.len().min(i32::MAX as usize) as c_int) };
        step_of(a, s.0 as P, r)
    }

    /// Bytes of `s` already decrypted, waiting to be read.
    pub fn pending(dl: (DlOpen, DlSym), s: Session) -> usize {
        match api(dl) {
            // SAFETY: a session
            Some(a) => unsafe { (a.ssl_pending)(s.0 as P) }.max(0) as usize,
            None => 0,
        }
    }

    /// Some of the bytes of `buf` sent by `s` (all of them on a blocking
    /// socket).
    pub fn write(dl: (DlOpen, DlSym), s: Session, buf: &[u8]) -> Step {
        let Some(a) = api(dl) else { return Step::Fault };
        if buf.is_empty() {
            return Step::Done(0);
        }
        // SAFETY: a session, a buffer of that length
        let r = unsafe { (a.ssl_write)(s.0 as P, buf.as_ptr() as *const c_void, buf.len().min(i32::MAX as usize) as c_int) };
        step_of(a, s.0 as P, r)
    }

    /// The end of the stream of `s` sent (`close_notify`), the session kept
    /// for reading.
    pub fn notify_close(dl: (DlOpen, DlSym), s: Session) {
        if let Some(a) = api(dl) {
            // SAFETY: a session
            unsafe {
                (a.ssl_shutdown)(s.0 as P);
                (a.err_clear)();
            }
        }
    }

    /// The end of the session sent (`close_notify`, not waiting for the
    /// peer's), then the session freed; its socket is the caller's.
    pub fn close(dl: (DlOpen, DlSym), s: Session) {
        if let Some(a) = api(dl) {
            // SAFETY: a session, freed once
            unsafe {
                (a.ssl_shutdown)(s.0 as P);
                (a.err_clear)();
                (a.ssl_free)(s.0 as P);
            }
        }
    }
}

#[cfg(windows)]
pub use schannel::*;

/// SChannel (Windows): the library of TLS of the system, linked as the
/// other libraries of Windows (`secur32`, `crypt32`, `ncrypt`); it does no
/// input or output, the bytes of the handshake and of the records go
/// through the socket here (`ws2_32`).
#[cfg(windows)]
mod schannel {
    use super::*;

    type Handle = [usize; 2];
    type W = *const u16;

    #[repr(C)]
    struct SecBuffer {
        cb: u32,
        kind: u32,
        pv: *mut c_void,
    }

    #[repr(C)]
    struct SecBufferDesc {
        version: u32,
        count: u32,
        buffers: *mut SecBuffer,
    }

    #[repr(C)]
    struct SchannelCred {
        version: u32,
        c_creds: u32,
        pa_cred: *const *const c_void,
        root_store: *mut c_void,
        c_mappers: u32,
        mappers: *mut c_void,
        c_algs: u32,
        algs: *mut u32,
        protocols: u32,
        min_strength: u32,
        max_strength: u32,
        lifespan: u32,
        flags: u32,
        cred_format: u32,
    }

    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct StreamSizes {
        header: u32,
        trailer: u32,
        max_message: u32,
        buffers: u32,
        block: u32,
    }

    #[repr(C)]
    struct CertContext {
        encoding: u32,
        encoded: *const u8,
        len: u32,
        info: *const c_void,
        store: *mut c_void,
    }

    #[repr(C)]
    struct ChainEngineConfig {
        size: u32,
        restricted_root: *mut c_void,
        restricted_trust: *mut c_void,
        restricted_other: *mut c_void,
        c_additional: u32,
        additional: *mut c_void,
        flags: u32,
        url_timeout: u32,
        max_cached: u32,
        cycle_modulus: u32,
        exclusive_root: *mut c_void,
        exclusive_people: *mut c_void,
        exclusive_flags: u32,
    }

    #[repr(C)]
    struct ChainPara {
        size: u32,
        usage_type: u32,
        usage_count: u32,
        usage_ids: *const c_void,
    }

    #[repr(C)]
    struct SslPolicyPara {
        size: u32,
        auth_type: u32,
        checks: u32,
        server_name: W,
    }

    #[repr(C)]
    struct PolicyPara {
        size: u32,
        flags: u32,
        extra: *const c_void,
    }

    #[repr(C)]
    struct PolicyStatus {
        size: u32,
        error: u32,
        chain_index: i32,
        element_index: i32,
        extra: *mut c_void,
    }

    #[repr(C)]
    struct KeyProvInfo {
        container: W,
        provider: W,
        prov_type: u32,
        flags: u32,
        c_params: u32,
        params: *const c_void,
        key_spec: u32,
    }

    #[link(name = "secur32")]
    extern "system" {
        fn AcquireCredentialsHandleW(principal: W, package: W, cred_use: u32, logon: *mut c_void, auth: *const c_void, get_key: *mut c_void, arg: *mut c_void, cred: *mut Handle, expiry: *mut i64) -> i32;
        fn FreeCredentialsHandle(cred: *mut Handle) -> i32;
        fn InitializeSecurityContextW(cred: *mut Handle, ctx: *mut Handle, target: W, req: u32, r1: u32, rep: u32, input: *mut SecBufferDesc, r2: u32, new_ctx: *mut Handle, output: *mut SecBufferDesc, attrs: *mut u32, expiry: *mut i64) -> i32;
        fn AcceptSecurityContext(cred: *mut Handle, ctx: *mut Handle, input: *mut SecBufferDesc, req: u32, rep: u32, new_ctx: *mut Handle, output: *mut SecBufferDesc, attrs: *mut u32, expiry: *mut i64) -> i32;
        fn DeleteSecurityContext(ctx: *mut Handle) -> i32;
        fn FreeContextBuffer(p: *mut c_void) -> i32;
        fn QueryContextAttributesW(ctx: *mut Handle, attr: u32, buf: *mut c_void) -> i32;
        fn EncryptMessage(ctx: *mut Handle, qop: u32, msg: *mut SecBufferDesc, seq: u32) -> i32;
        fn DecryptMessage(ctx: *mut Handle, msg: *mut SecBufferDesc, seq: u32, qop: *mut u32) -> i32;
        fn ApplyControlToken(ctx: *mut Handle, input: *mut SecBufferDesc) -> i32;
    }

    #[link(name = "crypt32")]
    extern "system" {
        fn CertCreateCertificateContext(enc: u32, der: *const u8, len: u32) -> *const CertContext;
        fn CertFreeCertificateContext(c: *const CertContext) -> i32;
        fn CertOpenStore(provider: usize, enc: u32, prov: usize, flags: u32, para: *const c_void) -> *mut c_void;
        fn CertAddCertificateContextToStore(store: *mut c_void, c: *const CertContext, disposition: u32, out: *mut *const CertContext) -> i32;
        fn CertCloseStore(store: *mut c_void, flags: u32) -> i32;
        fn CertCreateCertificateChainEngine(config: *const ChainEngineConfig, engine: *mut *mut c_void) -> i32;
        fn CertFreeCertificateChainEngine(engine: *mut c_void);
        fn CertGetCertificateChain(engine: *mut c_void, cert: *const CertContext, time: *const c_void, store: *mut c_void, para: *const ChainPara, flags: u32, reserved: *mut c_void, chain: *mut *const c_void) -> i32;
        fn CertFreeCertificateChain(chain: *const c_void);
        fn CertVerifyCertificateChainPolicy(oid: usize, chain: *const c_void, para: *const PolicyPara, status: *mut PolicyStatus) -> i32;
    }

    #[link(name = "ncrypt")]
    extern "system" {
        fn NCryptOpenStorageProvider(prov: *mut usize, name: W, flags: u32) -> i32;
        fn NCryptDeleteKey(key: usize, flags: u32) -> i32;
        fn NCryptFreeObject(h: usize) -> i32;
    }

    #[link(name = "ws2_32")]
    extern "system" {
        fn send(s: usize, buf: *const u8, len: c_int, flags: c_int) -> c_int;
        fn recv(s: usize, buf: *mut u8, len: c_int, flags: c_int) -> c_int;
        fn WSAGetLastError() -> c_int;
        fn WSAPoll(fds: *mut PollFd, n: u32, timeout: c_int) -> c_int;
    }

    #[repr(C)]
    struct PollFd {
        fd: usize,
        events: i16,
        revents: i16,
    }

    const SECPKG_CRED_INBOUND: u32 = 1;
    const SECPKG_CRED_OUTBOUND: u32 = 2;
    const SCHANNEL_CRED_VERSION: u32 = 4;
    const SP_PROT_TLS1_2_SERVER: u32 = 0x400;
    const SP_PROT_TLS1_2_CLIENT: u32 = 0x800;
    const SCH_CRED_MANUAL_CRED_VALIDATION: u32 = 0x8;
    const SCH_CRED_NO_DEFAULT_CREDS: u32 = 0x10;
    const SCH_CRED_AUTO_CRED_VALIDATION: u32 = 0x20;
    const SCH_USE_STRONG_CRYPTO: u32 = 0x0040_0000;
    const ISC_REQ: u32 = 0x4 | 0x8 | 0x10 | 0x100 | 0x4000 | 0x8000;
    const ASC_REQ: u32 = 0x4 | 0x8 | 0x10 | 0x100 | 0x8000 | 0x10000;
    const SECBUFFER_EMPTY: u32 = 0;
    const SECBUFFER_DATA: u32 = 1;
    const SECBUFFER_TOKEN: u32 = 2;
    const SECBUFFER_EXTRA: u32 = 5;
    const SECBUFFER_STREAM_TRAILER: u32 = 6;
    const SECBUFFER_STREAM_HEADER: u32 = 7;
    const SEC_E_OK: i32 = 0;
    const SEC_I_CONTINUE_NEEDED: i32 = 0x0009_0312;
    const SEC_I_CONTEXT_EXPIRED: i32 = 0x0009_0317;
    const SEC_E_INCOMPLETE_MESSAGE: i32 = 0x8009_0318u32 as i32;
    const SECPKG_ATTR_STREAM_SIZES: u32 = 4;
    const SECPKG_ATTR_REMOTE_CERT_CONTEXT: u32 = 0x53;
    const ENCODING: u32 = 0x1 | 0x10000;
    const CERT_STORE_PROV_MEMORY: usize = 2;
    const CERT_STORE_ADD_ALWAYS: u32 = 4;
    const CERT_CHAIN_POLICY_SSL: usize = 4;
    const AUTHTYPE_SERVER: u32 = 2;
    const CERT_KEY_PROV_INFO_PROP_ID: u32 = 2;
    const WSAEWOULDBLOCK: c_int = 10035;
    const POLLWRNORM: i16 = 0x0010;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(core::iter::once(0)).collect()
    }

    /// The state of a session: its handles, the bytes received not yet
    /// used (a token of the handshake, records not yet decrypted), the
    /// bytes decrypted not yet given.
    struct State {
        cred: Handle,
        own_cred: bool,
        ctx: Handle,
        has_ctx: bool,
        sizes: StreamSizes,
        fd: usize,
        raw: Vec<u8>,
        plain: Vec<u8>,
        done: bool,
        ended: bool,
        server: bool,
    }

    fn state<'a>(s: Session) -> &'a mut State {
        // SAFETY: a session of `connect` or `accept`, not yet closed
        unsafe { &mut *(s.0 as *mut State) }
    }

    /// The bytes of `b` sent, waiting for a socket that does not take them
    /// at once (non-blocking: at most 30 s).
    fn send_all(fd: usize, b: &[u8]) -> bool {
        let mut at = 0;
        while at < b.len() {
            // SAFETY: the bytes of `b` from `at`
            let r = unsafe { send(fd, b[at..].as_ptr(), (b.len() - at).min(1 << 30) as c_int, 0) };
            if r > 0 {
                at += r as usize;
                continue;
            }
            // SAFETY: the error of this thread
            if r < 0 && unsafe { WSAGetLastError() } == WSAEWOULDBLOCK {
                let mut p = PollFd { fd, events: POLLWRNORM, revents: 0 };
                // SAFETY: one structure, written by Winsock
                if unsafe { WSAPoll(&mut p, 1, 30_000) } <= 0 {
                    return false;
                }
                continue;
            }
            return false;
        }
        true
    }

    /// What a read of the socket gave.
    enum Got {
        Bytes,
        WouldBlock,
        End,
        Fault,
    }

    /// Some bytes of the socket added to `raw`.
    fn receive(fd: usize, raw: &mut Vec<u8>) -> Got {
        let at = raw.len();
        raw.resize(at + 32768, 0);
        // SAFETY: the room of 32768 bytes after `at`
        let r = unsafe { recv(fd, raw.as_mut_ptr().add(at), 32768, 0) };
        if r > 0 {
            raw.truncate(at + r as usize);
            return Got::Bytes;
        }
        raw.truncate(at);
        if r == 0 {
            return Got::End;
        }
        // SAFETY: the error of this thread
        match unsafe { WSAGetLastError() } {
            WSAEWOULDBLOCK | 10060 => Got::WouldBlock,
            _ => Got::Fault,
        }
    }

    /// The tokens of SChannel in `out` sent, then freed.
    fn send_tokens(fd: usize, out: &mut [SecBuffer]) -> bool {
        let mut ok = true;
        for b in out.iter_mut() {
            if !b.pv.is_null() {
                if b.cb > 0 {
                    // SAFETY: a token of `cb` bytes that SChannel made
                    ok &= send_all(fd, unsafe { core::slice::from_raw_parts(b.pv as *const u8, b.cb as usize) });
                }
                // SAFETY: memory of SChannel, freed once
                unsafe { FreeContextBuffer(b.pv) };
                b.pv = core::ptr::null_mut();
            }
        }
        ok
    }

    /// The bytes of `raw` left after a step: those of the buffer `EXTRA`
    /// (at its end), or none.
    fn keep_extra(raw: &mut Vec<u8>, inp: &[SecBuffer]) {
        match inp.iter().find(|b| b.kind == SECBUFFER_EXTRA) {
            Some(b) => {
                let n = (b.cb as usize).min(raw.len());
                let start = raw.len() - n;
                raw.drain(..start);
            }
            None => raw.clear(),
        }
    }

    /// The text of a fault of SChannel or of a chain of certificates.
    fn fault_text(code: u32, host: &str) -> String {
        let cert = |why: &str| alloc::format!("the certificate of {}: {}", host, why);
        match code {
            0x8009_0325 | 0x800B_0109 => cert("unable to get local issuer certificate (its authority is not trusted)"),
            0x8009_0322 | 0x800B_010F => cert("the name is not the server's"),
            0x8009_0328 | 0x800B_0101 => cert("expired"),
            0x800B_010A => cert("its chain does not end at a trusted authority"),
            0x8009_0327 | 0x8009_0350 => cert("revoked or not checked"),
            _ => alloc::format!("the handshake: error 0x{:08x} of SChannel", code),
        }
    }

    /// The certificate of the server of `ctx` checked against the
    /// authorities `roots` only and the name `host`.
    fn verify(ctx: &mut Handle, roots: &[Vec<u8>], host: &str) -> Result<(), String> {
        let mut cert: *const CertContext = core::ptr::null();
        // SAFETY: the context of a handshake done; its certificate given to us
        if unsafe { QueryContextAttributesW(ctx, SECPKG_ATTR_REMOTE_CERT_CONTEXT, &mut cert as *mut _ as *mut c_void) } != SEC_E_OK || cert.is_null() {
            return Err(alloc::format!("the certificate of {}: none given", host));
        }
        // SAFETY: a store in memory, the certificates added, each freed
        // after; the engine, the chain and the store freed at the end
        unsafe {
            let store = CertOpenStore(CERT_STORE_PROV_MEMORY, 0, 0, 0, core::ptr::null());
            for der in roots {
                let c = CertCreateCertificateContext(ENCODING, der.as_ptr(), der.len() as u32);
                if !c.is_null() {
                    CertAddCertificateContextToStore(store, c, CERT_STORE_ADD_ALWAYS, core::ptr::null_mut());
                    CertFreeCertificateContext(c);
                }
            }
            let config = ChainEngineConfig {
                size: core::mem::size_of::<ChainEngineConfig>() as u32,
                restricted_root: core::ptr::null_mut(),
                restricted_trust: core::ptr::null_mut(),
                restricted_other: core::ptr::null_mut(),
                c_additional: 0,
                additional: core::ptr::null_mut(),
                flags: 0,
                url_timeout: 0,
                max_cached: 0,
                cycle_modulus: 0,
                exclusive_root: store,
                exclusive_people: core::ptr::null_mut(),
                exclusive_flags: 1,
            };
            let mut engine = core::ptr::null_mut();
            let mut result = Err(alloc::format!("the certificate of {}: no chain", host));
            if CertCreateCertificateChainEngine(&config, &mut engine) != 0 {
                let para = ChainPara { size: core::mem::size_of::<ChainPara>() as u32, usage_type: 0, usage_count: 0, usage_ids: core::ptr::null() };
                let mut chain = core::ptr::null();
                if CertGetCertificateChain(engine, cert, core::ptr::null(), (*cert).store, &para, 0, core::ptr::null_mut(), &mut chain) != 0 {
                    let name = wide(host);
                    let ssl = SslPolicyPara { size: core::mem::size_of::<SslPolicyPara>() as u32, auth_type: AUTHTYPE_SERVER, checks: 0, server_name: name.as_ptr() };
                    let policy = PolicyPara { size: core::mem::size_of::<PolicyPara>() as u32, flags: 0, extra: &ssl as *const _ as *const c_void };
                    let mut status = PolicyStatus { size: core::mem::size_of::<PolicyStatus>() as u32, error: 0, chain_index: 0, element_index: 0, extra: core::ptr::null_mut() };
                    if CertVerifyCertificateChainPolicy(CERT_CHAIN_POLICY_SSL, chain, &policy, &mut status) != 0 {
                        result = if status.error == 0 { Ok(()) } else { Err(fault_text(status.error, host)) };
                    }
                    CertFreeCertificateChain(chain);
                }
                CertFreeCertificateChainEngine(engine);
            }
            CertCloseStore(store, 0);
            CertFreeCertificateContext(cert);
            result
        }
    }

    fn sizes_of(ctx: &mut Handle) -> StreamSizes {
        let mut z = StreamSizes::default();
        // SAFETY: a context whose handshake is done
        unsafe { QueryContextAttributesW(ctx, SECPKG_ATTR_STREAM_SIZES, &mut z as *mut _ as *mut c_void) };
        z
    }

    /// The client of TLS on the connected socket `fd` (blocking): the
    /// certificate checked by SChannel against the authorities of the
    /// system and the name `host`, or here against those of `ca` only.
    pub fn connect(_dl: (DlOpen, DlSym), fd: i64, host: &[u8], ca: &[u8]) -> Result<Session, (i64, String)> {
        let host = String::from_utf8_lossy(host).into_owned();
        let roots = if ca.is_empty() { Vec::new() } else { pem_blocks(ca, "CERTIFICATE") };
        if !ca.is_empty() && roots.is_empty() {
            return Err((TLS_FAULT, String::from("no certificate in the PEM text")));
        }
        let fd = fd as usize;
        let cred_data = SchannelCred {
            version: SCHANNEL_CRED_VERSION,
            c_creds: 0,
            pa_cred: core::ptr::null(),
            root_store: core::ptr::null_mut(),
            c_mappers: 0,
            mappers: core::ptr::null_mut(),
            c_algs: 0,
            algs: core::ptr::null_mut(),
            protocols: SP_PROT_TLS1_2_CLIENT,
            min_strength: 0,
            max_strength: 0,
            lifespan: 0,
            flags: SCH_CRED_NO_DEFAULT_CREDS | SCH_USE_STRONG_CRYPTO | if ca.is_empty() { SCH_CRED_AUTO_CRED_VALIDATION } else { SCH_CRED_MANUAL_CRED_VALIDATION },
            cred_format: 0,
        };
        let package = wide("Microsoft Unified Security Protocol Provider");
        let mut cred: Handle = [0; 2];
        // SAFETY: the credentials of a client, written at `cred`
        let r = unsafe { AcquireCredentialsHandleW(core::ptr::null(), package.as_ptr(), SECPKG_CRED_OUTBOUND, core::ptr::null_mut(), &cred_data as *const _ as *const c_void, core::ptr::null_mut(), core::ptr::null_mut(), &mut cred, core::ptr::null_mut()) };
        if r != SEC_E_OK {
            return Err((TLS_FAULT, fault_text(r as u32, &host)));
        }
        let target = wide(&host);
        let mut ctx: Handle = [0; 2];
        let mut has_ctx = false;
        let mut raw: Vec<u8> = Vec::new();
        let fail = |cred: &mut Handle, ctx: &mut Handle, has: bool, t: String| {
            // SAFETY: the handles made above, freed once
            unsafe {
                if has {
                    DeleteSecurityContext(ctx);
                }
                FreeCredentialsHandle(cred);
            }
            Err((TLS_FAULT, t))
        };
        loop {
            let mut inp = [SecBuffer { cb: raw.len() as u32, kind: SECBUFFER_TOKEN, pv: raw.as_mut_ptr() as *mut c_void }, SecBuffer { cb: 0, kind: SECBUFFER_EMPTY, pv: core::ptr::null_mut() }];
            let mut in_desc = SecBufferDesc { version: 0, count: 2, buffers: inp.as_mut_ptr() };
            let mut out = [SecBuffer { cb: 0, kind: SECBUFFER_TOKEN, pv: core::ptr::null_mut() }];
            let mut out_desc = SecBufferDesc { version: 0, count: 1, buffers: out.as_mut_ptr() };
            let mut attrs = 0u32;
            // SAFETY: the handles of this handshake, the buffers above
            let r = unsafe {
                InitializeSecurityContextW(
                    &mut cred,
                    if has_ctx { &mut ctx } else { core::ptr::null_mut() },
                    target.as_ptr(),
                    ISC_REQ,
                    0,
                    0,
                    if has_ctx { &mut in_desc } else { core::ptr::null_mut() },
                    0,
                    &mut ctx,
                    &mut out_desc,
                    &mut attrs,
                    core::ptr::null_mut(),
                )
            };
            if r == SEC_E_OK || r == SEC_I_CONTINUE_NEEDED {
                has_ctx = true;
            }
            if !send_tokens(fd, &mut out) {
                return fail(&mut cred, &mut ctx, has_ctx, String::from("the handshake: the server went"));
            }
            match r {
                SEC_E_OK => {
                    keep_extra(&mut raw, &inp);
                    break;
                }
                SEC_I_CONTINUE_NEEDED => {
                    keep_extra(&mut raw, &inp);
                    if raw.is_empty() && !matches!(receive(fd, &mut raw), Got::Bytes) {
                        return fail(&mut cred, &mut ctx, has_ctx, String::from("the handshake: the server went"));
                    }
                }
                SEC_E_INCOMPLETE_MESSAGE => {
                    if !matches!(receive(fd, &mut raw), Got::Bytes) {
                        return fail(&mut cred, &mut ctx, has_ctx, String::from("the handshake: the server went"));
                    }
                }
                _ => return fail(&mut cred, &mut ctx, has_ctx, fault_text(r as u32, &host)),
            }
        }
        if !roots.is_empty() {
            if let Err(t) = verify(&mut ctx, &roots, &host) {
                return fail(&mut cred, &mut ctx, true, t);
            }
        }
        let sizes = sizes_of(&mut ctx);
        let st = alloc::boxed::Box::new(State { cred, own_cred: true, ctx, has_ctx: true, sizes, fd, raw, plain: Vec::new(), done: true, ended: false, server: false });
        Ok(Session(alloc::boxed::Box::into_raw(st) as usize))
    }

    /// A value of DER: its tag, its length, its bytes.
    fn der(tag: u8, body: &[u8]) -> Vec<u8> {
        let mut v = alloc::vec![tag];
        let n = body.len();
        if n < 0x80 {
            v.push(n as u8);
        } else {
            let bytes = n.to_be_bytes();
            let skip = bytes.iter().take_while(|&&b| b == 0).count();
            v.push(0x80 | (8 - skip) as u8);
            v.extend_from_slice(&bytes[skip..]);
        }
        v.extend_from_slice(body);
        v
    }

    fn seq(parts: &[&[u8]]) -> Vec<u8> {
        der(0x30, &parts.concat())
    }

    /// A PKCS #12 (a PFX) of the certificate `cert` and the private key of
    /// PKCS #8 `key`, neither encrypted nor signed (no password, no MAC):
    /// the form that `PFXImportCertStore` reads, the bags of the key and of
    /// the certificate tied by the same `localKeyID`.
    fn pfx(cert: &[u8], key: &[u8]) -> Vec<u8> {
        const DATA: &[u8] = &[0x06, 0x09, 0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x07, 0x01];
        const KEY_BAG: &[u8] = &[0x06, 0x0B, 0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x0C, 0x0A, 0x01, 0x01];
        const CERT_BAG: &[u8] = &[0x06, 0x0B, 0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x0C, 0x0A, 0x01, 0x03];
        const X509: &[u8] = &[0x06, 0x0A, 0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x09, 0x16, 0x01];
        const LOCAL_KEY_ID: &[u8] = &[0x06, 0x09, 0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x09, 0x15];
        let attrs = der(0x31, &seq(&[LOCAL_KEY_ID, &der(0x31, &der(0x04, &[1, 2, 3, 4]))]));
        let key_bag = seq(&[KEY_BAG, &der(0xA0, key), &attrs]);
        let cert_bag = seq(&[CERT_BAG, &der(0xA0, &seq(&[X509, &der(0xA0, &der(0x04, cert))])), &attrs]);
        let contents = seq(&[&key_bag, &cert_bag]);
        let info = |body: &[u8]| seq(&[DATA, &der(0xA0, &der(0x04, body))]);
        let safe = seq(&[&info(&contents)]);
        seq(&[&[0x02, 0x01, 0x03], &info(&safe)])
    }

    /// The credentials of a server: its certificate and its key, imported
    /// by `PFXImportCertStore` from a PFX made here (SChannel signs in the
    /// process of the system, `lsass`, which uses the keys that this import
    /// makes; a key that `NCryptImportKey` makes and that this process can
    /// use, it refuses: `0x8009030d`, D343), the container of the key
    /// removed with the server.
    struct Server {
        cred: Handle,
        cert: *const CertContext,
        store: *mut c_void,
        provider: Vec<u16>,
        container: Vec<u16>,
    }

    extern "system" {
        fn PFXImportCertStore(blob: *const [usize; 2], password: W, flags: u32) -> *mut c_void;
        fn CertEnumCertificatesInStore(store: *mut c_void, prev: *const CertContext) -> *const CertContext;
        fn CertGetCertificateContextProperty(c: *const CertContext, prop: u32, data: *mut c_void, n: *mut u32) -> i32;
        fn NCryptOpenKey(prov: usize, key: *mut usize, name: W, spec: u32, flags: u32) -> i32;
    }

    /// The text of a wide string ended by a zero, with its zero.
    fn wide_of(p: W) -> Vec<u16> {
        let mut v = Vec::new();
        // SAFETY: a text of the system ended by a zero
        unsafe {
            let mut i = 0;
            while !p.is_null() && *p.add(i) != 0 {
                v.push(*p.add(i));
                i += 1;
            }
        }
        v.push(0);
        v
    }

    /// The key of the container `container` of the provider of keys
    /// `provider` removed.
    fn remove_key(provider: &[u16], container: &[u16]) {
        // SAFETY: the provider and the key opened, the key deleted (which
        // frees it), the provider freed
        unsafe {
            let mut prov = 0usize;
            if NCryptOpenStorageProvider(&mut prov, provider.as_ptr(), 0) == 0 {
                let mut k = 0usize;
                if NCryptOpenKey(prov, &mut k, container.as_ptr(), 0, 0) == 0 {
                    NCryptDeleteKey(k, 0);
                }
                NCryptFreeObject(prov);
            }
        }
    }

    pub fn server_ctx(_dl: (DlOpen, DlSym), cert: &[u8], key: &[u8]) -> Result<ServerCtx, (i64, String)> {
        let certs = pem_blocks(cert, "CERTIFICATE");
        let Some(der_cert) = certs.first() else { return Err((TLS_FAULT, String::from("no certificate in the PEM text"))) };
        let keys = pem_blocks(key, "PRIVATE KEY");
        let Some(der_key) = keys.first() else { return Err((TLS_FAULT, String::from("the certificate or the key of the server: no key of PKCS #8 (BEGIN PRIVATE KEY) in the PEM text"))) };
        let p = pfx(der_cert, der_key);
        const PKCS12_PREFER_CNG_KSP: u32 = 0x100;
        // SAFETY: each call on the objects made just before, each freed on
        // the paths that fail
        unsafe {
            let blob: [usize; 2] = [p.len(), p.as_ptr() as usize];
            let empty = wide("");
            let store = PFXImportCertStore(&blob, empty.as_ptr(), PKCS12_PREFER_CNG_KSP);
            if store.is_null() {
                return Err((TLS_FAULT, String::from("the certificate or the key of the server: the certificate and the key do not go together")));
            }
            let c = CertEnumCertificatesInStore(store, core::ptr::null());
            let mut info = [0u64; 128];
            let mut n = 1024u32;
            if c.is_null() || CertGetCertificateContextProperty(c, CERT_KEY_PROV_INFO_PROP_ID, info.as_mut_ptr() as *mut c_void, &mut n) == 0 {
                if !c.is_null() {
                    CertFreeCertificateContext(c);
                }
                CertCloseStore(store, 0);
                return Err((TLS_FAULT, String::from("the certificate or the key of the server: the key does not go with the certificate")));
            }
            let ki = &*(info.as_ptr() as *const KeyProvInfo);
            let (provider, container) = (wide_of(ki.provider), wide_of(ki.container));
            let creds = [c];
            let data = SchannelCred {
                version: SCHANNEL_CRED_VERSION,
                c_creds: 1,
                pa_cred: creds.as_ptr() as *const *const c_void,
                root_store: core::ptr::null_mut(),
                c_mappers: 0,
                mappers: core::ptr::null_mut(),
                c_algs: 0,
                algs: core::ptr::null_mut(),
                protocols: SP_PROT_TLS1_2_SERVER,
                min_strength: 0,
                max_strength: 0,
                lifespan: 0,
                flags: SCH_USE_STRONG_CRYPTO,
                cred_format: 0,
            };
            let package = wide("Microsoft Unified Security Protocol Provider");
            let mut cred: Handle = [0; 2];
            let r = AcquireCredentialsHandleW(core::ptr::null(), package.as_ptr(), SECPKG_CRED_INBOUND, core::ptr::null_mut(), &data as *const _ as *const c_void, core::ptr::null_mut(), core::ptr::null_mut(), &mut cred, core::ptr::null_mut());
            if r != SEC_E_OK {
                CertFreeCertificateContext(c);
                CertCloseStore(store, 0);
                remove_key(&provider, &container);
                return Err((TLS_FAULT, alloc::format!("the certificate or the key of the server: 0x{:08x} of SChannel", r as u32)));
            }
            let s = alloc::boxed::Box::new(Server { cred, cert: c, store, provider, container });
            Ok(ServerCtx(alloc::boxed::Box::into_raw(s) as usize))
        }
    }

    pub fn free_server_ctx(_dl: (DlOpen, DlSym), c: ServerCtx) {
        // SAFETY: a context of `server_ctx`, freed once
        unsafe {
            let mut s = alloc::boxed::Box::from_raw(c.0 as *mut Server);
            FreeCredentialsHandle(&mut s.cred);
            CertFreeCertificateContext(s.cert);
            CertCloseStore(s.store, 0);
            // the key and its container removed
            remove_key(&s.provider, &s.container);
        }
    }

    /// A session of the server on the accepted socket `fd` (non-blocking).
    pub fn accept(_dl: (DlOpen, DlSym), c: ServerCtx, fd: i64) -> Option<Session> {
        // SAFETY: a context of `server_ctx`
        let cred = unsafe { (*(c.0 as *const Server)).cred };
        let st = alloc::boxed::Box::new(State { cred, own_cred: false, ctx: [0; 2], has_ctx: false, sizes: StreamSizes::default(), fd: fd as usize, raw: Vec::new(), plain: Vec::new(), done: false, ended: false, server: true });
        Some(Session(alloc::boxed::Box::into_raw(st) as usize))
    }

    /// The handshake of a session of the server, as far as its bytes go.
    pub fn handshake(_dl: (DlOpen, DlSym), s: Session) -> Step {
        let st = state(s);
        if st.done {
            return Step::Done(0);
        }
        match receive(st.fd, &mut st.raw) {
            Got::Bytes => {}
            Got::WouldBlock if !st.raw.is_empty() => {}
            Got::WouldBlock => return Step::WantRead,
            Got::End | Got::Fault => return Step::Fault,
        }
        loop {
            let mut inp = [SecBuffer { cb: st.raw.len() as u32, kind: SECBUFFER_TOKEN, pv: st.raw.as_mut_ptr() as *mut c_void }, SecBuffer { cb: 0, kind: SECBUFFER_EMPTY, pv: core::ptr::null_mut() }];
            let mut in_desc = SecBufferDesc { version: 0, count: 2, buffers: inp.as_mut_ptr() };
            let mut out = [SecBuffer { cb: 0, kind: SECBUFFER_TOKEN, pv: core::ptr::null_mut() }];
            let mut out_desc = SecBufferDesc { version: 0, count: 1, buffers: out.as_mut_ptr() };
            let mut attrs = 0u32;
            let has = st.has_ctx;
            // SAFETY: the handles of this handshake, the buffers above
            let r = unsafe { AcceptSecurityContext(&mut st.cred, if has { &mut st.ctx } else { core::ptr::null_mut() }, &mut in_desc, ASC_REQ, 0, &mut st.ctx, &mut out_desc, &mut attrs, core::ptr::null_mut()) };
            if r == SEC_E_OK || r == SEC_I_CONTINUE_NEEDED {
                st.has_ctx = true;
            }
            if !send_tokens(st.fd, &mut out) {
                return Step::Fault;
            }
            match r {
                SEC_E_OK => {
                    keep_extra(&mut st.raw, &inp);
                    st.done = true;
                    st.sizes = sizes_of(&mut st.ctx);
                    return Step::Done(0);
                }
                SEC_I_CONTINUE_NEEDED => {
                    keep_extra(&mut st.raw, &inp);
                    if st.raw.is_empty() {
                        return Step::WantRead;
                    }
                }
                SEC_E_INCOMPLETE_MESSAGE => return Step::WantRead,
                _ => return Step::Fault,
            }
        }
    }

    /// The records of `raw` decrypted into `plain` while they are whole.
    fn decrypt(st: &mut State) -> Result<(), Step> {
        while !st.raw.is_empty() {
            let mut bufs = [
                SecBuffer { cb: st.raw.len() as u32, kind: SECBUFFER_DATA, pv: st.raw.as_mut_ptr() as *mut c_void },
                SecBuffer { cb: 0, kind: SECBUFFER_EMPTY, pv: core::ptr::null_mut() },
                SecBuffer { cb: 0, kind: SECBUFFER_EMPTY, pv: core::ptr::null_mut() },
                SecBuffer { cb: 0, kind: SECBUFFER_EMPTY, pv: core::ptr::null_mut() },
            ];
            let mut desc = SecBufferDesc { version: 0, count: 4, buffers: bufs.as_mut_ptr() };
            // SAFETY: a context whose handshake is done, the record in place
            let r = unsafe { DecryptMessage(&mut st.ctx, &mut desc, 0, core::ptr::null_mut()) };
            match r {
                SEC_E_OK => {
                    if let Some(d) = bufs.iter().find(|b| b.kind == SECBUFFER_DATA) {
                        // SAFETY: the decrypted bytes, inside `raw`
                        st.plain.extend_from_slice(unsafe { core::slice::from_raw_parts(d.pv as *const u8, d.cb as usize) });
                    }
                    keep_extra(&mut st.raw, &bufs);
                }
                SEC_E_INCOMPLETE_MESSAGE => return Ok(()),
                SEC_I_CONTEXT_EXPIRED => {
                    st.ended = true;
                    st.raw.clear();
                    return Ok(());
                }
                _ => return Err(Step::Fault),
            }
        }
        Ok(())
    }

    /// At most `buf.len()` bytes of the stream of `s`.
    pub fn read(_dl: (DlOpen, DlSym), s: Session, buf: &mut [u8]) -> Step {
        let st = state(s);
        loop {
            if !st.plain.is_empty() {
                let n = buf.len().min(st.plain.len());
                buf[..n].copy_from_slice(&st.plain[..n]);
                st.plain.drain(..n);
                return Step::Done(n);
            }
            if st.ended {
                return Step::End;
            }
            if let Err(e) = decrypt(st) {
                return e;
            }
            if !st.plain.is_empty() || st.ended {
                continue;
            }
            match receive(st.fd, &mut st.raw) {
                Got::Bytes => {}
                Got::WouldBlock => return Step::WantRead,
                Got::End => return Step::End,
                Got::Fault => return Step::Fault,
            }
        }
    }

    /// Bytes decrypted or received, waiting to be read.
    pub fn pending(_dl: (DlOpen, DlSym), s: Session) -> usize {
        let st = state(s);
        st.plain.len() + st.raw.len()
    }

    /// Some of the bytes of `buf` sent by `s`: a record (all of them when
    /// they fit in one).
    pub fn write(_dl: (DlOpen, DlSym), s: Session, buf: &[u8]) -> Step {
        let st = state(s);
        if buf.is_empty() {
            return Step::Done(0);
        }
        let z = st.sizes;
        let n = buf.len().min(z.max_message.max(1) as usize);
        let (h, t) = (z.header as usize, z.trailer as usize);
        let mut msg = alloc::vec![0u8; h + n + t];
        msg[h..h + n].copy_from_slice(&buf[..n]);
        let base = msg.as_mut_ptr();
        let mut bufs = [
            SecBuffer { cb: h as u32, kind: SECBUFFER_STREAM_HEADER, pv: base as *mut c_void },
            // SAFETY: inside `msg`
            SecBuffer { cb: n as u32, kind: SECBUFFER_DATA, pv: unsafe { base.add(h) } as *mut c_void },
            SecBuffer { cb: t as u32, kind: SECBUFFER_STREAM_TRAILER, pv: unsafe { base.add(h + n) } as *mut c_void },
            SecBuffer { cb: 0, kind: SECBUFFER_EMPTY, pv: core::ptr::null_mut() },
        ];
        let mut desc = SecBufferDesc { version: 0, count: 4, buffers: bufs.as_mut_ptr() };
        // SAFETY: a context whose handshake is done, the message in place
        if unsafe { EncryptMessage(&mut st.ctx, 0, &mut desc, 0) } != SEC_E_OK {
            return Step::Fault;
        }
        let total = (bufs[0].cb + bufs[1].cb + bufs[2].cb) as usize;
        if send_all(st.fd, &msg[..total]) {
            Step::Done(n)
        } else {
            Step::Fault
        }
    }

    /// The end of the stream of `s` sent (`close_notify`).
    pub fn notify_close(_dl: (DlOpen, DlSym), s: Session) {
        let st = state(s);
        if !st.has_ctx || !st.done {
            return;
        }
        let mut token = 1u32;
        let mut tb = [SecBuffer { cb: 4, kind: SECBUFFER_TOKEN, pv: &mut token as *mut u32 as *mut c_void }];
        let mut td = SecBufferDesc { version: 0, count: 1, buffers: tb.as_mut_ptr() };
        // SAFETY: the context, the token SCHANNEL_SHUTDOWN
        if unsafe { ApplyControlToken(&mut st.ctx, &mut td) } != SEC_E_OK {
            return;
        }
        let mut out = [SecBuffer { cb: 0, kind: SECBUFFER_TOKEN, pv: core::ptr::null_mut() }];
        let mut out_desc = SecBufferDesc { version: 0, count: 1, buffers: out.as_mut_ptr() };
        let mut attrs = 0u32;
        // SAFETY: the context; the alert of the end made by SChannel
        unsafe {
            if st.server {
                AcceptSecurityContext(&mut st.cred, &mut st.ctx, core::ptr::null_mut(), ASC_REQ, 0, &mut st.ctx, &mut out_desc, &mut attrs, core::ptr::null_mut());
            } else {
                InitializeSecurityContextW(&mut st.cred, &mut st.ctx, core::ptr::null(), ISC_REQ, 0, 0, core::ptr::null_mut(), 0, &mut st.ctx, &mut out_desc, &mut attrs, core::ptr::null_mut());
            }
        }
        send_tokens(st.fd, &mut out);
    }

    /// The end of the session sent, then the session freed; its socket is
    /// the caller's.
    pub fn close(dl: (DlOpen, DlSym), s: Session) {
        notify_close(dl, s);
        // SAFETY: a session, freed once
        unsafe {
            let mut st = alloc::boxed::Box::from_raw(s.0 as *mut State);
            if st.has_ctx {
                DeleteSecurityContext(&mut st.ctx);
            }
            if st.own_cred {
                FreeCredentialsHandle(&mut st.cred);
            }
        }
    }
}
