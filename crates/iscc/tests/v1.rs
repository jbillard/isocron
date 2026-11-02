//! The points of SPEC section 16 marked v1, decided provisionally
//! (DECISIONS D111 and after). Each is compared with the interpreter bit for
//! bit, with 1 and 4 threads.

mod common;
use common::*;

fn same_everywhere(src: &str, name: &str, inputs: &[&str]) {
    let bin = compile(src, name);
    let o_ref = tmp(&format!("{}_ref.out", name)).to_string_lossy().to_string();
    let mut a = inputs.to_vec();
    a.push(&o_ref);
    let printed = interp_ok(src, &a);
    for t in [1, 4] {
        let o = tmp(&format!("{}_{}.out", name, t)).to_string_lossy().to_string();
        let mut a = inputs.to_vec();
        a.push(&o);
        let r = run_bin_threads(&bin, &a, Some(t));
        assert_eq!(r.code, 0, "{} with {} threads: {}", name, t, r.stderr);
        assert_eq!(r.stdout, printed, "{} with {} threads: printed text", name, t);
        assert_eq!(std::fs::read(&o).unwrap(), std::fs::read(&o_ref).unwrap(), "{} with {} threads", name, t);
    }
}

/// `scatter` (D111): values combined at their indices in the order of k.
#[test]
fn scatter() {
    let src = "action main(args: string[k]): i32 {
    const x = readF64(args[0]);
    const h = scatter(fill(4, 0), [1, 3, 1, 0, 3, 3], [1, 1, 1, 1, 1, 1], (+));
    print(h);
    const firsts = scatter(fill(4, -1.0), [2, 0, 2], [5.0, 6.0, 7.0], (a, b) => a < 0.0 ? b : a);
    print(firsts);
    const buckets = scatter(fill(16, 0.0), map(x, (v) => i64(v * 1000.0) % 16 + 15 - 15), map(x, (v) => v * v), (+));
    writeF64(args[1], buckets);
    return 0;
}
";
    for (i, n) in [0usize, 7, 70_001].iter().enumerate() {
        let x = write_f64s(&format!("scatter_x{}.f64", n), &random_f64s(2300 + i as u64, *n).iter().map(|v| v.abs()).collect::<Vec<_>>());
        same_everywhere(src, &format!("scatter{}", n), &[&x]);
    }
    // an index out of bounds: a panic at the call, not inside the prelude
    let bad = "action main(args: string[k]): i32 {\n    print(scatter([1, 2], [5], [1], (+)));\n    return 0;\n}\n";
    assert!(interp_panic(bad, &[]).contains("panic at line 2, column"), "{}", interp_panic(bad, &[]));
    let r = run_bin_threads(&compile(bad, "scatter_bad"), &[], Some(1));
    assert!(r.stderr.contains("panic at line 2, column") && r.stderr.contains("the index is 5 but the length is 2"), "{}", r.stderr);
}

/// The fixed form of a scan of floats (D112): the same values compiled with
/// 1 and 4 threads and interpreted, for sizes around the blocks.
#[test]
fn float_scan_in_its_fixed_form() {
    let src = "action main(args: string[k]): i32 {
    const x = readF64(args[0]);
    const s = scan(x, (+), 0.0);
    const m = scan(x, (a, b) => a * 0.5 + b, 3.0);
    const f = scan(map(x, (v) => f32(v)), (+), f32(0.0));
    writeF64(args[1], s);
    print(sum(m) + f64(sum(f)));
    return 0;
}
";
    for (i, n) in [0usize, 4096, 4097, 8192, 70_001].iter().enumerate() {
        let x = write_f64s(&format!("fscan_x{}.f64", n), &random_f64s(2400 + i as u64, *n));
        same_everywhere(src, &format!("fscan{}", n), &[&x]);
    }
}

/// Mutual tail recursion (D113): a tail call to another function reuses the
/// frame, compiled (`tailcc`) and interpreted, for 3 000 000 calls.
#[test]
fn mutual_tail_recursion() {
    let src = "@requireTail
function even(n: i64): boolean {
    if (n == 0) {
        return true;
    } else {
        return odd(n - 1);
    }
}

function odd(n: i64): boolean {
    if (n == 0) {
        return false;
    } else {
        return even(n - 1);
    }
}

function ping(x: f64[n], k: i64, acc: f64): f64 {
    if (k == 0) {
        return acc + sum(x);
    } else {
        return pong(x, k - 1, acc + x[0]);
    }
}

function pong(x: f64[n], k: i64, acc: f64): f64 {
    if (k == 0) {
        return acc - sum(x);
    } else {
        return ping(x, k - 1, acc * 0.5 + x[1]);
    }
}

action main(args: string[k]): i32 {
    const m = parseI64(args[0]);
    print(even(m));
    print(odd(m + 1));
    writeF64(args[1], [ping([1.0, 2.0, 3.0], m, 0.0)]);
    return 0;
}
";
    same_everywhere(src, "mutual", &["3000000"]);
    let p = iscc::frontend("m.isc", src).unwrap();
    let tails = isc_ir::report::entries(&p).into_iter().filter(|e| e.kind == "tail").count();
    assert_eq!(tails, 4);
}

/// Functions stored in tuples, records and arrays (D114).
#[test]
fn functions_in_structures() {
    let src = "function double(v: f64): f64 {
    return v * 2.0;
}

function square(v: f64): f64 {
    return v * v;
}

function apply_both(ops: [(f64) => f64, (f64) => f64], x: f64): f64 {
    const [f, g] = ops;
    return f(x) + g(x);
}

action main(args: string[k]): i32 {
    const ops = [double, square] as const;
    print(apply_both(ops, 3.0));
    const r = { up: double, sq: square };
    print(r.sq(5.0));
    const fs = [double, square, (v) => v + 1.0];
    const i = parseI64(args[0]);
    print(fs[i](10.0));
    writeF64(args[1], map([1.0, 2.0], fs[i]));
    return 0;
}
";
    for i in ["0", "1", "2"] {
        same_everywhere(src, &format!("fstruct{}", i), &[i]);
    }
    assert!(interp_panic(src, &["3", "/dev/null"]).contains("the index is 3 but the length is 3"));
}
