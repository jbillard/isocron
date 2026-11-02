//! Stage 2, phase 3 (SPEC section 11): recursive functions, not inlined,
//! compiled as functions; a direct tail call is a loop; a stack overflow is
//! a panic (DECISIONS D102). Each program is compared with the interpreter
//! bit for bit, with 1 and 4 threads.

mod common;
use common::*;

/// Compiles `src`, runs it with the interpreter and compiled with 1 and 4
/// threads on `inputs` plus an output file, and compares the outputs and the
/// printed text.
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

pub const RECURSIVE: &str = "function fact(n: i64): i64 {
    if (n <= 1) {
        return 1;
    } else {
        return n * fact(n - 1);
    }
}

function fib(n: i64): i64 {
    if (n < 2) {
        return n;
    } else {
        return fib(n - 1) + fib(n - 2);
    }
}

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

function halve_sum(x: f64[n]): f64 {
    if (n <= 4) {
        return sum(x);
    } else {
        return halve_sum(x.slice(0, n / 2)) + halve_sum(x.slice(n / 2, n));
    }
}

function doubled(x: f64[n], k: i64): f64[n] {
    if (k == 0) {
        return x;
    } else {
        return doubled(map(x, (v) => v * 1.5), k - 1);
    }
}

function count<T>(x: T[n], i: i64): i64 {
    if (i == n) {
        return 0;
    } else {
        return 1 + count(x, i + 1);
    }
}

function walk(x: f64[n], i: i64, acc: f64): f64 {
    if (i == n) {
        return acc;
    } else {
        return walk(x, i + 1, acc + x[i] * 0.5);
    }
}

function squares_plus(x: f64[n], k: i64): f64 {
    if (k == 0) {
        return sum(map(x, (v) => v * v));
    } else {
        return squares_plus(x, k - 1) + 1.0;
    }
}

action main(args: string[k]): i32 {
    const x = readF64(args[0]);
    const small = map(iota(9), (i) => f64(i));
    const d = doubled(small, 7);
    const fibs = map(iota(20), (i) => f64(fib(i)));
    const deep = map(iota(4), (i) => f64(count(small, i)));
    print(even(1001));
    writeF64(args[1], [f64(fact(20)), sum(fibs), halve_sum(x), sum(x), sum(d), sum(deep), f64(count(fibs, 0)), walk(x, 0, 0.0), squares_plus(x, 3)]);
    return 0;
}
";

#[test]
fn recursive_functions() {
    for (i, n) in [0usize, 5, 1000, 70_001].iter().enumerate() {
        let x = write_f64s(&format!("rec_x{}.f64", n), &random_f64s(1800 + i as u64, *n));
        same_everywhere(RECURSIVE, &format!("recursive{}", n), &[&x]);
    }
}

/// A recursive function called by a parallel `map`, with a parallel
/// `reduce` inside: the inner loop runs in the task (D102).
#[test]
fn recursion_inside_parallel_loops() {
    let src = "function total(x: f64[n], k: i64): f64 {
    if (k == 0) {
        return sum(x);
    } else {
        return total(x, k - 1) + 0.25;
    }
}

action main(args: string[k]): i32 {
    const x = readF64(args[0]);
    const r = map(iota(64), (i) => total(x, i % 5));
    writeF64(args[1], r);
    return 0;
}
";
    let x = write_f64s("rec_par.f64", &random_f64s(1900, 70_001));
    same_everywhere(src, "rec_parallel", &[&x]);
}

/// SPEC section 11: a direct tail call is a loop, with no growth of the
/// stack, also when it passes a new array each time.
#[test]
fn deep_tail_recursion_is_a_loop() {
    let src = "function count_down(n: i64, acc: i64): i64 {
    if (n == 0) {
        return acc;
    } else {
        return count_down(n - 1, acc + n % 7);
    }
}

function doubled(x: f64[n], k: i64): f64[n] {
    if (k == 0) {
        return x;
    } else {
        return doubled(map(x, (v) => v * 1.0000001), k - 1);
    }
}

action main(args: string[k]): i32 {
    const m = parseI64(args[0]);
    print(count_down(m, 0));
    print(sum(doubled([1.0, 2.0, 3.0], m)));
    writeF64(args[1], [1.0]);
    return 0;
}
";
    same_everywhere(src, "rec_tail", &["3000000"]);
}

