//! The interpreter's side of the runtime: file I/O with the standard library,
//! and every message and number printed through `isc-runtime-core`, exactly
//! like compiled programs.

use isc_runtime_core::{self as sem, Panic};
pub use isc_runtime_core::{maximum_f32, maximum_f64, minimum_f32, minimum_f64, CANONICAL_NAN_BITS, PANIC_EXIT_CODE};

pub fn text(p: &Panic) -> String {
    p.to_string()
}

/// Shortest decimal form that reads back to the same value; NaN without sign.
pub fn format_f64(v: f64) -> String {
    let mut s = String::new();
    let _ = sem::write_f64(&mut s, v);
    s
}

pub fn panic_text(line: u32, col: u32, msg: &str) -> String {
    let mut s = String::new();
    let _ = sem::write_panic(&mut s, line, col, &msg);
    s
}

pub fn msg_bounds(index: i128, len: i64) -> String {
    text(&Panic::Bounds { index, len })
}

pub fn msg_slice(a: i64, b: i64, len: i64) -> String {
    text(&Panic::Slice { a, b, len })
}

pub fn msg_window(k: i64, len: i64) -> String {
    text(&Panic::Window { k, len })
}

pub fn msg_size(a: i64, b: i64) -> String {
    text(&Panic::Size { a, b })
}

pub fn msg_div0() -> String {
    text(&Panic::DivByZero)
}

pub fn msg_negsize(k: i64) -> String {
    text(&Panic::NegativeSize(k))
}

pub fn msg_negexp(k: i64) -> String {
    text(&Panic::NegativeExponent(k))
}

pub fn msg_stack(name: &str) -> String {
    text(&Panic::StackOverflow(name))
}

pub fn msg_bench(r: i64) -> String {
    text(&Panic::BenchReps(r))
}

pub fn parse_i64(s: &str) -> Result<i64, String> {
    sem::parse_i64(s).ok_or_else(|| text(&Panic::ParseI64(s)))
}

pub fn parse_f64(s: &str) -> Result<f64, String> {
    sem::parse_f64(s).ok_or_else(|| text(&Panic::ParseF64(s)))
}

pub fn msg_not_utf8(i: i64) -> String {
    text(&Panic::NotUtf8(i))
}

pub fn msg_split_empty() -> String {
    text(&Panic::SplitEmpty)
}

/// `readText` (D135): the file, checked to be UTF-8.
pub fn read_text_file(path: &str) -> Result<String, String> {
    let b = read_bytes_file(path)?;
    String::from_utf8(b).map_err(|e| text(&Panic::TextNotUtf8 { path, at: e.utf8_error().valid_up_to() as i64 }))
}

/// `parseCsvF64` (D135): the rows.
pub fn parse_csv(t: &str, skip: i64) -> Result<Vec<Vec<f64>>, String> {
    let mut vals = Vec::new();
    let (rows, cols) = sem::parse_csv_f64(t, skip, &mut |_, v| vals.push(v)).map_err(|p| text(&p))?;
    Ok((0..rows as usize).map(|r| vals[r * cols as usize..(r + 1) * cols as usize].to_vec()).collect())
}

/// The OS error in the same form as the compiled runtime (`strerror` text and
/// code; it is also the `Display` of `std::io::Error`).
fn io_text(e: &std::io::Error, path: &str, reading: bool) -> String {
    match e.raw_os_error() {
        // under Windows, the POSIX code and the text of glibc, as a program
        // on Windows (D247)
        #[cfg(windows)]
        Some(code) => {
            let code = sem::oserr::from_win32(code as u32);
            let detail = sem::oserr::text(code);
            if reading {
                text(&Panic::Read { path, detail, code })
            } else {
                text(&Panic::Write { path, detail, code })
            }
        }
        #[cfg(not(windows))]
        Some(code) => {
            let full = e.to_string();
            let detail = full.split(" (os error").next().unwrap_or(&full);
            if reading {
                text(&Panic::Read { path, detail, code })
            } else {
                text(&Panic::Write { path, detail, code })
            }
        }
        None => format!("cannot {} `{}`: {}", if reading { "read" } else { "write" }, path, e),
    }
}

