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

/// D296: the reads `a[p + j]` of the index `j` of a `fold` over an `iota`
/// in vector code, checked at the first and the last `j` before the loop;
/// the loop without its checks when both are in bounds, the loop as written
/// otherwise: the results and the panics of the interpreter (an index past
/// the end at the last `j` only, a negative one at the first only, an empty
/// `fold`), the check before the loop there.
#[test]
fn fold_reads_checked_before_their_loop() {
    let want = interp_ok(KMERS, &[]);
    let bin = compile(KMERS, "vector_fold_precheck");
    let ll = std::fs::read_to_string(bin.with_extension("ll")).unwrap();
    assert!(ll.contains("@llvm.vector.reduce.and.v"), "no check before the loop of the fold");
    for t in [1, 4] {
        let run = run_bin_threads(&bin, &[], Some(t));
        assert_eq!((run.code, run.stdout.as_str()), (0, want.as_str()), "{} threads: {}", t, run.stderr);
    }
    // the empty `fold`: k = 0 in the lanes
    let src = KMERS.replace("[1, 3, 6, 12][q]", "[1, 3, 6, 12][q] * (q % 2)");
    let want = interp_ok(&src, &[]);
    let bin = compile(&src, "vector_fold_precheck_empty");
    let run = run_bin_threads(&bin, &[], Some(4));
    assert_eq!((run.code, run.stdout.as_str()), (0, want.as_str()), "empty: {}", run.stderr);
    // past the end at the last `j`; before the start at the first `j`
    for (name, from, to) in [
        ("end", "count(iota(n - k + 1)", "count(iota(n - k + 2)"),
        ("start", "codes[i + j]", "codes[i + j - 1]"),
    ] {
        let src = KMERS.replace(from, to);
        let want = match interp(&src, &[]) {
            Outcome::Panic(m, _) => m,
            Outcome::Exit(c, out) => panic!("{}: no panic ({} {})", name, c, out),
        };
        let bin = compile(&src, &format!("vector_fold_precheck_{}", name));
        for t in [1, 4] {
            let run = run_bin_threads(&bin, &[], Some(t));
            assert_eq!((run.code, run.stderr.trim_end()), (101, want.as_str()), "{} {} threads", name, t);
        }
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

#[test]
fn divisions_of_64_bit_lanes_by_constants_are_exact() {
    // D192: `x / c` and `x % c` of 64-bit lanes by a constant through the
    // 52-bit products of AVX-512 IFMA when every lane is in 0 .. 2^52 / c,
    // LLVM's division otherwise (negative, large); the values of the
    // interpreter at the edges and at random
    let mut xs: Vec<i64> = vec![0, 1, -1, i64::MIN, i64::MAX, -139968, 1 << 52, (1 << 52) - 1];
    for c in [2i64, 3, 7, 61, 139968, 1 << 40] {
        let bound = (1i64 << 52) / c;
        for d in [-2i64, -1, 0, 1, 2] {
            xs.push(bound + d);
            xs.push(c * 1000 + d);
            xs.push(c + d);
        }
    }
    let mut r = Rng(5);
    for _ in 0..3000 {
        xs.push((r.next() % (1 << 40)) as i64);
    }
    for _ in 0..200 {
        xs.push(r.next() as i64);
    }
    let list = xs.iter().map(|x| format!("{}", x)).collect::<Vec<_>>().join(", ");
    let src = format!(
        "action main(args: string[k]): i32 {{\n    const x = [{list}];\n    \
         print(sum(map(x, (v) => v / 2 + v % 2)));\n    print(sum(map(x, (v) => v / 3 - v % 3)));\n    \
         print(sum(map(x, (v) => v / 7 * 5 + v % 7)));\n    print(sum(map(x, (v) => v / 61 + v % 61 * 1000)));\n    \
         print(sum(map(x, (v) => v / 139968 + v % 139968)));\n    print(sum(map(x, (v) => v / 1099511627776 + v % 1099511627776)));\n    \
         print(map(x, (v) => v % 139968).slice(0, 40));\n    return 0;\n}}\n"
    );
    let want = interp_ok(&src, &[]);
    let bin = compile(&src, "vector_ifma");
    let ll = std::fs::read_to_string(bin.with_extension("ll")).unwrap();
    let f = isc_codegen::backend::host_features();
    if ["+avx512ifma", "+avx512vl"].iter().all(|x| f.split(',').any(|y| y == *x)) {
        assert!(ll.contains("vpmadd52h"), "no product of 52 bits");
    }
    let run = run_bin(&bin, &[]);
    assert_eq!((run.code, run.stdout.as_str()), (0, want.as_str()), "{}", run.stderr);
}

#[test]
fn filters_compress_their_vectors() {
    // D195: `filter` w elements at a time, its mask counted, the kept
    // elements stored as a vector (all kept), compressed (some), or not at
    // all (none); bytes, 32 and 64 bits, integers and floats, an `iota`
    // and a predicate reading an array, one block and several, against the
    // interpreter
    let src = "action main(args: string[k]): i32 {
    const n = 5 * 4096 + 37;
    const b = map(iota(n), (i) => u8((i * 7 + i / 61) % 256));
    const x = map(iota(n), (i) => f64(i % 1000) - 500.25);
    const y = map(iota(n), (i) => i32((i * 13) % 997) - 400i32);
    const f = map(iota(n), (i) => f32(i % 77) * 0.5f32);
    const nl = b.slice(3, n - 2).filter((v) => v != 10u8);
    const few = filter(iota(n), (i) => b[i] == 62u8);
    const none = filter(iota(n), (i) => b[i] == 255u8 && i < 0);
    const all = filter(iota(n), (i) => i >= 0);
    const xs = filter(x, (v) => v > 100.5 || v < -400.0);
    const ys = filter(y, (v) => v % 3i32 == 1i32);
    const fs = filter(f, (v) => v >= 10.0f32);
    const small = filter(iota(100), (i) => i % 3 == 0);
    print([nl.length, i64(sum(map(nl, (v) => u64(v)))), few.length, few.length > 0 ? few[few.length - 1] : -1, none.length, all.length, sum(all)] as const);
    print([xs.length, sum(xs), ys.length, sum(map(ys, (v) => i64(v))), fs.length, f64(sum(fs))] as const);
    print(small);
    return 0;
}
";
    let want = interp_ok(src, &[]);
    let bin = compile(src, "vector_filters");
    let ll = std::fs::read_to_string(bin.with_extension("ll")).unwrap();
    let f = isc_codegen::backend::host_features();
    if ["+avx512f", "+avx512vl", "+avx512vbmi2", "+avx512bw"].iter().all(|x| f.split(',').any(|y| y == *x)) {
        assert!(ll.contains("masked.compressstore.v32i8") && ll.contains("masked.compressstore.v16i64"), "the filters are not compressed");
    }
    for t in [1, 4] {
        let run = run_bin_threads(&bin, &[], Some(t));
        assert_eq!((run.code, run.stdout.as_str()), (0, want.as_str()), "{} threads: {}", t, run.stderr);
    }
}

/// D214: a `map` over an index divided by a constant of at least the width
/// of its vectors runs by rows (the quotient the same for the lanes, the
/// remainder consecutive): bytes in lines of 61 read backwards (as
/// reverse-complement), 64-bit elements in rows of 17, rows cut by the
/// chunks of the threads, a range shorter than a row, an index past the end
/// (the panic of the same element as the interpreter). Any number of
/// threads.
#[test]
fn maps_by_rows_of_their_divisor() {
    let src = "function lines(m: i64, base: (i: i64) => u8): u8[k] {
    const length = m + (m + 59) / 60;
    return map(iota(length), (q) => q % 61 == 60 || q == length - 1 ? 10u8 : base(q / 61 * 60 + q % 61));
}
action main(args: string[k]): i32 {
    const n = parseI64(args[0]);
    const bases = map(iota(n), (i) => u8(65 + (i * 7 + i / 3) % 26));
    const out = lines(n, (i) => bases[n - 1 - i]);
    print(sum(map(out, (b) => i64(b))));
    print(out.slice(0, 70));
    const grid = map(iota(n), (q) => (q / 17) * 1000 + q % 17 * 3 + q);
    print(sum(grid));
    print(grid.slice(n - 5, n));
    const short = map(iota(9), (q) => q / 40 + q % 40);
    print(short);
    const k = parseI64(args[1]);
    const far = map(iota(n), (q) => bases[q / 61 * 60 + q % 61 + k]);
    print(far.length);
    return 0;
}
";
    let bin = compile(src, "by_rows");
    let ll = std::fs::read_to_string(bin.with_extension("ll")).unwrap();
    assert!(ll.contains(", i64 61)"), "the map is not by rows");
    for args in [["100000", "0"], ["7", "0"], ["250", "0"]] {
        let want = interp(src, &args);
        for t in [1, 4] {
            let r = run_bin_threads(&bin, &args, Some(t));
            match &want {
                Outcome::Exit(c, out) => assert_eq!((r.code, r.stdout.as_str()), (*c, out.as_str()), "{:?} {} threads: {}", args, t, r.stderr),
                Outcome::Panic(m, _) => {
                    assert_eq!(r.code, 101, "{:?}: {}", args, r.stderr);
                    assert!(r.stderr.contains(m.as_str()), "{:?}: {} / {}", args, r.stderr, m);
                }
            }
        }
    }
    let m = interp_panic(src, &["100000", "2000"]);
    for t in [1, 4] {
        let r = run_bin_threads(&bin, &["100000", "2000"], Some(t));
        assert_eq!(r.code, 101, "{}", r.stderr);
        assert!(r.stderr.contains(&m), "{} threads: {} / {}", t, r.stderr, m);
    }
}

