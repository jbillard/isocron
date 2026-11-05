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
