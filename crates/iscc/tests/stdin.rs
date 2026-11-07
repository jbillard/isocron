//! The standard input (D155): `readStdin()`, `readStdinText()`,
//! `readStdinLines()`, the same in the interpreter and in the compiled
//! program. One test: the input of the interpreter is a global.

mod common;
use common::*;

/// The program `src` run by valgrind (glibc's allocator, 2 threads): no
/// error, nothing lost.
fn valgrind_clean(src: &str, name: &str, args: &[&str]) {
    let vg = std::path::PathBuf::from("/usr/bin/valgrind");
    if !vg.is_file() {
        eprintln!("valgrind not installed: {} skipped", name);
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
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "{} under valgrind:\n{}", name, err);
}
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

/// D298: the lines of a large text made in parallel are kept in one block,
/// freed with the last of them: lines kept in another array and sorted
/// after the array of all the lines is gone, empty lines, `\r\n` lines;
/// on 1 (each line its own block), 3, 4 and 8 threads, the same lines as
/// Rust's rules; nothing lost, nothing read or freed out of place
/// (valgrind, glibc's allocator).
#[test]
fn lines_kept_in_one_block_freed_with_the_last() {
    let src = "action kept(): string[k] {
    const lines = readStdinLines();
    print(lines.length);
    print(lines.slice(lines.length / 2, lines.length / 2 + 3).join(\"|\"));
    return filter(lines, (l) => l.length % 7 == 3);
}
action main(args: string[k]): i32 {
    const keep = kept();
    const sorted = sortBy(keep, (l) => l);
    print(keep.length);
    print(sum(map(keep, (l) => l.length)));
    print([sorted[0], sorted[sorted.length - 1]]);
    return 0;
}
";
    let mut x = 777u64;
    let mut text = Vec::new();
    while text.len() < 3 << 20 {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        let len = if x % 9 == 0 { 0 } else { (x >> 8) as usize % 90 };
        text.extend((0..len).map(|j| b'a' + ((x >> (j % 40)) % 26) as u8));
        if x % 11 == 0 {
            text.push(b'\r');
        }
        text.push(b'\n');
    }
    let st = std::str::from_utf8(&text).unwrap();
    let lines: Vec<&str> = st.strip_suffix('\n').unwrap().split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l)).collect();
    let n = lines.len();
    let keep: Vec<&str> = lines.iter().copied().filter(|l| l.len() % 7 == 3).collect();
    let mut sorted = keep.clone();
    sorted.sort();
    let want = format!(
        "{}\n{}\n{}\n{}\n[{}, {}]\n",
        n,
        lines[n / 2..n / 2 + 3].join("|"),
        keep.len(),
        keep.iter().map(|l| l.len()).sum::<usize>(),
        sorted[0],
        sorted[sorted.len() - 1]
    );
    let path = tmp("stdin_lines_block.txt");
    std::fs::write(&path, &text).unwrap();
    let bin = compile(src, "stdin_lines_block");
    for threads in ["1", "3", "4", "8"] {
        let o = std::process::Command::new(&bin).env("ISC_THREADS", threads).stdin(std::fs::File::open(&path).unwrap()).output().unwrap();
        assert_eq!((o.status.code(), String::from_utf8_lossy(&o.stdout).to_string()), (Some(0), want.clone()), "{} threads: {}", threads, String::from_utf8_lossy(&o.stderr));
    }
    let vg = std::path::PathBuf::from("/usr/bin/valgrind");
    if !vg.is_file() {
        eprintln!("valgrind not installed: stdin_lines_block skipped");
        return;
    }
    let opts = isc_codegen::Options { isa: isc_codegen::Isa::Avx2, ..isc_codegen::Options::default() };
    let bin = compile_glibc(src, "stdin_lines_block_vg", opts);
    let o = std::process::Command::new(vg)
        .args(["--error-exitcode=9", "--leak-check=full", "--errors-for-leak-kinds=definite,indirect", "-q"])
        .arg(&bin)
        .env("ISC_ALLOC", "glibc")
        .env("ISC_THREADS", "2")
        .stdin(std::fs::File::open(&path).unwrap())
        .output()
        .unwrap();
    assert_eq!((o.status.code(), String::from_utf8_lossy(&o.stdout).to_string()), (Some(0), want), "under valgrind:\n{}", String::from_utf8_lossy(&o.stderr));
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

