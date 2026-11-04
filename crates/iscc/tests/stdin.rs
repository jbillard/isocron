//! The standard input (D155): `readStdin()`, `readStdinText()`,
//! `readStdinLines()`, the same in the interpreter and in the compiled
//! program. One test: the input of the interpreter is a global.

mod common;
use common::*;
use std::io::Write as _;

fn run_with_stdin(bin: &std::path::Path, input: &[u8]) -> (i32, String, String) {
    let mut c = std::process::Command::new(bin)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    c.stdin.take().unwrap().write_all(input).unwrap();
    let o = c.wait_with_output().unwrap();
    (o.status.code().unwrap_or(-1), String::from_utf8_lossy(&o.stdout).to_string(), String::from_utf8_lossy(&o.stderr).to_string())
}

fn interp_with_stdin(src: &str, input: &[u8]) -> Outcome {
    *isc_interp::rt::STDIN.lock().unwrap() = Some(input.to_vec());
    interp(src, &[])
}

const LINES: &str = "action main(args: string[k]): i32 {
    const ls = readStdinLines();
    print(ls.length);
    print(ls.map((l) => l.toUpperCase()));
    return 0;
}
";

const BYTES: &str = "action main(args: string[k]): i32 {
    const b = readStdin();
    print([b.length, i64(sum(map(b, (x) => u64(x))))] as const);
    print(readStdinText().length);
    return 0;
}
";

#[test]
fn standard_input() {
    let input = "abc\r\ndéf\n\nxyz".as_bytes();
    let want = "4\n[ABC, DÉF, , XYZ]\n";
    match interp_with_stdin(LINES, input) {
        Outcome::Exit(0, out) => assert_eq!(out, want),
        _ => panic!("interpreter"),
    }
    let bin = compile(LINES, "stdin_lines");
    assert_eq!(run_with_stdin(&bin, input), (0, want.to_string(), String::new()));
    // a large input (several reads), then nothing left for the text
    let big: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
    let total: u64 = big.iter().map(|&b| b as u64).sum();
    let want = format!("({}, {})\n0\n", big.len(), total);
    match interp_with_stdin(BYTES, &big) {
        Outcome::Exit(0, out) => assert_eq!(out, want),
        _ => panic!("interpreter"),
    }
    let bin = compile(BYTES, "stdin_bytes");
    assert_eq!(run_with_stdin(&bin, &big).1, want);
    // not UTF-8
    let m = match interp_with_stdin(LINES, b"ab\xffc") {
        Outcome::Panic(m, _) => m,
        _ => panic!("interpreter"),
    };
    let want = "panic at line 2, column 30: readStdinText: the standard input is not UTF-8 text (invalid from byte 2)";
    assert_eq!(m, want);
    let bin = compile(LINES, "stdin_bad");
    let (code, _, err) = run_with_stdin(&bin, b"ab\xffc");
    assert_eq!(code, 101);
    assert!(err.contains(want), "{}", err);
}

#[test]
fn standard_input_from_a_file() {
    // a file: the bytes left from its offset, read in a buffer of their size
    // (D179); a file larger than the first guess for a pipe (64 KiB)
    let big: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
    let path = tmp("stdin_file.bin");
    std::fs::write(&path, &big).unwrap();
    let bin = compile(BYTES, "stdin_file");
    for skip in [0usize, 1000, 300_000] {
        let rest = &big[skip..];
        let total: u64 = rest.iter().map(|&b| b as u64).sum();
        let want = format!("({}, {})\n0\n", rest.len(), total);
        let mut f = std::fs::File::open(&path).unwrap();
        std::io::Seek::seek(&mut f, std::io::SeekFrom::Start(skip as u64)).unwrap();
        let o = std::process::Command::new(&bin).stdin(f).output().unwrap();
        assert_eq!((o.status.code(), String::from_utf8_lossy(&o.stdout).to_string()), (Some(0), want), "from byte {}", skip);
    }
}