/// D272: the rows wholly inside the range of a loop (all but the first and
/// the last) by vectors at constant places: rows of bytes as wide as the
/// vectors, one lane wider, twice as wide and wider still, lengths that are
/// a multiple of the row or not, elements that depend on one place of the
/// row (`q % c == k` for each k at or around the places of the vectors),
/// 64-bit elements; ranges cut by the chunks of the threads. The
/// interpreter's results, any number of threads.
#[test]
fn whole_rows_by_vectors_at_constant_places() {
    // one `map` a row width and a place, the width a literal (by rows)
    let mut body = String::new();
    for k in [0, 1, 28, 29, 30, 31, 32, 33, 60, 61, 62, 63] {
        for c in [32, 33, 64, 95] {
            body.push_str(&format!("    printBytes(map(iota(n), (q) => q % {c} == {k} ? 10u8 : u8(65 + (q / {c} * 7 + q % {c}) % 26)));\n"));
        }
    }
    let src = format!(
        "action main(args: string[k]): i32 {{
    const n = parseI64(args[0]);
{body}    const lines = map(iota(n), (q) => q % 61 == 60 ? 10u8 : u8(48 + q / 61 % 10));
    print(sum(map(lines, (b) => i64(b))));
    printBytes(lines);
    const grid = map(iota(n), (q) => (q / 17) * 1000 + q % 17 * 3 + (q % 17 == 16 ? 7 : 0));
    print(sum(grid));
    print(grid.slice(0, Math.min(n, 40)));
    return 0;
}}
"
    );
    let src = src.as_str();
    let bin = compile(src, "whole_rows");
    let ll = std::fs::read_to_string(bin.with_extension("ll")).unwrap();
    assert!(ll.contains(", i64 61)") && ll.contains(", i64 95)"), "the maps are not by rows");
    for n in ["0", "31", "32", "64", "95", "190", "1000", "6080", "100003"] {
        let want = interp(src, &[n]);
        for t in [1, 3, 4] {
            let r = run_bin_threads(&bin, &[n], Some(t));
            match &want {
                Outcome::Exit(c, out) => assert_eq!((r.code, r.stdout.as_str()), (*c, out.as_str()), "n = {} {} threads: {}", n, t, r.stderr),
                Outcome::Panic(m, _) => panic!("n = {}: the interpreter panics: {}", n, m),
            }
        }
    }
}

