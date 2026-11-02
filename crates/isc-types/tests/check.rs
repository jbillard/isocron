use isc_types::check_source;
use isc_types::tast::*;
use isc_types::types::{IntKind, Size, Ty};

const POC: [(&str, &str); 4] = [
    ("sum", include_str!("../../../examples/poc/sum.isc")),
    ("dot", include_str!("../../../examples/poc/dot.isc")),
    ("histogram", include_str!("../../../examples/poc/histogram.isc")),
    ("std_dev", include_str!("../../../examples/poc/std_dev.isc")),
];

#[test]
fn poc_programs_typecheck() {
    for (name, src) in POC {
        check_source(src).unwrap_or_else(|e| panic!("{}: {}", name, e));
    }
}

fn find<'a>(p: &'a TProgram, name: &str) -> &'a TFn {
    p.fns.iter().find(|f| f.name == name).unwrap()
}

#[test]
fn histogram_literal_is_u32() {
    let p = check_source(POC[2].1).unwrap();
    let h = find(&p, "histogram");
    assert_eq!(h.ret, Ty::Array(Size::Poly(isc_types::types::Poly::constant(256)), Box::new(Ty::Int(IntKind::U32))));
    // find the literal `1` and check its type
    fn lits(e: &TExpr, out: &mut Vec<Ty>) {
        match &e.kind {
            TExprKind::Lit(Lit::Int(1)) => out.push(e.ty.clone()),
            TExprKind::Call { args, callee, .. } => {
                if let Callee::Value(f) = callee {
                    lits(f, out);
                }
                args.iter().for_each(|a| lits(a, out))
            }
            TExprKind::Lambda(_, b) => lits(b, out),
            TExprKind::Update { target, index, value, .. } => {
                lits(target, out);
                lits(index, out);
                lits(value, out);
            }
            TExprKind::Binary { lhs, rhs, .. } => {
                lits(lhs, out);
                lits(rhs, out);
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    lits(&h.body, &mut out);
    assert_eq!(out, vec![Ty::Int(IntKind::U32)]);
}

#[test]
fn dot_has_runtime_size_check() {
    let p = check_source(POC[1].1).unwrap();
    let main = &p.fns[p.main];
    let mut found = false;
    fn walk(e: &TExpr, found: &mut bool) {
        match &e.kind {
            TExprKind::Call { callee: Callee::Fn { .. }, size_checks, args } => {
                if !size_checks.is_empty() {
                    assert_eq!(size_checks, &vec![1]);
                    *found = true;
                }
                args.iter().for_each(|a| walk(a, found));
            }
            TExprKind::Call { args, .. } => args.iter().for_each(|a| walk(a, found)),
            TExprKind::Lambda(_, b) => walk(b, found),
            TExprKind::Block(stmts, r) => {
                for s in stmts {
                    match s {
                        TStmt::Let(_, v) | TStmt::Expr(v) => walk(v, found),
                    }
                }
                walk(r, found);
            }
            _ => {}
        }
    }
    walk(&main.body, &mut found);
    assert!(found, "the call dot(a, b) must carry a run-time size check");
}

const MAIN: &str = "action main(args: string[k]): i32 {\n    return 0;\n}\n";

fn ok(src: &str) {
    let full = format!("{}{}", src, MAIN);
    check_source(&full).unwrap_or_else(|e| panic!("{}\n=> {}", src, e));
}

fn bad(src: &str, needle: &str) {
    let full = format!("{}{}", src, MAIN);
    match check_source(&full) {
        Ok(_) => panic!("accepted but should fail: {}", src),
        Err(e) => assert!(e.message.contains(needle), "{}\n=> got `{}`, wanted `{}`", src, e.message, needle),
    }
}

#[test]
fn positive_cases() {
    ok("function f(x: f64[n]): f64 {\n    return sum(x) / f64(n);\n}\n");
    ok("function f(x: f64[n]): f64[n] {\n    return x * 2.0 + x;\n}\n");
    ok("function f(x: f64[n]): f64[n] {\n    return 2 * x;\n}\n");
    ok("function f(x: i64[n]): i64 {\n    return reduce(x, (a, b) => a + b * 2, 0);\n}\n");
    ok("function f(x: u8[n]): u64 {\n    return fold(x, 0, (a, v) => a + u64(v));\n}\n");
    ok("function f(x: f64[n]): f64 {\n    const [a, b] = [x[0], x[1]] as const;\n    return a ** 3 + b;\n}\n");
    ok("function f(x: f64[n], y: f64[m]): f64[n] {\n    return x + y;\n}\n");
    ok("function f(n: i64): i64 {\n    if (n > 0) {\n        return -n;\n    } else {\n        return n % 3;\n    }\n}\n");
    ok("function f(x: f64[n]): f64[n] {\n    return map(x, sqrt);\n}\n");
    ok("function f(x: u32[n]): f64[n] {\n    return map(x, f64);\n}\n");
    ok("function twice(g: (f64) => f64, x: f64): f64 {\n    return g(g(x));\n}\n\nfunction f(x: f64): f64 {\n    return twice((v) => v * x, 1.0);\n}\n");
    ok("function f(x: f64[n]): boolean {\n    return x == x && !(x[0] < 1.0);\n}\n");
    ok("function f(): i64[5] {\n    return iota(5);\n}\n");
    ok("function f(x: f64[n]): f64[n] {\n    return fill(n, 1.5) - x;\n}\n");
    ok("function f(x: i64[n]): i64[n] {\n    return x.with(0, -9223372036854775808);\n}\n");
    ok("function f(x: f64[n]): f64 {\n    return dot(x, x);\n}\n");
    ok("function f(x: i32[n]): i32 {\n    return sum(x) + product(x);\n}\n");
    ok("function g<T extends Num>(x: T[n]): T {\n    return sum(x * x);\n}\n\nfunction f(x: f64[n]): f64 {\n    return g(x);\n}\n");
    ok("function f(x: u8): u8 {\n    return 255 + x;\n}\n");
    ok("function f(x: i64): i64 {\n    return 255 + 1000;\n}\n");
    ok("function f(v: u8): f64 {\n    return 2 ** 10 + f64(v) ** v;\n}\n");
}

#[test]
fn type_errors() {
    bad("function f(x: f64): i64 {\n    return x;\n}\n", "type mismatch");
    bad("function f(x: i64): f64 {\n    return x + 1.5;\n}\n", "floating-point literal");
    bad("function f(x: f64[n]): f64 {\n    return x;\n}\n", "type mismatch");
    bad("function f(b: boolean): boolean {\n    return b + 1;\n}\n", "numeric");
    bad("function f(x: f64): f64 {\n    return y;\n}\n", "unknown variable `y`");
    bad("function f(x: f64): f64 {\n    return g(x);\n}\n", "unknown function `g`");
    bad("function f(x: f64): f64 {\n    return sqrt(x, x);\n}\n", "takes 1 argument");
    bad("function f(x: f64[n]): f64 {\n    return x[1.5];\n}\n", "integer");
    bad("function f(x: f64[n]): f64 {\n    return x[true];\n}\n", "integer");
    bad("function f(x: u8): u8 {\n    return x + 256;\n}\n", "out of range");
    // class Ord allows the comparisons (SPEC section 4)
    ok("function largest<T extends Ord>(a: T, b: T): T {\n    if (a > b) {\n        return a;\n    } else {\n        return b;\n    }\n}\n");
    bad("function f<T extends Num>(a: T, b: T): boolean {\n    return a < b;\n}\n", "Ord");
    // nothing fixes the element type of `[]`
    bad("action g(): void {\n    print([]);\n}\n", "element type");
    bad("function f(): i8 {\n    return -129;\n}\n", "out of range");
    bad("function f(x: f64): boolean {\n    return x < true;\n}\n", "type mismatch");
    bad("function f(x: f64[n]): boolean {\n    return x < x;\n}\n", "cannot compare arrays");
    bad("function f(x: f64): f64 {\n    if (x) {\n        return 1.0;\n    } else {\n        return 2.0;\n    }\n}\n", "type mismatch");
    ok("function f(x: f64): f64 {\n    if (x > 0.0) {\n        return 1.0;\n    } else {\n        return 2;\n    }\n}\n");
    // a floating-point exponent: `pow` (SPEC section 10, D108)
    ok("function f(x: f64): f64 {\n    return x ** 2.0;\n}\n");
    ok("function f(x: f32): f32 {\n    return pow(x, x) + fma(x, x, x) + exp(x) + log(x) + sin(x) + cos(x);\n}\n");
    bad("function f(x: i64): i64 {\n    return exp(x);\n}\n", "");
    bad("function g<T>(x: T[n]): T {\n    return sum(x);\n}\n", "Num");
    bad("function f(x: f64): f64 {\n    return print(x);\n}\n", "cannot call the action");
    bad("function f(x: f64[n]): f64 {\n    return filter(x, (v) => v > 0.0);\n}\n", "type mismatch");
    // recursion (SPEC section 11, D102)
    ok("function f(x: f64): f64 {\n    if (x > 1.0) {\n        return f(x / 2.0);\n    } else {\n        return x;\n    }\n}\n");
    ok("function even(n: i64): boolean {\n    if (n == 0) {\n        return true;\n    } else {\n        return odd(n - 1);\n    }\n}\n\nfunction odd(n: i64): boolean {\n    if (n == 0) {\n        return false;\n    } else {\n        return even(n - 1);\n    }\n}\n");
    // a size only in the result is existential (SPEC section 4, D94)
    ok("function f(x: f64[n]): f64[m] {\n    return x;\n}\n");
    bad("function f(x: f64[n]): f64[m] {\n    return 1.0;\n}\n", "found `f64[_]`");
    // records (structural; fields sorted by name)
    ok("function f(t: { price: f64, volume: u32 }): f64 {\n    return t.price * f64(t.volume);\n}\n");
    ok("function f(t: { a: f64, b: i64 }): { b: i64, a: f64 } {\n    return { ...t, a: 1.5 };\n}\n");
    ok("function f(x: { a: f64, b: boolean }[n]): f64 {\n    return x[0].a;\n}\n");
    ok("function f(): boolean {\n    return { a: 1, b: 2.0 } == { b: 2.0, a: 1 };\n}\n");
    bad("function f(t: { a: f64 }): f64 {\n    return t.b;\n}\n", "no field `b`");
    bad("function f(x: f64): f64 {\n    return x.a;\n}\n", "needs a record");
    bad("function f(): { a: i64 } {\n    return { a: 1, a: 2 };\n}\n", "given twice");
    bad("function f(t: { a: i64, a: f64 }): i64 {\n    return 0;\n}\n", "appears twice");
    bad("function f(t: { a: f64 }): f64 {\n    return { ...t, a: true };\n}\n", "type mismatch");
    bad("function f(t: { a: f64 }): { a: f64, b: f64 } {\n    return t;\n}\n", "type mismatch");
    // sum types and match
    ok("type Option<T> = None | Some(T);\n\nfunction f(x: Option<f64>): f64 {\n    return match (x) {\n        Some(v) => v,\n        None => 0.0,\n    };\n}\n");
    ok("type S = A | B(i64);\n\nfunction f(x: S): i64 {\n    return match (x) {\n        B(0) => 1,\n        _ => 2,\n    };\n}\n");
    ok("function f(b: boolean): i64 {\n    return match (b) {\n        true => 1,\n        false => 0,\n    };\n}\n");
    bad("type S = A | B(i64);\n\nfunction f(x: S): i64 {\n    return match (x) {\n        A => 1,\n    };\n}\n", "does not cover the constructor(s) B");
    bad("type S = A | B(i64);\n\nfunction f(x: S): i64 {\n    return match (x) {\n        A => 1,\n        B(0) => 2,\n    };\n}\n", "does not cover the constructor(s) B");
    bad("function f(x: i64): i64 {\n    return match (x) {\n        0 => 1,\n    };\n}\n", "may not cover every value");
    bad("type S = A | B(i64);\n\nfunction f(): S {\n    return C;\n}\n", "unknown variable `C`");
    bad("type S = A | B(i64);\n\nfunction f(): S {\n    return B(1, 2);\n}\n", "takes 1 argument");
    bad("type S = A | B(i64);\n\nfunction f(x: S): i64 {\n    const B(v) = x;\n    return v;\n}\n", "may not match");
    // recursive types (D103)
    ok("type L = Nil | Cons(i64, L);\n\nfunction f(): i64 {\n    return 0;\n}\n");
    bad("type S = A | B;\n\ntype T = B | C;\n\nfunction f(): i64 {\n    return 0;\n}\n", "duplicate constructor `B`");
    bad("type S = A | B;\n\nfunction B(): i64 {\n    return 0;\n}\n", "both a function and a constructor");
    bad("type S = A | B;\n\nfunction f(x: S): boolean {\n    return x < x;\n}\n", "Ord");
    // size parameters
    ok("function f(x: f64[n], k: size): f64[k] {\n    return map(iota(k), (i) => x[i]);\n}\n");
    ok("function f(k: size): i64[k + 1] {\n    return iota(k + 1);\n}\n");
    ok("function g(k: size): i64[k] {\n    return iota(k);\n}\n\nfunction f(x: f64[n]): i64[2 * n] {\n    return g(2 * n);\n}\n");
    ok("function g(k: size): i64[k] {\n    return iota(k);\n}\n\nfunction f(m: i64): i64 {\n    return sum(g(m));\n}\n");
    bad("function g(k: size): i64[k] {\n    return iota(k);\n}\n\nfunction f(m: i64): i64 {\n    return sum(g(m + 1));\n}\n", "bind it with `const` first");
    bad("function f(x: f64[k], k: size): i64 {\n    return 0;\n}\n", "already a size variable");
    bad("function g(k: size): i64[k] {\n    return iota(k);\n}\n\nfunction f(): i64[3] {\n    return g(4);\n}\n", "size mismatch");
    // f32: a type of its own, no implicit conversion to or from f64
    ok("function f(x: f32): f32 {\n    return x * 2.5 + 1;\n}\n");
    ok("function f(x: f32[n]): f32 {\n    return sqrt(sum(x));\n}\n");
    bad("function f(x: f32): f64 {\n    return x;\n}\n", "type mismatch");
    bad("function f(x: i64): i64 {\n    return sqrt(x);\n}\n", "floating-point");
    bad("function f(x: f32): f32 {\n    return x + 1.5f64;\n}\n", "type mismatch");
    bad("function f(x: i64): i64 {\n    x;\n    return x;\n}\n", "only allowed in an `action`");
    bad("function f(x: i64): i64 {\n    return x(1);\n}\n", "not a function");
    bad("function f(): f64 {\n    return sum(zeros());\n}\n", "");
}

#[test]
fn size_errors() {
    bad("function f(x: f64[n]): f64[n + 1] {\n    return x;\n}\n", "size mismatch");
    bad("function f(x: f64[3], y: f64[4]): f64[3] {\n    return x + y;\n}\n", "size mismatch");
    bad("function g(a: f64[n], b: f64[n]): f64 {\n    return 0.0;\n}\n\nfunction f(x: f64[3], y: f64[4]): f64 {\n    return g(x, y);\n}\n", "size mismatch");
    bad("function f(x: f64[n]): f64[3] {\n    return x;\n}\n", "size mismatch");
    bad("function f(): i64[3] {\n    return [1, 2];\n}\n", "size mismatch");
    bad("function f(x: f64[n + 1]): f64 {\n    return 0.0;\n}\n", "must appear alone");
    bad("function f(x: f64[n], y: f64[m]): f64[n] {\n    return y;\n}\n", "size mismatch");
}

#[test]
fn main_shape_and_actions() {
    let e = check_source("function f(): i32 {\n    return 0;\n}\n").unwrap_err();
    assert!(e.message.contains("no `action main"));
    let e = check_source("action main(args: string[k]): i64 {\n    return 0;\n}\n").unwrap_err();
    assert!(e.message.contains("must be declared"));
    let e = check_source(
        "action main(args: string[k]): i32 {\n    const [r, ns] = bench(1, () => print(1));\n    return 0;\n}\n",
    )
    .unwrap_err();
    assert!(e.message.contains("inside a lambda"), "{}", e);
    check_source(
        "action hello(n: i64): void {\n    print(n);\n}\n\naction main(args: string[k]): i32 {\n    hello(3);\n    print(clock());\n    return 0;\n}\n",
    )
    .unwrap();
}

/// Stage 2, phase 1 (SPEC section 15): the examples of sections 3 and 4
/// type-check, except those that need later phases (`require_no_alloc`:
/// phase 4; `[k <= n]T` and `window` with `[n][m]T`: phase 5).
#[test]
fn examples_of_sections_3_and_4() {
    // section 3, without its `require_no_alloc` (phase 4)
    ok("function average(x: f64[n]): f64 {
    return sum(x) / f64(n);
}

function std_dev(x: f64[n]): f64 {
    const m = average(x);
    return sqrt(sum((x - m) ** 2) / f64(n));
}

function histogram(values: u8[n]): u32[256] {
    return fold(values, zeros(), (c, v) => require_unique(c.with(v, c[v] + 1)));
}
");
    // section 4
    ok("type Option<T> = None | Some(T);

type Result<T, E> = Ok(T) | Err(E);

function f(r: Result<f64, i64>): Option<f64> {
    return match (r) {
        Ok(x) => Some(x),
        Err(_) => None,
    };
}
");
    ok("function f(r: { price: f64, volume: u32 }): { price: f64, volume: u32 } {\n    return { ...r, price: 2.0 };\n}\n");
    ok("function first<T>(x: T[n]): T {\n    return x[0];\n}\n");
    ok("function total<T extends Num>(x: T[n]): T {\n    return reduce(x, (+), 0);\n}\n");
    ok("function f(x: u32[256], c: u8[n]): u32 {\n    return x[c[0]] + 1;\n}\n");
    ok("function f(n: f64[k]): i32 {\n    return i32(n[0]);\n}\n");
}

