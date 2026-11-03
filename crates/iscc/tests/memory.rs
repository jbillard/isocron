//! No leak and no invalid access (SPEC section 15, stage 2, phase 3: "aucune
//! fuite (Valgrind ou AddressSanitizer)"): programs compiled for AVX2
//! (Valgrind does not decode AVX-512), with glibc's allocator so that
//! Valgrind sees every block, run under `valgrind --leak-check=full`.
//! Skipped when Valgrind is not installed.

mod common;
use common::*;

fn valgrind() -> Option<std::path::PathBuf> {
    let p = std::path::PathBuf::from("/usr/bin/valgrind");
    p.is_file().then_some(p)
}

fn clean(src: &str, name: &str, args: &[&str]) {
    let Some(vg) = valgrind() else {
        eprintln!("valgrind not installed: {} skipped", name);
        return;
    };
    let opts = isc_codegen::Options { isa: isc_codegen::Isa::Avx2, ..isc_codegen::Options::default() };
    let bin = compile_with(src, name, opts);
    let out = std::process::Command::new(vg)
        .args(["--error-exitcode=9", "--leak-check=full", "--errors-for-leak-kinds=definite,indirect", "-q"])
        .arg(&bin)
        .args(args)
        .env("ISC_ALLOC", "glibc")
        .env("ISC_THREADS", "2")
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "{} under valgrind:\n{}", name, err);
}

#[test]
fn poc_programs_free_everything() {
    let a = write_f64s("mem_a.f64", &random_f64s(1, 70_001));
    let b = write_f64s("mem_b.f64", &random_f64s(2, 70_001));
    let u = tmp("mem_u.u8");
    std::fs::write(&u, (0..70_001u32).map(|i| (i * 7 % 256) as u8).collect::<Vec<u8>>()).unwrap();
    let u = u.to_string_lossy().to_string();
    let o = tmp("mem.out").to_string_lossy().to_string();
    clean(SUM, "mem_sum", &[&a, &o, "2"]);
    clean(DOT, "mem_dot", &[&a, &b, &o, "2"]);
    clean(STD, "mem_std", &[&a, &o, "2"]);
    clean(HIST, "mem_hist", &[&u, &o, "2"]);
}

#[test]
fn phase5_primitives_free_everything() {
    let src = "function positives(x: f64[n]): f64[k <= n] {
    return filter(x, (v) => v > 0.0);
}

action main(args: string[k]): i32 {
    const x = map(iota(9000), (i) => f64(i * 37 % 101) - 50.0);
    const w = window(x, 3);
    print(sum(map(w, (r) => sum(r))));
    print(w[2]);
    print([[1, 2], [3, 4]]);
    print([[1, 2], [3, 4]][1]);
    print(sum(window(map(iota(6), (i) => f64(i) * 2.0), 3)[1]));
    print(sum(positives(x)));
    print(sum(x.slice(10, 20)));
    print(scan(iota(5000), (+), 0)[4999]);
    print(min(x) + max(x));
    print(map(zip(iota(3), [1.5, 2.5, 3.5]), (p) => {
        const [i, v] = p;
        return f64(i) * v;
    }));
    return 0;
}
";
    clean(src, "mem_phase5", &[]);
}

/// A row given to a literal: copied when the variable is used again, moved
/// at its last use (before D96, the row was freed by the literal and again
/// at the end of its scope).
#[test]
fn rows_given_to_a_literal_are_copied_or_moved() {
    let src = "action main(args: string[k]): i32 {
    const a = [1, 2];
    const m = [a, [3, 4]];
    print(m);
    print(a);
    const b = map(iota(3), (i) => i * 10);
    const n = [[7, 8, 9], b];
    print(n);
    return 0;
}
";
    clean(src, "mem_rows", &[]);
}

/// `main` owns `args`: copied by an update while it is used again, moved at
/// its last use, and then not freed by `main` (D96).
#[test]
fn args_copied_then_moved() {
    let src = "action main(args: string[k]): i32 {
    const b = args.with(0, args[1]);
    print(b);
    print(args);
    const c = args.with(1, args[0]);
    print(c);
    return 0;
}
";
    clean(src, "mem_args", &["p", "q"]);
}

