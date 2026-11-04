//! The standard output (D299): written by blocks into a file or a device
//! that is not a terminal, at each newline into a pipe (read as it comes);
//! the same bytes either way and as the interpreter, every line written
//! before a panic, and before the end of `main`.

mod common;
use common::*;

const LINES: &str = "action lines(i: i64, n: i64): void {
    if (i == n) return;
    print(i * 7919);
    print(\"line \" + toString(i));
    printBytes(bytes(i % 3 == 0 ? \"bytes\\n\" : \"\"));
    return lines(i + 1, n);
}

action main(args: string[k]): i32 {
    const n = parseI64(args[0]);
    lines(0, n);
    print(map(iota(20), (i) => i * i));
    if (args.length > 1) {
        print(\"before the panic\");
        print([1, 2, 3][n]);
    }
    return 0;
}
";

/// The program with its standard output into `to` (a file, a pipe or
/// `/dev/null`): its exit code, the bytes of the file (or of the pipe), its
/// standard error.
fn run_to(bin: &std::path::Path, args: &[&str], to: &str) -> (Option<i32>, String, String) {
    let mut cmd = std::process::Command::new(bin);
    cmd.args(args).stderr(std::process::Stdio::piped());
    let file = tmp(&format!("output_{}.txt", to));
    match to {
        "file" => {
            cmd.stdout(std::fs::File::create(&file).unwrap());
        }
        "null" => {
            cmd.stdout(std::fs::OpenOptions::new().write(true).open("/dev/null").unwrap());
        }
        _ => {
            cmd.stdout(std::process::Stdio::piped());
        }
    }
    let o = cmd.output().unwrap();
    let out = match to {
        "file" => std::fs::read_to_string(&file).unwrap(),
        "null" => String::new(),
        _ => String::from_utf8_lossy(&o.stdout).to_string(),
    };
    (o.status.code(), out, String::from_utf8_lossy(&o.stderr).to_string())
}

#[test]
fn output_by_blocks_into_files_and_by_lines_into_pipes() {
    let bin = compile(LINES, "output_blocks");
    // more than a buffer of 64 KiB of lines, and a few
    for n in ["20000", "3"] {
        let want = interp_ok(LINES, &[n]);
        for to in ["file", "pipe"] {
            let (code, out, err) = run_to(&bin, &[n], to);
            assert_eq!(code, Some(0), "{} lines into a {}: {}", n, to, err);
            assert!(out == want, "{} lines into a {}: {} bytes, {} expected", n, to, out.len(), want.len());
        }
        let (code, _, err) = run_to(&bin, &[n], "null");
        assert_eq!(code, Some(0), "{} lines into /dev/null: {}", n, err);
    }
    // a panic after the lines: every line before it written, then its message
    let (want_out, want_err) = match interp(LINES, &["20000", "panic"]) {
        Outcome::Panic(m, out) => (out, m),
        Outcome::Exit(c, out) => panic!("no panic ({} {})", c, out.len()),
    };
    for to in ["file", "pipe"] {
        let (code, out, err) = run_to(&bin, &["20000", "panic"], to);
        assert_eq!(code, Some(101), "a panic into a {}", to);
        assert!(out == want_out, "a panic into a {}: {} bytes, {} expected", to, out.len(), want_out.len());
        assert_eq!(err.trim_end(), want_err, "a panic into a {}", to);
    }
}

const NUMBERS: &str = "action main(args: string[k]): i32 {
    const ps = map(iota(19), (k) => fold(iota(k), 1, (p, j) => p * 10));
    const xs = map(iota(19 * 4), (i) => ps[i / 4] + [-1, 0, 1, 7][i % 4]);
    print(xs);
    print(map(xs, (x) => -x));
    print([9223372036854775807, -9223372036854775807 - 1, 0, -1]);
    print([18446744073709551615u64, 0u64, 10000000000000000000u64, 9999999999999999999u64]);
    print([255u8, 0u8]);
    print([-128i8, 127i8]);
    print([4294967295u32, 0u32]);
    print([-2147483647i32 - 1i32, 2147483647i32]);
    print(map(xs, (x) => toString(x)).join(\",\"));
    print(toString(-9223372036854775807 - 1) + \" \" + toString(18446744073709551615u64));
    print(`${-42} and ${100u64}`);
    return 0;
}
";

/// D300: the decimal text of integers (`print`, `toString`, templates):
/// every count of digits around its powers of ten, both signs, the extremes
/// of `i64` and `u64`, narrower integers: the text of the interpreter.
#[test]
fn integers_written_as_the_interpreter() {
    let want = interp_ok(NUMBERS, &[]);
    let bin = compile(NUMBERS, "output_numbers");
    let r = run_bin(&bin, &[]);
    assert_eq!((r.code, r.stdout.as_str()), (0, want.as_str()), "{}", r.stderr);
}