#[test]
fn large_files_read_by_several_threads() {
    // D221: a file of 8 MiB or more is read by every thread, each its part
    // (`pread`), from the offset of the standard input; the offset is left
    // at the end (nothing more for `readStdinText`). The same bytes on one
    // thread and four, from a pipe, and by `readBytes`.
    let src = "action main(args: string[k]): i32 {
    const b = readStdin();
    const n = b.length;
    print([n, i64(sum(map(b, (x) => u64(x)))), n > 0 ? i64(b[0]) : -1, n > 0 ? i64(b[n - 1]) : -1, n > 1 ? i64(b[n / 2]) : -1] as const);
    print(readStdinText().length);
    const f = readBytes(args[0]);
    print([f.length, i64(sum(map(f, (x) => u64(x))))] as const);
    return 0;
}
";
    let big: Vec<u8> = (0..20_000_011u32).map(|i| (i.wrapping_mul(2654435761) >> 13) as u8).collect();
    let path = tmp("stdin_large.bin");
    std::fs::write(&path, &big).unwrap();
    let all: u64 = big.iter().map(|&b| b as u64).sum();
    let bin = compile(src, "stdin_large");
    let arg = path.to_str().unwrap();
    let want_of = |rest: &[u8]| {
        let n = rest.len();
        let s: u64 = rest.iter().map(|&b| b as u64).sum();
        let at = |i: usize| if n > i { rest[i] as i64 } else { -1 };
        format!("({}, {}, {}, {}, {})\n0\n({}, {})\n", n, s, at(0), if n > 0 { rest[n - 1] as i64 } else { -1 }, if n > 1 { rest[n / 2] as i64 } else { -1 }, big.len(), all)
    };
    for skip in [0usize, 1, 12_345_679, 20_000_011] {
        let want = want_of(&big[skip..]);
        for t in ["1", "4"] {
            let mut f = std::fs::File::open(&path).unwrap();
            std::io::Seek::seek(&mut f, std::io::SeekFrom::Start(skip as u64)).unwrap();
            let o = std::process::Command::new(&bin).arg(arg).env("ISC_THREADS", t).stdin(f).output().unwrap();
            assert_eq!((o.status.code(), String::from_utf8_lossy(&o.stdout).to_string()), (Some(0), want.clone()), "from byte {}, {} threads: {}", skip, t, String::from_utf8_lossy(&o.stderr));
        }
    }
    let mut c = std::process::Command::new(&bin).arg(arg).env("ISC_THREADS", "4").stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).spawn().unwrap();
    let mut w = c.stdin.take().unwrap();
    let data = big.clone();
    let writer = std::thread::spawn(move || w.write_all(&data).unwrap());
    let o = c.wait_with_output().unwrap();
    writer.join().unwrap();
    assert_eq!((o.status.code(), String::from_utf8_lossy(&o.stdout).to_string()), (Some(0), want_of(&big)), "from a pipe");
}

/// The lines of a large text made by parts in parallel (D252): lines of 0
/// to 300 bytes, empty ones, `\r\n` ones, runs of `\n` across the cuts of
/// the parts, with and without a final `\n`; the same lines as Rust's rules
/// (`text_lines`), on 1, 3, 4 and 8 threads (other cuts); 3 MiB, and 40 MiB
/// whose lines fill many spans of each thread and several chunks (D294).
#[test]
fn lines_of_a_large_text_made_in_parallel() {
    let src = "action main(args: string[k]): i32 {
    const lines = readStdinLines();
    const n = lines.length;
    print(n);
    print(sum(map(iota(n), (i) => (i + 1) * lines[i].length)));
    print(sum(map(iota(n), (i) => lines[i].length > 0 ? i64(bytes(lines[i])[0]) * (i % 7) : 0)));
    print([lines[0], lines[n / 2], lines[n - 1]]);
    return 0;
}
";
    let bin = compile(src, "stdin_lines_parallel");
    let mut x = 12345u64;
    let mut next = || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    for size in [3usize << 20, 40 << 20] {
        let mut text = Vec::new();
        while text.len() < size {
            let r = next();
            let len = match r % 10 {
                0 => 0,
                1 => (r >> 8) as usize % 300,
                _ => (r >> 8) as usize % 70,
            };
            text.extend((0..len).map(|j| b'a' + ((r >> (j % 50)) % 26) as u8));
            if r % 13 == 0 {
                text.push(b'\r');
            }
            text.push(b'\n');
            if r % 97 == 0 {
                text.extend(std::iter::repeat(b'\n').take((r >> 20) as usize % 40));
            }
        }
        for final_newline in [true, false] {
            let mut t = text.clone();
            if !final_newline {
                t.extend_from_slice(b"last line without its newline");
            }
            let s = std::str::from_utf8(&t).unwrap();
            let body = s.strip_suffix('\n').unwrap_or(s);
            let lines: Vec<&str> = body.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l)).collect();
            let n = lines.len();
            let weighted: i64 = lines.iter().enumerate().map(|(i, l)| (i as i64 + 1) * l.len() as i64).sum();
            let firsts: i64 = lines.iter().enumerate().map(|(i, l)| if l.is_empty() { 0 } else { l.as_bytes()[0] as i64 * (i as i64 % 7) }).sum();
            let want = format!("{}\n{}\n{}\n[{}, {}, {}]\n", n, weighted, firsts, lines[0], lines[n / 2], lines[n - 1]);
            let path = tmp(&format!("stdin_lines_{}.txt", final_newline));
            std::fs::write(&path, &t).unwrap();
            for threads in ["1", "3", "4", "8"] {
                let o = std::process::Command::new(&bin).env("ISC_THREADS", threads).stdin(std::fs::File::open(&path).unwrap()).output().unwrap();
                assert_eq!((o.status.code(), String::from_utf8_lossy(&o.stdout).to_string()), (Some(0), want.clone()), "{} bytes, {} threads, final newline {}", size, threads, final_newline);
            }
        }
    }
}