/// D274: a whole row of 33 to 64 bytes in one vector of 64 lanes, the
/// lanes after the row masked: a table of exactly the row read at the
/// place (the masked lanes would read past it: no panic), rows of 33, 40,
/// 61 and 64, a row whose last place reads past its table (the panic of
/// the scalar code, its element), lengths cut in the middle of a row. The
/// interpreter's results, 1 and 4 threads.
#[test]
fn rows_narrower_than_the_vector_are_masked() {
    let src = "action main(args: string[k]): i32 {
    const n = parseI64(args[0]);
    const t = parseI64(args[1]);
    const tab = map(iota(40), (i) => u8(65 + i % 26));
    printBytes(map(iota(n), (q) => tab[q % 40]));
    printBytes(map(iota(n), (q) => q % 33 == 32 ? 10u8 : u8(97 + (q / 33 + q % 33) % 26)));
    printBytes(map(iota(n), (q) => q % 61 == 60 || q == n - 1 ? 10u8 : tab[(q / 61 * 60 + q % 61) % 40]));
    printBytes(map(iota(n), (q) => u8(48 + (q / 64 * 3 + q % 64) % 10)));
    const short = map(iota(t), (i) => u8(65 + i % 26));
    printBytes(map(iota(n), (q) => short[q % 40]));
    return 0;
}
";
    let bin = compile(src, "masked_rows");
    let ll = std::fs::read_to_string(bin.with_extension("ll")).unwrap();
    let f = isc_codegen::backend::host_features();
    if ["+avx512bw", "+avx512vl"].iter().all(|x| f.split(',').any(|y| y == *x)) {
        assert!(ll.contains("masked.store.v64i8"), "the rows are not in one vector");
    }
    for args in [["0", "40"], ["40", "40"], ["1000", "40"], ["100003", "40"], ["5000", "39"]] {
        let want = interp(src, &args);
        assert_eq!(matches!(want, Outcome::Panic(..)), args[1] == "39", "{:?}", args);
        for th in [1, 4] {
            let r = run_bin_threads(&bin, &args, Some(th));
            match &want {
                Outcome::Exit(code, out) => assert_eq!((r.code, r.stdout.as_str()), (*code, out.as_str()), "{:?} {} threads: {}", args, th, r.stderr),
                Outcome::Panic(m, _) => {
                    assert_eq!(r.code, 101, "{:?}: {}", args, r.stderr);
                    let site = if th == 1 { m.as_str() } else { m.split(": the index is").next().unwrap() };
                    assert!(r.stderr.contains(site), "{:?} {} threads: {} / {}", args, th, r.stderr, m);
                }
            }
        }
    }
}

