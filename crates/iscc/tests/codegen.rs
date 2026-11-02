//! Compiled programs are bit-identical to the interpreter (P4).

mod common;
use common::*;

const SIZES: [usize; 12] = [0, 1, 15, 16, 17, 31, 32, 33, 4095, 4096, 4097, 70_001];

fn same_file(a: &str, b: &str) {
    assert_eq!(std::fs::read(a).unwrap(), std::fs::read(b).unwrap(), "{} != {}", a, b);
}

#[test]
fn sum_dot_std_dev_match_interpreter() {
    let sum = compile(SUM, "c_sum");
    let dot = compile(DOT, "c_dot");
    let std = compile(STD, "c_std");
    for (i, n) in SIZES.iter().enumerate() {
        let a = random_f64s(100 + i as u64, *n);
        let b = random_f64s(200 + i as u64, *n);
        let pa = write_f64s(&format!("c_a{}.f64", n), &a);
        let pb = write_f64s(&format!("c_b{}.f64", n), &b);
        let o1 = tmp(&format!("c_o1_{}.f64", n)).to_string_lossy().to_string();
        let o2 = tmp(&format!("c_o2_{}.f64", n)).to_string_lossy().to_string();
        // sum
        let r = run_bin(&sum, &[&pa, &o1, "3"]);
        assert_eq!(r.code, 0, "{}", r.stderr);
        assert!(r.stdout.trim().parse::<u64>().is_ok());
        interp_ok(SUM, &[&pa, &o2, "1"]);
        same_file(&o1, &o2);
        assert_eq!(read_f64s(&o1)[0].to_bits(), fixed_reduce(&a, 0.0, &|x, y| x + y).to_bits());
        // dot
        let r = run_bin(&dot, &[&pa, &pb, &o1, "2"]);
        assert_eq!(r.code, 0, "{}", r.stderr);
        interp_ok(DOT, &[&pa, &pb, &o2, "1"]);
        same_file(&o1, &o2);
        // std_dev
        let r = run_bin(&std, &[&pa, &o1, "2"]);
        assert_eq!(r.code, 0, "{}", r.stderr);
        interp_ok(STD, &[&pa, &o2, "1"]);
        same_file(&o1, &o2);
    }
}

/// With few vector registers the accumulators are updated in several passes
/// over each block (`reduce_passes`): SSE with V = 32 (16 `xmm` registers),
/// AVX2 with V = 64. The result must stay the fixed form.
#[test]
fn reductions_in_passes_match_interpreter() {
    use isc_codegen::{emit::reduce_passes, Isa, Options};
    assert_eq!(reduce_passes(32, 8, 2, Isa::Sse.vector_registers()), 2);
    assert_eq!(reduce_passes(64, 8, 2, Isa::Avx2.vector_registers()), 2);
    assert_eq!(reduce_passes(32, 8, 1, Isa::Avx2.vector_registers()), 1);
    let mut targets = vec![(32u32, Isa::Sse), (64, Isa::Sse)];
    if std::arch::is_x86_feature_detected!("avx2") {
        targets.push((64, Isa::Avx2));
    }
    for (lanes, isa) in targets {
        let tag = format!("v{}_{:?}", lanes, isa);
        let opts = Options { lanes, isa, ..Options::default() };
        let sum = compile_with(SUM, &format!("p_sum_{}", tag), opts);
        let dot = compile_with(DOT, &format!("p_dot_{}", tag), opts);
        let std = compile_with(STD, &format!("p_std_{}", tag), opts);
        for (i, n) in [0usize, 1, 31, 33, 63, 65, 1000, 4095, 4096, 4097, 70_001].iter().enumerate() {
            let a = random_f64s(300 + i as u64, *n);
            let b = random_f64s(400 + i as u64, *n);
            let pa = write_f64s(&format!("p_a{}_{}.f64", n, tag), &a);
            let pb = write_f64s(&format!("p_b{}_{}.f64", n, tag), &b);
            let o1 = tmp(&format!("p_o1_{}_{}.f64", n, tag)).to_string_lossy().to_string();
            let o2 = tmp(&format!("p_o2_{}_{}.f64", n, tag)).to_string_lossy().to_string();
            for (bin, src, args) in [(&sum, SUM, vec![pa.as_str()]), (&dot, DOT, vec![&pa, &pb]), (&std, STD, vec![&pa])] {
                let mut a1 = args.clone();
                a1.extend([o1.as_str(), "2"]);
                let r = run_bin(bin, &a1);
                assert_eq!(r.code, 0, "{}", r.stderr);
                let mut a2 = args.clone();
                a2.extend([o2.as_str(), "1"]);
                interp_lanes_ok(src, &a2, lanes as usize);
                same_file(&o1, &o2);
            }
        }
    }
}

