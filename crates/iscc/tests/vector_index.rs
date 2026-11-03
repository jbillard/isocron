//! Indexing in vector code (D181): a lambda that reads an array of its
//! context by an index is vectorized (a load of consecutive elements, or a
//! gather), with the results and the panics of the interpreter.

mod common;
use common::*;

const OK: &str = "function table(): i32[256] {
    return map(iota(256), (i) => i32(i * 37 % 256) - 100i32);
}
action main(args: string[k]): i32 {
    const n = 3 * 4096 + 13;
    const u = map(iota(n), (i) => 1.0 / f64(i + 1));
    const s = map(iota(n), (i) => f32(i % 97) * 0.25f32);
    const q = map(iota(n), (i) => i * i - 5000);
    // consecutive elements, in a reduction and in a map
    print(sum(map(iota(n), (j) => f64(j) * u[j])));
    const m = map(iota(n), (j) => u[j] * 3.0 + 1.0);
    print(sum(m));
    // a gather (an index that cannot panic), floats of 32 bits, integers
    print(sum(map(iota(n), (j) => u[j * 7 % 1000])));
    print(sum(map(iota(n), (j) => s[(j * 13 + 5) % 4096])));
    print(sum(map(iota(n), (j) => q[n - 1 - j])));
    // a table indexed by bytes
    const t = table();
    const b = map(iota(n), (i) => u8(i * 11 % 256));
    print(sum(map(b, (x) => i64(t[x]))));
    // small arrays: fewer elements than a vector
    const w = [2.0, 3.0, 5.0];
    print(sum(map(iota(3), (j) => w[j] * f64(j))));
    return 0;
}
";

#[test]
fn indexed_lambdas_are_vectorized_with_the_results_of_the_interpreter() {
    let want = interp_ok(OK, &[]);
    let bin = compile(OK, "vector_index_ok");
    let ll = std::fs::read_to_string(bin.with_extension("ll")).unwrap();
    assert!(ll.contains("@llvm.masked.gather.v"), "no gather");
    assert!(ll.contains("icmp uge <"), "no vector check of the bounds");
    for t in [1, 3, 4] {
        let run = run_bin_threads(&bin, &[], Some(t));
        assert_eq!((run.code, run.stdout.as_str()), (0, want.as_str()), "{} threads: {}", t, run.stderr);
    }
}

#[test]
fn indexes_out_of_bounds_in_vector_code_panic_as_the_interpreter() {
    // the first element out of bounds, whatever its lane: consecutive
    // indices, a gather, a negative signed index, a map and a reduction
    let cases = [
        ("u[j + 5]", "f64[k]", "map(iota(n), (j) => u[j + 5])"),
        ("gather", "f64", "sum(map(iota(n), (j) => u[(j * 3) % 9000 + 4000]))"),
        ("negative", "f64", "sum(map(iota(n), (j) => u[i32(j % 50) - 37i32]))"),
        ("reduce", "f64", "sum(map(iota(n + 2), (j) => u[j]))"),
    ];
    for (name, ty, expr) in cases {
        let src = format!(
            "function f(u: f64[n]): {ty} {{\n    return {expr};\n}}\n\
             action main(args: string[k]): i32 {{\n    \
             const u = map(iota(9001), (i) => f64(i));\n    \
             const r = f(u);\n    \
             print(r);\n    \
             return 0;\n}}\n"
        );
        let want = match interp(&src, &[]) {
            Outcome::Panic(m, _) => m,
            Outcome::Exit(c, out) => panic!("{}: the interpreter did not panic ({} {})", name, c, out),
        };
        let bin = compile(&src, &format!("vector_index_panic_{}", name.replace(['[', ']', ' ', '+'], "")));
        // one thread: the panic of the interpreter; more: another block may
        // fail first (SPEC section 5: which panic is reported is not
        // specified), at the same place
        let place = &want[..want.find("the index is").unwrap()];
        for t in [1, 4] {
            let run = run_bin_threads(&bin, &[], Some(t));
            assert_eq!(run.code, 101, "{} T={}: {}", name, t, run.stdout);
            if t == 1 {
                assert_eq!(run.stderr.trim_end(), want, "{} T={}", name, t);
            } else {
                assert!(run.stderr.starts_with(place) && run.stderr.trim_end().ends_with("but the length is 9001"), "{} T={}: {}", name, t, run.stderr);
            }
        }
    }
}