/// D273: a table read at `x % c` (c a literal from the lanes to 4096) is
/// copied before the vector loop with its period continued, every vector
/// then one load wherever its remainders start: bytes in lines written as
/// fasta writes them, `f64` and booleans, c = 4096 and 4097 (not copied),
/// under a mask, a table shorter than c (not copied: the panic of the
/// scalar code), a negative `x`. The interpreter's results, 1 and 4
/// threads.
#[test]
fn tables_read_at_a_remainder_are_copied_with_their_period() {
    let src = "action main(args: string[k]): i32 {
    const n = parseI64(args[0]);
    const k = parseI64(args[1]);
    const alu = bytes(\"GGCCGGGCGCGGTGGCTCACGCCTGTAATCCCAGCACTTTGGGAGGCCGAGGCGGGCGGATCACCTGAGGTCAGGAGTTCGAGACCAGCCTGGCCAACATGGTGAAACCCCGTCTCTACTAAAAATACAAAAATTAGCCGGGCGTGGTGGCGCGCGCCTGTAATCCCAGCTACTCGGGAGGCTGAGGCAGGAGAATCGCTTGAACCCGGGAGGCGGAGGTTGCAGTGAGCCGAGATCGCGCCACTGCACTCCAGCCTGGGCGACAGAGCGAGACTCCGTCTCAAAAA\");
    const m = n + (n + 59) / 60;
    printBytes(map(iota(m), (q) => q % 61 == 60 || q == m - 1 ? 10u8 : alu[(q / 61 * 60 + q % 61 + k) % 287]));
    const big = map(iota(5000), (i) => (i * 13) % 241);
    print(sum(map(iota(n), (i) => big[(i + k) % 4096])));
    print(sum(map(iota(n), (i) => big[(i + k) % 4097])));
    const fs = map(iota(300), (i) => f64(i) * 0.25);
    print(sum(map(iota(n), (i) => fs[(i + k) % 300])));
    const bs = map(iota(100), (i) => i % 3 == 0);
    print(sum(map(iota(n), (i) => bs[(i + k) % 100] ? 1 : 0)));
    print(sum(map(iota(n), (i) => i % 5 == 0 ? big[(i + k) % 999] : 0)));
    const short = map(iota(parseI64(args[2])), (i) => i);
    print(sum(map(iota(n), (i) => short[(i + k) % 60])));
    return 0;
}
";
    let bin = compile(src, "period_tables");
    let ll = std::fs::read_to_string(bin.with_extension("ll")).unwrap();
    // 287 bytes and 31 more for 32 lanes, 63 more where a line is one
    // vector of 64 (D274)
    assert!(ll.contains("alloca [318 x i8], align 64") || ll.contains("alloca [350 x i8], align 64"), "the bytes of alu are not copied with their period");
    let cases = [["5000", "0", "60"], ["100000", "3", "60"], ["999", "4", "60"], ["64", "4000", "60"], ["31", "0", "60"], ["5000", "0", "50"], ["999", "-7", "60"]];
    for (i, args) in cases.iter().enumerate() {
        let want = interp(src, args);
        // the last two panic: a table shorter than c, a negative `x`
        assert_eq!(matches!(want, Outcome::Panic(..)), i >= 5, "{:?}", args);
        for th in [1, 4] {
            let r = run_bin_threads(&bin, args, Some(th));
            match &want {
                Outcome::Exit(code, out) => assert_eq!((r.code, r.stdout.as_str()), (*code, out.as_str()), "{:?} {} threads: {}", args, th, r.stderr),
                Outcome::Panic(m, _) => {
                    assert_eq!(r.code, 101, "{:?}: {}", args, r.stderr);
                    let site = if th == 1 { m.as_str() } else { m.split(": the index is").next().unwrap() };
                    assert!(r.stderr.contains(site), "{:?} {} threads: {} / {}", args, th, r.stderr, m);
                }
            }
        }
    }
}