#[test]
fn histogram_matches_interpreter() {
    let h = compile(HIST, "c_hist");
    let mut rng = Rng(7);
    for n in [0usize, 1, 255, 100_000] {
        let bytes: Vec<u8> = (0..n).map(|_| (rng.next() % 256) as u8).collect();
        let p = tmp(&format!("c_h{}.u8", n));
        std::fs::write(&p, &bytes).unwrap();
        let p = p.to_string_lossy().to_string();
        let o1 = tmp("c_h_o1.f64").to_string_lossy().to_string();
        let o2 = tmp("c_h_o2.f64").to_string_lossy().to_string();
        let r = run_bin(&h, &[&p, &o1, "3"]);
        assert_eq!(r.code, 0, "{}", r.stderr);
        interp_ok(HIST, &[&p, &o2, "1"]);
        same_file(&o1, &o2);
    }
}

#[test]
fn simd_is_visible_in_the_assembly() {
    for (name, src) in [("c_sum", SUM), ("c_dot", DOT), ("c_std", STD)] {
        let bin = compile(src, &format!("{}_asm", name));
        let asm = std::fs::read_to_string(bin.with_extension("s")).unwrap();
        // packed double-precision additions (SSE2 `addpd` or AVX `vaddpd`)
        let packed = asm.lines().filter(|l| l.contains("addpd")).count();
        assert!(packed >= 8, "{}: only {} packed additions", name, packed);
        let ll = std::fs::read_to_string(bin.with_extension("ll")).unwrap();
        assert!(ll.contains(&format!("<{} x double>", isc_codegen::emit::LANES)), "{}: no V-lane accumulator", name);
        // no fast-math flag on floating-point instructions (section 13)
        for flag in ["fast ", "reassoc", "contract", "nnan", "ninf", "nsz", "arcp", "afn"] {
            assert!(!ll.contains(flag), "{}: fast-math flag `{}`", name, flag);
        }
    }
}

