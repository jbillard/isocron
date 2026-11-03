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
    assert!(ll.contains("@llvm.cttz.i"), "no vector check of the bounds");
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
