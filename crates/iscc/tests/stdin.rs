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