#[test]
fn multi_level_reduction_matches_fixed_form() {
    // 4097 * 4096 + 5 elements: three levels of blocks
    let n: i64 = 4097 * 4096 + 5;
    let src = format!(
        "function f(): i64 {{\n    return reduce(iota({}), (a, b) => a * 3 - b, 1);\n}}\nfunction g(): f64 {{\n    return sum(map(iota({}), (i) => 1.0 / f64(i + 1)));\n}}\naction main(args: string[k]): i32 {{\n    print(f());\n    print(g());\n    return 0;\n}}\n",
        n, n
    );
    let bin = compile(&src, "c_levels");
    let r = run_bin(&bin, &[]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    let xs: Vec<i64> = (0..n).collect();
    let want_i = fixed_reduce(&xs, 1i64, &|a: i64, b: i64| a.wrapping_mul(3).wrapping_sub(b));
    let fs: Vec<f64> = (0..n).map(|i| 1.0 / (i + 1) as f64).collect();
    let want_f = fixed_reduce(&fs, 0.0, &|a, b| a + b);
    assert_eq!(r.stdout, format!("{}\n{}\n", want_i, isc_interp::rt::format_f64(want_f)));
}

#[test]
fn panics_match_interpreter() {
    let cases: Vec<(&str, Vec<&str>)> = vec![
        ("action main(args: string[k]): i32 {\n    const x = iota(3);\n    print(x[parseI64(args[0])]);\n    return 0;\n}\n", vec!["3"]),
        ("action main(args: string[k]): i32 {\n    const x = iota(3);\n    print(x[parseI64(args[0])]);\n    return 0;\n}\n", vec!["-1"]),
        ("function f(x: i64): i64 {\n    return 10 / x;\n}\n\naction main(args: string[k]): i32 {\n    print(f(parseI64(args[0])));\n    return 0;\n}\n", vec!["0"]),
        ("function f(x: i64): i64 {\n    return 2 ** x;\n}\n\naction main(args: string[k]): i32 {\n    print(f(parseI64(args[0])));\n    return 0;\n}\n", vec!["-3"]),
        ("action main(args: string[k]): i32 {\n    print(parseI64(args[0]));\n    return 0;\n}\n", vec!["x1"]),
        ("action main(args: string[k]): i32 {\n    const [r, ns] = bench(parseI64(args[0]), () => 1);\n    return 0;\n}\n", vec!["0"]),
        ("action main(args: string[k]): i32 {\n    const x = readF64(args[0]);\n    print(1);\n    return 0;\n}\n", vec!["/nonexistent"]),
        ("action main(args: string[k]): i32 {\n    const a = readF64(args[0]);\n    print(sum(a * readF64(args[1])));\n    return 0;\n}\n", vec!["@three", "/dev/null"]),
    ];
    let three = write_f64s("c_three.f64", &[1.0, 2.0, 3.0]);
    for (i, (src, args)) in cases.iter().enumerate() {
        let args: Vec<&str> = args.iter().map(|a| if *a == "@three" { three.as_str() } else { *a }).collect();
        let args = &args;
        let bin = compile(src, &format!("c_panic{}", i));
        let r = run_bin(&bin, args);
        let want = interp_panic(src, args);
        assert_eq!(r.code, 101, "case {}: {}", i, r.stderr);
        assert_eq!(r.stderr.trim_end(), want, "case {}", i);
    }
}

#[test]
fn printing_and_semantics_match_interpreter() {
    let src = r#"function ints(x: i64): [i64, i64, i64, u8, i32, u64] {
    return [x / -2, x % 3, -x, u8(x) * 7u8, i32(f64(x) * 1000000000.0), u64(x) - 1u64] as const;
}

function floats(y: f64): [f64, f64, f64, i8, u8] {
    return [y ** 3, sqrt(y), y % 0.7, i8(y * 100.0), u8(-y)] as const;
}

function arrs(n: i64): f64[3] {
    return [1.5, -0.0, 2.0] * f64(n);
}

action main(args: string[k]): i32 {
    const x = parseI64(args[0]);
    print(ints(x));
    print(floats(f64(x) / 7.0));
    print(arrs(x));
    print(iota(5) == iota(5) && !(x < 0));
    print(map(iota(20), (i) => i * x % 7));
    print(fold(iota(10), 0u64, (a, i) => a * 31u64 + u64(i)));
    print(reduce([1.0, 2.0, 3.0], (a, b) => a > b ? a : b, -1.0));
    print(clock() > 0u64);
    print(args);
    print(());
    return i32(x % 5);
}
"#;
    let bin = compile(src, "c_print");
    for x in ["17", "-123456789", "0", "9223372036854775807"] {
        let r = run_bin(&bin, &[x]);
        match interp(src, &[x]) {
            Outcome::Exit(code, out) => {
                assert_eq!(r.code, code & 0xff, "x = {}", x);
                assert_eq!(r.stdout, out, "x = {}", x);
            }
            Outcome::Panic(m, _) => panic!("{}", m),
        }
    }
}

