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
