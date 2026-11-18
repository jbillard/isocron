//! The standard input read by parts (D702): `readStdinText()` read once by
//! a `fold` (or the length) of its runs of a class of bytes, in the next
//! statement that reads anything, nothing between them having an effect:
//! never whole in memory. The same outputs and the same panics as the
//! interpreter, which reads it whole; from a pipe and from a file; a run
//! longer than the buffer; texts in another case; a panic for bytes that
//! are not UTF-8 at the place of the input where they are; the memory of
//! the program that of its parts and of its table, not of its input.

mod common;
use common::*;
use std::io::Write as _;

/// The words of the input in lower case, counted, the most frequent first
/// (countwords).
const COUNT: &str = "action main(args: string[k]): i32 {
    const text = readStdinText().toLowerCase();
    const words = matches(text, /[^ \\n]+/g);
    const counts = countBy(words, (w) => w);
    const ordered = sortBy(counts.entries(), (e) => {
        const [w, c] = e;
        return -c;
    });
    print(join(map(ordered, (e) => {
        const [w, c] = e;
        return `${w} ${c}`;
    }), \"\\n\"));
    return 0;
}
";

/// The number of runs of a class of ASCII letters (its complement holds
/// the bytes of the characters of several bytes).
const LENGTH: &str = "action main(args: string[k]): i32 {
    const t = readStdinText();
    print(matches(t, /[a-z]+/g).length);
    return 0;
}
";

/// A `fold` of the runs of a text in upper case (the case at the runs).
const UPPER: &str = "action main(args: string[k]): i32 {
    const t = readStdinText();
    const n = fold(matches(t.toUpperCase(), /[A-Z]+/g), 0, (a, w) => a * 31 % 1000003 + w.length * 2 + 1);
    print(n);
    return 0;
}
";

/// The program reads its input by parts.
fn reads_by_parts(src: &str) -> bool {
    fn has(e: &isc_ir::Expr) -> bool {
        matches!(e.kind, isc_ir::ExprKind::InputPieces(_)) || e.children().into_iter().any(has)
    }
    let p = iscc::frontend("parts.isc", src).unwrap_or_else(|e| panic!("{}", e));
    has(&p.body) || p.fns.iter().any(|f| has(&f.body))
}

fn run_with_stdin(bin: &std::path::Path, input: &[u8]) -> (i32, String, String) {
    let mut c = std::process::Command::new(bin)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut w = c.stdin.take().unwrap();
    let data = input.to_vec();
    // (written by another thread: the program writes as it reads)
    let t = std::thread::spawn(move || {
        let _ = w.write_all(&data);
    });
    let o = c.wait_with_output().unwrap();
    t.join().unwrap();
    (o.status.code().unwrap_or(-1), String::from_utf8_lossy(&o.stdout).to_string(), String::from_utf8_lossy(&o.stderr).to_string())
}

/// The input of the interpreter is a global: one test at a time sets it.
static INTERP_STDIN: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn interp_with_stdin(src: &str, input: &[u8]) -> Outcome {
    let _one = INTERP_STDIN.lock().unwrap_or_else(|e| e.into_inner());
    *isc_interp::rt::STDIN.lock().unwrap() = Some(input.to_vec());
    interp(src, &[])
}

/// Words of a vocabulary with texts of several bytes (some whose other case
/// has another length: `İ`, `ẞ`), separated by spaces, new lines and tabs,
/// one word longer than the buffer of the parts (64 KiB) in the middle.
fn words_input(seed: u64, words: usize, long: bool) -> Vec<u8> {
    let vocab: Vec<String> = (0..3000)
        .map(|i| match i % 7 {
            0 => format!("Été{}", i),
            1 => format!("straße{}", i),
            2 => format!("İstanbul{}", i % 50),
            3 => format!("ẞig{}", i % 40),
            4 => format!("WORD{}", i % 300),
            _ => format!("w{}x", i),
        })
        .collect();
    let mut x = seed;
    let mut out = Vec::new();
    for k in 0..words {
        x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        out.extend_from_slice(vocab[(x >> 33) as usize % vocab.len()].as_bytes());
        out.push(match (x >> 20) % 10 {
            0 => b'\n',
            1 => b'\t',
            _ => b' ',
        });
        if long && k == words / 2 {
            out.extend(std::iter::repeat_n(b'q', 100_000));
            out.push(b' ');
        }
    }
    out
}