/// Slices are views (rule F4, D97): shared, sliced again, copied when a
/// value must own them, used after their source was updated in place.
#[test]
fn slices_are_views_and_free_nothing() {
    let src = "action main(args: string[k]): i32 {
    const x = map(iota(20), (i) => f64(i) * 1.5);
    const s = x.slice(2, 8);
    print(s);
    print(sum(s) + sum(x.slice(10, 20)));
    print(x.slice(3, 9).slice(1, 4));
    const t = s.with(0, 99.0);
    print(t);
    print(s);
    const p = [x.slice(0, 2), 7] as const;
    print(p);
    const y = x.with(5, -1.0);
    print(y.slice(4, 7));
    const q = map(iota(6), (i) => [i, f64(i)] as const);
    print(q.slice(1, 3));
    return 0;
}
";
    clean(src, "mem_slices", &[]);
}

const ZIPS: &str = "function prod(p: [f64, f64]): f64 {
    const [u, v] = p;
    return u * v;
}

action main(args: string[k]): i32 {
    const a = readF64(args[0]);
    const b = readF64(args[1]);
    const z = zip(a, b);
    const s1 = sum(map(z, prod));
    const s2 = sum(map(iota(3), (i) => prod(z[i])));
    const c = map(a, (v) => v + 1.0);
    const w = zip(map(b, (v) => v * 3.0), c);
    const s3 = sum(map(w, prod));
    const s4 = sum(map(w, (p) => prod(p) + 1.0));
    const zz = zip(z, c);
    print(zz[1]);
    print(z[2]);
    writeF64(args[2], [s1, s2, s3, s4]);
    return 0;
}
";

/// Shared zips copied at each use (rule F4, D97) free everything.
#[test]
fn shared_zips_free_everything() {
    let a = write_f64s("mem_za.f64", &random_f64s(31, 5000));
    let b = write_f64s("mem_zb.f64", &random_f64s(32, 5000));
    let o = tmp("mem_z.out").to_string_lossy().to_string();
    clean(ZIPS, "mem_zips", &[&a, &b, &o]);
}

const FILTERS: &str = "function pos(x: f64[n]): f64 {
    return sum(filter(x, (v) => v > 0.25));
}

function pos_sq(x: f64[n]): f64 {
    return sum(map(filter(x, (v) => v > 0.25), (v) => v * v));
}

function big(x: f64[n]): f64[k <= n] {
    return map(filter(x, (v) => v > 0.5), (v) => v * 10.0);
}

function cnt(x: f64[n]): i64 {
    return fold(filter(x, (v) => v < 0.1), 0, (c, v) => c + 1);
}

function mx(x: i64[n]): i64 {
    return max(filter(x, (v) => v % 3 == 0));
}

function odd(x: f64[n]): f64 {
    return reduce(filter(x, (v) => v > 0.3), (a, b) => a * 0.5 + b, 1.0);
}

function lo(x: f64[n]): f64 {
    return min(map(filter(x, (v) => v > 0.2), (v) => v * 2.0));
}

function third(x: f64[n]): f64 {
    return filter(x, (v) => v > 0.5)[2];
}

action main(args: string[k]): i32 {
    const x = readF64(args[0]);
    const y = map(x, (v) => i64(v * 1000.0));
    const b = big(x);
    const t = count(x, (v) => v > 0.5) > 2 ? third(x) : 0.0;
    writeF64(args[1], [pos(x), pos_sq(x), sum(b), f64(cnt(x)), f64(mx(y)), odd(x), lo(x), t]);
    return 0;
}
";

/// Fused filters (rule F2, D99) free everything.
#[test]
fn fused_filters_free_everything() {
    let x = write_f64s("mem_f2.f64", &random_f64s(41, 9000));
    let o = tmp("mem_f2.out").to_string_lossy().to_string();
    clean(FILTERS, "mem_filters", &[&x, &o]);
}

const COLUMNS: &str = "function stats(x: f64[n]): [f64, f64, f64, f64] {
    const s = sum(x);
    const q = sum(map(x, (v) => v * v));
    const hi = max(x);
    const lo = min(map(x, (v) => v - 1.0));
    return [s, q, hi, lo] as const;
}

function spread(x: f64[n]): f64 {
    return max(x) - min(x);
}

function dep(x: f64[n]): f64 {
    const m = sum(x);
    return sum(map(x, (v) => v - m));
}

function ints(y: i64[n]): i64 {
    return sum(y) * 3 + max(y) - min(map(y, (v) => v * 2));
}

action main(args: string[k]): i32 {
    const x = readF64(args[0]);
    const y = map(x, (v) => i64(v * 1000.0));
    const [s, q, hi, lo] = stats(x);
    writeF64(args[1], [s, q, hi, lo, spread(x), dep(x), f64(ints(y)), sum(x) + max(x)]);
    return 0;
}
";