/// A recursion too deep for the stack is a panic, with the same message
/// compiled and interpreted.
#[test]
fn stack_overflow_is_a_panic() {
    let src = "function grow(x: f64[n], k: i64): f64 {
    if (k == 0) {
        return sum(x);
    } else {
        return grow(x, k - 1) + x[0];
    }
}

action main(args: string[k]): i32 {
    print(grow([1.0, 2.0], parseI64(args[0])));
    return 0;
}
";
    let want = "panic at line 1, column 10: stack overflow: too many nested calls of `grow`";
    let bin = compile(src, "rec_overflow");
    let r = run_bin_threads(&bin, &["100000000"], Some(1));
    assert_eq!(r.code, 101, "{}", r.stderr);
    assert!(r.stderr.contains(want), "{}", r.stderr);
    assert!(interp_panic(src, &["100000000"]).contains(want));
    // a depth the stack holds
    let r = run_bin_threads(&bin, &["100000"], Some(1));
    assert_eq!((r.code, r.stdout.trim()), (0, "100003"), "{}", r.stderr);
}

/// Recursive sum types (SPEC section 8, D103): nodes on the heap with a
/// reference count; built, matched, shared, printed.
pub const LISTS: &str = "type List = Nil | Cons(i64, List);

type Tree = Leaf(f64) | Node(Tree, Tree);

function build(n: i64, acc: List): List {
    if (n == 0) {
        return acc;
    } else {
        return build(n - 1, Cons(n, acc));
    }
}

function length(l: List): i64 {
    return match (l) {
        Nil => 0,
        Cons(h, t) => 1 + length(t),
    };
}

function total(l: List, acc: i64): i64 {
    return match (l) {
        Nil => acc,
        Cons(h, t) => total(t, acc + h),
    };
}

function tree(d: i64): Tree {
    if (d == 0) {
        return Leaf(f64(d) + 0.5);
    } else {
        return Node(tree(d - 1), tree(d - 1));
    }
}

function leaves(t: Tree): f64 {
    return match (t) {
        Leaf(v) => v,
        Node(a, b) => leaves(a) + leaves(b),
    };
}

function tail_of(l: List): List {
    return match (l) {
        Nil => Nil,
        Cons(h, t) => t,
    };
}

action main(args: string[k]): i32 {
    const n = parseI64(args[0]);
    const l = build(n, Nil);
    print(length(l));
    print(total(l, 0));
    const shared = tail_of(l);
    const other = Cons(100, shared);
    print(total(other, 0));
    print(build(3, Nil));
    print(leaves(tree(10)));
    print(tree(1));
    writeF64(args[1], [1.0]);
    return 0;
}
";

#[test]
fn recursive_types() {
    // `length` is not a tail call: a depth the interpreter holds, built
    // with or without optimizations (D102, D127)
    for n in ["0", "10", "500000"] {
        same_everywhere(LISTS, &format!("lists{}", n), &[n]);
    }
}

/// A list read by the tasks of a parallel `map`: the counts change
/// atomically in the tasks.
#[test]
fn recursive_types_in_parallel_tasks() {
    let src = "type List = Nil | Cons(i64, List);

function build(n: i64, acc: List): List {
    if (n == 0) {
        return acc;
    } else {
        return build(n - 1, Cons(n, acc));
    }
}

function total(l: List, acc: i64): i64 {
    return match (l) {
        Nil => acc,
        Cons(h, t) => total(t, acc + h),
    };
}

function skip(l: List, k: i64): List {
    if (k == 0) {
        return l;
    } else {
        return match (l) {
            Nil => Nil,
            Cons(h, t) => skip(t, k - 1),
        };
    }
}

action main(args: string[k]): i32 {
    const l = build(5000, Nil);
    const r = map(iota(64), (i) => f64(total(skip(l, i * 10), 0)));
    print(sum(r));
    writeF64(args[0], r);
    return 0;
}
";
    same_everywhere(src, "lists_parallel", &[]);
}

/// Uniqueness in the branches of an `if` (D105), against the interpreter.
#[test]
fn moves_in_branches() {
    let src = "function pick(x: f64[n], c: boolean): f64[n] {
    const a = map(x, (v) => v + 1.0);
    if (c) {
        return a.with(0, 100.0);
    } else {
        return a;
    }
}