/// A program that reads a large file as its standard input, prints its
/// length, computes for a while (the checksum then depends on it), and
/// prints the sum of its bytes.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const MAPPED_SRC: &str = "action main(args: string[k]): i32 {
    const data = readStdin();
    print(data.length);
    const x = fold(iota(300000000), 1, (a, i) => (a * 31 + i) % 1000003);
    const off = x == -1 ? 1 : 0;
    print(i64(sum(map(data.slice(off, data.length), (b) => u64(b)))));
    print(x >= 0);
    return 0;
}
";

/// The program of MAPPED_SRC on `path`: its first line, then what `during`
/// does while it computes, then the rest of its output and its exit code.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn run_mapped(bin: &std::path::Path, path: &std::path::Path, during: &dyn Fn(u32)) -> (String, String, Option<i32>) {
    use std::io::{BufRead, Read};
    let mut c = std::process::Command::new(bin)
        .env("ISC_THREADS", "4")
        .stdin(std::fs::File::open(path).unwrap())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut out = std::io::BufReader::new(c.stdout.take().unwrap());
    let mut first = String::new();
    out.read_line(&mut first).unwrap();
    during(c.id());
    let mut rest = String::new();
    out.read_to_string(&mut rest).unwrap();
    let status = c.wait().unwrap();
    (first, rest, status.code())
}

/// D257: a large file as the standard input is mapped, held by a read
/// lease; written and truncated while the program runs, the program still
/// sees the bytes it had (the pages made private before the lease is given
/// back), and the writer does not wait for the end of the lease (45 s). A
/// file already open for writing is read (no lease); the bytes freed
/// early give the lease back.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn a_mapped_input_keeps_its_bytes_when_the_file_is_written() {
    let n = 24_000_000usize;
    let big: Vec<u8> = (0..n as u32).map(|i| (i.wrapping_mul(2654435761) >> 11) as u8).collect();
    let sum: u64 = big.iter().map(|&b| b as u64).sum();
    let want = format!("{}\ntrue\n", sum);
    let bin = compile(MAPPED_SRC, "stdin_mapped");
    // written while it runs. The kernel refuses the lease while any
    // process has the file open for writing, a child of another test of
    // this process too, between its `clone` and its `exec` (it holds a copy
    // of every descriptor; once in a run of the complete suite): the file is
    // then read, which this checks as well; up to 3 tries, one mapped.
    let mut mapped = 0;
    for k in 0..3 {
        let path = tmp(&format!("stdin_mapped_{}.bin", k));
        std::fs::write(&path, &big).unwrap();
        let p = path.clone();
        let seen = std::cell::Cell::new(false);
        let (first, rest, code) = run_mapped(&bin, &path, &|pid| {
            let maps = std::fs::read_to_string(format!("/proc/{}/maps", pid)).unwrap();
            seen.set(maps.contains(p.to_str().unwrap()));
            let t = std::time::Instant::now();
            let mut f = std::fs::File::create(&p).unwrap();
            assert!(t.elapsed() < std::time::Duration::from_secs(10), "the writer waited {:?}", t.elapsed());
            f.write_all(&vec![7u8; n / 2]).unwrap();
        });
        assert_eq!((first.as_str(), rest.as_str(), code), (format!("{}\n", n).as_str(), want.as_str(), Some(0)), "try {}", k);
        if seen.get() {
            mapped += 1;
            break;
        }
    }
    assert!(mapped == 1, "the input was never mapped");
    // a file open for writing: read, the same bytes
    let path2 = tmp("stdin_mapped_open.bin");
    std::fs::write(&path2, &big).unwrap();
    let writer = std::fs::OpenOptions::new().write(true).open(&path2).unwrap();
    let p2 = path2.clone();
    let (first, rest, code) = run_mapped(&bin, &path2, &|pid| {
        let maps = std::fs::read_to_string(format!("/proc/{}/maps", pid)).unwrap();
        assert!(!maps.contains(p2.to_str().unwrap()), "a file open for writing is mapped");
    });
    drop(writer);
    assert_eq!((first.as_str(), rest.as_str(), code), (format!("{}\n", n).as_str(), want.as_str(), Some(0)));
}

