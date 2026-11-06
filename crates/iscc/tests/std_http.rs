//! The module `http` of the standard library, its client (SPEC section 18,
//! D340): responses by `Content-Length`, `chunked` and the end of the
//! stream, their faults, the addresses; requests to a server of HTTP
//! written here on the local machine; the same outputs in the interpreter
//! and in the compiled program; Valgrind.

mod common;
use common::*;
use std::io::{Read, Write};
use std::net::TcpListener;

fn valgrind_clean(src: &str, name: &str, args: &[&str]) {
    let vg = std::path::PathBuf::from("/usr/bin/valgrind");
    if !vg.is_file() {
        return;
    }
    let opts = isc_codegen::Options { isa: isc_codegen::Isa::Avx2, ..isc_codegen::Options::default() };
    let bin = compile_glibc(src, &format!("{}_vg", name), opts);
    let out = std::process::Command::new(vg)
        .args(["--error-exitcode=9", "--leak-check=full", "--errors-for-leak-kinds=definite,indirect", "-q"])
        .arg(&bin)
        .args(args)
        .env("ISC_ALLOC", "glibc")
        .env("ISC_THREADS", "2")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{} under valgrind:\n{}", name, String::from_utf8_lossy(&out.stderr));
}

/// D340: the client on responses written by the server side of the same
/// program (the kernel keeps them until the client reads): a length, a
/// body in chunks with an extension, a body to the end of the stream (HTTP
/// 1.0), a redirection given as it is, a body shorter than its length, a
/// status line and a length that are not; the parts of addresses (IPv6,
/// ports, schemes), `https://` sent to its module; compiled (1 and 4
/// threads) and interpreted, the same outputs; Valgrind.
#[test]
fn client_on_responses_of_every_form() {
    let src = include_str!("data/std_http.isc");
    let want = include_str!("data/std_http.out");
    assert_eq!(interp_ok(src, &[]), want, "interpreter");
    let bin = compile(src, "std_http");
    for t in [1, 4] {
        let r = run_bin_threads(&bin, &[], Some(t));
        assert_eq!((r.code, r.stdout.as_str()), (0, want), "compiled, {} threads: {}", t, r.stderr);
    }
    valgrind_clean(src, "std_http", &[]);
}

/// The sizes of the chunks of `/split`: 1 to 9, 4096, 300000, 1 to 40
fn split_sizes() -> Vec<usize> {
    (1..10).chain([4096, 300000]).chain(1..41).collect()
}