/// D315: the standard input mapped from its file (D257), read only, given
/// to a replacement of each match by as many bytes at its last use: not
/// written where it is (the result is new), freed.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn a_mapped_input_given_to_a_replacement_is_not_written() {
    let src = "action main(args: string[k]): i32 {
    const r = readStdinText().replace(/ab/g, \"cd\");
    print(r.length);
    print(countMatches(r, /cd/g));
    print(r.slice(0, 12));
    return 0;
}
";
    let n = 8_000_000usize;
    let text: Vec<u8> = (0..n).map(|i| b"abxa"[i % 4]).collect();
    let path = tmp("stdin_given.txt");
    std::fs::write(&path, &text).unwrap();
    let bin = compile(src, "stdin_given");
    let want = format!("{}\n{}\ncdxacdxacdxa\n", n, n / 4);
    for threads in ["1", "4"] {
        let o = std::process::Command::new(&bin).env("ISC_THREADS", threads).stdin(std::fs::File::open(&path).unwrap()).output().unwrap();
        assert_eq!((o.status.code(), String::from_utf8_lossy(&o.stdout).to_string()), (Some(0), want.clone()), "{} threads: {}", threads, String::from_utf8_lossy(&o.stderr));
    }
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

/// D427: a large join made by ranges of its strings in parallel: strings
/// of 0 to 299 bytes, empty ones, one of 3 MB (a range of its own),
/// separators of 0, 1 and 3 bytes; 1 to 8 threads (other ranges); the same
/// bytes as Rust's `join` and as the interpreter.
#[test]
fn a_large_join_made_in_parallel() {
    let src = "action main(args: string[k]): i32 {
    const n = parseI64(args[0]);
    const big = parseI64(args[1]);
    const parts = map(iota(n), (i) => stringOf(map(iota(i == big ? 3000000 : (i * 7919) % 300 * (i % 5 == 0 ? 0 : 1)), (j) => u8(97 + (i + j) % 26))));
    const seps = [\"\", \",\", \"--|\"];
    print(map(seps, (sep) => {
        const t = parts.join(sep);
        const b = bytes(t);
        return [t.length, sum(map(iota(b.length), (i) => i64(b[i]) * (i % 7 + 1)))];
    }));
    return 0;
}
";
    let want = |n: usize, big: usize| -> String {
        let parts: Vec<String> = (0..n).map(|i| {
            let len = if i == big { 3_000_000 } else { (i * 7919) % 300 * if i % 5 == 0 { 0 } else { 1 } };
            (0..len).map(|j| (97 + ((i + j) % 26) as u8) as char).collect()
        }).collect();
        let r: Vec<String> = ["", ",", "--|"].iter().map(|sep| {
            let t = parts.join(sep);
            let sum: i64 = t.bytes().enumerate().map(|(i, b)| b as i64 * (i as i64 % 7 + 1)).sum();
            format!("[{}, {}]", t.len(), sum)
        }).collect();
        format!("[{}]\n", r.join(", "))
    };
    let bin = compile(src, "join_parallel");
    for (n, big) in [(1usize, 0usize), (2, 1), (30000, 100000), (30000, 0), (30000, 17), (30000, 29999), (90000, 45000)] {
        let w = want(n, big);
        for t in ["1", "2", "3", "4", "8"] {
            let o = std::process::Command::new(&bin).args([n.to_string(), big.to_string()]).env("ISC_THREADS", t).output().unwrap();
            assert_eq!((o.status.code(), String::from_utf8_lossy(&o.stdout).to_string()), (Some(0), w.clone()), "{} strings, the large one {}, {} threads", n, big, t);
        }
    }
    let p = iscc::frontend("join_parallel.isc", src).unwrap();
    let mut out = Vec::new();
    assert_eq!(isc_interp::run(&p, &["30000".to_string(), "17".to_string()], &mut out).unwrap(), 0);
    assert_eq!(String::from_utf8_lossy(&out), want(30000, 17), "interpreter");
}