/// Reductions in columns (rule F3, D100) free their block results.
#[test]
fn reductions_in_columns_free_everything() {
    let x = write_f64s("mem_f3.f64", &random_f64s(51, 70_001));
    let o = tmp("mem_f3.out").to_string_lossy().to_string();
    clean(COLUMNS, "mem_columns", &[&x, &o]);
}

const SLICED_PRODUCERS: &str = "function mid(x: f64[n]): f64 {
    return sum(map(x, (v) => v * 3.0).slice(2, n - 2));
}

function part(x: f64[n], a: i64, b: i64): f64[k] {
    return map(x, (v) => v + 1.0).slice(a, b);
}

function pairs(x: f64[n]): f64 {
    const q = map(x, (v) => [v, v * 2.0] as const).slice(1, 3);
    return fold(q, 0.0, (c, p) => {
        const [u, w] = p;
        return c + u * w;
    });
}

function wsum(x: f64[n]): f64 {
    return sum(map(window(map(x, (v) => v * 0.5), 3), (r) => sum(r)));
}

action main(args: string[k]): i32 {
    const x = readF64(args[0]);
    const p = part(x, 1, 4);
    const sq = map(iota(10), (i) => i * i).slice(3, 6);
    writeF64(args[1], [mid(x), sum(p), pairs(x), wsum(x), f64(sum(sq))]);
    return 0;
}
";

/// Slices of producers (D101) free everything.
#[test]
fn slices_of_producers_free_everything() {
    let x = write_f64s("mem_sp.f64", &random_f64s(61, 5000));
    let o = tmp("mem_sp.out").to_string_lossy().to_string();
    clean(SLICED_PRODUCERS, "mem_sliced", &[&x, &o]);
}

/// Recursive functions (D102): arrays passed and returned, a tail call that
/// moves a new array into its loop each time (the previous one freed).
#[test]
fn recursion_frees_everything() {
    let src = "function halve_sum(x: f64[n]): f64 {
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

function keep(x: f64[n], k: i64): f64 {
    const t = map(x, (v) => v + 1.0);
    if (k == 0) {
        return sum(t);
    } else {
        return keep(x, k - 1);
    }
}

function fresh(k: i64): f64[3] {
    if (k == 0) {
        return [1.0, 2.0, 3.0];
    } else {
        return map(fresh(k - 1), (v) => v * 2.0);
    }
}

action main(args: string[k]): i32 {
    const x = map(iota(1000), (i) => f64(i));
    print(halve_sum(x));
    print(doubled([1.0, 2.0, 3.0], 50));
    print(doubled(x, 0)[5]);
    print(keep(x, 20));
    print(fresh(10));
    return 0;
}
";
    clean(src, "mem_recursion", &[]);
}

/// Recursive types (D103): every node freed once, shared tails, a tree, and
/// a list read by parallel tasks (atomic counts).
#[test]
fn recursive_types_free_everything() {
    clean(LISTS_SRC, "mem_lists", &["1000"]);
    clean(PAR_SRC, "mem_lists_par", &[]);
}

const LISTS_SRC: &str = "type List = Nil | Cons(i64, List);

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
    return 0;
}
";

const PAR_SRC: &str = "type List = Nil | Cons(i64, List);

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
    return 0;
}
";

/// Moves in the branches of an `if`, and frees in the branches that do not
/// use the variable (D105).
#[test]
fn moves_in_branches_free_everything() {
    clean(BRANCHES, "mem_branches", &[]);
}

pub const BRANCHES: &str = "function pick(x: f64[n], c: boolean): f64[n] {
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
    return 0;
}
";

/// Arrays of nodes of a recursive type (D130): every node freed once, with
/// two threads (each copy of an element is one owner more).
#[test]
fn arrays_of_nodes_free_everything() {
    clean(NODE_ARRAYS, "node_arrays_vg", &["1000", "/dev/null"]);
}

const NODE_ARRAYS: &str = "type List = Nil | Cons(i64, List);

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