function half(x: f64[n], c: boolean): f64 {
    const a = map(x, (v) => v * 2.0);
    if (c) {
        return sum(a.with(1, 5.0));
    } else {
        return sum(x);
    }
}

function nested(x: f64[n], k: i64): f64[n] {
    const a = map(x, (v) => v - 1.0);
    if (k == 0) {
        return a;
    } else if (k == 1) {
        return a.with(0, 7.0);
    } else {
        return map(x, (v) => v);
    }
}

action main(args: string[k]): i32 {
    const x = map(iota(5), (i) => f64(i));
    print(pick(x, true));
    print(pick(x, false));
    print(half(x, true));
    print(half(x, false));
    print(nested(x, 0));
    print(nested(x, 1));
    print(nested(x, 2));
    writeF64(args[0], pick(x, true));
    return 0;
}
";
    same_everywhere(src, "branches", &[]);
}

/// A function chosen at run time (SPEC section 11, D107): each call is a
/// branch on the condition, computed once where the function is bound.
#[test]
fn functions_chosen_at_run_time() {
    let src = "function double(v: f64): f64 {
    return v * 2.0;
}

function square(v: f64): f64 {
    return v * v;
}

function apply_all(x: f64[n], f: (f64) => f64): f64 {
    return sum(map(x, f));
}

action main(args: string[k]): i32 {
    const k = parseI64(args[0]);
    const x = map(iota(10), (i) => f64(i));
    const f = k > 0 ? double : square;
    print(sum(map(x, f)));
    print(apply_all(x, f));
    const offset = 0.5;
    const g = k > 1 ? (v) => v + offset : k > 0 ? double : (v) => v - offset;
    print(map(x, g));
    print(sum(map(x, k % 2 == 0 ? square : double)));
    writeF64(args[1], map(x, g));
    return 0;
}
";
    for k in ["0", "1", "2", "3"] {
        same_everywhere(src, &format!("chosen{}", k), &[k]);
    }
    let p = iscc::frontend("d.isc", src).unwrap();
    let switches = isc_ir::report::entries(&p).into_iter().filter(|e| e.kind == "switch").count();
    assert_eq!(switches, 4);
}

/// A function chosen by `match` (D127): the choice of each arm, the first
/// that matches, computed once where the function is bound; the variables
/// of a pattern of one level read by the function (D132); an arm whose
/// pattern binds a variable below its first level is refused.
const MATCHED: &str = "type Op = Double | Square | Shift | Halve;

function double(v: f64): f64 {
    return v * 2.0;
}

function square(v: f64): f64 {
    return v * v;
}

action main(args: string[k]): i32 {
    const k = parseI64(args[0]);
    const x = map(iota(6), (i) => f64(i));
    const op = k == 0 ? Double : k == 1 ? Square : k == 2 ? Shift : Halve;
    const f = match (op) {
        Double => double,
        Square => square,
        Shift => (v) => v + 1.0,
        Halve => (v) => v * 0.5,
    };
    print(map(x, f));
    const g = match (k) {
        0 => square,
        5 => double,
        _ => (v) => v - 1.0,
    };
    print(sum(map(x, g)));
    writeF64(args[1], map(x, f));
    return 0;
}
";

#[test]
fn functions_chosen_by_match() {
    for k in ["0", "1", "2", "3", "5"] {
        same_everywhere(MATCHED, &format!("matched{}", k), &[k]);
    }
    for k in ["0", "1", "2"] {
        same_everywhere(MATCHED_BINDS, &format!("matched_binds{}", k), &[k]);
    }
    // variables bound below the first level of the pattern (D152)
    let src = "type L = Nil | Cons(i64, L);\n\naction main(args: string[k]): i32 {\n    const l = Cons(1, Cons(2, Nil));\n    const f = match (l) {\n        Cons(a, Cons(b, t)) => (v) => v + a + b,\n        _ => (v) => v,\n    };\n    print(map([1], f));\n    return 0;\n}\n";
    assert_eq!(interp_ok(src, &[]), "[4]\n");
    let bin = compile(src, "matched_deep");
    assert_eq!(run_bin(&bin, &[]).stdout, "[4]\n");
}

/// A recursive function with a function parameter (D128): one
/// specialization per function given (a function of the program, a lambda
/// without captures, an operator, a primitive), the function passed again
/// to the recursive calls; tail calls stay loops.
const RECURSIVE_HIGHER: &str = "function double(v: f64): f64 {
    return v * 2.0;
}

