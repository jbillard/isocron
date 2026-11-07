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
    let bin = compile_glibc(src, &format!("{}_vg", name), opts);
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

const CONCAT_COPIED: &str = r#"// each array passed out of line (a recursion): its concat materialized
function keep(x: u8[n], k: i64): u8[n] {
    return k == 0 ? x : keep(x, k - 1);
}

function keepF(x: f32[n], k: i64): f32[n] {
    return k == 0 ? x : keepF(x, k - 1);
}

function keepB(x: boolean[n], k: i64): boolean[n] {
    return k == 0 ? x : keepB(x, k - 1);
}

function keepT(x: [i64, f64][n], k: i64): [i64, f64][n] {
    return k == 0 ? x : keepT(x, k - 1);
}

action main(args: string[k]): i32 {
    const b = bytes("hello, world");
    printBytes(concat(keep(concat(b.slice(7, 12), b.slice(0, 5)), 1), [10u8]));
    print(keepF(concat(map(iota(3), (i) => f32(i) * 0.5f32), [f32(9.5)]), 1));
    print(keepB(concat([true, false], [false]), 1));
    print(keepT(concat([[1, 1.5] as const], map(iota(2), (i) => [i, f64(i) / 4.0] as const)), 1));
    print([keep(concat(b.slice(3, 3), b.slice(0, 2)), 1).length, keep(concat(b, b.slice(0, 0)), 1).length, keep(concat(b.slice(0, 0), b.slice(0, 0)), 1).length] as const);
    const big = keep(concat(map(iota(100000), (i) => u8(i % 251)), map(iota(70000), (i) => u8(i % 13))), 1);
    print(`${big.length} ${sum(map(big, (x) => i64(x)))}`);
    return 0;
}
"#;

/// D340: `concat` of arrays of scalars and of tuples of scalars, made into
/// memory: each side copied (slices of bytes, floats of 32 bits, booleans,
/// columns of tuples, sides empty, 170000 bytes on 1 and 4 threads); the
/// same outputs compiled and interpreted, Valgrind.
#[test]
fn concatenations_made_by_copies() {
    agree(
        CONCAT_COPIED,
        "concat_copied",
        &[],
        "worldhello\n[0, 0.5, 1, 9.5]\n[true, false, false]\n[(1, 1.5), (0, 0), (1, 0.25)]\n(2, 12, 0)\n170000 12912381\n",
    );
    let bin = compile(CONCAT_COPIED, "concat_copied_4");
    let r = run_bin_threads(&bin, &[], Some(4));
    assert_eq!(r.stdout, interp_ok(CONCAT_COPIED, &[]), "4 threads");
}