/// A server of HTTP on the local machine, for a while, a request at a time:
/// `/hello` (a length, a header of its own), `/big` (3 MiB), `/chunks`
/// (1000 chunks), `/split` (chunks of every size sent in pieces cut
/// anywhere), `/redirect` (302), `/echo` (the method, a header and the
/// body of the request back), any other path 404. Its port.
fn http_server() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for s in l.incoming() {
            let Ok(mut s) = s else { continue };
            let mut req = Vec::new();
            let mut buf = [0u8; 4096];
            // the head, then the body by its length
            let head_end = loop {
                let n = s.read(&mut buf).unwrap_or(0);
                if n == 0 {
                    break None;
                }
                req.extend_from_slice(&buf[..n]);
                if let Some(i) = req.windows(4).position(|w| w == b"\r\n\r\n") {
                    break Some(i + 4);
                }
            };
            let Some(head_end) = head_end else { continue };
            let head = String::from_utf8_lossy(&req[..head_end]).to_string();
            let length = head.lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0))).unwrap_or(0);
            while req.len() < head_end + length {
                let n = s.read(&mut buf).unwrap_or(0);
                if n == 0 {
                    break;
                }
                req.extend_from_slice(&buf[..n]);
            }
            let first = head.lines().next().unwrap_or("").to_string();
            let path = first.split(' ').nth(1).unwrap_or("").to_string();
            let reply: Vec<u8> = match path.as_str() {
                "/hello" => b"HTTP/1.1 200 OK\r\nX-Server: test\r\nContent-Length: 2\r\n\r\nhi".to_vec(),
                "/big" => {
                    let body: Vec<u8> = (0..3 * 1024 * 1024).map(|i| (i % 253) as u8).collect();
                    let mut r = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len()).into_bytes();
                    r.extend_from_slice(&body);
                    r
                }
                "/chunks" => {
                    let mut r = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
                    for i in 0..1000 {
                        let piece: Vec<u8> = (0..(i % 37 + 1)).map(|j| ((i + j) % 256) as u8).collect();
                        r.extend_from_slice(format!("{:x}\r\n", piece.len()).as_bytes());
                        r.extend_from_slice(&piece);
                        r.extend_from_slice(b"\r\n");
                    }
                    r.extend_from_slice(b"0\r\n\r\n");
                    r
                }
                "/redirect" => b"HTTP/1.1 302 Found\r\nLocation: /hello\r\nContent-Length: 0\r\n\r\n".to_vec(),
                // chunks of every size (one of 300000 bytes), a head of 40
                // headers, all sent below in pieces cut anywhere
                "/split" => {
                    let mut r = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n".to_vec();
                    for i in 0..40 {
                        r.extend_from_slice(format!("X-Header-{}: {}\r\n", i, "v".repeat(i * 7)).as_bytes());
                    }
                    r.extend_from_slice(b"\r\n");
                    for len in split_sizes() {
                        let piece: Vec<u8> = (0..len).map(|j| ((j * 7 + len) % 256) as u8).collect();
                        r.extend_from_slice(format!("{:X};ext=1\r\n", len).as_bytes());
                        r.extend_from_slice(&piece);
                        r.extend_from_slice(b"\r\n");
                    }
                    r.extend_from_slice(b"0\r\nTrailer: x\r\n\r\n");
                    r
                }
                "/echo" => {
                    let method = first.split(' ').next().unwrap_or("");
                    let x = head.lines().find_map(|l| l.strip_prefix("X-Test: ").map(|v| v.to_string())).unwrap_or_default();
                    let body = format!("{} {} {}", method, x, String::from_utf8_lossy(&req[head_end..]));
                    format!("HTTP/1.1 201 Created\r\nContent-Length: {}\r\n\r\n{}", body.len(), body).into_bytes()
                }
                _ => b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_vec(),
            };
            if path == "/split" {
                // pieces of 1 to 97 bytes, a pause now and then: the client
                // reads them as they come, cut anywhere (in a line, a chunk,
                // its CR LF)
                let _ = s.set_nodelay(true);
                let mut at = 0;
                let mut k = 0;
                while at < reply.len() {
                    let n = (k * 31 % 97 + 1).min(reply.len() - at);
                    if s.write_all(&reply[at..at + n]).is_err() {
                        break;
                    }
                    at += n;
                    k += 1;
                    if k % 40 == 0 {
                        std::thread::sleep(std::time::Duration::from_micros(300));
                    }
                }
                continue;
            }
            let _ = s.write_all(&reply);
        }
    });
    port
}

/// D340: `get` and `request` to the server of the test: a small body and a
/// header, 3 MiB read by their length, 1000 chunks, chunks of every size
/// received in pieces cut anywhere (a chunk of 300000 bytes read by
/// pieces), a redirection not
/// followed, a POST with a header of UTF-8 and a body, a connection
/// refused, a header that is not a token refused before sending; compiled
/// and interpreted, the same outputs; Valgrind.
#[test]
fn client_against_a_server_of_the_test() {
    let port = http_server().to_string();
    let big: u64 = (0..3u64 * 1024 * 1024).map(|i| i % 253).sum();
    let chunks: (u64, u64) = (0..1000u64).fold((0, 0), |(n, s), i| (n + i % 37 + 1, s + (0..(i % 37 + 1)).map(|j| (i + j) % 256).sum::<u64>()));
    let split: (usize, usize) = split_sizes().iter().fold((0, 0), |(n, s), &len| (n + len, s + (0..len).map(|j| (j * 7 + len) % 256).sum::<usize>()));
    let want = format!(
        "[/hello, /big, /chunks, /redirect, /missing]\n200 2 209 hi test\n200 3145728 {}\n200 {} {}\n200 {} {} {}\n302 /hello\n201 POST été a body\nrefused: 127.0.0.1:1\nmalformed: the header Bad Name\n",
        big, chunks.0, chunks.1, split.0, split.1, "v".repeat(39 * 7)
    );
    let src = include_str!("data/std_http_get.isc");
    assert_eq!(interp_ok(src, &[&port]), want, "interpreter");
    let r = run_bin(&compile(src, "std_http_get"), &[&port]);
    assert_eq!((r.code, r.stdout.as_str()), (0, want.as_str()), "compiled: {}", r.stderr);
    valgrind_clean(src, "std_http_get", &[&port]);
}