function apply_n(f: (f64) => f64, x: f64, n: i64): f64 {
    if (n == 0) {
        return x;
    } else {
        return apply_n(f, f(x), n - 1);
    }
}

function fold_list(op: (i64, i64) => i64, acc: i64, k: i64): i64 {
    if (k == 0) {
        return acc;
    } else {
        return op(k, fold_list(op, acc, k - 1));
    }
}

function sum_tree(f: (f64) => f64, x: f64[n]): f64 {
    if (n <= 2) {
        return sum(map(x, f));
    } else {
        return sum_tree(f, x.slice(0, n / 2)) + sum_tree(f, x.slice(n / 2, n));
    }
}

action main(args: string[k]): i32 {
    const n = parseI64(args[0]);
    print(apply_n(double, 1.0, n));
    print(apply_n((v) => v * 0.5 + 1.0, 3.0, n));
    print(apply_n(sqrt, 65536.0, 3));
    print(fold_list((+), 0, n));
    print(fold_list(max, 0, n));
    print(fold_list((a, b) => a * 3 + b, 1, 5));
    const x = map(iota(9), (i) => f64(i));
    print(sum_tree(double, x));
    print(sum_tree((v) => v * v, x));
    writeF64(args[1], [apply_n(double, 1.5, 4)]);
    return 0;
}
";

#[test]
fn recursive_functions_with_function_parameters() {
    for n in ["0", "5", "30"] {
        same_everywhere(RECURSIVE_HIGHER, &format!("rec_higher{}", n), &[n]);
    }
}

/// `==` on values of a recursive type (D129): structural, IEEE equality of
/// the floats (NaN differs from itself, 0.0 equals -0.0), the last node by a
/// tail call (a list of 1 000 000 compared); the same as the interpreter.
const RECURSIVE_EQ: &str = "type List = Nil | Cons(i64, List);

type Tree = Leaf(f64) | Node(Tree, Tree);

type Bag = Empty | Item([i64, boolean], f64, Bag);

function build(n: i64, acc: List): List {
    if (n == 0) {
        return acc;
    } else {
        return build(n - 1, Cons(n, acc));
    }
}

function tree(d: i64, v: f64): Tree {
    if (d == 0) {
        return Leaf(v);
    } else {
        return Node(tree(d - 1, v), tree(d - 1, v + 1.0));
    }
}

action main(args: string[k]): i32 {
    const n = parseI64(args[0]);
    const a = build(n, Nil);
    const b = build(n, Nil);
    const c = build(n, Cons(0, Nil));
    print([a == b, a == c, Nil == Nil, Cons(1, Nil) == Cons(2, Nil), Cons(1, Nil) == Nil]);
    const t = tree(8, 0.5);
    print([t == tree(8, 0.5), t == tree(8, 1.5), Leaf(0.0 / 0.0) == Leaf(0.0 / 0.0), Leaf(0.0) == Leaf(-0.0)]);
    const bag = Item([1, true] as const, 2.5, Item([2, false] as const, 0.0, Empty));
    print([bag == Item([1, true] as const, 2.5, Item([2, false] as const, -0.0, Empty)), bag == Item([1, true] as const, 2.5, Empty), bag == Item([1, false] as const, 2.5, Item([2, false] as const, 0.0, Empty))]);
    writeF64(args[1], [1.0]);
    return 0;
}
";

#[test]
fn equality_of_recursive_values() {
    for n in ["0", "10", "1000000"] {
        same_everywhere(RECURSIVE_EQ, &format!("rec_eq{}", n), &[n]);
    }
}

/// Arrays of a recursive type (D130): literals, `map`s that build nodes or
/// return elements, `with`, `filter`, `fold`, slices, `==`, a recursive
/// function over the array, tuples of nodes; the same as the interpreter.
pub const NODE_ARRAYS: &str = "type List = Nil | Cons(i64, List);

function len(l: List, acc: i64): i64 {
    return match (l) {
        Nil => acc,
        Cons(h, t) => len(t, acc + 1),
    };
}

function build(n: i64, acc: List): List {
    if (n == 0) {
        return acc;
    } else {
        return build(n - 1, Cons(n, acc));
    }
}