/// D340: a `map` of the form of `concat` written by hand, its lengths not
/// those of a `concat`: the loop of the `map` and its checks, the same panic
/// as the interpreter.
#[test]
fn a_map_of_the_form_of_concat_with_other_lengths() {
    let src = "function bad(a: i64[n], b: i64[m]): i64[k] {\n    return map(iota(a.length + b.length + 1), (i) => i < a.length ? a[i] : b[i - a.length]);\n}\n\nfunction keep(x: i64[n], k: i64): i64[n] {\n    return k == 0 ? x : keep(x, k - 1);\n}\n\naction main(args: string[k]): i32 {\n    print(keep(bad(iota(3), iota(parseI64(args[0]))), 1));\n    return 0;\n}\n";
    let want = interp_panic(src, &["2"]);
    assert!(want.contains("out of bounds"), "{}", want);
    let bin = compile(src, "concat_form_bad");
    let r = run_bin(&bin, &["2"]);
    assert_eq!(r.code, 101, "{}", r.stderr);
    assert_eq!(r.stderr.trim(), want.trim());
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

/// D225: `printBytes(map(...))` of bytes made only to be written: made and
/// written by blocks of 1 MiB in one buffer, never whole. Lengths around the
/// blocks (none, one byte, one block less one, one, one more, several),
/// rows of 61 (the code by rows of D214), between `print`s (the order of
/// the output), on 1 and 4 threads, against the bytes computed here;
/// nothing lost or freed twice (valgrind).
#[test]
fn bytes_written_by_blocks() {
    let src = "action main(args: string[k]): i32 {
    const n = parseI64(args[0]);
    const t = bytes(\"ACGT\");
    print(\"before\");
    printBytes(map(iota(n), (q) => q % 61 == 60 || q == n - 1 ? 10u8 : t[(q / 61 * 60 + q % 61) % 4]));
    printBytes(map(iota(n % 1000), (q) => u8(65 + q % 26)));
    print(\"after\");
    return 0;
}
";
    let want = |n: usize| -> Vec<u8> {
        let t = b"ACGT";
        let mut out = b"before\n".to_vec();
        out.extend((0..n).map(|q| if q % 61 == 60 || q + 1 == n { 10u8 } else { t[(q / 61 * 60 + q % 61) % 4] }));
        out.extend((0..n % 1000).map(|q| 65 + (q % 26) as u8));
        out.extend_from_slice(b"after\n");
        out
    };
    let bin = compile(src, "bytes_by_blocks");
    for n in [0usize, 1, (1 << 20) - 1, 1 << 20, (1 << 20) + 1, (3 << 20) + 61 * 7 + 5] {
        for t in ["1", "4"] {
            let o = std::process::Command::new(&bin).arg(n.to_string()).env("ISC_THREADS", t).output().unwrap();
            assert_eq!(o.status.code(), Some(0), "n {}: {}", n, String::from_utf8_lossy(&o.stderr));
            assert!(o.stdout == want(n), "n {}, {} threads: {} bytes, {} expected", n, t, o.stdout.len(), want(n).len());
        }
    }
    valgrind_clean(src, "bytes_by_blocks", &["1048639"]);
}

/// D234: the bytes written by blocks (D225) through the choice of a table
/// (D230: `if ... then (let H = ... in map) else map`), the test of the
/// length and the `let`s before the `map`: the branch with the table by
/// blocks, the other made whole; the same bytes as the interpreter, above
/// and below the size of the table, 1 and 4 threads; the code by blocks is
/// there (a buffer of 1 MiB).
#[test]
fn bytes_written_by_blocks_through_a_table() {
    let src = "const P = [0.27, 0.12, 0.12, 0.27, 0.22];
action main(args: string[k]): i32 {
    const n = parseI64(args[0]);
    const values = map(iota(1000), (i) => (i * 7919) % 1000);
    const cp = map(iota(5), (i) => fold(iota(i + 1), 0.0, (c, j) => c + P[j]));
    const codes = bytes(\"acgtN\");
    const m = n * 3;
    const length = m + (m + 59) / 60;
    printBytes(map(iota(length), (q) => q % 61 == 60 || q == length - 1 ? 10u8 : codes[Math.min(count(cp, (c) => c <= f64(values[(q / 61 * 60 + q % 61) % 1000]) / 1000.0), 4)]));
    print(\"end\");
    return 0;
}
";
    let bin = compile(src, "blocks_table");
    for n in ["0", "100", "700", "400000"] {
        let want = interp_ok(src, &[n]);
        for t in [1, 4] {
            let r = run_bin_threads(&bin, &[n], Some(t));
            assert_eq!((r.code, r.stdout.as_str()), (0, want.as_str()), "n {} {} threads: {}", n, t, r.stderr);
        }
    }
    let p = iscc::frontend("blocks_table.isc", src).unwrap();
    let ll = tmp("blocks_table.ll");
    iscc::build(&p, &iscc::BuildOptions { output: tmp("blocks_table_ll"), emit_llvm: Some(ll.clone()), ..Default::default() }).unwrap();
    let text = std::fs::read_to_string(&ll).unwrap();
    assert!(text.contains("i64 1048576"), "no buffer of 1 MiB: the bytes are not written by blocks");
}

/// D426: the counts of the predicate of a windowed `filter` (D293) over the
/// ranges of an array, made by blocks in the pass of the caller's `filter`
/// over that array (reverse-complement: the heads and the bases in one
/// read). Records from each head (`>`, every `step` bytes: within one block,
/// across many, one record for all), bytes and `i32`; a record that starts
/// and ends inside a block, at its edges, across; the same function given
/// another array (its own counts); arrays of at most 4096 elements (no
/// counts made: the function counts); 1 to 8 threads; the same bytes as
/// computed here and as the interpreter; nothing lost (valgrind).
#[test]
fn counts_made_by_a_caller() {
    let src = "action records(data: u8[n], heads: i64[h], r: i64): void {
    if (r == h) return;
    const start = heads[r];
    const end = r + 1 < h ? heads[r + 1] : n;
    const bases = data.slice(start + 1, end).filter((b) => b != 10u8);
    const m = bases.length;
    print(m);
    printBytes(map(iota(m + (m + 59) / 60), (q) => q % 61 == 60 || q == m + (m + 59) / 60 - 1 ? 10u8 : bases[m - 1 - (q / 61 * 60 + q % 61)]));
    return records(data, heads, r + 1);
}

action sums(v: i32[n], heads: i64[h], r: i64): void {
    if (r == h) return;
    const start = heads[r];
    const end = r + 1 < h ? heads[r + 1] : n;
    const kept = v.slice(start, end).filter((x) => x % 3 != 0);
    const m = kept.length;
    printBytes(map(iota(m), (q) => u8(65 + kept[m - 1 - q] % 26)));
    return sums(v, heads, r + 1);
}

action main(args: string[k]): i32 {
    const n = parseI64(args[0]);
    const step = parseI64(args[1]);
    const data = map(iota(n), (i) => i % step == 0 ? 62u8 : (i % 61 == 60 ? 10u8 : u8(65 + (i * 7 + i / 13) % 26)));
    const heads = filter(iota(data.length), (i) => data[i] == 62u8);
    records(data, heads, 0);
    const other = map(iota(n / 2 + 1), (i) => i % 997 == 0 ? 62u8 : (i % 37 == 0 ? 10u8 : 66u8));
    records(other, heads.filter((x) => x < 0), 0);
    records(other, [0], 0);
    const v = map(iota(n), (i) => i32((i * 7919) % 1000));
    const marks = filter(iota(v.length), (i) => v[i] % i32(step) == 1i32);
    sums(v, marks, 0);
    print(\"end\");
    return 0;
}
";
    let want = |n: usize, step: usize| -> Vec<u8> {
        let mut out = Vec::new();
        let mut records = |data: &[u8], heads: &[usize]| {
            for (r, &start) in heads.iter().enumerate() {
                let end = if r + 1 < heads.len() { heads[r + 1] } else { data.len() };
                let bases: Vec<u8> = data[(start + 1).min(end)..end].iter().copied().filter(|&b| b != 10).collect();
                let m = bases.len();
                out.extend(format!("{}\n", m).into_bytes());
                let len = m + m.div_ceil(60);
                out.extend((0..len).map(|q| if q % 61 == 60 || q + 1 == len { 10 } else { bases[m - 1 - (q / 61 * 60 + q % 61)] }));
            }
        };
        let data: Vec<u8> = (0..n).map(|i| if i % step == 0 { 62 } else if i % 61 == 60 { 10 } else { 65 + ((i * 7 + i / 13) % 26) as u8 }).collect();
        let heads: Vec<usize> = (0..n).filter(|&i| data[i] == 62).collect();
        records(&data, &heads);
        let other: Vec<u8> = (0..n / 2 + 1).map(|i| if i % 997 == 0 { 62 } else if i % 37 == 0 { 10 } else { 66 }).collect();
        records(&other, &[0]);
        let v: Vec<i64> = (0..n).map(|i| ((i * 7919) % 1000) as i64).collect();
        let marks: Vec<usize> = (0..n).filter(|&i| v[i] % step as i64 == 1).collect();
        for (r, &start) in marks.iter().enumerate() {
            let end = if r + 1 < marks.len() { marks[r + 1] } else { n };
            let kept: Vec<i64> = v[start..end].iter().copied().filter(|x| x % 3 != 0).collect();
            let m = kept.len();
            out.extend((0..m).map(|q| 65 + (kept[m - 1 - q] % 26) as u8));
        }
        out.extend_from_slice(b"end\n");
        out
    };
    let bin = compile(src, "counts_by_a_caller");
    let ll = std::fs::read_to_string(bin.with_extension("ll")).unwrap();
    assert!(ll.contains("_sm(ptr %sm"), "no function given the counts of its caller");
    for (n, step) in [(1000usize, 300usize), (5000, 1500), (70001, 5000), (70001, 1024), (70001, 1_000_003), ((1 << 20) + 9, 4096), ((1 << 20) + 9, 777), ((3 << 20) + 12345, 1_000_003), ((3 << 20) + 12345, 65_537)] {
        for t in ["1", "2", "3", "4", "8"] {
            let o = std::process::Command::new(&bin).args([n.to_string(), step.to_string()]).env("ISC_THREADS", t).output().unwrap();
            assert_eq!(o.status.code(), Some(0), "n {} step {}: {}", n, step, String::from_utf8_lossy(&o.stderr));
            assert!(o.stdout == want(n, step), "n {}, step {}, {} threads: {} bytes, {} expected", n, step, t, o.stdout.len(), want(n, step).len());
        }
    }
    let p = iscc::frontend("counts_by_a_caller.isc", src).unwrap();
    let mut out = Vec::new();
    assert_eq!(isc_interp::run(&p, &["70001".to_string(), "5000".to_string()], &mut out).unwrap(), 0);
    assert!(out == want(70001, 5000), "interpreter");
    valgrind_clean(src, "counts_by_a_caller", &["70001", "5000"]);
}

/// D293: a `filter` read only by arrays written by blocks (D225), each
/// block making the part of it that it reads: the elements of the source
/// counted by blocks, then filtered again for each block of the output from
/// the bounds of its indices. Reversed in lines of 60 (reverse-complement),
/// forward with a stride and an offset, by an index of 8 bits (the range of
/// its type), too wide for windows (the whole array made for that output),
/// elements of 8 bytes (windows on the heap), an index past the end (the
/// panic of the program), nothing kept; sizes around the blocks of the
/// count, of the tasks and of the output; 1 to 8 threads; the same bytes as
/// computed here and as the interpreter; nothing lost (valgrind).
#[test]
fn filters_read_by_windows() {
    let src = "action main(args: string[k]): i32 {
    const n = parseI64(args[0]);
    const data = map(iota(n), (i) => i % 61 == 60 ? 10u8 : u8(65 + (i * 7 + i / 13) % 26));
    const bases = data.slice(3, n).filter((b) => b != 10u8);
    const m = bases.length;
    print(m);
    printBytes(map(iota(m + (m + 59) / 60), (q) => q % 61 == 60 || q == m + (m + 59) / 60 - 1 ? 10u8 : bases[m - 1 - (q / 61 * 60 + q % 61)]));
    printBytes(map(iota(m / 2), (q) => bases[2 * q + 1]));
    printBytes(map(iota(m >= 256 ? m : 0), (q) => bases[u8(q % 200)]));
    printBytes(map(iota(m / 1000), (q) => bases[q * 997]));
    const v = map(iota(n), (i) => (i * 7919) % 1000);
    const big = v.filter((x) => x % 3 != 0);
    const k = big.length;
    printBytes(map(iota(k), (q) => u8(65 + big[k - 1 - q] % 26)));
    print(\"end\");
    return 0;
}
";
    let want = |n: usize| -> Vec<u8> {
        let data: Vec<u8> = (0..n).map(|i| if i % 61 == 60 { 10 } else { 65 + ((i * 7 + i / 13) % 26) as u8 }).collect();
        let bases: Vec<u8> = data[3.min(n)..].iter().copied().filter(|&b| b != 10).collect();
        let m = bases.len();
        let mut out = format!("{}\n", m).into_bytes();
        let len = m + m.div_ceil(60);
        out.extend((0..len).map(|q| if q % 61 == 60 || q + 1 == len { 10 } else { bases[m - 1 - (q / 61 * 60 + q % 61)] }));
        out.extend((0..m / 2).map(|q| bases[2 * q + 1]));
        out.extend((0..if m >= 256 { m } else { 0 }).map(|q| bases[q % 200]));
        out.extend((0..m / 1000).map(|q| bases[q * 997]));
        let big: Vec<usize> = (0..n).map(|i| (i * 7919) % 1000).filter(|x| x % 3 != 0).collect();
        let k = big.len();
        out.extend((0..k).map(|q| 65 + (big[k - 1 - q] % 26) as u8));
        out.extend_from_slice(b"end\n");
        out
    };
    let bin = compile(src, "filter_windows");
    let ll = std::fs::read_to_string(bin.with_extension("ll")).unwrap();
    assert!(ll.contains("alloca [32768 x i8], align 64"), "no windows");
    for n in [4usize, 1030, 5000, 70001, (1 << 20) + 9, (3 << 20) + 12345] {
        for t in ["1", "2", "3", "4", "8"] {
            let o = std::process::Command::new(&bin).arg(n.to_string()).env("ISC_THREADS", t).output().unwrap();
            assert_eq!(o.status.code(), Some(0), "n {}: {}", n, String::from_utf8_lossy(&o.stderr));
            assert!(o.stdout == want(n), "n {}, {} threads: {} bytes, {} expected", n, t, o.stdout.len(), want(n).len());
        }
    }
    let p = iscc::frontend("filter_windows.isc", src).unwrap();
    let mut out = Vec::new();
    assert_eq!(isc_interp::run(&p, &["70001".to_string()], &mut out).unwrap(), 0);
    assert!(out == want(70001), "interpreter");
    valgrind_clean(src, "filter_windows", &["70001"]);
    // past the end, and nothing kept: the panics of the program
    for (name, body) in [
        ("past", "printBytes(map(iota(m), (q) => bases[q + 1]));"),
        ("none", "printBytes(map(iota(5), (q) => bases[q * 0 + 2]));"),
    ] {
        let fill = if name == "none" { "10u8" } else { "u8(65 + i % 26)" };
        let src = format!(
            "action main(args: string[k]): i32 {{\n    const n = parseI64(args[0]);\n    const data = map(iota(n), (i) => {});\n    const bases = data.filter((b) => b != 10u8);\n    const m = bases.length;\n    {}\n    return 0;\n}}\n",
            fill, body
        );
        let bin = compile(&src, &format!("filter_windows_{}", name));
        let Outcome::Panic(msg, _) = interp(&src, &["9000"]) else { panic!("{}: no panic in the interpreter", name) };
        for t in [1, 4] {
            let r = run_bin_threads(&bin, &["9000"], Some(t));
            assert_eq!(r.code, 101, "{} T={}: {}", name, t, r.stderr);
            assert_eq!(r.stderr.trim_end(), msg, "{} T={}", name, t);
        }
    }
}

/// D437: the rest of a count (the elements after its whole vectors) in one
/// vector whose lanes past the end are masked, on a target with masked
/// loads of bytes; one element at a time on the others. Arrays of bytes and
/// of `i32` of 0 to 100 elements (every rest of the vectors), slices at
/// every offset, predicates of comparisons, logic and arithmetic (the
/// masked lanes zero), counts and `filter`s; the interpreter's values, with
/// the code for the machine and for AVX2.
#[test]
fn counts_with_a_masked_rest() {
    let src = "action main(args: string[k]): i32 {
    const n = parseI64(args[0]);
    const b = map(iota(n), (i) => u8((i * 37 + 11) % 256));
    const w = map(iota(n), (i) => i32((i * 7919) % 2000 - 1000));
    const offs = iota(Math.min(n, 9));
    print(map(offs, (o) => count(b.slice(o, n), (c) => c < 32u8 || c == 34u8 || c >= 200u8)));
    print(map(offs, (o) => count(b.slice(o, n), (c) => (c & 7u8) == 3u8)));
    print(map(offs, (o) => count(w.slice(o, n), (x) => x * 3 + 1 > 0)));
    print(map(offs, (o) => b.slice(o, n).filter((c) => c != 10u8 && c > 100u8).length));
    print(count(b, (c) => c == 0u8));
    return 0;
}
";
    let native = compile(src, "masked_rest");
    let avx2 = compile_with(src, "masked_rest_avx2", isc_codegen::Options { isa: isc_codegen::Isa::Avx2, ..isc_codegen::Options::default() });
    let ll = std::fs::read_to_string(native.with_extension("ll")).unwrap();
    let wide = std::fs::read_to_string("/proc/cpuinfo").unwrap_or_default().contains("avx512bw");
    assert_eq!(ll.contains("llvm.masked.load.v32i8"), wide, "a masked rest on the machine");
    for n in 0..=100 {
        let a = n.to_string();
        let Outcome::Exit(0, want) = interp(src, &[&a]) else { panic!("the interpreter failed for {}", n) };
        for bin in [&native, &avx2] {
            let r = run_bin_threads(bin, &[&a], Some(1));
            assert_eq!(r.code, 0, "n {}: {}", n, r.stderr);
            assert_eq!(r.stdout, want, "n {}", n);
        }
    }
}
