//! The standard library of v1 (D155): the functions of `Math`, seeded
//! random numbers, sorting. The interpreter and the compiled program print
//! the same text, with 1 and 4 threads, and free everything (Valgrind).

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
        .env("ISC_THREADS", "2")
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "{} under valgrind:\n{}", name, err);
}

fn agree(src: &str, name: &str, args: &[&str], expected: &str) {
    assert_eq!(interp_ok(src, args), expected, "interpreter");
    let bin = compile(src, name);
    for t in [1, 4] {
        let r = run_bin_threads(&bin, args, Some(t));
        assert_eq!(r.code, 0, "{}", r.stderr);
        assert_eq!(r.stdout, expected, "compiled, {} threads", t);
    }
    valgrind_clean(src, name, args);
}

const MATH: &str = r#"action main(args: string[k]): i32 {
    print([Math.round(2.5), Math.round(-2.5), Math.round(-0.25), Math.round(0.49999999999999994)]);
    print([Math.floor(-1.5), Math.ceil(-1.5), Math.trunc(-1.5), Math.abs(-3.5)]);
    print([Math.abs(-7), Math.abs(-9223372036854775808), abs(-3i8), abs(250u8)] as const);
    print([Math.tan(1.0), Math.atan(1.0) * 4.0, Math.atan2(1.0, -1.0), Math.asin(1.0), Math.acos(-1.0)]);
    print(map([-1.5f32, 2.5f32], Math.round));
    print([-1.0, 2.0].map(abs));
    return 0;
}
"#;

/// `Math.round` as TypeScript (the ECMAScript standard: a half toward
/// +inf, -0 kept), `floor`, `ceil`, `trunc`, `abs` of integers (modulo
/// 2^bits) and floats, `tan`, `atan`, `atan2`, `asin`, `acos`.
#[test]
fn functions_of_math() {
    agree(MATH, "std_math", &[], "[3, -2, -0, 0]\n[-2, -1, -1, 3.5]\n(7, -9223372036854775808, 3, 250)\n[1.5574077246549023, 3.141592653589793, 2.356194490192345, 1.5707963267948966, 3.141592653589793]\n[-1, 3]\n[1, 2]\n");
}

const SORT: &str = r#"action main(args: string[k]): i32 {
    print(map(iota(3), (i) => random(42, i)));
    print(map(iota(8), (i) => randomInt(7, i, 10)));
    print([3, 1, 2].toReversed());
    print(sort([5, -1, 3, 3, -7]));
    print(sort([2.5, -0.0, 0.0, -1.0, 0.0 / 0.0, -3.5]));
    print(sort(["pomme", "kiwi", "Abricot", "kiwi", ""]));
    print(sort([200u8, 3u8, 255u8]) == [3u8, 200u8, 255u8]);
    print(sort([18446744073709551615u64, 1u64]));
    print(sort([true, false, true]));
    const words = ["ccc", "a", "bb", "dd", "e"];
    print(words.sortBy((w) => w.length));
    print(words.sortBy((w) => -w.length));
    const xs = map(iota(1000), (i) => random(1, i));
    const s = sort(xs);
    print(fold(iota(999), true, (ok, i) => ok && s[i] <= s[i + 1]));
    print(sum(s) == sum(xs));
    return 0;
}
"#;

/// SplitMix64 on a counter, written independently.
fn reference_random(seed: i64, i: i64) -> f64 {
    fn mix(z: u64) -> u64 {
        let z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        let z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        z ^ (z >> 31)
    }
    let x = mix(mix(seed as u64).wrapping_add((i as u64 + 1).wrapping_mul(0x9e3779b97f4a7c15)));
    (x >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
}

/// Seeded random numbers and stable sorting (D155).
#[test]
fn random_numbers_and_sorting() {
    let expected = "[0.5961188718302076, 0.1603653875985772, 0.16639780398145976]\n[5, 3, 9, 8, 6, 3, 3, 6]\n[2, 1, 3]\n[-7, -1, 3, 3, 5]\n[-3.5, -1, -0, 0, 2.5, NaN]\n[, Abricot, kiwi, kiwi, pomme]\ntrue\n[1, 18446744073709551615]\n[false, true, true]\n[a, e, bb, dd, ccc]\n[ccc, bb, dd, a, e]\ntrue\ntrue\n";
    agree(SORT, "std_sort", &[], expected);
    let first = format!("[{}, {}, {}]", reference_random(42, 0), reference_random(42, 1), reference_random(42, 2));
    assert_eq!(expected.lines().next().unwrap(), first);
}

const LENGTHS: &str = r#"action main(args: string[k]): i32 {
    const n = parseI64(args[0]);
    print(sum(iota(n).map((i) => i * i)));
    print(fill(n, "ab"));
    const m = 3;
    print(iota(m));
    print(iota(args.length + 1));
    return 0;
}
"#;

/// `iota(n)` and `fill(n, v)` of any `i64` (D155): a size known at run
/// time, a panic when it is negative.
#[test]
fn lengths_known_at_run_time() {
    agree(LENGTHS, "std_lengths", &["4"], "14\n[ab, ab, ab, ab]\n[0, 1, 2]\n[0, 1]\n");
    let want = "panic at line 3, column 19: negative size: -2";
    assert_eq!(interp_panic(LENGTHS, &["-2"]), want);
    let bin = compile(LENGTHS, "std_lengths_neg");
    let r = run_bin(&bin, &["-2"]);
    assert_eq!(r.code, 101);
    assert!(r.stderr.contains(want), "{}", r.stderr);
}