/// D217: vectors of consecutive indices compared with a value of every lane
/// (a mask of at most one bit from the first index), and their bounds
/// checked from their first and last lanes (every lane only when those
/// fail, for the message): values inside, outside and below the range,
/// indices up and down, past each end (the panic of the same element as the
/// interpreter). Any number of threads.
#[test]
fn consecutive_indices_compared_and_checked_from_their_ends() {
    let src = "action main(args: string[k]): i32 {
    const n = parseI64(args[0]);
    const c = parseI64(args[1]);
    const xs = map(iota(n), (i) => (i * 13) % 101);
    print(sum(map(iota(n), (q) => q == c ? 1 : 0)));
    print(sum(map(iota(n), (q) => q != c - 3 ? q : 0)));
    print(sum(map(iota(n), (q) => n - 1 - q == c ? 5 : 1)));
    print(sum(map(iota(n), (q) => xs[q] + xs[n - 1 - q])));
    const k = parseI64(args[2]);
    print(sum(map(iota(n), (q) => xs[q + k])));
    print(sum(map(iota(n), (q) => xs[n - 1 - q - k])));
    return 0;
}
";
    let bin = compile(src, "consecutive_checks");
    for args in [["1000", "17", "0"], ["1000", "-4", "0"], ["1000", "5000", "0"], ["40", "39", "0"], ["1000", "17", "1"], ["1000", "17", "-1"]] {
        let want = interp(src, &args);
        for t in [1, 4] {
            let r = run_bin_threads(&bin, &args, Some(t));
            match &want {
                Outcome::Exit(code, out) => assert_eq!((r.code, r.stdout.as_str()), (*code, out.as_str()), "{:?} {} threads: {}", args, t, r.stderr),
                Outcome::Panic(m, _) => {
                    assert_eq!(r.code, 101, "{:?}: {}", args, r.stderr);
                    assert!(r.stderr.contains(m.as_str()), "{:?} {} threads: {} / {}", args, t, r.stderr, m);
                }
            }
        }
    }
}