/// The bytes given in place of the standard input (the tests); none: the
/// standard input of the process.
pub static STDIN: std::sync::Mutex<Option<Vec<u8>>> = std::sync::Mutex::new(None);

/// The whole standard input (D155): read once; a later read finds its end.
pub fn read_stdin() -> Result<Vec<u8>, String> {
    if let Some(b) = STDIN.lock().unwrap().as_mut() {
        return Ok(std::mem::take(b));
    }
    let mut b = Vec::new();
    std::io::Read::read_to_end(&mut std::io::stdin(), &mut b).map_err(|e| io_text(&e, "<standard input>", true))?;
    Ok(b)
}

pub fn read_bytes_file(path: &str) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|e| io_text(&e, path, true))
}

pub fn read_f64_file(path: &str) -> Result<Vec<f64>, String> {
    let b = read_bytes_file(path)?;
    if b.len() % 8 != 0 {
        return Err(text(&Panic::NotMultipleOf { path, len: b.len() as u64, size: 8 }));
    }
    Ok(b.chunks_exact(8).map(|c| f64::from_le_bytes(c.try_into().unwrap())).collect())
}

pub fn read_f32_file(path: &str) -> Result<Vec<f32>, String> {
    let b = read_bytes_file(path)?;
    if b.len() % 4 != 0 {
        return Err(text(&Panic::NotMultipleOf { path, len: b.len() as u64, size: 4 }));
    }
    Ok(b.chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect())
}

pub fn write_f32_file(path: &str, xs: &[f32]) -> Result<(), String> {
    let mut out = Vec::with_capacity(xs.len() * 4);
    for &x in xs {
        out.extend_from_slice(&sem::canonical_bits32(x).to_le_bytes());
    }
    write_bytes_file(path, &out)
}

pub fn format_f32(v: f32) -> String {
    let mut s = String::new();
    let _ = sem::write_f32(&mut s, v);
    s
}

pub fn f64_to_le_canonical(xs: &[f64]) -> Vec<u8> {
    let mut out = Vec::with_capacity(xs.len() * 8);
    for &x in xs {
        out.extend_from_slice(&sem::canonical_bits(x).to_le_bytes());
    }
    out
}

pub fn write_bytes_file(path: &str, data: &[u8]) -> Result<(), String> {
    std::fs::write(path, data).map_err(|e| io_text(&e, path, false))
}

pub fn write_f64_file(path: &str, xs: &[f64]) -> Result<(), String> {
    write_bytes_file(path, &f64_to_le_canonical(xs))
}

/// The code of an error of the system (D322), the same as the runtime's:
/// 1 not found, 2 permission denied, 3 already exists, 4 not empty, 5
/// refused, 6 timed out, otherwise the error of the system negated.
pub fn io_code(e: &std::io::Error) -> i64 {
    use std::io::ErrorKind as K;
    match e.kind() {
        K::NotFound => 1,
        K::PermissionDenied => 2,
        // an address already in use as the compiled programs say it
        K::AlreadyExists | K::AddrInUse => 3,
        K::DirectoryNotEmpty => 4,
        K::ConnectionRefused => 5,
        // a read or a write past its delay (`SO_RCVTIMEO`: `EAGAIN`)
        K::TimedOut | K::WouldBlock => 6,
        _ => -(e.raw_os_error().unwrap_or(1) as i64),
    }
}

