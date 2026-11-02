//! `extern` (SPEC section 12, DECISIONS D125): C functions called from an
//! action, compiled and interpreted, with the same results: scalars of every
//! class and width, `string` (a NUL-terminated copy), arrays of scalars
//! (a pointer and a length), functions of libc, of libm and of a library
//! built here; the errors found by the type checker.

mod common;
use common::*;

/// The library of `tests/data/extern_helper.c`, built once per run.
fn helper() -> String {
    let lib = tmp("libisc_extern_helper.so");
    let src = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/extern_helper.c");
    let st = std::process::Command::new(std::env::var("ISC_CC").unwrap_or_else(|_| "cc".into()))
        .args(["-O2", "-shared", "-fPIC", "-o"])
        .arg(&lib)
        .arg(src)
        .status()
        .expect("cannot run the C compiler");
    assert!(st.success());
    let path = lib.to_string_lossy().to_string();
    isc_interp::ffi::load_library(&path).unwrap();
    path
}

const PROGRAM: &str = "declare function abs(v: i32): i32;

declare function labs(v: i64): i64;

declare function strlen(s: string): u64;

declare function atoi(s: string): i32;

declare function cbrt(v: f64): f64;

declare function fabsf(v: f32): f32;

declare function toupper(c: i32): i32;

declare function sum_f64(x: f64[n]): f64;

declare function sum_f32(x: f32[n]): f32;

declare function sum_i32(x: i32[n]): i64;

declare function count_true(x: boolean[n]): i64;

declare function neg8(v: i8): i8;

declare function twice16(v: u16): u16;

declare function is_even(v: i64): boolean;

declare function mix(a: i64, b: f64, c: i32, d: f32, e: boolean): f64;

action main(args: string[k]): i32 {
    print(abs(-7i32));
    print(labs(-1234567890123));
    print(strlen(args[0]));
    print(atoi(args[1]));
    print(cbrt(27.0));
    print(fabsf(-2.5f32));
    print(toupper(97i32));
    const x = map(iota(10), (i) => f64(i) * 0.5);
    print(sum_f64(x));
    print(sum_f64(x.slice(2, 6)));
    print(sum_f32(map(x, (v) => f32(v))));
    print(sum_i32(map(iota(5), (i) => i32(i) - 2i32)));
    print(count_true(map(iota(7), (i) => i % 3 == 0)));
    print(neg8(5i8));
    print(twice16(40000u16));
    print(is_even(10));
    print(mix(1, 2.0, 3i32, 0.5f32, true));
    return 0;
}
";

#[test]
fn extern_functions_compiled_and_interpreted() {
    let lib = helper();
    let printed = interp_ok(PROGRAM, &["hello", "42"]);
    let bin = compile_linked(PROGRAM, "extern_calls", isc_codegen::Options::default(), &[&lib]);
    let r = run_bin(&bin, &["hello", "42"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(r.stdout, printed);
    // glibc's `cbrt` everywhere (not a copy from the runtime's archive, not
    // computed by LLVM): the same last bit
    assert_eq!(printed.lines().nth(4), Some("3.0000000000000004"));
    assert_eq!(printed.lines().nth(13), Some("14464"));
}

#[test]
fn a_string_with_a_nul_byte_is_a_panic() {
    let src = "declare function strlen(s: string): u64;

action main(args: string[k]): i32 {
    print(strlen(args[0]));
    return 0;
}
";
    let want = "a string passed to the extern function `strlen` contains a NUL byte";
    // an argument cannot hold a NUL byte: the interpreter is given one
    let p = iscc::frontend("nul.isc", src).unwrap();
    let mut out = Vec::new();
    let e = isc_interp::run(&p, &["a\0b".to_string()], &mut out).unwrap_err();
    assert!(e.to_string().contains(want), "{}", e);
    assert!(interp_ok(src, &["abc"]).starts_with('3'));
}

#[test]
fn extern_errors() {
    for (src, want) in [
        (
            "declare function abs(v: i32): i32;\n\nfunction f(x: i32): i32 {\n    return abs(x);\n}\n\naction main(args: string[k]): i32 {\n    print(f(1i32));\n    return 0;\n}\n",
            "a `function` cannot call the extern function `abs` (SPEC section 12)",
        ),
        (
            "declare function abs(v: i32): i32;\n\naction main(args: string[k]): i32 {\n    print(map([1i32], abs));\n    return 0;\n}\n",
            "the extern function `abs` cannot be used as a value",
        ),
        (
            "declare function f(v: [i64, i64]): i64;\n\naction main(args: string[k]): i32 {\n    return 0;\n}\n",
            "the parameter `v` of an extern function has the type `[i64, i64]`",
        ),
        (
            "declare function f(v: i64): f64[n];\n\naction main(args: string[k]): i32 {\n    return 0;\n}\n",
            "an extern function returns a scalar or `void`",
        ),
        (
            "declare function f(a: f64[n], b: f64[m], c: f64[k], d: i64): i64;\n\naction main(args: string[k]): i32 {\n    return 0;\n}\n",
            "takes too many parameters",
        ),
        (
            "declare function isc_alloc(n: i64): i64;\n\naction main(args: string[k]): i32 {\n    return 0;\n}\n",
            "cannot be the name of an extern function",
        ),
        (
            "declare function abs(v: i32): i32;\n\ndeclare function abs(v: i32): i32;\n\naction main(args: string[k]): i32 {\n    return 0;\n}\n",
            "duplicate declaration of `abs`",
        ),
    ] {
        let e = iscc::frontend("e.isc", src).unwrap_err();
        assert!(e.contains(want), "wanted `{}` in:\n{}", want, e);
    }
}

#[test]
fn a_missing_extern_function_is_reported() {
    let src = "declare function isocron_no_such_function(v: i64): i64;\n\naction main(args: string[k]): i32 {\n    print(isocron_no_such_function(1));\n    return 0;\n}\n";
    assert!(interp_panic(src, &[]).contains("the extern function `isocron_no_such_function` is not found"));
}