/// D257: the mapped input freed before the end of the program gives the
/// lease back: a writer does not wait, the program goes on.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn a_mapped_input_freed_early_gives_the_file_back() {
    let src = "action main(args: string[k]): i32 {
    const data = readStdin();
    print([data.length, i64(sum(map(data, (b) => u64(b))))] as const);
    print(fold(iota(300000000), 1, (a, i) => (a * 31 + i) % 1000003) >= 0);
    return 0;
}
";
    let n = 16_000_000usize;
    let big: Vec<u8> = (0..n as u32).map(|i| (i % 253) as u8).collect();
    let sum: u64 = big.iter().map(|&b| b as u64).sum();
    let path = tmp("stdin_mapped_freed.bin");
    std::fs::write(&path, &big).unwrap();
    let bin = compile(src, "stdin_mapped_freed");
    let p = path.clone();
    let (first, rest, code) = run_mapped(&bin, &path, &|_| {
        let t = std::time::Instant::now();
        let mut f = std::fs::File::create(&p).unwrap();
        assert!(t.elapsed() < std::time::Duration::from_secs(10), "the writer waited {:?}", t.elapsed());
        f.write_all(b"x").unwrap();
    });
    assert_eq!((first, rest.as_str(), code), (format!("({}, {})\n", n, sum), "true\n", Some(0)));
}

/// D257: the mapped input changed in place (a `map` of the same bytes at
/// its last use writes into it, D207): the write faults on the read-only
/// mapping, its bytes are copied into memory of the program, the write is
/// done again; on 1 and 4 threads.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn a_mapped_input_changed_in_place() {
    // (the input moved into the `map` of `flip`: `(move)` in the IR)
    let src = "function flip(d: u8[n]): u8[n] {
    return map(d, (b) => b ^ 1u8);
}
action main(args: string[k]): i32 {
    const flipped = flip(readStdin());
    print([flipped.length, i64(sum(map(flipped, (b) => u64(b)))), i64(flipped[0]), i64(flipped[flipped.length - 1])] as const);
    return 0;
}
";
    let n = 20_000_000usize;
    let big: Vec<u8> = (0..n as u32).map(|i| (i.wrapping_mul(40503) >> 7) as u8).collect();
    let sum: u64 = big.iter().map(|&b| (b ^ 1) as u64).sum();
    let want = format!("({}, {}, {}, {})\n", n, sum, big[0] ^ 1, big[n - 1] ^ 1);
    let path = tmp("stdin_mapped_in_place.bin");
    std::fs::write(&path, &big).unwrap();
    let bin = compile(src, "stdin_mapped_in_place");
    for t in ["1", "4"] {
        let o = std::process::Command::new(&bin).env("ISC_THREADS", t).stdin(std::fs::File::open(&path).unwrap()).output().unwrap();
        assert_eq!((o.status.code(), String::from_utf8_lossy(&o.stdout).to_string()), (Some(0), want.clone()), "{} threads: {}", t, String::from_utf8_lossy(&o.stderr));
    }
}