/// D226: consecutive indices under a mask (a branch of an `if`): checked
/// from their first and last lanes, as without a mask; lanes left out past
/// the end of the array (the test of the active lanes then, no panic), an
/// active lane past the end (the panic of the scalar code, its index),
/// upwards and downwards, rows of 61 (D214). The interpreter's results.
#[test]
fn consecutive_indices_under_a_mask() {
    let src = "action main(args: string[k]): i32 {
    const n = parseI64(args[0]);
    const k = parseI64(args[1]);
    const a = map(iota(n), (i) => (i * 3) % 17);
    print(sum(map(iota(n + 40), (i) => i >= n ? 0 : a[n - 1 - i])));
    print(sum(map(iota(n + 40), (i) => i % 5 == 0 || i >= n ? 1 : a[i])));
    print(sum(map(iota(n), (q) => q % 61 == 60 ? 7 : a[q / 61 * 60 + q % 61 - q / 61])));
    print(sum(map(iota(n), (i) => i % 7 == 3 ? 0 : a[i + k])));
    return 0;
}
";
    let bin = compile(src, "masked_consecutive");
    for args in [["1000", "0"], ["61", "0"], ["1000", "1"], ["1000", "-1"], ["300", "40"]] {
        let want = interp(src, &args);
        for t in [1, 4] {
            let r = run_bin_threads(&bin, &args, Some(t));
            match &want {
                Outcome::Exit(code, out) => assert_eq!((r.code, r.stdout.as_str()), (*code, out.as_str()), "{:?} {} threads: {}", args, t, r.stderr),
                Outcome::Panic(m, _) => {
                    assert_eq!(r.code, 101, "{:?}: {}", args, r.stderr);
                    // several elements fail: on several threads which one is
                    // reported is not specified (SPEC section 5), its site is
                    let site = if t == 1 { m.as_str() } else { m.split(": the index is").next().unwrap() };
                    assert!(r.stderr.contains(site), "{:?} {} threads: {} / {}", args, t, r.stderr, m);
                }
            }
        }
    }
}

/// D229: `t[x % c]` with consecutive `x` and a constant c of at least the
/// lanes: one load from the first remainder when the lanes do not cross a
/// multiple of c, the general code when they do, when `x` is negative (the
/// remainder of a negative), when c is smaller than the lanes; an index past
/// the end (the panic of the scalar code). The interpreter's results.
#[test]
fn consecutive_indices_through_a_remainder() {
    let src = "action main(args: string[k]): i32 {
    const n = parseI64(args[0]);
    const k = parseI64(args[1]);
    const t = map(iota(1000), (i) => (i * 7) % 251);
    print(sum(map(iota(n), (i) => t[(i + k) % 1000])));
    print(sum(map(iota(n), (i) => t[(i + k) % 997])));
    print(sum(map(iota(n), (i) => t[(i + k) % 5])));
    print(sum(map(iota(n), (q) => q % 61 == 60 ? 1 : t[(q / 61 * 60 + q % 61 + k) % 999])));
    print(sum(map(iota(n), (i) => t[(i + k) % 1001])));
    return 0;
}
";
    let bin = compile(src, "remainder_indices");
    for args in [["5000", "0"], ["5000", "3"], ["999", "-7"], ["64", "990"], ["5000", "-5000"]] {
        let want = interp(src, &args);
        for th in [1, 4] {
            let r = run_bin_threads(&bin, &args, Some(th));
            match &want {
                Outcome::Exit(code, out) => assert_eq!((r.code, r.stdout.as_str()), (*code, out.as_str()), "{:?} {} threads: {}", args, th, r.stderr),
                Outcome::Panic(m, _) => {
                    assert_eq!(r.code, 101, "{:?}: {}", args, r.stderr);
                    // several elements fail: on several threads which one is
                    // reported is not specified (SPEC section 5), its site is
                    let site = if th == 1 { m.as_str() } else { m.split(": the index is").next().unwrap() };
                    assert!(r.stderr.contains(site), "{:?} {} threads: {} / {}", args, th, r.stderr, m);
                }
            }
        }
    }
}

