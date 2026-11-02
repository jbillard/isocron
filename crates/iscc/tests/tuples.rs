//! Arrays of tuples, stored in columns (SPEC section 4; DECISIONS D69):
//! compiled programs give the interpreter's output, natively with 1, 3 and 8
//! threads, and as WebAssembly.

mod common;
use common::*;

const PROG: &str = "function pick(a: [f64, i64], b: [f64, i64]): [f64, i64] {
    const [va, ia] = a;
    const [vb, ib] = b;
    if (vb < va) {
        return b;
    } else {
        return a;
    }
}

function indexed(x: f64[n]): [f64, i64][n] {
    return map(iota(n), (i) => [x[i], i] as const);
}

function add3(a: [i64, f64, boolean], b: [i64, f64, boolean]): [i64, f64, boolean] {
    const [i, x, p] = a;
    const [j, y, q] = b;
    return [i + j, x + y, p && q] as const;
}

action main(args: string[k]): i32 {
    const x = readF64(args[0]);
    const pairs = indexed(x);
    print(reduce(pairs, pick, [1e308, -1] as const));
    const mixed = map(x, (v) => [u8(v * 100.0), v, v > 0.0] as const);
    print(mixed[0]);
    print(mixed[3 * 4096 + 1]);
    const [c, s, all] = reduce(map(mixed, (m) => {
        const [a, b, p] = m;
        return [i64(a), b, p] as const;
    }), add3, [0, 0.0, true] as const);
    print([c, s, all] as const);
    print(fold(mixed, 0, (acc, m) => {
        const [a, b, p] = m;
        if (p) {
            return acc + i64(a);
        } else {
            return acc - 1;
        }
    }));
    const changed = mixed.with(7, [255u8, -2.5, false] as const);
    print(changed[7]);
    print(changed == mixed);
    print(mixed == mixed);
    return 0;
}
";

#[test]
fn arrays_of_tuples_match_interpreter() {
    let n = 3 * 4096 + 5;
    let x: Vec<f64> = random_f64s(41, n).into_iter().map(|v| v.abs()).collect();
    let px = write_f64s("tup_x.f64", &x);
    let want = interp_ok(PROG, &[&px]);
    assert!(want.lines().count() == 8, "{}", want);
    let bin = compile(PROG, "tup_prog");
    for t in [1, 3, 8] {
        let r = run_bin_threads(&bin, &[&px], Some(t));
        assert_eq!(r.code, 0, "{}", r.stderr);
        assert_eq!(r.stdout, want, "{} threads", t);
    }
    if let Some(runner) = wasi_runner() {
        let w = compile_wasm(PROG, "tup_prog");
        let r = run_wasm(&runner, &w, &[&px]);
        assert_eq!(r.code, 0, "{}", r.stderr);
        assert_eq!(r.stdout, want, "WebAssembly");
    }
}