/// `iscc build --target wasm32-wasi`: the POC kernels give the same bits as
/// the interpreter (and the same exit code and panic message), run by a WASI
/// engine. Without an engine, only the compilation is checked.
#[test]
fn webassembly_matches_interpreter() {
    let progs = [("w_sum", SUM), ("w_dot", DOT), ("w_std", STD)];
    let bins: Vec<_> = progs.iter().map(|(n, s)| compile_wasm(s, n)).collect();
    for b in &bins {
        let head = std::fs::read(b).unwrap();
        assert_eq!(&head[..4], b"\0asm", "{}: not a WebAssembly module", b.display());
    }
    let Some(runner) = wasi_runner() else {
        eprintln!("no WASI engine (wasmtime): the WebAssembly programs were compiled but not run");
        return;
    };
    for n in [0usize, 1, 33, 4097, 70_001] {
        let a = random_f64s(500 + n as u64, n);
        let b = random_f64s(600 + n as u64, n);
        let pa = write_f64s(&format!("w_a{}.f64", n), &a);
        let pb = write_f64s(&format!("w_b{}.f64", n), &b);
        let o1 = tmp(&format!("w_o1_{}.f64", n)).to_string_lossy().to_string();
        let o2 = tmp(&format!("w_o2_{}.f64", n)).to_string_lossy().to_string();
        for (k, (_, src)) in progs.iter().enumerate() {
            let inputs: Vec<&str> = if k == 1 { vec![&pa, &pb] } else { vec![&pa] };
            let mut a1 = inputs.clone();
            a1.extend([o1.as_str(), "2"]);
            let r = run_wasm(&runner, &bins[k], &a1);
            assert_eq!(r.code, 0, "{}", r.stderr);
            let mut a2 = inputs.clone();
            a2.extend([o2.as_str(), "1"]);
            interp_ok(src, &a2);
            same_file(&o1, &o2);
        }
    }
    // a panic: same message and exit code
    let src = "action main(args: string[k]): i32 {\n    const x = iota(3);\n    print(x[parseI64(args[0])]);\n    return 0;\n}\n";
    let w = compile_wasm(src, "w_panic");
    let r = run_wasm(&runner, &w, &["7"]);
    assert_eq!(r.code, 101);
    assert_eq!(r.stderr.trim(), interp_panic(src, &["7"]).trim());
    // input/output errors: the Linux numbering of the interpreter (D67)
    let missing = tmp("w_missing.f64").to_string_lossy().to_string();
    let dir = tmp("").to_string_lossy().to_string();
    for path in [missing.as_str(), dir.as_str()] {
        let r = run_wasm(&runner, &bins[0], &[path, &o1_path(), "1"]);
        assert_eq!(r.code, 101, "{}", r.stderr);
        assert_eq!(r.stderr.trim(), interp_panic(SUM, &[path, &o1_path(), "1"]).trim());
    }
    // wasm32-wasi-threads: the same bits with 1, 2, 3, 4 and 8 threads
    let n = 4096 * 9 + 17;
    let a = random_f64s(700, n);
    let b = random_f64s(701, n);
    let pa = write_f64s("wt_a.f64", &a);
    let pb = write_f64s("wt_b.f64", &b);
    let o2 = tmp("wt_o2.f64").to_string_lossy().to_string();
    let o1 = o1_path();
    for (k, (name, src)) in progs.iter().enumerate() {
        let wt = compile_wasm_isa(src, &format!("{}_threads", name), isc_codegen::Isa::Wasm32Threads);
        let inputs: Vec<&str> = if k == 1 { vec![&pa, &pb] } else { vec![&pa] };
        let mut a2 = inputs.clone();
        a2.extend([o2.as_str(), "1"]);
        interp_ok(src, &a2);
        for t in [1, 2, 3, 4, 8] {
            let mut a1 = inputs.clone();
            a1.extend([o1.as_str(), "2"]);
            let r = run_wasm_threads(&runner, &wt, &a1, Some(t));
            assert_eq!(r.code, 0, "{} with {} threads: {}", name, t, r.stderr);
            same_file(&o1, &o2);
        }
    }
}

/// Rule P on `wasm32-wasi-threads`: private copies allocated by the tasks
/// (the allocator of wasi-libc is shared by the threads), same counts for
/// any number of threads.
#[test]
fn webassembly_threads_histogram() {
    let Some(runner) = wasi_runner() else {
        eprintln!("no WASI engine (wasmtime): not run");
        return;
    };
    let mut rng = Rng(11);
    let bytes: Vec<u8> = (0..300_001).map(|_| (rng.next() % 256) as u8).collect();
    let p = tmp("wt_h.u8");
    std::fs::write(&p, &bytes).unwrap();
    let p = p.to_string_lossy().to_string();
    let o1 = tmp("wt_h1.f64").to_string_lossy().to_string();
    let o2 = tmp("wt_h2.f64").to_string_lossy().to_string();
    interp_ok(HIST, &[&p, &o2, "1"]);
    let w = compile_wasm_isa(HIST, "wt_hist", isc_codegen::Isa::Wasm32Threads);
    for t in [1, 2, 3, 8] {
        let r = run_wasm_threads(&runner, &w, &[&p, &o1, "3"], Some(t));
        assert_eq!(r.code, 0, "{} threads: {}", t, r.stderr);
        same_file(&o1, &o2);
    }
}

fn o1_path() -> String {
    tmp("w_o1_err.f64").to_string_lossy().to_string()
}