const KMERS: &str = "function kmer(codes: i64[n], i: i64, k: i64): i64 {
    return fold(iota(k), 0, (c, j) => c * 4 + codes[i + j]);
}
function occurrences(codes: i64[n], k: i64, target: i64): i64 {
    return count(iota(n - k + 1), (i) => kmer(codes, i, k) == target);
}
action main(args: string[k]): i32 {
    const n = 5 * 4096 + 11;
    const codes = map(iota(n), (i) => (i * 7 + i / 13) % 4);
    print(map(iota(4), (q) => occurrences(codes, [1, 3, 6, 12][q], kmer(codes, 100 * q, [1, 3, 6, 12][q]))));
    print(sum(map(iota(n - 2), (i) => codes[i + 2] - codes[i])));
    return 0;
}
";

#[test]
fn folds_over_a_size_of_every_lane_and_shifted_indices_are_vector_code() {
    // a `fold` over `iota(k)`, its size checked once for all the lanes, and
    // indices `i + j` (consecutive plus a value of every lane): vector code
    // whose loads are of consecutive elements (D184)
    let want = interp_ok(KMERS, &[]);
    let bin = compile(KMERS, "vector_kmers");
    let ll = std::fs::read_to_string(bin.with_extension("ll")).unwrap();
    assert!(!ll.contains("@llvm.masked.gather"), "a gather where the indices are consecutive");
    assert!(ll.contains("icmp uge <"), "the k-mers are not vector code");
    for t in [1, 4] {
        let run = run_bin_threads(&bin, &[], Some(t));
        assert_eq!((run.code, run.stdout.as_str()), (0, want.as_str()), "{} threads: {}", t, run.stderr);
    }
    // the panics of the interpreter: a negative size, an index past the end
    for (name, from, to) in [
        ("negsize", "occurrences(codes, [1, 3, 6, 12][q]", "occurrences(codes, [1, 3, 6, 12][q] - 7"),
        ("bounds", "codes[i + 2] - codes[i]", "codes[i + 3] - 1"),
    ] {
        let src = KMERS.replace(from, to);
        let want = match interp(&src, &[]) {
            Outcome::Panic(m, _) => m,
            Outcome::Exit(c, out) => panic!("{}: no panic ({} {})", name, c, out),
        };
        let bin = compile(&src, &format!("vector_kmers_{}", name));
        let run = run_bin_threads(&bin, &[], Some(1));
        assert_eq!((run.code, run.stderr.trim_end()), (101, want.as_str()), "{}", name);
    }
}

const MASKED: &str = "function pick(p: f64[n], codes: u8[n], r: f64): u8 {
    return codes[Math.min(count(p, (c) => c <= r), n - 1)];
}
action main(args: string[k]): i32 {
    const n = 2 * 4096 + 21;
    const u = map(iota(n), (i) => f64(i) * 0.5);
    const p = [0.1, 0.35, 0.6, 0.9];
    const codes = [65u8, 67u8, 71u8, 84u8];
    // indices past the end in the lanes a branch does not take: neither a
    // panic nor a read outside the array (consecutive, then a gather)
    print(sum(map(iota(n), (i) => i < 10 ? u[i + n - 10] : -1.0)));
    print(sum(map(iota(n), (i) => i < 10 ? u[i * 7 + n - 70] : -1.0)));
    // a size of every lane checked in a branch no lane takes
    const one = filter(codes, (c) => c == 65u8);
    print(sum(map(iota(n), (i) => i < 0 ? f64(pick(p, one, 0.5)) : 2.0)));
    // a count of an array of the context in each lane, a branch with a
    // gather and an index computed from another index
    const t = map(iota(n), (i) => i % 61 == 60 ? 10u8 : pick(p, codes, f64(i % 1000) / 1000.0));
    print(sum(map(t, (x) => u64(x))));
    const table = map(iota(256), (b) => u8(255 - b));
    print(sum(map(iota(n), (i) => i % 7 == 0 ? 0u64 : u64(table[i64(t[n - 1 - i])]))));
    // an order that is not free: the fixed form in each lane (not vector)
    print(sum(map(iota(100), (i) => reduce(p, (a, b) => a + b, f64(i)))));
    return 0;
}
";