/// D318: the whole rows of a `map` by rows (one vector a row, D274) take
/// out of their loop an equality of the index with a value of before it
/// (`q == n - 1`, `q == k` for k inside, before, after the rows, at the
/// first and the last row; the rows apart, the row of k with the equality)
/// and carry from a row to the next the remainder of the first lane of a
/// table read at `x % c` (`x` linear in the row and the place: steps 60, 7
/// and 1000 with c = 997, a table longer than a copy with its period
/// (139968, D273), a first lane below 0 (the general code and its panic),
/// a table shorter than c (its panic)), as the interpreter on 1, 3 and 4
/// threads.
#[test]
fn rows_take_their_equalities_and_remainders_out_of_their_loop() {
    let src = "action main(args: string[k]): i32 {
    const n = parseI64(args[0]);
    const k = parseI64(args[1]);
    const t = parseI64(args[2]);
    const big = map(iota(139968), (i) => u8(65 + (i * 7) % 26));
    const tab = map(iota(t), (i) => u8(97 + i % 26));
    printBytes(map(iota(n), (q) => q % 61 == 60 || q == n - 1 ? 10u8 : big[(1 + 3 * n + q / 61 * 60 + q % 61) % 139968]));
    printBytes(map(iota(n), (q) => q % 61 == 60 || q == k ? 10u8 : tab[(q / 61 * 7 + q % 61 + k + 1) % 997]));
    printBytes(map(iota(n), (q) => q == k + 5 ? 33u8 : tab[(q / 61 * 1000 + q % 61) % 997]));
    printBytes(map(iota(n), (q) => q % 61 == 60 ? 10u8 : tab[(q / 61 * 60 + q % 61 + 300000 - k) % 997]));
    return 0;
}
";
    let bin = compile(src, "rows_hoisted");
    let cases: [[&str; 3]; 9] = [
        ["100003", "70000", "997"],
        ["100003", "-1", "997"],
        ["100003", "200000", "997"],
        ["100003", "0", "997"],
        ["100003", "100002", "997"],
        ["61", "30", "997"],
        ["1000", "500", "997"],
        ["5000", "100", "996"],
        ["100003", "400000", "997"],
    ];
    for args in cases {
        let want = interp(src, &args);
        assert_eq!(matches!(want, Outcome::Panic(..)), args[2] == "996" || args[1] == "400000", "{:?}", args);
        for th in [1, 3, 4] {
            let r = run_bin_threads(&bin, &args, Some(th));
            match &want {
                Outcome::Exit(code, out) => assert_eq!((r.code, r.stdout.as_str()), (*code, out.as_str()), "{:?} {} threads: {}", args, th, r.stderr),
                Outcome::Panic(m, _) => {
                    assert_eq!(r.code, 101, "{:?}: {}", args, r.stderr);
                    let site = if th == 1 { m.as_str() } else { m.split(": the index is").next().unwrap() };
                    assert!(r.stderr.contains(site), "{:?} {} threads: {} / {}", args, th, r.stderr, m);
                }
            }
        }
    }
}