/// D342: the server answering the clients that the same program wrote
/// first (the kernel keeps their requests until it accepts them): two
/// requests one after the other on a connection, a body by its length,
/// HEAD (no body), HTTP/1.0 (closed after), a request that is not one
/// (400), `chunked` (501), a path not known (404), no `Host` (400); the
/// date hidden; compiled (1 and 4 threads) and interpreted, the same
/// outputs; Valgrind.
#[test]
fn server_on_requests_of_every_form() {
    let src = include_str!("data/std_http_serve.isc");
    let want = include_str!("data/std_http_serve.out");
    assert_eq!(interp_ok(src, &[]), want, "interpreter");
    let bin = compile(src, "std_http_serve");
    for t in [1, 4] {
        let r = run_bin_threads(&bin, &[], Some(t));
        assert_eq!((r.code, r.stdout.as_str()), (0, want), "compiled, {} threads: {}", t, r.stderr);
    }
    valgrind_clean(src, "std_http_serve", &[]);
}

/// One response read from `r`: its status, its headers (names in lower
/// case) and its body (by its length).
fn read_response(r: &mut std::io::BufReader<std::net::TcpStream>) -> (u16, Vec<(String, String)>, Vec<u8>) {
    use std::io::BufRead;
    let mut line = String::new();
    r.read_line(&mut line).unwrap();
    let status: u16 = line.split(' ').nth(1).unwrap_or("0").parse().unwrap_or(0);
    let mut headers = Vec::new();
    loop {
        let mut h = String::new();
        if r.read_line(&mut h).unwrap() == 0 || h == "\r\n" {
            break;
        }
        let (k, v) = h.trim_end().split_once(':').unwrap();
        headers.push((k.to_ascii_lowercase(), v.trim().to_string()));
    }
    let len: usize = headers.iter().find(|(k, _)| k == "content-length").map(|(_, v)| v.parse().unwrap()).unwrap_or(0);
    let mut body = vec![0u8; len];
    r.read_exact(&mut body).unwrap();
    (status, headers, body)
}