/// D428: a join read once in order by a `map` of its bytes, never made:
/// its ranges gathered 16 KiB at a time and mapped from there. Separators
/// of 0, 1 and 3 bytes, empty strings, strings longer than the buffer, no
/// string; results of 8 bits (narrowed, D218), of 64 bits and through a
/// table; 1 to 8 threads; the values of Rust and of the interpreter; a
/// failed read of the table, the panic of the interpreter.
#[test]
fn a_join_read_by_a_map_is_never_made() {
    let src = "action main(args: string[k]): i32 {
    const n = parseI64(args[0]);
    const big = parseI64(args[1]);
    const parts = map(iota(n), (i) => stringOf(map(iota(i == big ? 40000 : (i * 7919) % 300 * (i % 5 == 0 ? 0 : 1)), (j) => u8(97 + (i + j) % 26))));
    const table = map(iota(128), (c) => c * 3 + 1);
    const a = parts.join(\"\");
    const codes = map(bytes(a), (b) => u8((b | 32u8) == 97u8 ? 0 : 3));
    const c = parts.join(\",\");
    const wide = map(bytes(c), (b) => i64(b) * 1000 + 7);
    const d = parts.join(\"--|\");
    const looked = map(bytes(d), (b) => table[i64(b)]);
    print([codes.length, sum(map(iota(codes.length), (i) => i64(codes[i]) * (i % 7 + 1)))]);
    print([wide.length, sum(map(iota(wide.length), (i) => wide[i] % 1000003 * (i % 5 + 1)))]);
    print([looked.length, sum(map(iota(looked.length), (i) => looked[i] * (i % 3 + 1)))]);
    return 0;
}
";
    let want = |n: usize, big: usize| -> String {
        let parts: Vec<String> = (0..n).map(|i| {
            let len = if i == big { 40000 } else { (i * 7919) % 300 * if i % 5 == 0 { 0 } else { 1 } };
            (0..len).map(|j| (97 + ((i + j) % 26) as u8) as char).collect()
        }).collect();
        let a = parts.join("");
        let codes: i64 = a.bytes().enumerate().map(|(i, b)| (if b | 32 == 97 { 0 } else { 3 }) * (i as i64 % 7 + 1)).sum();
        let c = parts.join(",");
        let wide: i64 = c.bytes().enumerate().map(|(i, b)| (b as i64 * 1000 + 7) % 1000003 * (i as i64 % 5 + 1)).sum();
        let d = parts.join("--|");
        let looked: i64 = d.bytes().enumerate().map(|(i, b)| (b as i64 * 3 + 1) * (i as i64 % 3 + 1)).sum();
        format!("[{}, {}]\n[{}, {}]\n[{}, {}]\n", a.len(), codes, c.len(), wide, d.len(), looked)
    };
    let bin = compile(src, "join_map");
    let ll = std::fs::read_to_string(bin.with_extension("ll")).unwrap();
    assert_eq!(ll.matches("call i64 @isc_join_ranges").count(), 3, "a join made");
    for (n, big) in [(0usize, 0usize), (1, 0), (1, 5), (2, 1), (3000, 1234), (30000, 100000), (30000, 29999), (90000, 0)] {
        let w = want(n, big);
        for t in ["1", "2", "3", "4", "8"] {
            let o = std::process::Command::new(&bin).args([n.to_string(), big.to_string()]).env("ISC_THREADS", t).output().unwrap();
            assert_eq!((o.status.code(), String::from_utf8_lossy(&o.stdout).to_string()), (Some(0), w.clone()), "{} strings, the long one {}, {} threads", n, big, t);
        }
    }
    let p = iscc::frontend("join_map.isc", src).unwrap();
    let mut out = Vec::new();
    assert_eq!(isc_interp::run(&p, &["3000".to_string(), "1234".to_string()], &mut out).unwrap(), 0);
    assert_eq!(String::from_utf8_lossy(&out), want(3000, 1234), "interpreter");
    valgrind_clean(src, "join_map", &["3000", "1234"]);
    // a byte past the table: the panic of the program
    let bad = src.replace("map(iota(128), (c) => c * 3 + 1)", "map(iota(100), (c) => c * 3 + 1)");
    let bin = compile(&bad, "join_map_panic");
    let Outcome::Panic(msg, _) = interp(&bad, &["3000", "1234"]) else { panic!("no panic in the interpreter") };
    for t in [1, 4] {
        let r = run_bin_threads(&bin, &["3000", "1234"], Some(t));
        assert_eq!(r.code, 101, "T={}: {}", t, r.stderr);
        assert_eq!(r.stderr.trim_end(), msg, "T={}", t);
    }
}