/// `src` compiled gives what the interpreter gives for each input, read
/// from a pipe.
fn same_as_interpreter(src: &str, name: &str, inputs: &[Vec<u8>]) {
    assert!(reads_by_parts(src), "{}: not read by parts", name);
    let bin = compile(src, name);
    for (k, input) in inputs.iter().enumerate() {
        let want = match interp_with_stdin(src, input) {
            Outcome::Exit(0, out) => out,
            Outcome::Exit(c, out) => panic!("{} {}: the interpreter exited with {}: {}", name, k, c, out),
            Outcome::Panic(m, _) => panic!("{} {}: the interpreter panicked: {}", name, k, m),
        };
        let (code, out, err) = run_with_stdin(&bin, input);
        assert_eq!(code, 0, "{} {}: {}", name, k, err);
        assert!(out == want, "{} {} ({} bytes): the compiled program differs from the interpreter", name, k, input.len());
    }
}

#[test]
fn words_counted_as_the_input_is_read() {
    let inputs = vec![
        Vec::new(),
        b"Hello world\nhello".to_vec(),
        b"  \n\n ".to_vec(),
        words_input(1, 40_000, true),
        words_input(2, 60_000, false),
    ];
    same_as_interpreter(COUNT, "parts_count", &inputs);
    same_as_interpreter(LENGTH, "parts_length", &inputs);
    same_as_interpreter(UPPER, "parts_upper", &inputs);
    // the counts themselves, by another program: the words of the input in
    // lower case as Rust has them (Unicode's mappings, each character alone)
    let input = &inputs[3];
    let text = String::from_utf8(input.clone()).unwrap().to_lowercase();
    let mut want: std::collections::HashMap<&str, u64> = Default::default();
    for w in text.split([' ', '\n']).filter(|w| !w.is_empty()) {
        *want.entry(w).or_insert(0) += 1;
    }
    let bin = compile(COUNT, "parts_count");
    let (_, out, _) = run_with_stdin(&bin, input);
    let got: std::collections::HashMap<&str, u64> = out
        .lines()
        .map(|l| {
            let (w, c) = l.rsplit_once(' ').unwrap();
            (w, c.parse().unwrap())
        })
        .collect();
    assert_eq!(got, want);
}

#[test]
fn input_parts_from_a_file() {
    // the same bytes from a file (read by the system in parts, as from a
    // pipe), from an offset of it
    let input = words_input(3, 50_000, true);
    let path = tmp("parts_input.txt");
    std::fs::write(&path, &input).unwrap();
    let bin = compile(COUNT, "parts_count_file");
    for skip in [0usize, 7, 200_000] {
        let want = match interp_with_stdin(COUNT, &input[skip..]) {
            Outcome::Exit(0, out) => out,
            _ => panic!("interpreter"),
        };
        let mut f = std::fs::File::open(&path).unwrap();
        std::io::Seek::seek(&mut f, std::io::SeekFrom::Start(skip as u64)).unwrap();
        let o = std::process::Command::new(&bin).stdin(f).output().unwrap();
        assert_eq!(o.status.code(), Some(0), "from byte {}: {}", skip, String::from_utf8_lossy(&o.stderr));
        assert!(String::from_utf8_lossy(&o.stdout) == want, "from byte {}", skip);
    }
}

#[test]
fn input_parts_that_are_not_utf8() {
    // the panic of `readStdinText`, at its place, its byte counted from the
    // start of the input: one past the first part, one cut by the end
    let mut far = words_input(4, 30_000, false);
    let at = far.len() * 2 / 3;
    // (the start of a character)
    let at = (at..).find(|&i| far[i] < 0x80 || far[i] >= 0xC0).unwrap();
    far.insert(at, 0xFF);
    let mut cut = words_input(5, 2_000, false);
    cut.extend_from_slice(&[b' ', 0xE2, 0x82]);
    for (k, input) in [far, cut].iter().enumerate() {
        for (src, name) in [(COUNT, "parts_bad_count"), (UPPER, "parts_bad_upper")] {
            let want = match interp_with_stdin(src, input) {
                Outcome::Panic(m, _) => m,
                _ => panic!("{} {}: the interpreter did not panic", name, k),
            };
            assert!(want.contains("readStdinText: the standard input is not UTF-8 text"), "{}", want);
            let bin = compile(src, name);
            let (code, out, err) = run_with_stdin(&bin, input);
            assert_eq!((code, out.as_str()), (101, ""), "{} {}: {}", name, k, err);
            assert!(err.contains(&want), "{} {}: {} (want {})", name, k, err, want);
        }
    }
}