/// The clients of `server_against_clients_of_the_test`, on `port`: the
/// number of the requests they made that are answered.
fn server_clients(port: u16) -> usize {
    use std::io::BufReader;
    use std::net::TcpStream;
    let connect = move || {
        let s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.set_read_timeout(Some(std::time::Duration::from_secs(120))).unwrap();
        s
    };
    let mut threads = Vec::new();
    // 8 connections kept alive, 25 requests each
    for t in 0..8 {
        threads.push(std::thread::spawn(move || {
            let s = connect();
            let mut r = BufReader::new(s.try_clone().unwrap());
            let mut w = s;
            for i in 0..25 {
                write!(w, "GET /keep?{}.{} HTTP/1.1\r\nHost: test\r\n\r\n", t, i).unwrap();
                let (status, headers, body) = read_response(&mut r);
                assert_eq!((status, String::from_utf8(body).unwrap()), (200, format!("GET /keep {}.{}", t, i)));
                assert!(headers.iter().any(|(k, _)| k == "date"), "{:?}", headers);
                assert!(!headers.iter().any(|(k, _)| k == "connection"), "{:?}", headers);
            }
            25
        }));
    }
    // a body of 1 MiB sent back
    threads.push(std::thread::spawn(move || {
        let s = connect();
        let mut r = BufReader::new(s.try_clone().unwrap());
        let mut w = s;
        let body: Vec<u8> = (0..1 << 20).map(|i| (i % 253) as u8).collect();
        write!(w, "POST /echo HTTP/1.1\r\nHost: test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
        w.write_all(&body).unwrap();
        let (status, headers, got) = read_response(&mut r);
        assert_eq!((status, got == body), (200, true));
        assert!(headers.contains(&("x-length".to_string(), (1 << 20).to_string())), "{:?}", headers);
        1
    }));
    // a body sent slowly, in pieces
    threads.push(std::thread::spawn(move || {
        let s = connect();
        s.set_nodelay(true).unwrap();
        let mut r = BufReader::new(s.try_clone().unwrap());
        let mut w = s;
        let body: Vec<u8> = (0..40000).map(|i| (i % 7) as u8).collect();
        let head = format!("POST /sum HTTP/1.1\r\nHost: test\r\nContent-Length: {}\r\n\r\n", body.len());
        let all = [head.as_bytes(), &body].concat();
        for piece in all.chunks(9973) {
            w.write_all(piece).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let (status, _, got) = read_response(&mut r);
        let want: u64 = body.iter().map(|&x| x as u64).sum();
        assert_eq!((status, String::from_utf8(got).unwrap()), (200, want.to_string()));
        1
    }));
    // a response of 2000000 bytes, then two requests written at once
    threads.push(std::thread::spawn(move || {
        let s = connect();
        let mut r = BufReader::new(s.try_clone().unwrap());
        let mut w = s;
        w.write_all(b"GET /big?2000000 HTTP/1.1\r\nHost: test\r\n\r\n").unwrap();
        let (status, _, got) = read_response(&mut r);
        assert_eq!((status, got.len()), (200, 2000000));
        assert!(got.iter().enumerate().all(|(i, &x)| x == (i % 251) as u8));
        w.write_all(b"GET /a HTTP/1.1\r\nHost: test\r\n\r\nDELETE /b?c HTTP/1.1\r\nHost: test\r\nConnection: close\r\n\r\n").unwrap();
        let a = read_response(&mut r);
        let b = read_response(&mut r);
        assert_eq!((a.0, a.2, b.0, b.2), (200, b"GET /a ".to_vec(), 405, b"DELETE /b c".to_vec()));
        3
    }));
    // a body longer than 16 MiB: 413 at once, the connection closed
    threads.push(std::thread::spawn(move || {
        let s = connect();
        let mut r = BufReader::new(s.try_clone().unwrap());
        let mut w = s;
        w.write_all(b"POST /echo HTTP/1.1\r\nHost: test\r\nContent-Length: 17000000\r\n\r\n").unwrap();
        let (status, headers, _) = read_response(&mut r);
        assert_eq!(status, 413);
        assert!(headers.contains(&("connection".to_string(), "close".to_string())), "{:?}", headers);
        1
    }));
    // a client that leaves in the middle of its request: no answer
    threads.push(std::thread::spawn(move || {
        let mut s = connect();
        s.write_all(b"GET /gone HTTP/1.1\r\nHo").unwrap();
        0
    }));
    threads.into_iter().map(|t| t.join().unwrap()).sum()
}

/// D342: `serveOn` in a program of its own against clients of the test,
/// at the same time: 8 connections kept alive for 25 requests each, a body
/// of 1 MiB sent back, a body sent slowly in pieces, a response of 2000000
/// bytes, two requests written at once, a body over the limit (413), a
/// client that leaves in the middle of a request; the server ends after
/// the 206 answers; compiled, interpreted (`iscc run`) and under Valgrind.
#[test]
fn server_against_clients_of_the_test() {
    let src = include_str!("data/std_http_server.isc");
    let bin = compile(src, "std_http_server");
    let dir = tmp("std_http_server_src");
    let _ = std::fs::create_dir_all(&dir);
    let file = dir.join("server.isc");
    std::fs::write(&file, src).unwrap();
    let vg = std::path::PathBuf::from("/usr/bin/valgrind");
    let mut runs: Vec<(&str, std::process::Command)> = Vec::new();
    let mut c = std::process::Command::new(&bin);
    c.arg("206").env("ISC_THREADS", "4");
    runs.push(("compiled", c));
    let mut c = std::process::Command::new(env!("CARGO_BIN_EXE_iscc"));
    c.arg("run").arg(&file).arg("206");
    runs.push(("interpreted", c));
    if vg.is_file() {
        let opts = isc_codegen::Options { isa: isc_codegen::Isa::Avx2, ..isc_codegen::Options::default() };
        let vbin = compile_glibc(src, "std_http_server_vg", opts);
        let mut c = std::process::Command::new(&vg);
        c.args(["--error-exitcode=9", "--leak-check=full", "--errors-for-leak-kinds=definite,indirect", "-q"]).arg(&vbin).arg("206").env("ISC_ALLOC", "glibc").env("ISC_THREADS", "2");
        runs.push(("valgrind", c));
    }
    for (name, mut c) in runs {
        use std::io::BufRead;
        let mut child = c.stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().unwrap();
        let mut out = std::io::BufReader::new(child.stdout.take().unwrap());
        let mut line = String::new();
        out.read_line(&mut line).unwrap();
        let port: u16 = line.trim().parse().unwrap_or_else(|_| panic!("{}: no port: {:?}", name, line));
        assert_eq!(server_clients(port), 206, "{}", name);
        let o = child.wait_with_output().unwrap();
        let mut rest = String::new();
        std::io::Read::read_to_string(&mut out, &mut rest).unwrap();
        assert_eq!((o.status.code(), rest.as_str()), (Some(0), "done\n"), "{}: {}", name, String::from_utf8_lossy(&o.stderr));
    }
}

/// D352: a handler that is an action (`serveOn`, D386): it prints each
/// request and reads a file, the requests of one connection answered one at
/// a time in their order; a pure function where an action is expected;
/// `forEach` with a lambda that prints. Run in a folder of its own, the
/// interpreter (`iscc run`) and compiled on 1 and 4 threads: the same output.
#[test]
fn server_with_a_handler_that_is_an_action() {
    let src = include_str!("data/std_http_serve_action.isc");
    let want = include_str!("data/std_http_serve_action.out");
    let d = tmp("std_http_serve_action_dir");
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("serve_action.isc"), src).unwrap();
    let o = std::process::Command::new(env!("CARGO_BIN_EXE_iscc")).args(["run", "serve_action.isc"]).current_dir(&d).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&o.stdout), want, "interpreter: {}", String::from_utf8_lossy(&o.stderr));
    let bin = compile(src, "std_http_serve_action");
    for t in ["1", "4"] {
        let o = std::process::Command::new(&bin).current_dir(&d).env("ISC_THREADS", t).output().unwrap();
        assert_eq!((o.status.code(), String::from_utf8_lossy(&o.stdout).into_owned()), (Some(0), want.to_string()), "compiled, {} threads", t);
    }
    assert!(!d.join("isc-serve-action.txt").exists(), "the file of the test left");
}

