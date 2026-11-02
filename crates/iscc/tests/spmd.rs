//! SPMD calls (DECISIONS D140): a tail-recursive function of scalars that
//! cannot panic, called in vector code, is computed for all the lanes at
//! once with a mask of the active lanes; the results are those of the
//! interpreter, and the vector loop is there.

mod common;
use common::*;

const PROGRAMS: &str = r#"// steps of the Collatz sequence: integer state, a branch in the step
function collatz(x: i64, steps: i64): i64 {
    if (x <= 1) {
        return steps;
    } else if (x % 2 == 0) {
        return collatz(x / 2, steps + 1);
    } else {
        return collatz(3 * x + 1, steps + 1);
    }
}

// Euclid: `%` by a variable can panic: not SPMD, still right
function gcd(a: i64, b: i64): i64 {
    if (b == 0) {
        return a;
    } else {
        return gcd(b, a % b);
    }
}

// Newton's square root, float state, stops on a tolerance
function newton(a: f64, x: f64, k: i64): f64 {
    const y = 0.5 * (x + a / x);
    if (k == 60 || (y - x) * (y - x) < 1e-24) {
        return y;
    } else {
        return newton(a, y, k + 1);
    }
}

// a non-tail call: not SPMD
function depth(n: i64): i64 {
    if (n <= 0) {
        return 0;
    } else {
        return 1 + depth(n - 1);
    }
}

action main(args: string[k]): i32 {
    print(map(iota(20), (i) => collatz(i + 1, 0)));
    print(sum(map(iota(100000), (i) => collatz(i + 1, 0))));
    print(map(iota(12), (i) => i % 3 == 0 ? collatz(i + 7, 0) : -1));
    print(map(iota(10), (i) => gcd(i * 6, 84)));
    print(map(iota(9), (i) => newton(f64(i + 1), 1.0, 0)));
    print(map(iota(8), (i) => depth(i)));
    print(map(iota(17), (i) => fold(iota(3), 0, (acc, j) => acc + collatz(i + j + 1, 0))));
    return 0;
}
"#;

#[test]
fn spmd_calls_give_the_same_results() {
    let expected = "[0, 1, 7, 2, 5, 8, 16, 3, 19, 6, 14, 9, 9, 17, 17, 4, 12, 20, 20, 7]\n10753840\n[16, -1, -1, 6, -1, -1, 9, -1, -1, 4, -1, -1]\n[84, 6, 12, 6, 12, 6, 12, 42, 12, 6]\n[1, 1.414213562373095, 1.7320508075688772, 2, 2.23606797749979, 2.449489742783178, 2.6457513110645907, 2.82842712474619, 3]\n[0, 1, 2, 3, 4, 5, 6, 7]\n[8, 10, 14, 15, 29, 27, 38, 28, 39, 29, 32, 35, 43, 38, 33, 36, 52]\n";
    assert_eq!(interp_ok(PROGRAMS, &[]), expected, "interpreter");
    let bin = compile(PROGRAMS, "spmd_calls");
    let r = run_bin(&bin, &[]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(r.stdout, expected, "compiled");
    // `collatz` and `newton` in vector loops; `gcd` (a `%` by a variable)
    // and `depth` (not a tail call) one element at a time
    let ll = std::fs::read_to_string(tmp("spmd_calls").with_extension("ll")).unwrap();
    assert!(ll.contains("llvm.vector.reduce.or"), "no SPMD loop");
}

#[test]
fn mandelbrot_is_vectorized() {
    for file in ["mandelbrot.isc", "mandelbrot.simple.isc"] {
        let path = format!("{}/../../examples/benchmarks_game/{}", env!("CARGO_MANIFEST_DIR"), file);
        let src = std::fs::read_to_string(path).unwrap();
        let name = format!("spmd_{}", file.replace('.', "_"));
        let _ = compile(&src, &name);
        let ll = std::fs::read_to_string(tmp(&name).with_extension("ll")).unwrap();
        assert!(ll.contains("llvm.vector.reduce.or"), "{}: `inside` is not computed lane by lane", file);
    }
}
