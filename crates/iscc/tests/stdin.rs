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
/// (`text_lines`), on 1, 3 and 4 threads (other cuts).
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
    let mut text = Vec::new();
    while text.len() < 3 << 20 {
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
        for threads in ["1", "3", "4"] {
            let o = std::process::Command::new(&bin).env("ISC_THREADS", threads).stdin(std::fs::File::open(&path).unwrap()).output().unwrap();
            assert_eq!((o.status.code(), String::from_utf8_lossy(&o.stdout).to_string()), (Some(0), want.clone()), "{} threads, final newline {}", threads, final_newline);
        }
    }
}