#[test]
fn input_parts_under_valgrind() {
    let Some(vg) = valgrind_path() else {
        eprintln!("valgrind not run (not installed, or ISC_TEST_NO_VALGRIND): skipped");
        return;
    };
    let input = words_input(6, 30_000, true);
    let want = match interp_with_stdin(COUNT, &input) {
        Outcome::Exit(0, out) => out,
        _ => panic!("interpreter"),
    };
    let bin = compile(COUNT, "parts_checked");
    checked_clean_bin(&bin, &[], Some(&input));
    let opts = isc_codegen::Options { isa: isc_codegen::Isa::Avx2, ..isc_codegen::Options::default() };
    let bin = compile_glibc(COUNT, "parts_vg", opts);
    let mut c = std::process::Command::new(vg)
        .args(["--error-exitcode=9", "--leak-check=full", "--errors-for-leak-kinds=definite,indirect", "-q"])
        .arg(&bin)
        .env("ISC_ALLOC", "glibc")
        .env("ISC_THREADS", "2")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut w = c.stdin.take().unwrap();
    let t = std::thread::spawn(move || {
        let _ = w.write_all(&input);
    });
    let o = c.wait_with_output().unwrap();
    t.join().unwrap();
    assert_eq!(o.status.code(), Some(0), "under valgrind:\n{}", String::from_utf8_lossy(&o.stderr));
    assert!(String::from_utf8_lossy(&o.stdout) == want);
}

/// The most memory of the program counting the words of 24 MB (`VmHWM`, as
/// it prints it after): its parts and its table, never its input (before:
/// the input and its copy in lower case, 48 MB). Isocron's allocator only.
#[test]
fn input_parts_bound_the_memory() {
    if memcheck() || std::env::var("ISC_ALLOC").is_ok() {
        return;
    }
    let src = format!(
        "{}",
        COUNT.replace(
            "    return 0;\n}",
            "    const status = lines(readText(\"/proc/self/status\"));\n    print(filter(status, (l) => l.startsWith(\"VmHWM\")));\n    return 0;\n}"
        )
    );
    assert!(reads_by_parts(&src));
    let words: Vec<String> = (0..200).map(|i| format!("word{} ", i)).collect();
    let mut input = Vec::new();
    let mut x = 7u64;
    while input.len() < 24_000_000 {
        x = x.wrapping_mul(6364136223846793005).wrapping_add(1);
        input.extend_from_slice(words[(x >> 33) as usize % 200].as_bytes());
    }
    let bin = compile(&src, "parts_memory");
    let (code, out, err) = run_with_stdin(&bin, &input);
    assert_eq!(code, 0, "{}", err);
    let line = out.lines().last().unwrap_or("");
    let kib: u64 = line.chars().filter(|c| c.is_ascii_digit()).collect::<String>().parse().unwrap_or_else(|_| panic!("{}", out));
    assert!(kib < 12 * 1024, "{} KiB", kib);
}

#[test]
fn inputs_read_whole_when_an_effect_is_between() {
    // an effect between the read and the runs, the text read again, the runs
    // in a branch: read whole, as written (section 12)
    let between = "action main(args: string[k]): i32 {
    const t = readStdinText();
    print(\"words:\");
    print(matches(t, /[a-z]+/g).length);
    return 0;
}
";
    let clock = "action main(args: string[k]): i32 {
    const t = readStdinText();
    const t0 = clock();
    print(matches(t, /[a-z]+/g).length);
    return 0;
}
";
    let twice = "action main(args: string[k]): i32 {
    const t = readStdinText();
    print(matches(t, /[a-z]+/g).length + t.length);
    return 0;
}
";
    let branch = "action main(args: string[k]): i32 {
    const t = readStdinText();
    print(args.length > 5 ? matches(t, /[a-z]+/g).length : 0);
    return 0;
}
";
    for (src, name) in [(between, "between"), (clock, "clock"), (twice, "twice"), (branch, "branch")] {
        assert!(!reads_by_parts(src), "{}: read by parts", name);
    }
    let bin = compile(between, "parts_between");
    let (code, out, err) = run_with_stdin(&bin, b"ab cd\nef");
    assert_eq!((code, out.as_str()), (0, "words:\n3\n"), "{}", err);
}