/// The primitives of a server (D342), as the runtime does them with
/// `epoll`: here the sockets are non-blocking and a wait tries each of them
/// until one has bytes or ends, or until its delay.
fn serve(o: isc_ir::SysOp, vs: &[crate::Value]) -> crate::Value {
    use crate::handles::{tls, Cell, Resource};
    use isc_runtime_core::tls as core_tls;
    use crate::{Array, Value};
    use isc_ir::SysOp as S;
    use std::io::{ErrorKind, Read, Write};
    use std::rc::Rc;
    let int = |v: &Value| match v {
        Value::Int(x) => *x,
        v => panic!("an integer expected, got {:?}", v),
    };
    let cell = |v: &Value| match v {
        Value::Handle(c) => c.clone(),
        v => panic!("a resource expected, got {:?}", v),
    };
    let tuple = |xs: Vec<Value>| Value::Tuple(Rc::new(xs));
    let ints = |xs: Vec<i64>| Value::Array(Rc::new(Array::Int(xs)));
    match o {
        S::ServerOpen => {
            let r = cell(&vs[0]).0.borrow_mut().take();
            match r {
                Some(Resource::Listener(l)) => match l.set_nonblocking(true) {
                    Ok(()) => tuple(vec![Value::Handle(Rc::new(Cell(std::cell::RefCell::new(Some(Resource::Server { listener: l, next: 1, conns: Vec::new(), tls: 0, sessions: Vec::new() }))))), Value::Int(0)]),
                    Err(e) => tuple(vec![Value::Handle(Rc::new(Cell::default())), Value::Int(io_code(&e))]),
                },
                _ => tuple(vec![Value::Handle(Rc::new(Cell::default())), Value::Int(-9)]),
            }
        }
        S::ServerOpenTls => {
            let text = |v: &Value| match v {
                Value::Str(s) => s.to_string(),
                v => panic!("a string expected, got {:?}", v),
            };
            let fail = |t: &str, code: i64| tuple(vec![Value::Handle(Rc::new(Cell::default())), Value::Str(Rc::from(t)), Value::Int(code)]);
            let r = cell(&vs[0]).0.borrow_mut().take();
            let Some(Resource::Listener(l)) = r else { return fail("a closed listener", -9) };
            let Some(d) = tls::dl() else { return fail("no TLS in this interpreter", core_tls::NO_LIBRARY) };
            match core_tls::server_ctx(d, text(&vs[1]).as_bytes(), text(&vs[2]).as_bytes()) {
                Ok(ctx) => match l.set_nonblocking(true) {
                    Ok(()) => tuple(vec![Value::Handle(Rc::new(Cell(std::cell::RefCell::new(Some(Resource::Server { listener: l, next: 1, conns: Vec::new(), tls: ctx.0, sessions: Vec::new() }))))), Value::Str(Rc::from("")), Value::Int(0)]),
                    Err(e) => {
                        core_tls::free_server_ctx(d, ctx);
                        fail("the listener", io_code(&e))
                    }
                },
                Err((code, t)) => fail(&t, code),
            }
        }
        S::ServerWait => {
            let c = cell(&vs[0]);
            let max = int(&vs[1]).clamp(1, 1 << 24) as usize;
            let ms = int(&vs[2]);
            let start = std::time::Instant::now();
            let mut guard = c.0.borrow_mut();
            let Some(Resource::Server { listener, next, conns, tls: ctx, sessions }) = &mut *guard else {
                return tuple(vec![ints(Vec::new()), ints(Vec::new()), ints(Vec::new()), Value::Int(-9)]);
            };
            loop {
                while let Ok((s, _)) = listener.accept() {
                    if s.set_nonblocking(true).is_ok() && s.set_nodelay(true).is_ok() {
                        // a server of TLS: the session of the connection
                        if *ctx != 0 {
                            match tls::dl().and_then(|d| core_tls::accept(d, core_tls::ServerCtx(*ctx), tls::fd_of(&s))) {
                                Some(sess) => sessions.push((*next, sess.0, false)),
                                None => continue,
                            }
                        }
                        conns.push((*next, s));
                        *next += 1;
                    }
                }
                let (mut ids, mut lens, mut data) = (Vec::new(), Vec::new(), Vec::new());
                for (id, s) in conns.iter_mut() {
                    let mut buf = vec![0u8; max];
                    // through its session: the handshake a step further, then
                    // its stream (a fault or an end: 0)
                    if *ctx != 0 {
                        let (Some(d), Some(x)) = (tls::dl(), sessions.iter_mut().find(|x| x.0 == *id)) else { continue };
                        let sess = core_tls::Session(x.1);
                        if !x.2 {
                            match core_tls::handshake(d, sess) {
                                core_tls::Step::Done(_) => x.2 = true,
                                core_tls::Step::WantRead | core_tls::Step::WantWrite => continue,
                                _ => {
                                    ids.push(*id);
                                    lens.push(0);
                                    continue;
                                }
                            }
                        }
                        match core_tls::read(d, sess, &mut buf) {
                            core_tls::Step::Done(n) => {
                                ids.push(*id);
                                lens.push(n as i64);
                                data.extend(buf[..n].iter().map(|&b| b as i64));
                            }
                            core_tls::Step::WantRead | core_tls::Step::WantWrite => {}
                            _ => {
                                ids.push(*id);
                                lens.push(0);
                            }
                        }
                        continue;
                    }
                    match s.read(&mut buf) {
                        Err(e) if e.kind() == ErrorKind::WouldBlock => continue,
                        r => {
                            let n = r.unwrap_or(0);
                            ids.push(*id);
                            lens.push(n as i64);
                            data.extend(buf[..n].iter().map(|&b| b as i64));
                        }
                    }
                }
                let late = ms >= 0 && start.elapsed() >= std::time::Duration::from_millis(ms as u64);
                if !ids.is_empty() || late {
                    return tuple(vec![ints(ids), ints(lens), ints(data), Value::Int(0)]);
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
        S::ServerSend => {
            let c = cell(&vs[0]);
            let id = int(&vs[1]);
            let data = byte_values(&vs[2]);
            let mut guard = c.0.borrow_mut();
            let r = match &mut *guard {
                Some(Resource::Server { conns, tls: ctx, sessions, .. }) => match conns.iter_mut().find(|x| x.0 == id) {
                    // through its session, the socket non-blocking: tried
                    // again every millisecond, at most 30 s
                    Some(_) if *ctx != 0 => match (tls::dl(), sessions.iter().find(|x| x.0 == id)) {
                        (Some(d), Some(x)) => {
                            let sess = core_tls::Session(x.1);
                            let start = std::time::Instant::now();
                            let mut at = 0;
                            loop {
                                if at >= data.len() {
                                    break 0;
                                }
                                match core_tls::write(d, sess, &data[at..]) {
                                    core_tls::Step::Done(k) => at += k,
                                    core_tls::Step::WantRead | core_tls::Step::WantWrite if start.elapsed() < std::time::Duration::from_secs(30) => std::thread::sleep(std::time::Duration::from_millis(1)),
                                    core_tls::Step::WantRead | core_tls::Step::WantWrite => break 6,
                                    _ => break core_tls::TLS_FAULT,
                                }
                            }
                        }
                        _ => -9,
                    },
                    Some((_, s)) => {
                        // the socket blocking for the write, as the runtime
                        // waits for a peer that reads slowly
                        let w = s.set_nonblocking(false).and_then(|_| s.set_write_timeout(Some(std::time::Duration::from_secs(30)))).and_then(|_| s.write_all(&data));
                        let back = s.set_nonblocking(true);
                        w.and(back).map_or_else(|e| io_code(&e), |_| 0)
                    }
                    None => -9,
                },
                _ => -9,
            };
            Value::Int(r)
        }
        S::ServerDrop => {
            let c = cell(&vs[0]);
            let id = int(&vs[1]);
            let mut guard = c.0.borrow_mut();
            let r = match &mut *guard {
                Some(Resource::Server { conns, sessions, .. }) => match conns.iter().position(|x| x.0 == id) {
                    Some(i) => {
                        if let Some(j) = sessions.iter().position(|x| x.0 == id) {
                            let (_, sess, _) = sessions.remove(j);
                            if let Some(d) = tls::dl() {
                                core_tls::close(d, core_tls::Session(sess));
                            }
                        }
                        drop(conns.remove(i));
                        0
                    }
                    None => -9,
                },
                _ => -9,
            };
            Value::Int(r)
        }
        S::ServerClose => {
            // its sessions and its context ended as the value of the cell
            // goes (`Cell::drop`), now: a cell of its own takes it
            let r = cell(&vs[0]).0.borrow_mut().take();
            drop(Cell(std::cell::RefCell::new(r)));
            Value::Int(0)
        }
        _ => unreachable!("not a primitive of a server: {:?}", o),
    }
}

/// A primitive of the system (D322), as the runtime does it: its value and
/// the code of its error in a tuple.
pub fn sys(o: isc_ir::SysOp, vs: &[crate::Value]) -> crate::Value {
    use crate::{Array, Value};
    use isc_ir::SysOp as S;
    use std::rc::Rc;
    let text = |v: &Value| match v {
        Value::Str(s) => s.to_string(),
        v => panic!("a string expected, got {:?}", v),
    };
    let tuple = |xs: Vec<Value>| Value::Tuple(Rc::new(xs));
    let code = |r: std::io::Result<()>| Value::Int(match r {
        Ok(()) => 0,
        Err(e) => io_code(&e),
    });
    let strs = |v: Vec<String>| Value::Array(Rc::new(Array::Str(v.into_iter().map(|x| Rc::from(x.as_str())).collect())));
    let lossy = |b: &[u8]| String::from_utf8_lossy(b).into_owned();
    match o {
        S::Getenv => match Some(text(&vs[0])).filter(|n| !n.is_empty() && !n.contains(['=', '\0'])).and_then(std::env::var_os) {
            Some(v) => tuple(vec![Value::Str(Rc::from(lossy(v.as_encoded_bytes()).as_str())), Value::Int(1)]),
            None => tuple(vec![Value::Str(Rc::from("")), Value::Int(0)]),
        },
        S::Environ => {
            let mut all: Vec<Vec<u8>> = std::env::vars_os().map(|(k, v)| [k.as_encoded_bytes(), b"=", v.as_encoded_bytes()].concat()).collect();
            // by the bytes of their names (D423), as the runtime
            // the length of each name measured once (D435)
            let len = |e: &[u8]| e.iter().position(|&c| c == b'=').unwrap_or(e.len());
            all.sort_by_cached_key(|e| (e[..len(e)].to_vec(), e.clone()));
            strs(all.iter().map(|b| lossy(b)).collect())
        }
        S::Stat => match std::fs::metadata(text(&vs[0])) {
            Ok(m) => {
                let kind = if m.is_file() { 1 } else if m.is_dir() { 2 } else { 3 };
                let at = m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos() as i64);
                tuple(vec![Value::Int(kind), Value::Int(m.len() as i64), Value::Int(at), Value::Int(0)])
            }
            Err(e) => tuple(vec![Value::Int(0), Value::Int(0), Value::Int(0), Value::Int(io_code(&e))]),
        },
        S::ReadDir => match std::fs::read_dir(text(&vs[0])) {
            Ok(d) => {
                let mut names: Vec<Vec<u8>> = d.filter_map(|x| x.ok()).map(|x| x.file_name().as_encoded_bytes().to_vec()).collect();
                names.sort();
                tuple(vec![strs(names.iter().map(|b| lossy(b)).collect()), Value::Int(0)])
            }
            Err(e) => tuple(vec![strs(Vec::new()), Value::Int(io_code(&e))]),
        },
        S::Mkdir => code(std::fs::create_dir(text(&vs[0]))),
        S::RemoveFile => code(std::fs::remove_file(text(&vs[0]))),
        S::RemoveDir => code(std::fs::remove_dir(text(&vs[0]))),
        S::Rename => code(std::fs::rename(text(&vs[0]), text(&vs[1]))),
        S::Now => Value::Int(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as i64)),
        S::Cwd => match std::env::current_dir() {
            Ok(p) => tuple(vec![Value::Str(Rc::from(lossy(p.as_os_str().as_encoded_bytes()).as_str())), Value::Int(0)]),
            Err(e) => tuple(vec![Value::Str(Rc::from("")), Value::Int(io_code(&e))]),
        },
        S::ReadFile => match std::fs::read(text(&vs[0])) {
            Ok(b) => tuple(vec![Value::Array(Rc::new(Array::Int(b.into_iter().map(|x| x as i64).collect()))), Value::Int(0)]),
            Err(e) => tuple(vec![Value::Array(Rc::new(Array::Int(Vec::new()))), Value::Int(io_code(&e))]),
        },
        S::WriteFile => {
            let bytes: Vec<u8> = match &vs[1] {
                Value::Array(a) => match &**a {
                    Array::Int(v) => v.iter().map(|&x| x as u8).collect(),
                    a => panic!("bytes expected, got {:?}", a),
                },
                v => panic!("bytes expected, got {:?}", v),
            };
            let append = matches!(vs[2], Value::Bool(true));
            let r = std::fs::OpenOptions::new().create(true).write(true).append(append).truncate(!append).open(text(&vs[0])).and_then(|mut f| std::io::Write::write_all(&mut f, &bytes));
            code(r)
        }
        S::Connect | S::Listen | S::Accept | S::Recv | S::Send | S::Close | S::CloseListener | S::SetTimeout | S::LocalPort | S::UdpQuery | S::Platform | S::Threads | S::ListenShared | S::Resolve | S::Shutdown | S::TlsStart => net(o, vs),
        S::ServerOpen | S::ServerWait | S::ServerSend | S::ServerDrop | S::ServerClose | S::ServerOpenTls => serve(o, vs),
    }
}

/// The bytes of an array of `u8`.
fn byte_values(v: &crate::Value) -> Vec<u8> {
    use crate::{Array, Value};
    match v {
        Value::Array(a) => match &**a {
            Array::Int(v) => v.iter().map(|&x| x as u8).collect(),
            a => panic!("bytes expected, got {:?}", a),
        },
        v => panic!("bytes expected, got {:?}", v),
    }
}

/// The primitives of the network (D339), as the runtime does them: a
/// connection or a listener in a cell, which `close` empties.
fn net(o: isc_ir::SysOp, vs: &[crate::Value]) -> crate::Value {
    use crate::handles::{Cell, Resource};
    use crate::{Array, Value};
    use isc_ir::SysOp as S;
    use std::io::{Read, Write};
    use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream, UdpSocket};
    use std::rc::Rc;
    let text = |v: &Value| match v {
        Value::Str(s) => s.to_string(),
        v => panic!("a string expected, got {:?}", v),
    };
    let int = |v: &Value| match v {
        Value::Int(x) => *x,
        v => panic!("an integer expected, got {:?}", v),
    };
    let tuple = |xs: Vec<Value>| Value::Tuple(Rc::new(xs));
    let handle = |r: Option<Resource>| Value::Handle(Rc::new(Cell(std::cell::RefCell::new(r))));
    let bytes = |b: Vec<u8>| Value::Array(Rc::new(Array::Int(b.into_iter().map(|x| x as i64).collect())));
    let cell = |v: &Value| match v {
        Value::Handle(c) => c.clone(),
        v => panic!("a resource expected, got {:?}", v),
    };
    // a numeric address and a port, or the code of an invalid argument
    let address = |a: &str, port: i64| -> Result<SocketAddr, i64> {
        let ip: IpAddr = a.parse().map_err(|_| -22)?;
        u16::try_from(port).map(|p| SocketAddr::new(ip, p)).map_err(|_| -22)
    };
    match o {
        S::Connect => match address(&text(&vs[0]), int(&vs[1])).map(|a| TcpStream::connect(a).map_err(|e| io_code(&e))) {
            Ok(Ok(c)) => tuple(vec![handle(Some(Resource::Conn(c))), Value::Int(0)]),
            Ok(Err(e)) | Err(e) => tuple(vec![handle(None), Value::Int(e)]),
        },
        // one listener: the interpreter has one loop of events (D374)
        S::Listen | S::ListenShared => match address(&text(&vs[0]), int(&vs[1])).map(|a| TcpListener::bind(a).map_err(|e| io_code(&e))) {
            Ok(Ok(l)) => tuple(vec![handle(Some(Resource::Listener(l))), Value::Int(0)]),
            Ok(Err(e)) | Err(e) => tuple(vec![handle(None), Value::Int(e)]),
        },
        S::Accept => {
            let c = cell(&vs[0]);
            let r = match &*c.0.borrow() {
                Some(Resource::Listener(l)) => l.accept().map_err(|e| io_code(&e)),
                _ => Err(-9),
            };
            match r {
                Ok((s, peer)) => tuple(vec![handle(Some(Resource::Conn(s))), Value::Str(Rc::from(peer.to_string().as_str())), Value::Int(0)]),
                Err(e) => tuple(vec![handle(None), Value::Str(Rc::from("")), Value::Int(e)]),
            }
        }
        S::TlsStart => {
            use crate::handles::tls;
            use isc_runtime_core::tls as core_tls;
            let c = cell(&vs[0]);
            let (host, ca) = (text(&vs[1]), text(&vs[2]));
            let fd = match &*c.0.borrow() {
                Some(Resource::Conn(s)) => Some(tls::fd_of(s)),
                _ => None,
            };
            let r = match (fd, tls::dl()) {
                (None, _) => Err((-9, String::from("a closed connection"))),
                (_, None) => Err((core_tls::NO_LIBRARY, String::from("no TLS in this interpreter"))),
                (Some(fd), Some(d)) => core_tls::connect(d, fd, host.as_bytes(), ca.as_bytes()).map(|sess| tls::add(fd, sess)),
            };
            match r {
                Ok(()) => tuple(vec![Value::Str(Rc::from("")), Value::Int(0)]),
                Err((code, t)) => tuple(vec![Value::Str(Rc::from(t.as_str())), Value::Int(code)]),
            }
        }
        S::Recv => {
            let c = cell(&vs[0]);
            let max = int(&vs[1]).max(0) as usize;
            let mut buf = vec![0u8; max];
            let r = match &mut *c.0.borrow_mut() {
                Some(Resource::Conn(s)) => match (crate::handles::tls::get(crate::handles::tls::fd_of(s)), crate::handles::tls::dl()) {
                    (Some(sess), Some(d)) => match isc_runtime_core::tls::read(d, sess, &mut buf) {
                        isc_runtime_core::tls::Step::Done(n) => Ok(n),
                        isc_runtime_core::tls::Step::End => Ok(0),
                        isc_runtime_core::tls::Step::WantRead | isc_runtime_core::tls::Step::WantWrite => Err(6),
                        isc_runtime_core::tls::Step::Fault => Err(isc_runtime_core::tls::TLS_FAULT),
                    },
                    _ => s.read(&mut buf).map_err(|e| io_code(&e)),
                },
                _ => Err(-9),
            };
            match r {
                Ok(n) => {
                    buf.truncate(n);
                    tuple(vec![bytes(buf), Value::Int(0)])
                }
                Err(e) => tuple(vec![bytes(Vec::new()), Value::Int(e)]),
            }
        }
        S::Send => {
            let c = cell(&vs[0]);
            let data = byte_values(&vs[1]);
            let r = match &mut *c.0.borrow_mut() {
                Some(Resource::Conn(s)) => match (crate::handles::tls::get(crate::handles::tls::fd_of(s)), crate::handles::tls::dl()) {
                    (Some(sess), Some(d)) => {
                        let mut at = 0;
                        loop {
                            if at >= data.len() {
                                break 0;
                            }
                            match isc_runtime_core::tls::write(d, sess, &data[at..]) {
                                isc_runtime_core::tls::Step::Done(k) => at += k,
                                isc_runtime_core::tls::Step::WantRead | isc_runtime_core::tls::Step::WantWrite => break 6,
                                _ => break isc_runtime_core::tls::TLS_FAULT,
                            }
                        }
                    }
                    _ => s.write_all(&data).map_or_else(|e| io_code(&e), |_| 0),
                },
                _ => -9,
            };
            Value::Int(r)
        }
        S::Close | S::CloseListener => {
            // the resource given back to the system now (its session of
            // TLS ended first)
            let r = cell(&vs[0]).0.borrow_mut().take();
            if let Some(Resource::Conn(s)) = &r {
                crate::handles::tls::end(crate::handles::tls::fd_of(s));
            }
            drop(r);
            Value::Int(0)
        }
        S::SetTimeout => {
            let c = cell(&vs[0]);
            let ms = int(&vs[1]);
            let d = (ms > 0).then(|| std::time::Duration::from_millis(ms as u64));
            let r = match &*c.0.borrow() {
                Some(Resource::Conn(s)) => s.set_read_timeout(d).and_then(|_| s.set_write_timeout(d)).map_or_else(|e| io_code(&e), |_| 0),
                _ => -9,
            };
            Value::Int(r)
        }
        S::Shutdown => {
            let c = cell(&vs[0]);
            let r = match &*c.0.borrow() {
                Some(Resource::Conn(s)) => {
                    if let (Some(sess), Some(d)) = (crate::handles::tls::get(crate::handles::tls::fd_of(s)), crate::handles::tls::dl()) {
                        isc_runtime_core::tls::notify_close(d, sess);
                    }
                    s.shutdown(std::net::Shutdown::Write).map_or_else(|e| io_code(&e), |_| 0)
                }
                _ => -9,
            };
            Value::Int(r)
        }
        S::LocalPort => {
            let c = cell(&vs[0]);
            let r = match &*c.0.borrow() {
                Some(Resource::Listener(l)) => l.local_addr().map_or(-1, |a| a.port() as i64),
                _ => -1,
            };
            Value::Int(r)
        }
        S::UdpQuery => {
            let r = (|| -> Result<Vec<u8>, i64> {
                let to = address(&text(&vs[0]), int(&vs[1]))?;
                let local: SocketAddr = if to.is_ipv4() { "0.0.0.0:0".parse().unwrap() } else { "[::]:0".parse().unwrap() };
                let u = UdpSocket::bind(local).map_err(|e| io_code(&e))?;
                let ms = int(&vs[3]).max(1) as u64;
                u.set_read_timeout(Some(std::time::Duration::from_millis(ms))).map_err(|e| io_code(&e))?;
                u.send_to(&byte_values(&vs[2]), to).map_err(|e| io_code(&e))?;
                let mut buf = vec![0u8; 65536];
                loop {
                    let (n, from) = u.recv_from(&mut buf).map_err(|e| io_code(&e))?;
                    if from == to {
                        buf.truncate(n);
                        return Ok(buf);
                    }
                }
            })();
            match r {
                Ok(b) => tuple(vec![bytes(b), Value::Int(0)]),
                Err(e) => tuple(vec![bytes(Vec::new()), Value::Int(e)]),
            }
        }
        S::Platform => Value::Int(if cfg!(windows) { 1 } else { 0 }),
        // one thread: the tasks of `mapParallel` one after the other (D373)
        S::Threads => Value::Int(1),
        S::Resolve => {
            let strs = |v: Vec<String>| Value::Array(Rc::new(Array::Str(v.into_iter().map(|x| Rc::from(x.as_str())).collect())));
            if !cfg!(windows) {
                return tuple(vec![strs(Vec::new()), Value::Int(7)]);
            }
            match std::net::ToSocketAddrs::to_socket_addrs(&(text(&vs[0]).as_str(), 0)) {
                Ok(it) => tuple(vec![strs(it.map(|a| a.ip().to_string()).collect()), Value::Int(0)]),
                Err(e) => tuple(vec![strs(Vec::new()), Value::Int(io_code(&e))]),
            }
        }
        _ => unreachable!(),
    }
}