#[test]
fn masked_branches_and_replayed_checks_give_the_interpreter() {
    // `if` with branches that index arrays, under the mask of their lanes,
    // and checks that fail replayed one element at a time (D185)
    let want = interp_ok(MASKED, &[]);
    let bin = compile(MASKED, "vector_masked");
    let ll = std::fs::read_to_string(bin.with_extension("ll")).unwrap();
    assert!(ll.contains("@llvm.masked.load.v"), "no masked load");
    for t in [1, 4] {
        let run = run_bin_threads(&bin, &[], Some(t));
        assert_eq!((run.code, run.stdout.as_str()), (0, want.as_str()), "{} threads: {}", t, run.stderr);
    }
    // two sites that fail in the same vector: the scalar order decides
    // (element 4 reads b[4] before element 5 reads a[10]); a site under a
    // mask; the size of `pick` checked in a branch some lanes take
    for (name, src) in [
        ("two_sites", "action main(args: string[k]): i32 {\n    const a = map(iota(9), (i) => f64(i));\n    const b = map(iota(4), (i) => f64(i));\n    print(sum(map(iota(40), (j) => a[j * 2] + b[j])));\n    return 0;\n}\n"),
        ("masked_site", "action main(args: string[k]): i32 {\n    const a = map(iota(50), (i) => f64(i));\n    print(sum(map(iota(4100), (j) => j % 5 == 3 ? a[j / 5] : a[j % 50 + 1])));\n    return 0;\n}\n"),
        ("size_in_branch", &MASKED.replace("i < 0 ? f64(pick(p, one, 0.5))", "i > 77 ? f64(pick(p, one, 0.5))")),
    ] {
        let want = match interp(src, &[]) {
            Outcome::Panic(m, _) => m,
            Outcome::Exit(c, out) => panic!("{}: no panic ({} {})", name, c, out),
        };
        let bin = compile(src, &format!("vector_masked_{}", name));
        let run = run_bin_threads(&bin, &[], Some(1));
        assert_eq!((run.code, run.stderr.trim_end()), (101, want.as_str()), "{}", name);
    }
}

#[test]
fn tables_of_bytes_are_permuted_when_small_and_read_one_by_one_otherwise() {
    // an index into an array of bytes that is not consecutive (D185): with
    // AVX-512 VBMI, permutations in registers when the array has at most
    // 256 bytes (masked loads: nothing read past its end), the scalar loop
    // when it is larger; the values of the interpreter in every case
    let src = "action main(args: string[k]): i32 {
    const n = 3 * 4096 + 29;
    const t256 = map(iota(256), (b) => u8((b * 7 + 3) % 256));
    const t4 = [11u8, 22u8, 33u8, 44u8];
    const t300 = map(iota(300), (b) => u8(b % 251));
    const k = map(iota(n), (i) => (i * 31 + i / 7) % 256);
    // materialized (each used twice): the loops of `fill_range`
    const a = map(iota(n), (i) => i % 9 == 8 ? 0u8 : t256[k[i]]);
    const b = map(iota(n), (i) => t4[k[i] % 4]);
    const c = map(iota(n), (i) => t300[(k[i] * 3) % 300]);
    print([a[100], b[100], c[100]] as const);
    print(sum(map(a, (x) => u64(x))));
    print(sum(map(b, (x) => u64(x))));
    print(sum(map(c, (x) => u64(x))));
    return 0;
}
";
    let want = interp_ok(src, &[]);
    let bin = compile(src, "vector_bytes");
    let ll = std::fs::read_to_string(bin.with_extension("ll")).unwrap();
    if isc_codegen::backend::host_features().split(',').any(|f| f == "+avx512vbmi") {
        assert!(ll.contains("vpermi2var.qi.256"), "no permutation of bytes");
    }
    for t in [1, 4] {
        let run = run_bin_threads(&bin, &[], Some(t));
        assert_eq!((run.code, run.stdout.as_str()), (0, want.as_str()), "{} threads: {}", t, run.stderr);
    }
}