/// The counts of the nodes of a task are atomic only for the recursive
/// types the task may share with another thread (D175): a tree it builds
/// and drops itself is counted plainly; a tree it captures, atomically.
#[test]
fn atomic_counts_only_for_shared_nodes() {
    let local = "type Tree = Leaf | Node(Tree, Tree);

function make(d: i64): Tree {
    if (d == 0) return Leaf;
    return Node(make(d - 1), make(d - 1));
}

function nodes(t: Tree): i64 {
    return match (t) {
        Leaf => 1,
        Node(l, r) => 1 + nodes(l) + nodes(r),
    };
}

action main(args: string[k]): i32 {
    print(sum(map(iota(10000), (i) => nodes(make(i % 5)))));
    return 0;
}
";
    let shared = local.replace("print(sum(map(iota(10000), (i) => nodes(make(i % 5)))));", "const t = make(4);\n    print(sum(map(iota(10000), (i) => nodes(Node(t, make(i % 3))))));");
    for (src, name, atomic, want) in [(local, "atomic_local", false, "114000\n"), (shared.as_str(), "atomic_shared", true, "356664\n")] {
        let ll = tmp(&format!("{}.ll", name));
        let p = iscc::frontend("a.isc", src).unwrap();
        let bin = tmp(&format!("{}_bin", name));
        iscc::build(&p, &iscc::BuildOptions { output: bin.clone(), emit_llvm: Some(ll.clone()), ..Default::default() }).unwrap();
        let text = std::fs::read_to_string(&ll).unwrap();
        assert_eq!(text.contains("atomicrmw"), atomic, "{}", name);
        assert_eq!(run_bin_threads(&bin, &[], Some(4)).stdout, want, "{}", name);
        assert_eq!(interp_ok(src, &[]), want, "{} interpreted", name);
    }
}

#[test]
fn literal_arrays_only_read_live_on_the_stack() {
    // D191: an array literal of scalars that its scope only reads (an
    // element, its length, the source of a map, reduce or fold, a copy) is
    // on the stack, without allocation; returned, viewed, passed to a
    // function or updated, it stays on the heap. The results of the
    // interpreter, nothing lost or freed twice under Valgrind.
    let src = "function weights(): f64[3] {
    const s = 2.0;
    return [s, 0.5 * s, 0.25 * s];
}
function total(x: f64[n]): f64 {
    return sum(x);
}
function steps(acc: f64, k: i64): f64 {
    if (k == 0) return acc;
    const w = weights();
    const v = fold(iota(3), acc, (a, i) => a + w[i] * f64(k % 7));
    const m = sum(map(w, (x) => x * 2.0)) + reduce(w, (a, b) => Math.max(a, b), 0.0) + f64(w.length);
    return steps(v + m * 0.001, k - 1);
}
action main(args: string[k]): i32 {
    print(steps(0.0, 100000));
    const kept = weights();
    print(kept);
    const view = [1.0, 2.0, 3.0, 4.0].slice(1, 3);
    print(view);
    print(total([4.0, 5.0, 6.0]));
    const changed = [1, 2, 3].with(1, 20);
    print(changed);
    const copied = map(iota(2), (i) => { const t = [7, 8, 9]; return t[i] + t.length; });
    print(copied);
    return 0;
}
";
    let want = interp_ok(src, &[]);
    let bin = compile(src, "stack_literals");
    let ll = std::fs::read_to_string(bin.with_extension("ll")).unwrap();
    assert!(ll.contains("alloca [3 x double]"), "the weights are not on the stack");
    let run = run_bin(&bin, &[]);
    assert_eq!((run.code, run.stdout.as_str()), (0, want.as_str()), "{}", run.stderr);
    clean(src, "stack_literals_vg", &[]);
}

#[test]
fn slices_of_arrays_of_arrays_and_rows_of_constants() {
    // D193: a slice of an array of arrays (of a variable, of a `map`: rows
    // copied into a new block), rows that are arrays of constants (copied,
    // never freed); the compiler refused the first and freed the data of
    // the program for the second. The results of the interpreter; Valgrind
    // clean.
    let src = "action main(args: string[k]): i32 {
    const rows = map(iota(8), (k) => [k, 2 * k, 3]);
    print(rows.slice(2, 5));
    print(map(iota(6), (k) => [1, 2, 3]).slice(1, 3));
    const wide = map(iota(5000), (k) => [k % 7, k / 7]);
    const part = wide.slice(4000, 4003);
    print([part.length, part[0][1], part[2][0]] as const);
    print(rows.slice(3, 3).length);
    return 0;
}
";
    let want = interp_ok(src, &[]);
    let bin = compile(src, "nested_slices");
    let run = run_bin(&bin, &[]);
    assert_eq!((run.code, run.stdout.as_str()), (0, want.as_str()), "{}", run.stderr);
    clean(src, "nested_slices_vg", &[]);
}