/// The listening sockets of `port` (IPv4) of the machine.
fn listeners_of(port: u16) -> usize {
    let t = std::fs::read_to_string("/proc/net/tcp").unwrap();
    t.lines()
        .skip(1)
        .filter(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            f.len() > 3 && f[3] == "0A" && f[1].rsplit(':').next().and_then(|p| u16::from_str_radix(p, 16).ok()) == Some(port)
        })
        .count()
}

/// D374: `serve(port, handler)` runs a loop of events on each thread of the
/// pool under Linux, each on a listener of its own of the port (the kernel
/// shares the connections out): 4 listeners with 4 threads, 1 interpreted
/// (one loop); a second server on the port refused; 64 connections of 50
/// requests each all answered.
#[test]
fn serve_runs_a_loop_on_each_thread() {
    loops_on_each_thread(include_str!("data/std_http_serve_loops.isc"), "std_http_serve_loops");
}

/// D385: `serve(port, handler)` given an action too (D386), one that
/// prints: 4 listeners with 4 threads, 1 interpreted, a second server
/// refused, the requests of each connection answered in their order.
#[test]
fn serve_action_runs_a_loop_on_each_thread() {
    loops_on_each_thread(include_str!("data/std_http_serve_action_loops.isc"), "std_http_serve_action_loops");
}

fn loops_on_each_thread(src: &str, name: &str) {
    use std::io::BufReader;
    let bin = compile(src, name);
    let dir = tmp(&format!("{}_src", name));
    let _ = std::fs::create_dir_all(&dir);
    let file = dir.join(format!("{}.isc", name));
    std::fs::write(&file, src).unwrap();
    for (name, loops) in [("compiled", 4), ("interpreted", 1)] {
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let mut c = if name == "compiled" {
            let mut c = std::process::Command::new(&bin);
            c.arg(port.to_string()).env("ISC_THREADS", "4");
            c
        } else {
            let mut c = std::process::Command::new(env!("CARGO_BIN_EXE_iscc"));
            c.arg("run").arg(&file).arg(port.to_string());
            c
        };
        let mut child = c.stdout(std::process::Stdio::null()).stderr(std::process::Stdio::piped()).spawn().unwrap();
        let start = std::time::Instant::now();
        while listeners_of(port) < loops {
            assert!(start.elapsed() < std::time::Duration::from_secs(60), "{}: {} listeners", name, listeners_of(port));
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(listeners_of(port), loops, "{}", name);
        // a second server on the port refused, whatever its threads: it
        // never joins the listeners of the first
        let mut second = std::process::Command::new(&bin);
        let o = second.arg(port.to_string()).env("ISC_THREADS", "4").output().unwrap();
        assert_eq!((o.status.code(), String::from_utf8_lossy(&o.stdout).into_owned()), (Some(1), "error\n".to_string()), "{}: a second server", name);
        assert_eq!(listeners_of(port), loops, "{}: after the second server", name);
        let threads: Vec<_> = (0..64)
            .map(|t| {
                std::thread::spawn(move || {
                    let s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
                    s.set_read_timeout(Some(std::time::Duration::from_secs(60))).unwrap();
                    let mut r = BufReader::new(s.try_clone().unwrap());
                    let mut w = s;
                    let mut ok = 0;
                    for i in 0..50 {
                        write!(w, "GET /c{}.{} HTTP/1.1\r\nHost: test\r\n\r\n", t, i).unwrap();
                        let (status, _, body) = read_response(&mut r);
                        ok += (status == 200 && body == format!("GET /c{}.{}", t, i).into_bytes()) as usize;
                    }
                    ok
                })
            })
            .collect();
        let answered: usize = threads.into_iter().map(|t| t.join().unwrap()).sum();
        child.kill().unwrap();
        let o = child.wait_with_output().unwrap();
        assert_eq!(answered, 64 * 50, "{}: {}", name, String::from_utf8_lossy(&o.stderr));
    }
}