#[test]
fn divisions_of_the_index_are_carried_by_the_vector_loop() {
    // `q / c` and `q % c` of the index of a vector loop (D186): divisors
    // larger and smaller than the lanes, 1 and 16, from the starts of the
    // blocks (not multiples of them), the values of the interpreter
    let src = r#"action main(args: string[k]): i32 {
    const n = 3 * 4096 + 77;
    const a = map(iota(n), (q) => q / 61 * 1000 + q % 61 + q % 7 * 100000 + q / 7 + q % 1 + q / 16 * 3 + q % 16);
    print([a[0], a[60], a[61], a[4095], a[4096], a[n - 1]] as const);
    print(sum(a));
    const b = map(iota(n), (q) => q % 61 == 60 ? 10u8 : u8(q / 61 % 200));
    print(sum(map(b, (x) => u64(x))));
    print(b[n - 1]);
    return 0;
}
"#;
    let want = interp_ok(src, &[]);
    let bin = compile(src, "vector_divmod");
    let ll = std::fs::read_to_string(bin.with_extension("ll")).unwrap();
    assert!(ll.contains("icmp sge <"), "the remainders are not carried by the vector loop");
    for t in [1, 3, 4] {
        let run = run_bin_threads(&bin, &[], Some(t));
        assert_eq!((run.code, run.stdout.as_str()), (0, want.as_str()), "{} threads: {}", t, run.stderr);
    }
}

#[test]
fn bytes_close_together_are_read_through_a_window() {
    // an index of an array of more than 256 bytes computed from the index
    // of the loop alone (D187): a window of 64 bytes and a permutation when
    // the lanes are close (downwards with breaks, as lines of 60, up to the
    // last byte of the array), a gather when they are far apart; under a
    // mask; the values of the interpreter
    let src = "action main(args: string[k]): i32 {
    const m = 5 * 4096 + 33;
    const bases = map(iota(m), (i) => u8(65 + (i * 7 + i / 3) % 26));
    const length = m + (m + 59) / 60;
    const lines = map(iota(length), (q) => q % 61 == 60 || q == length - 1 ? 10u8 : bases[m - 1 - (q / 61 * 60 + q % 61)]);
    const far = map(iota(m / 80), (q) => bases[q * 80 % m]);
    const tail = map(iota(1000), (q) => bases[m - 1 - q % 50]);
    print([lines[0], lines[60], lines[length - 1], far[3], tail[999]] as const);
    print(sum(map(lines, (x) => u64(x))));
    print(sum(map(far, (x) => u64(x))));
    print(sum(map(tail, (x) => u64(x))));
    return 0;
}
";
    let want = interp_ok(src, &[]);
    let bin = compile(src, "vector_window");
    let ll = std::fs::read_to_string(bin.with_extension("ll")).unwrap();
    if isc_codegen::backend::host_features().split(',').any(|f| f == "+avx512vbmi") {
        assert!(ll.contains("llvm.vector.reduce.umin"), "no window of bytes");
    }
    for t in [1, 4] {
        let run = run_bin_threads(&bin, &[], Some(t));
        assert_eq!((run.code, run.stdout.as_str()), (0, want.as_str()), "{} threads: {}", t, run.stderr);
    }
}
