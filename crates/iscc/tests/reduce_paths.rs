//! The paths of `reduce` chosen by size at run time (DECISIONS D86): four rows
//! per iteration for an array read from memory, the scratch arrays on the
//! heap beyond the bound of the stack. The thresholds are lowered so that
//! small arrays take them; with every combination, and with 1 and 4 threads,
//! the result is the interpreter's bit for bit, and the fixed form's.

mod common;
use common::*;

fn same_file(a: &str, b: &str) {
    assert_eq!(std::fs::read(a).unwrap(), std::fs::read(b).unwrap(), "{} != {}", a, b);
}

#[test]
fn unrolled_loop_and_heap_scratch_match_the_interpreter() {
    use isc_codegen::Options;
    let variants = [
        ("unrolled", Options { stream_bytes: 0, ..Options::default() }),
        ("heap", Options { stack_scratch: 0, ..Options::default() }),
        ("unrolled_heap", Options { stream_bytes: 0, stack_scratch: 0, ..Options::default() }),
        ("default", Options::default()),
    ];
    for (tag, opts) in variants {
        let sum = compile_with(SUM, &format!("rp_sum_{}", tag), opts);
        let dot = compile_with(DOT, &format!("rp_dot_{}", tag), opts);
        let std = compile_with(STD, &format!("rp_std_{}", tag), opts);
        // around multiples of V (32) and of four rows (128), one block, several
        for (i, n) in [0usize, 1, 31, 32, 33, 127, 128, 129, 160, 4095, 4096, 4097, 9000, 70_001].iter().enumerate() {
            let a = random_f64s(500 + i as u64, *n);
            let b = random_f64s(600 + i as u64, *n);
            let pa = write_f64s(&format!("rp_a{}_{}.f64", n, tag), &a);
            let pb = write_f64s(&format!("rp_b{}_{}.f64", n, tag), &b);
            let o1 = tmp(&format!("rp_o1_{}_{}.f64", n, tag)).to_string_lossy().to_string();
            let o2 = tmp(&format!("rp_o2_{}_{}.f64", n, tag)).to_string_lossy().to_string();
            for (bin, src, args) in [(&sum, SUM, vec![pa.as_str()]), (&dot, DOT, vec![&pa, &pb]), (&std, STD, vec![&pa])] {
                let mut a2 = args.clone();
                a2.extend([o2.as_str(), "1"]);
                interp_ok(src, &a2);
                for t in [1, 4] {
                    let mut a1 = args.clone();
                    a1.extend([o1.as_str(), "2"]);
                    let r = run_bin_threads(bin, &a1, Some(t));
                    assert_eq!(r.code, 0, "{} {} n = {} with {} threads: {}", tag, src.lines().next().unwrap(), n, t, r.stderr);
                    same_file(&o1, &o2);
                }
            }
            let o = tmp(&format!("rp_o3_{}_{}.f64", n, tag)).to_string_lossy().to_string();
            let r = run_bin(&sum, &[&pa, &o, "1"]);
            assert_eq!(r.code, 0, "{}", r.stderr);
            assert_eq!(read_f64s(&o)[0].to_bits(), fixed_reduce(&a, 0.0, &|x, y| x + y).to_bits(), "{} n = {}", tag, n);
        }
    }
}

#[test]
fn exact_reductions_in_one_loop_give_the_results_of_the_interpreter() {
    // exact operators (D183): at most one block, or inside a task (the
    // rows of a map), one loop instead of the fixed form; integer sums and
    // products that wrap, `min` and `max` of floats with NaN and -0
    let src = "action main(args: string[k]): i32 {
    const n = 3 * 4096 + 7;
    const big = map(iota(n), (i) => i * 7919 + 9223372036854775000);
    print(sum(big));
    print(reduce(map(iota(40), (i) => i + 3), (a, b) => a * b, 1));
    const bytes = map(iota(1000), (i) => u8(i * 37));
    print(sum(bytes));
    const xs = map(iota(50), (i) => i % 17 == 3 ? 0.0 / 0.0 : (i % 5 == 0 ? -0.0 : f64(i) - 20.5));
    print([reduce(xs, (a, b) => Math.min(a, b), 1e308), reduce(xs.slice(0, 3), (a, b) => Math.max(a, b), -1e308)] as const);
    const zs = [0.0, -0.0, 0.0];
    print([1.0 / reduce(zs, (a, b) => Math.min(a, b), 1.0), 1.0 / reduce(zs, (a, b) => Math.max(a, b), -1.0)] as const);
    // in the tasks of a map: a count per row
    const rows = map(iota(5000), (r) => count(iota(r % 37), (j) => (j * r) % 3 == 1));
    print(sum(rows));
    print(rows.slice(0, 12));
    return 0;
}
";
    let want = interp_ok(src, &[]);
    let bin = compile(src, "exact_loops");
    for t in [1, 4] {
        let run = run_bin_threads(&bin, &[], Some(t));
        assert_eq!((run.code, run.stdout.as_str()), (0, want.as_str()), "{} threads: {}", t, run.stderr);
    }
}
