//! DECISIONS D136: bitwise operators on integers, `++` on arrays,
//! `printBytes`, and the program `mandelbrot` of the Benchmarks Game
//! written with them (examples/benchmarks_game), checked against an
//! independent implementation of the classic algorithm.

mod common;
use common::*;

fn valgrind_clean(src: &str, name: &str, args: &[&str]) {
    let vg = std::path::PathBuf::from("/usr/bin/valgrind");
    if !vg.is_file() {
        eprintln!("valgrind not installed: {} skipped", name);
        return;
    }
    let opts = isc_codegen::Options { isa: isc_codegen::Isa::Avx2, ..isc_codegen::Options::default() };
    let bin = compile_with(src, &format!("{}_vg", name), opts);
    let out = std::process::Command::new(vg)
        .args(["--error-exitcode=9", "--leak-check=full", "--errors-for-leak-kinds=definite,indirect", "-q"])
        .arg(&bin)
        .args(args)
        .env("ISC_ALLOC", "glibc")
        // 2 threads, as the other suites: under valgrind (one thread at a
        // time) each idle worker of the pool spins before it sleeps
        .env("ISC_THREADS", "2")
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "{} under valgrind:\n{}", name, err);
}

fn agree(src: &str, name: &str, args: &[&str], expected: &str) {
    assert_eq!(interp_ok(src, args), expected, "interpreter");
    let bin = compile(src, name);
    let r = run_bin(&bin, args);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(r.stdout, expected, "compiled");
    valgrind_clean(src, name, args);
}

const BITS: &str = r#"type Opt<T> = Some(T) | None;

function depth(x: Opt<Opt<i64>>): i64 {
    return match (x) {
        Some(y) => 1,
        None => 0,
    };
}

action main(args: string[k]): i32 {
    print([12 & 10, 12 | 10, 12 ^ 10] as const);
    print([1 << 4, 256 >> 3, -256 >> 3, 1 << 65] as const);
    print([255u8 << 4, 240u8 >> 4, 128u8 >> 7, -128i8 >> 7, 1i8 << 7] as const);
    print(240u8 & 60u8 | 1u8);
    print(1 + 2 << 3);
    print(1 << 2 + 1 == 8);
    print((6 & 3) == 2);
    print((3 | 4) ^ 1);
    print(iota(8) & 3);
    print(iota(5) << 2);
    print(reduce(iota(9), (|), 0));
    print(reduce(map(iota(6), (i) => 1 << i), (^), 0));
    print(fold(iota(8), 0u8, (acc, b) => acc << 1 | (b % 3 == 0 ? 1u8 : 0u8)));
    print(depth(Some(Some(7))));
    const xor = 5;
    print(xor + 1);
    print(-1 >> 63);
    print(u64(1) << 63 >> 63);
    print(map(iota(4), (i) => u32(i) << 30));
    return 0;
}
"#;

#[test]
fn bitwise_operators() {
    agree(
        BITS,
        "bits",
        &[],
        "(8, 14, 6)\n(16, 32, -32, 2)\n(240, 15, 1, -1, -128)\n49\n24\ntrue\ntrue\n6\n[0, 1, 2, 3, 0, 1, 2, 3]\n[0, 4, 8, 12, 16]\n15\n63\n146\n1\n6\n-1\n1\n[0, 1073741824, 2147483648, 3221225472]\n",
    );
}

#[test]
fn bitwise_errors() {
    let e = iscc::frontend("t.isc", "action main(args: string[k]): i32 {\n    print(1.5 & 2.0);\n    return 0;\n}\n").unwrap_err();
    assert!(e.contains("integer"), "{}", e);
    let e = iscc::frontend("t.isc", "action main(args: string[k]): i32 {\n    print(true | false);\n    return 0;\n}\n").unwrap_err();
    assert!(e.contains("integer"), "{}", e);
    // `>` `>` apart is two comparisons, not a shift
    assert!(iscc::frontend("t.isc", "action main(args: string[k]): i32 {\n    print(8 > > 1);\n    return 0;\n}\n").is_err());
}

const CONCAT: &str = r#"function both(a: i64[n], b: i64[m]): i64[n + m] {
    return concat(a, b);
}

action main(args: string[k]): i32 {
    print(both(iota(3), [7, 8]));
    print(concat([1.5], map(iota(2), (i) => f64(i))));
    print(concat([[1, true] as const], [[2, false] as const, [3, true] as const]));
    print(concat(args, ["fin"]));
    print(concat(filter(iota(10), (i) => i % 3 == 0), iota(2)));
    print(len(concat(iota(0), iota(0))));
    print(sum(concat(iota(1000), iota(1000))));
    printBytes(concat(bytes("abc"), [10u8]));
    print("after");
    printBytes(bytes("x\n"));
    return 0;
}
"#;

#[test]
fn array_concatenation_and_print_bytes() {
    agree(
        CONCAT,
        "concat",
        &["un", "deux"],
        "[0, 1, 2, 7, 8]\n[1.5, 0, 1]\n[(1, true), (2, false), (3, true)]\n[un, deux, fin]\n[0, 3, 6, 9, 0, 1]\n0\n999000\nabc\nafter\nx\n",
    );
}

/// The classic algorithm of the Benchmarks Game, independently.
fn mandelbrot_reference(n: usize) -> Vec<u8> {
    let mut out = format!("P4\n{} {}\n", n, n).into_bytes();
    for y in 0..n {
        let ci = 2.0 * y as f64 / n as f64 - 1.0;
        let mut byte = 0u8;
        for x in 0..n {
            let cr = 2.0 * x as f64 / n as f64 - 1.5;
            let (mut zr, mut zi, mut tr, mut ti) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
            let mut i = 0;
            while i < 50 && tr + ti <= 4.0 {
                zi = 2.0 * zr * zi + ci;
                zr = tr - ti + cr;
                tr = zr * zr;
                ti = zi * zi;
                i += 1;
            }
            byte = (byte << 1) | (tr + ti <= 4.0) as u8;
            if x % 8 == 7 {
                out.push(byte);
                byte = 0;
            } else if x == n - 1 {
                out.push(byte << (7 - x % 8));
                byte = 0;
            }
        }
    }
    out
}

#[test]
fn mandelbrot_of_the_benchmarks_game() {
    // the program of the Benchmarks Game (the escape tested every 4
    // iterations) and its simplest form (at each iteration): the same image
    for file in ["mandelbrot.isc", "mandelbrot.simple.isc"] {
        mandelbrot_program(file);
    }
}

fn mandelbrot_program(file: &str) {
    let path = format!("{}/../../examples/benchmarks_game/{}", env!("CARGO_MANIFEST_DIR"), file);
    let src = std::fs::read_to_string(path).unwrap();
    let name = file.replace('.', "_");
    let bin = compile(&src, &name);
    for n in [1usize, 7, 8, 9, 200, 201, 1000] {
        let o = std::process::Command::new(&bin).arg(n.to_string()).output().unwrap();
        assert_eq!(o.status.code(), Some(0));
        assert!(o.stdout == mandelbrot_reference(n), "N = {}", n);
    }
    // the interpreter, on a small image
    let p = iscc::frontend("mandelbrot.isc", &src).unwrap();
    let mut out = Vec::new();
    assert_eq!(isc_interp::run(&p, &["57".to_string()], &mut out).unwrap(), 0);
    assert!(out == mandelbrot_reference(57));
    valgrind_clean(&src, &name, &["64"]);
}