function lists(m: size): List[m] {
    return map(iota(m), (i) => build(i % 7, Nil));
}

function total(x: List[n], i: i64): i64 {
    if (i == n) {
        return 0;
    } else {
        return len(x[i], 0) + total(x, i + 1);
    }
}

action main(args: string[k]): i32 {
    const m = parseI64(args[0]);
    const a = [Nil, Cons(1, Nil), Cons(2, Cons(3, Nil))];
    print(a);
    print(map(a, (l) => len(l, 0)));
    const b = lists(m);
    print(sum(map(b, (l) => len(l, 0))));
    const c = b.with(1, Cons(100, b[3]));
    print(c[1]);
    print(b[1]);
    const d = filter(b, (l) => len(l, 0) > 3);
    print(sum(map(d, (l) => len(l, 0))));
    print(fold(b, 0, (acc, l) => acc + len(l, 0)));
    print(total(b.slice(0, 5), 0));
    print([a == a, a == [Nil, Cons(1, Nil), Cons(2, Nil)]]);
    const pairs = map(iota(3), (i) => [i, build(i, Nil)] as const);
    print(pairs);
    writeF64(args[1], map(b, (l) => f64(len(l, 0))));
    return 0;
}
";

#[test]
fn arrays_of_recursive_types() {
    for m in ["10", "70001"] {
        same_everywhere(NODE_ARRAYS, &format!("node_arrays{}", m), &[m]);
    }
}

/// A lambda that captures values given to a recursive function (D131): its
/// captured values are parameters of the specialization, passed again to
/// the recursive calls (scalars, an array, two captures, values computed at
/// run time).
const RECURSIVE_CAPTURES: &str = "function apply_n(f: (f64) => f64, x: f64, n: i64): f64 {
    if (n == 0) {
        return x;
    } else {
        return apply_n(f, f(x), n - 1);
    }
}

function count_if(p: (i64) => boolean, k: i64): i64 {
    if (k == 0) {
        return 0;
    } else {
        return (p(k) ? 1 : 0) + count_if(p, k - 1);
    }
}

action main(args: string[k]): i32 {
    const n = parseI64(args[0]);
    const c = 2.5;
    const w = map(iota(4), (i) => f64(i) * 0.25);
    print(apply_n((v) => v * c, 1.0, n));
    print(apply_n((v) => v + sum(w), 0.0, n));
    const d = 0.5;
    print(apply_n((v) => v * c + d, 1.0, n));
    print([apply_n((v) => v + c, 0.0, 3), apply_n((v) => v + d, 0.0, 3)]);
    const m = n % 4 + 2;
    print(count_if((k) => k % m == 0, 50));
    writeF64(args[1], [apply_n((v) => v * w[1] + c, 1.0, n)]);
    return 0;
}
";

#[test]
fn recursive_functions_given_lambdas_with_captures() {
    for n in ["0", "4", "25"] {
        same_everywhere(RECURSIVE_CAPTURES, &format!("rec_captures{}", n), &[n]);
    }
}

/// The functions chosen by `match` that read the variables of their
/// pattern (D132): a payload, the components of a tuple (with a literal),
/// the head of a list (read from `Nil` too: a zero slot), the whole value.
const MATCHED_BINDS: &str = "type Op = Scale(f64) | Shift(f64) | Same;

type List = Nil | Cons(i64, List);

action main(args: string[k]): i32 {
    const k = parseI64(args[0]);
    const x = map(iota(5), (i) => f64(i));
    const op = k == 0 ? Scale(2.5) : k == 1 ? Shift(-1.0) : Same;
    const f = match (op) {
        Scale(c) => (v) => v * c,
        Shift(d) => (v) => v + d,
        Same => (v) => v,
    };
    print(map(x, f));
    const pair = [k, 0.5] as const;
    const g = match (pair) {
        [0, w] => (v) => v * w,
        [n, w] => (v) => v + f64(n) * w,
    };
    print(sum(map(x, g)));
    const l = k == 1 ? Cons(7, Cons(8, Nil)) : Nil;
    const h = match (l) {
        Nil => (v) => v,
        Cons(head, rest) => (v) => v + f64(head),
    };
    print(map(x, h));
    const m = match (k) {
        0 => (v) => v,
        z => (v) => v * f64(z),
    };
    print(map(x, m));
    writeF64(args[1], map(x, f));
    return 0;
}
";
