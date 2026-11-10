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
    run_to_threads(bin, args, to, "")
}

/// `run_to` with `ISC_THREADS = threads` (the default for none).
fn run_to_threads(bin: &std::path::Path, args: &[&str], to: &str, threads: &str) -> (Option<i32>, String, String) {
    let mut cmd = std::process::Command::new(bin);
    cmd.args(args).stderr(std::process::Stdio::piped());
    if !threads.is_empty() {
        cmd.env("ISC_THREADS", threads);
    }
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

const FLOATS: &str = r#"type Shape = Circle(f64) | Rect(f64, f32) | Dot;
type Chain = End | Link(f64, Chain);
type Tree = Leaf(f32) | Node(Tree, Tree);

function chain(xs: f64[n], i: i64): Chain {
    return i == n ? End : Link(xs[i], chain(xs, i + 1));
}

action pair(x: f64): i64 {
    print([x, f32(x)] as const);
    return 1;
}

action main(args: string[k]): i32 {
    const zero = f64(args.length) * 0.0;
    const xs = [0.0, -0.0, 1.0, -1.5, 0.1, 1.0 / 3.0, 1e21, 1e-7, 123456789.125, 5e-324, 1.7976931348623157e308, 9007199254740993.0,
        0.0 / zero, 1.0 / zero, -1.0 / zero, 4.35, 0.000001, 1e22, 2.5e-10, 65536.0, -123456.7890625, 1e-300, 99999999999999990000.0, 0.5];
    const ys = map(xs, (x) => f32(x));
    print(xs);
    print(ys);
    forEach(xs, (x) => {
        print(x);
    });
    forEach(ys, (y) => {
        print(y);
    });
    print([xs[12], xs[14], xs[13], xs[4]]);
    print([ys[13], ys[12], ys[14]]);
    print([xs[3], ys[4], xs[12]] as const);
    print({ a: xs[4], b: ys[5], name: "p", inner: [xs[7], ys[13]] as const });
    print(zip(xs, ys));
    print(map(iota(4), (i) => [xs[i], xs[i + 4], xs[i + 8], xs[i + 12], xs[i + 16], xs[i + 20]]));
    print(map(iota(3), (i) => [ys[i], ys[i + 3], ys[i + 20]]));
    print([Circle(xs[5]), Rect(xs[4], ys[3]), Dot]);
    print([Some(xs[6]), None, Some(xs[13])]);
    print(chain(xs, 0));
    print(Node(Leaf(ys[5]), Node(Leaf(ys[12]), Leaf(ys[1]))));
    print(`${xs[5]} ${ys[5]} ${xs[4].toFixed(3)} ${toString(xs[9])}`);
    print(sum(mapParallel(xs, pair)));
    return 0;
}
"#;

/// `print` of a float by the prelude (`$rtF64Text`, `$rtF32Text`, in place
/// of the runtime's `isc_print_f64` and `isc_print_f32`, removed), in
/// every place `print` writes one: alone, in an array, a tuple, a record,
/// an array of tuples, rows, values of sum types and nodes of recursive
/// types, by the tasks of `mapParallel`; a `f64` and a `f32`, zeros, what
/// is not a number, the least and the greatest double. The bytes of the
/// interpreter (its own text of a float, in Rust) into a file, a pipe and
/// `/dev/null`, on 1 thread and on 4 (the lines of the tasks then in any
/// order), in WebAssembly, with the allocator checked. Its text is
/// written in a buffer of the stack: the functions that write it there
/// allocate no block, and the program holds the table of the powers of
/// ten once, for `print` and for `toString`. In an array, a tuple and a
/// value of a sum type, the separator before a float is written with it
/// (a NaN first, in the middle and last). (The texts of 73000 floats
/// printed: `float_text_written_in_isocron` of `strings`.)
#[test]
fn floats_printed_by_the_prelude() {
    let want = interp_ok(FLOATS, &[]);
    assert!(want.starts_with("[0, -0, 1, -1.5, 0.1, 0.3333333333333333, 1000000000000000000000, 0.0000001, 123456789.125, 0."), "{}", want);
    assert!(want.contains("NaN, inf, -inf, 4.35, 0.000001, 10000000000000000000000, 0.00000000025, 65536, -123456.7890625, 0."), "{}", want);
    assert!(want.contains("\n[NaN, -inf, inf, 0.1]\n[inf, NaN, -inf]\n(-1.5, 0.1, NaN)\n{a: 0.1, b: 0.33333334, inner: (0.0000001, inf), name: p}\n"), "{}", want);
    assert!(want.contains("\n[Circle(0.3333333333333333), Rect(0.1, -1.5), Dot]\n[Some(1000000000000000000000), None, Some(inf)]\nLink(0, Link(-0, Link(1, "), "{}", want);
    assert!(want.contains("\nNode(Leaf(0.33333334), Node(Leaf(NaN), Leaf(-0)))\n0.3333333333333333 0.33333334 0.100 0."), "{}", want);
    let sorted = |t: &str| -> Vec<String> {
        let mut lines: Vec<String> = t.lines().map(String::from).collect();
        lines.sort();
        lines
    };
    let bin = compile(FLOATS, "output_floats");
    for threads in ["1", "4"] {
        for to in ["file", "pipe"] {
            let (code, out, err) = run_to_threads(&bin, &[], to, threads);
            assert_eq!(code, Some(0), "{} threads into a {}: {}", threads, to, err);
            if threads == "1" {
                assert_eq!(out, want, "1 thread into a {}", to);
            } else {
                assert_eq!(sorted(&out), sorted(&want), "4 threads into a {}", to);
            }
        }
        let (code, _, err) = run_to_threads(&bin, &[], "null", threads);
        assert_eq!(code, Some(0), "{} threads into /dev/null: {}", threads, err);
    }
    checked_clean_bin(&bin, &[], None);
    match wasi_runner() {
        Some(runner) => {
            let wasm = compile_wasm(FLOATS, "output_floats");
            let run = run_wasm(&runner, &wasm, &[]);
            assert_eq!((run.code, run.stdout.as_str()), (0, want.as_str()), "WebAssembly: {}", run.stderr);
        }
        None => eprintln!("wasmtime not installed: `print` of floats in WebAssembly skipped"),
    }
    // the code: no function of the runtime prints a float; the variants
    // of the functions of the prelude that write the text in the buffer
    // of `print` allocate nothing; one table of the powers of ten
    let ll = std::fs::read_to_string(bin.with_extension("ll")).unwrap();
    assert!(!ll.contains("isc_print_f64") && !ll.contains("isc_print_f32"), "a float printed by the runtime");
    let buffered: Vec<&str> = ll.split("\ndefine ").filter(|f| f.split('(').next().is_some_and(|head| head.ends_with("_buf"))).collect();
    assert!(buffered.len() >= 2, "{} functions that write in the buffer of `print`", buffered.len());
    for f in &buffered {
        let name = f.split('(').next().unwrap();
        assert!(!f.contains("@isc_alloc") && !f.contains("@isc_free"), "{} allocates or frees", name);
    }
    assert_eq!(ll.matches("_print(double %x, i64 %pre)").count(), 1, "the function that prints a `f64`");
    assert_eq!(ll.matches("_print(float %x, i64 %pre)").count(), 1, "the function that prints a `f32`");
    // (the first word of the table of `$pow10G`, 10^324, in the assembly:
    // LLVM makes one of the equal tables of the functions that read it)
    let asm = std::fs::read_to_string(bin.with_extension("s")).unwrap();
    assert_eq!(asm.matches(".quad\t5696189077778435540").count(), 1, "the table of the powers of ten, once");
}

/// The functions of the prelude lowered for the texts of the code generator
/// (`Program::text_fns`) are emitted when the program reaches them only:
/// a program that prints a `f64` has the variant of its function that
/// writes in the buffer of `print`, not the function that makes a block,
/// nor anything of a `f32` (13500 lines of LLVM IR for one `print` when
/// every function was emitted, 117 ms to build it where it took 19); a
/// program that writes the text of a `f64` has that function alone. Each
/// program prints what the interpreter prints.
#[test]
fn text_functions_emitted_when_reached() {
    let main = |body: &str| format!("action main(args: string[k]): i32 {{\n    {}\n    return 0;\n}}\n", body);
    let ir = |body: &str, name: &str| -> String {
        let src = main(body);
        let bin = compile(&src, name);
        let run = run_bin(&bin, &[]);
        assert_eq!((run.code, run.stdout.as_str()), (0, interp_ok(&src, &[]).as_str()), "{}: {}", body, run.stderr);
        std::fs::read_to_string(bin.with_extension("ll")).unwrap()
    };
    // (the function that makes the text of a `f64` in a block, that of a
    // `f32`, a block allocated: through the entry of the module, D605, whose
    // own text calls the runtime's allocator in every program)
    let has = |ll: &str| (ll.contains("(double %a0)"), ll.contains("(float %a0)"), ll.contains("call noalias ptr @isc.late.alloc("));
    let ll = ir("print(1.5 * f64(args.length + 1));\n    print(5e-324 * f64(args.length + 1));\n    print(0.0 / (f64(args.length) * 0.0));", "text_fns_print");
    assert!(ll.contains("_buf(ptr %tb, double %a0)") && !ll.contains("_buf(ptr %tb, float %a0)"), "the variant for `print` of a `f64` alone");
    assert_eq!(has(&ll), (false, false, false), "`print` of a `f64`");
    assert!(ll.lines().count() < 7000, "`print` of a `f64`: {} lines of LLVM IR", ll.lines().count());
    let ll = ir("print(toString(1.5 * f64(args.length + 1)).length);", "text_fns_text");
    assert!(!ll.contains("_buf("), "a variant for `print` in a program that prints no float");
    assert_eq!(has(&ll), (true, false, true), "`toString` of a `f64`");
    let ll = ir("const x = f32(args.length) + 0.5f32;\n    print(x);\n    print(toString(f64(x)) + (1.0 / 3.0).toFixed(args.length + 20));", "text_fns_both");
    assert!(ll.contains("_buf(ptr %tb, float %a0)") && !ll.contains("_buf(ptr %tb, double %a0)"), "the variant for `print` of a `f32` alone");
    assert_eq!(has(&ll), (true, false, true), "`print` of a `f32`, `toString` of a `f64`");
    let ll = ir("print(args.length + 1);\n    print(`${args.length}`);", "text_fns_none");
    assert_eq!((has(&ll).0, has(&ll).1, ll.contains("_buf(")), (false, false, false), "no float written");
}
