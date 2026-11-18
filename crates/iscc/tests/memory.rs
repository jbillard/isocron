//! No leak and no invalid access (SPEC section 15, stage 2, phase 3: "aucune
//! fuite (Valgrind ou AddressSanitizer)"): programs compiled for AVX2
//! (Valgrind does not decode AVX-512), with glibc's allocator so that
//! Valgrind sees every block, run under `valgrind --leak-check=full`.
//! Skipped when Valgrind is not installed.

mod common;
use common::*;

fn valgrind() -> Option<std::path::PathBuf> {
    valgrind_path()
}

fn clean(src: &str, name: &str, args: &[&str]) {
    checked_clean(src, name, args);
    let Some(vg) = valgrind() else {
        eprintln!("valgrind not installed: {} skipped", name);
        return;
    };
    let opts = isc_codegen::Options { isa: isc_codegen::Isa::Avx2, ..isc_codegen::Options::default() };
    let bin = compile_glibc(src, name, opts);
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
/// and drops itself is counted plainly; a tree it captures, atomically when
/// its results hold nodes, and not at all when they hold none: the
/// iteration then counts the nodes it made alone (D683, `_own`), the tree it
/// captures never written by the threads.
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
    // (the results of the first loop hold the shared tree: counted atomically)
    let held = local.replace(
        "print(sum(map(iota(10000), (i) => nodes(make(i % 5)))));",
        "const t = make(4);\n    const ts = map(iota(10000), (i) => Node(t, make(i % 3)));\n    print(sum(map(ts, (x) => nodes(x))));",
    );
    for (src, name, atomic, own, want) in [
        (local, "atomic_local", false, false, "114000\n"),
        (shared.as_str(), "atomic_shared", false, true, "356664\n"),
        (held.as_str(), "atomic_held", true, false, "356664\n"),
    ] {
        let ll = tmp(&format!("{}.ll", name));
        let p = iscc::frontend("a.isc", src).unwrap();
        let bin = tmp(&format!("{}_bin", name));
        iscc::build(&p, &iscc::BuildOptions { output: bin.clone(), emit_llvm: Some(ll.clone()), ..Default::default() }).unwrap();
        let text = std::fs::read_to_string(&ll).unwrap();
        // (an atomic count of a node: `atomicrmw` on a register, not on the
        // marks of the runtime)
        assert_eq!(text.lines().any(|l| l.contains("atomicrmw") && !l.contains("@isc_token_threads")), atomic, "{}", name);
        assert_eq!(text.contains("_own("), own, "{}", name);
        for t in [1, 4, 8] {
            assert_eq!(run_bin_threads(&bin, &[], Some(t)).stdout, want, "{}", name);
        }
        assert_eq!(interp_ok(src, &[]), want, "{} interpreted", name);
        checked_clean(src, name, &[]);
    }
}

/// The recursive functions a task calls (D678). A function was emitted once,
/// with plain counts: called from a task, the copies it made of the nodes
/// of a tree the threads share (here the subtree it returns, a new owner)
/// raced with the other threads, and a node was freed twice (the port of
/// rtiow, `bench/rtiow`: "invalid free" one run in three). The task calls
/// its variant `_tk`, whose counts of the shared types are atomic; a
/// function that holds no node (`steps`) is called as it is.
#[test]
fn functions_called_by_tasks_count_shared_nodes_atomically() {
    let src = "type Tree = Leaf | Node(Tree, Tree);

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

// the subtree `k` steps down the left side: a new owner of a shared node
function leftAt(t: Tree, k: i64): Tree {
    if (k == 0) return t;
    return match (t) {
        Leaf => Leaf,
        Node(l, r) => leftAt(l, k - 1),
    };
}

function steps(n: i64, acc: i64): i64 {
    if (n == 0) return acc;
    return steps(n - 1, acc + 1);
}

action main(args: string[k]): i32 {
    const t = make(10);
    print(sum(map(iota(200000), (i) => nodes(leftAt(t, 6 + i % 4)) + steps(i % 3, 0))));
    print(nodes(t));
    return 0;
}
";
    let want = "2999999\n2047\n";
    let ll = tmp("tk_variants.ll");
    let p = iscc::frontend("a.isc", src).unwrap();
    let bin = tmp("tk_variants_bin");
    iscc::build(&p, &iscc::BuildOptions { output: bin.clone(), emit_llvm: Some(ll.clone()), ..Default::default() }).unwrap();
    let text = std::fs::read_to_string(&ll).unwrap();
    // the variants of `nodes` and `leftAt`, not of `steps`; the copy of a
    // node in that of `leftAt` counted by the iteration of the task alone
    // when the node is its own (D683: the results of the loop hold no
    // node), never by a plain increment of a node of the shared tree
    let variants: Vec<&str> = text.split("\ndefine ").filter(|f| f.lines().next().is_some_and(|h| h.contains("_tk0("))).collect();
    assert_eq!(variants.len(), 2, "{}", text);
    assert!(variants.iter().any(|f| f.contains("lshr i64") && f.contains(", 40")), "{}", text);
    for _ in 0..5 {
        assert_eq!(run_bin_threads(&bin, &[], Some(8)).stdout, want);
    }
    assert_eq!(interp_ok(src, &[]), want, "interpreted");
    checked_clean(src, "tk_variants", &[]);
}

/// A node of a recursive type holds the slots of its own variant alone
/// (D682): a `Circle` 32 bytes, not the room of a `Poly` of 13 slots; read
/// slot by slot where a pattern reads it, freed variant by variant; equal,
/// printed and freed in tasks as before.
#[test]
fn nodes_hold_their_variant_alone() {
    let src = r#"// A tree of shapes: variants of very different sizes in one recursive type;
// each node holds its own variant's slots alone
type Shape =
    Dot
    | Circle(f64)
    | Poly(f64, f64, f64, f64, f64, f64, f64, f64, f64, f64, f64, f64, string)
    | Group(Shape[])
    | Moved(Shape, f64, f64);

function area(s: Shape): f64 {
    return match (s) {
        Dot => 0.0,
        Circle(r) => 3.0 * r * r,
        Poly(a, b, c, d, e, f, g, h, i, j, k, l, name) => a + b + c + d + e + f + g + h + i + j + k + l + f64(name.length),
        Group(xs) => sum(map(xs, (x) => area(x))),
        Moved(inner, dx, dy) => area(inner) + dx * dy,
    };
}

function made(n: i64): Shape {
    if (n == 0) {
        return Circle(1.5);
    }
    if (n % 4 == 1) {
        return Moved(made(n - 1), f64(n), 0.5);
    }
    if (n % 4 == 2) {
        const p = Poly(1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, f64(n), `p${n}`);
        return Group([made(n - 1), p, Dot, Circle(f64(n))]);
    }
    return Group([made(n - 1), Moved(Dot, 1.0, f64(n))]);
}

action main(args: string[k]): i32 {
    const t = made(9);
    print(area(t));
    print(made(2));
    print(made(3) == made(3));
    print(made(3) == made(4));
    print(Moved(Circle(2.0), 1.0, 1.0) == Moved(Circle(2.0), 1.0, 1.0));
    // many small nodes in parallel tasks, freed there
    print(sum(map(iota(2000), (q) => area(made(q % 13)))));
    return 0;
}
"#;
    let want = interp_ok(src, &[]);
    let ll = tmp("variant_nodes.ll");
    let p = iscc::frontend("a.isc", src).unwrap();
    let bin = tmp("variant_nodes_bin");
    iscc::build(&p, &iscc::BuildOptions { output: bin.clone(), emit_llvm: Some(ll.clone()), ..Default::default() }).unwrap();
    let text = std::fs::read_to_string(&ll).unwrap();
    // (the size of the node of a `Circle`: its count, its tag, its radius)
    assert!(text.contains("getelementptr { i64, { i32, double } }, ptr null, i32 1"), "{}", text);
    for t in [1, 4, 8] {
        assert_eq!(run_bin_threads(&bin, &[], Some(t)).stdout, want);
    }
    checked_clean(src, "variant_nodes", &[]);
}

/// The tasks of a loop whose results hold no node count the nodes their
/// iteration made alone (D683): trees of each iteration holding parts of a
/// world the threads share, freed in it; a loop inside each task sharing the
/// task's own tree with its threads; a loop whose results hold nodes, counted
/// atomically. The same results on 1 to 16 threads, as interpreted, nothing
/// leaked nor freed twice.
#[test]
fn tasks_count_their_own_nodes() {
    let src = r#"// Tasks that make nodes of the types they share with the other threads:
// a world built before the loops, read by every task; each iteration makes
// its own trees holding parts of the world, frees them, and hands some of
// them to a loop inside it; the results hold no node
type Tree = Leaf(i64) | Pair(Tree, Tree) | Tag(string, Tree);

function made(d: i64, k: i64): Tree {
    if (d == 0) {
        return Leaf(k);
    }
    return d % 3 == 0 ? Tag(`t${k}`, made(d - 1, k * 2)) : Pair(made(d - 1, k * 2), made(d - 1, k * 2 + 1));
}

function total(t: Tree): i64 {
    return match (t) {
        Leaf(k) => k,
        Pair(a, b) => total(a) + total(b),
        Tag(s, x) => s.length + total(x),
    };
}

// a tree of this iteration holding the parts of the world it picks
function picked(w: Tree, q: i64): Tree {
    return match (w) {
        Leaf(k) => Pair(w, Leaf(k + q)),
        Pair(a, b) => q % 2 == 0 ? Pair(picked(a, q / 2), b) : Tag(`q${q}`, picked(b, q / 2)),
        Tag(s, x) => Pair(x, picked(x, q + 1)),
    };
}

function first(t: Tree): Tree {
    return match (t) {
        Pair(a, b) => a,
        Tag(s, x) => x,
        Leaf(k) => t,
    };
}

action main(args: string[k]): i32 {
    const world = made(9, 1);
    const n = args.length > 0 ? parseI64(args[0]) : 3000;
    // each task: its own trees with parts of the world, freed in it
    print(sum(map(iota(n), (q) => {
        const mine = picked(world, q);
        const more = Pair(mine, first(world));
        return total(more) + total(picked(first(mine), q + 3));
    })));
    // a loop inside each task, sharing the task's own tree with its threads
    print(sum(map(iota(n / 10), (q) => {
        const mine = picked(world, q);
        return sum(map(iota(20), (r) => total(picked(mine, r)) + total(first(mine))));
    })));
    // a loop whose results hold nodes: counted by atomic instructions
    const kept = map(iota(n / 10), (q) => picked(world, q));
    print(sum(map(kept, (t) => total(t))));
    print(total(world));
    return 0;
}
"#;
    let want = interp_ok(src, &[]);
    let ll = tmp("own_counts.ll");
    let p = iscc::frontend("a.isc", src).unwrap();
    let bin = tmp("own_counts_bin");
    iscc::build(&p, &iscc::BuildOptions { output: bin.clone(), emit_llvm: Some(ll.clone()), ..Default::default() }).unwrap();
    let text = std::fs::read_to_string(&ll).unwrap();
    assert!(text.contains("_own("), "{}", text);
    for t in [1, 2, 4, 8, 16] {
        assert_eq!(run_bin_threads(&bin, &[], Some(t)).stdout, want, "{} threads", t);
    }
    checked_clean(src, "own_counts", &[]);
}

/// A tree a builder makes (D200) is made in a region and freed with it,
/// its nodes never one by one: a node holding a text of its own then lost
/// it (D684: five texts of a tree of depth 6, found by the tasks of D683).
/// Such a node is no longer made in a region.
#[test]
fn trees_of_regions_hold_no_texts() {
    let src = r#"type Tree = Leaf(i64) | Pair(Tree, Tree) | Tag(string, Tree);

function made(d: i64, k: i64): Tree {
    if (d == 0) {
        return Leaf(k);
    }
    return d % 3 == 0 ? Tag(`t${k}`, made(d - 1, k * 2)) : Pair(made(d - 1, k * 2), made(d - 1, k * 2 + 1));
}

function total(t: Tree): i64 {
    return match (t) {
        Leaf(k) => k,
        Pair(a, b) => total(a) + total(b),
        Tag(s, x) => s.length + total(x),
    };
}

action main(args: string[k]): i32 {
    const world = made(6, 1);
    print(total(world));
    return 0;
}
"#;
    let want = interp_ok(src, &[]);
    assert_eq!(run_bin_threads(&compile(src, "region_texts"), &[], Some(1)).stdout, want);
    checked_clean(src, "region_texts", &[]);
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

#[test]
fn trees_that_die_whole_are_made_in_a_region() {
    // D200: a tree of new nodes (the result of a function that builds only)
    // whose scope only lends it to functions returning no tree is made in a
    // region, freed at once; moved into a node, or given to a function that
    // returns a part of it, it stays on the heap. The results of the
    // interpreter, nothing lost or freed twice under Valgrind.
    let src = "type Tree = Leaf | Node(Tree, Tree);
type List = Nil | Cons(i64, List);
function bottomUp(depth: i64): Tree {
    if (depth == 0) return Leaf;
    return Node(bottomUp(depth - 1), bottomUp(depth - 1));
}
function check(t: Tree): i64 {
    return match (t) {
        Leaf => 1,
        Node(l, r) => 1 + check(l) + check(r),
    };
}
function upTo(k: i64): List {
    if (k == 0) return Nil;
    return Cons(k * k, upTo(k - 1));
}
function total(l: List): i64 {
    return match (l) {
        Nil => 0,
        Cons(x, rest) => x + total(rest),
    };
}
function even(d: i64): Tree {
    if (d == 0) return Leaf;
    return Node(odd(d - 1), Leaf);
}
function odd(d: i64): Tree {
    if (d == 0) return Leaf;
    return Node(even(d - 1), even(d - 1));
}
function leftOf(t: Tree): Tree {
    return match (t) {
        Leaf => Leaf,
        Node(l, r) => l,
    };
}
function steps(k: i64, acc: i64): i64 {
    if (k == 0) return acc;
    const t = bottomUp(k % 6);
    return steps(k - 1, acc + check(t));
}
action main(args: string[k]): i32 {
    print(check(bottomUp(10)));
    const t = bottomUp(8);
    print(check(t) + check(t));
    print(total(upTo(1000)));
    print(check(odd(9)));
    print(steps(200, 0));
    const kept = Node(bottomUp(4), Leaf);
    print(check(kept));
    const big = bottomUp(6);
    const part = leftOf(big);
    print(check(part));
    print(sum(map(iota(64), (i) => check(bottomUp(i % 7)))));
    return 0;
}
";
    let want = interp_ok(src, &[]);
    let bin = compile(src, "regions");
    let ll = std::fs::read_to_string(bin.with_extension("ll")).unwrap();
    assert!(ll.contains("@isc_region_new()"), "no region");
    assert!(ll.contains("@isc_fn_0_rg(") && ll.contains("@isc_fn_2_rg("), "no variant of the builders");
    let run = run_bin(&bin, &[]);
    assert_eq!((run.code, run.stdout.as_str()), (0, want.as_str()), "{}", run.stderr);
    clean(src, "regions_vg", &[]);
}

#[test]
fn loops_shared_from_tasks_free_everything() {
    // D201: the loops of tasks taken by other threads (2 under Valgrind),
    // their trees in regions (D200): nothing lost or freed twice.
    let src = "type Tree = Leaf | Node(Tree, Tree);
function bottomUp(depth: i64): Tree {
    if (depth == 0) return Leaf;
    return Node(bottomUp(depth - 1), bottomUp(depth - 1));
}
function check(t: Tree): i64 {
    return match (t) {
        Leaf => 1,
        Node(l, r) => 1 + check(l) + check(r),
    };
}
action main(args: string[k]): i32 {
    print(map(iota(5), (d) => sum(map(iota(1 << (8 - d)), (i) => check(bottomUp(d))))));
    print(map(iota(3), (d) => map(iota(4), (i) => check(bottomUp(d + i)))));
    return 0;
}
";
    let want = interp_ok(src, &[]);
    let bin = compile(src, "nested_tasks");
    let run = run_bin(&bin, &[]);
    assert_eq!((run.code, run.stdout.as_str()), (0, want.as_str()), "{}", run.stderr);
    clean(src, "nested_tasks_vg", &[]);
}

#[test]
fn states_of_loops_are_owned() {
    // D206: a loop (its calls to itself all tail calls) whose other result
    // is its tuple parameter takes it apart on entry, the parameter copied
    // once: its fields moved, updated in place, or freed when the new state
    // drops them; a state given whole to a function, a loop returning a
    // field, a recursion that is not a loop keep borrowed parameters. The
    // results of the interpreter, nothing lost or freed twice.
    let src = "type S = { a: f64[3], b: f64[3], k: i64 };
function step(s: S): S {
    const a = s.a.with(0, s.a[0] + s.b[1]);
    return { a: a, b: s.b * 0.5 + a, k: s.k + 1 };
}
function run(s: S, n: i64): S {
    if (n == 0) return s;
    return run(step(s), n - 1);
}
function drop(s: S, n: i64): S {
    if (n == 0) return s;
    return drop({ a: s.b, b: s.b + 1.0, k: s.k }, n - 1);
}
function total(s: S): f64 {
    return sum(s.a) + sum(s.b);
}
function whole(s: S, n: i64): f64 {
    if (n == 0) return total(s);
    return whole(step(s), n - 1);
}
function deep(s: S, n: i64): f64 {
    if (n == 0) return s.a[0];
    return deep(step(s), n - 1) + 1.0;
}
action main(args: string[k]): i32 {
    const s0 = { a: [1.0, 2.0, 3.0], b: [0.5, 0.25, 0.125], k: 0 };
    const r = run(s0, 1000);
    print(r.a);
    print(r.k);
    print(s0.a);
    print(drop(s0, 50).b);
    print(whole(s0, 20));
    print(deep(s0, 30));
    print(map(iota(4), (i) => run({ a: [f64(i), 1.0, 2.0], b: [1.0, 1.0, 1.0], k: i }, i * 3).k));
    return 0;
}
";
    let want = interp_ok(src, &[]);
    let bin = compile(src, "states");
    let run = run_bin(&bin, &[]);
    assert_eq!((run.code, run.stdout.as_str()), (0, want.as_str()), "{}", run.stderr);
    let p = iscc::frontend("states.isc", src).unwrap();
    let names: Vec<(String, Vec<bool>)> = p.fns.iter().map(|f| (f.name.clone(), f.consumed.clone())).collect();
    let consumed = |n: &str| names.iter().any(|(m, c)| m == n && c.first() == Some(&true));
    assert!(consumed("run") && consumed("drop"), "{:?}", names);
    assert!(!consumed("whole") && !consumed("deep"), "{:?}", names);
    clean(src, "states_vg", &[]);
}

#[test]
fn maps_write_into_inputs_that_die() {
    // D207: a `map` of scalars materialized in a `let` or a tuple, one of
    // whose inputs is an owned array of the same scalars at its last use,
    // writes into it; an input read later, borrowed (a parameter), of other
    // elements, a `map` that may panic or that a `reduce` reads, keep their
    // memory. The results of the interpreter, nothing lost or freed twice.
    let src = "type P = { a: f64[4], b: f64[4] };
function scale(x: f64[n], k: f64): f64[n] {
    return x * k;
}
function grow(p: P, n: i64): P {
    if (n == 0) return p;
    return grow({ a: p.a + p.b * 0.5, b: p.b * 0.75 }, n - 1);
}
action main(args: string[k]): i32 {
    const x = map(iota(1000), (i) => f64(i) * 0.5);
    const y = x * 2.0 + 1.0;
    print(sum(y));
    const u = map(iota(8), (i) => f64(i));
    const v = u * 3.0;
    print([sum(u), sum(v)]);
    const w = map(iota(8), (i) => i * i);
    const z = map(w, (t) => f64(t) / 2.0);
    print(z);
    const idx = [3, 1, 2, 0];
    const q = map(iota(4), (i) => f64(i) + 0.25);
    const r = map(q, (t) => t + q[idx[0]]);
    print(r);
    const s = map(iota(100), (i) => f64(i));
    print(sum(map(s, (t) => t * t)));
    print(scale(y, 0.5)[999]);
    const g = grow({ a: [1.0, 2.0, 3.0, 4.0], b: [4.0, 3.0, 2.0, 1.0] }, 200);
    print(g.a);
    return 0;
}
";
    let want = interp_ok(src, &[]);
    let bin = compile(src, "reuse");
    let run = run_bin(&bin, &[]);
    assert_eq!((run.code, run.stdout.as_str()), (0, want.as_str()), "{}", run.stderr);
    let p = iscc::frontend("reuse.isc", src).unwrap();
    let es = isc_ir::report::entries_of(&p, true);
    let grow: Vec<_> = es.iter().filter(|e| e.line == 7 && e.kind == "alloc").collect();
    assert!(grow.is_empty(), "the maps of `grow` still allocate: {:?}", grow);
    clean(src, "reuse_vg", &[]);
}

#[test]
fn large_blocks_grow_from_the_cache() {
    // D211: arrays larger at each phase (above the medium blocks) take the
    // pages of the cached block of the phase before, grown; a new array of
    // zeros on such pages is cleared. The results of the interpreter.
    let src = "function phase(k: i64): i64 {
    const n = k * 5000000;
    const a = map(iota(n), (i) => i % 251);
    const z = fill(n, 0);
    return sum(a) + sum(z) + a[n - 1];
}
action main(args: string[k]): i32 {
    print(map(iota(4), (k) => phase(k + 1)));
    return 0;
}
";
    let want = interp_ok(src, &[]);
    let bin = compile(src, "grow_large");
    for t in [1, 4] {
        let run = run_bin_threads(&bin, &[], Some(t));
        assert_eq!((run.code, run.stdout.as_str()), (0, want.as_str()), "{} threads: {}", t, run.stderr);
    }
}

#[test]
fn every_array_of_a_phase_grows_from_the_cache() {
    // D220: phases one after the other, of increasing sizes, with three
    // large arrays each (of
    // f64: not stored narrower, above the medium blocks), freed at the end
    // of the phase while a large array stays in use (the cache may then
    // hold as much): each array of the next phase grows one of the cached
    // blocks, the smaller ones stay cached for the next requests. The pages
    // of a grown block are not zero: a new array of zeros on them is
    // cleared, the contents of the others are theirs. The results of the
    // interpreter, on one thread and four.
    let src = "function phase(keep: f64[m], k: i64): f64 {
    const n = 4300000 + k * 900000;
    const a = map(iota(n), (i) => f64(i % 253) + keep[i % m]);
    const b = map(iota(n + 3), (i) => a[n - 1 - i % n] + 1.0);
    const z = fill(n + 5, 0.0);
    return sum(a) + sum(b) * 3.0 + sum(z) + b[n + 2] + z[n + 4] + a[n / 2];
}
action main(args: string[k]): i32 {
    const keep = map(iota(11000000), (i) => f64(i % 7));
    print(phase(keep, 0));
    print(phase(keep, 1));
    print(phase(keep, 2));
    print(phase(keep, 0));
    return 0;
}
";
    let want = interp_ok(src, &[]);
    let bin = compile(src, "grow_phases");
    for t in [1, 4] {
        let run = run_bin_threads(&bin, &[], Some(t));
        assert_eq!((run.code, run.stdout.as_str()), (0, want.as_str()), "{} threads: {}", t, run.stderr);
    }
}

#[test]
fn arrays_stored_in_a_narrower_type() {
    // D218: a `map` whose values fit a narrower integer type (the leaves of
    // an `if`, `x & c`), read only by element and length, is stored in that
    // type and read back converted: the same values for the program. Left
    // as it is: values not bounded, an array read whole. The results of the
    // interpreter, nothing lost or freed twice.
    let src = "function code(b: u8): i64 {
    const c = b | 32u8;
    return c == 97u8 ? 0 : (c == 99u8 ? 1 : (c == 103u8 ? 2 : 3));
}
action main(args: string[k]): i32 {
    const text = bytes(\"GATTACAgattacaCCGGTTAAggccttaa\");
    const codes = map(text, code);
    const n = codes.length;
    print(map(iota(n - 3), (i) => fold(iota(4), 0, (c, j) => c * 4 + codes[i + j])));
    const wide = map(iota(40), (i) => i % 3 == 0 ? 300 : 7);
    print(sum(map(iota(40), (i) => wide[39 - i] * i)));
    const small = map(iota(40), (i) => i % 2 == 0 ? -5 : 5);
    print(sum(map(iota(40), (i) => small[i] * i)));
    const masked = map(iota(40), (i) => (i * 37) & 15);
    print(masked[3] + masked[39] + masked.length);
    const free = map(iota(40), (i) => i * i);
    print(free[39]);
    const whole = map(iota(40), (i) => i % 2 == 0 ? 1 : 2);
    print(sum(whole));
    return 0;
}
";
    let want = interp_ok(src, &[]);
    let bin = compile(src, "narrow");
    let run = run_bin(&bin, &[]);
    assert_eq!((run.code, run.stdout.as_str()), (0, want.as_str()), "{}", run.stderr);
    let p = iscc::frontend("narrow.isc", src).unwrap();
    let ty = |name: &str| p.vars.iter().find(|v| v.name == name).map(|v| isc_ir::print::type_str(&v.ty)).unwrap_or_default();
    assert_eq!(ty("codes"), "u8[]");
    assert_eq!(ty("wide"), "u16[]");
    assert_eq!(ty("small"), "i8[]");
    assert_eq!(ty("masked"), "u8[]");
    assert_eq!(ty("free"), "i64[]");
    clean(src, "narrow_vg", &[]);
}

/// D688: the first chunk of the arrays of 16 KiB or more in pages of 4
/// KiB, as the first chunk of the smaller blocks: a small program makes its
/// pages resident, not a huge page of 2 MiB (where the kernel gives huge
/// pages to any mapping, or to those that ask for them: its file of 64 KiB
/// read, or an array of 100 KiB, took one). The most memory of the program
/// (`VmHWM`, as it prints it) under 1.5 MiB, with an array of 1 KiB and
/// one of 100 KiB (before: 2256 and 2356 KiB, now 216 and 412 on this
/// machine). Isocron's allocator only: glibc's maps its library, the
/// checked one keeps a quarantine.
#[test]
fn the_first_array_takes_small_pages() {
    if memcheck() || std::env::var("ISC_ALLOC").is_ok() {
        return;
    }
    let src = "action main(args: string[k]): i32 {
    const n = parseI64(args[0]);
    const xs = map(iota(n), (i) => i * 3 + 1);
    print(sum(xs) > 0 ? 1 : 0);
    const status = lines(readText(\"/proc/self/status\"));
    print(filter(status, (l) => l.startsWith(\"VmHWM\")));
    return 0;
}
";
    let bin = compile(src, "first_array_pages");
    let most = |n: &str| -> u64 {
        let r = run_bin(&bin, &[n]);
        assert_eq!(r.code, 0, "{}", r.stderr);
        let line = r.stdout.lines().nth(1).unwrap_or("").to_string();
        line.chars().filter(|c| c.is_ascii_digit()).collect::<String>().parse().unwrap_or_else(|_| panic!("{}", r.stdout))
    };
    let (small, large) = (most("128"), most("12800"));
    assert!(small < 1536 && large < 1536, "{} KiB, then {} KiB", small, large);
}

/// D701: the values of the statements of an action read by the next one,
/// no effect between, computed there: an array of 80 MB summed by the next
/// statement is never made (`VmHWM` under 8 MiB); with a clock read between
/// them it is made whole before the clock (section 12). Isocron's allocator
/// only.
#[test]
fn statement_values_computed_where_they_are_read() {
    if memcheck() || std::env::var("ISC_ALLOC").is_ok() {
        return;
    }
    let fused = "action main(args: string[k]): i32 {
    const n = parseI64(args[0]);
    const xs = map(iota(n), (i) => i * 3 + 1);
    const s = sum(xs);
    print(s);
    const status = lines(readText(\"/proc/self/status\"));
    print(filter(status, (l) => l.startsWith(\"VmHWM\")));
    return 0;
}
";
    let whole = fused.replace("    const s = sum(xs);\n", "    const t0 = clock();\n    const s = sum(xs);\n");
    let most = |src: &str, name: &str| -> u64 {
        let bin = compile(src, name);
        let r = run_bin(&bin, &["10000000"]);
        assert_eq!(r.code, 0, "{}", r.stderr);
        let mut ls = r.stdout.lines();
        assert_eq!(ls.next(), Some("149999995000000"), "{}", r.stdout);
        ls.next().unwrap_or("").chars().filter(|c| c.is_ascii_digit()).collect::<String>().parse().unwrap_or_else(|_| panic!("{}", r.stdout))
    };
    let (a, b) = (most(fused, "statement_values_fused"), most(&whole, "statement_values_whole"));
    assert!(a < 8 * 1024 && b > 78_000, "{} KiB, then {} KiB", a, b);
}

/// D705, D706: texts moved, not copied. A tuple takes a variable read by a
/// later part of it (the parts computed first, as the arguments of a call);
/// a `map` of the places of an array read nowhere after takes each of its
/// texts (`entries` of a `Map`, the keys of a table up to its count, the
/// rest freed with the block). The most memory (`VmHWM`) with 300000 texts
/// of 40 bytes: the moves add no text; the same program reading the array
/// again copies them all. Isocron's allocator only; the moves under the
/// checked allocator (each block freed once).
#[test]
fn texts_moved_into_tuples_and_gathers() {
    if memcheck() || std::env::var("ISC_ALLOC").is_ok() {
        return;
    }
    let moved = "function pack(ks: string[n]): [string[], i64] {
    return [ks, sum(map(ks, (k) => k.length))] as const;
}

action main(args: string[k]): i32 {
    const n = parseI64(args[0]);
    const m = parseI64(args[1]);
    const xs = map(iota(n), (i) => `${i}-0123456789012345678901234567890`);
    const [ks, total] = pack(xs);
    const pairs = map(iota(m), (i) => [ks[i], i] as const);
    print([total, pairs.length, sum(map(pairs, (p) => {
        const [w, i] = p;
        return w.length + i;
    }))] as const);
    const status = lines(readText(\"/proc/self/status\"));
    print(filter(status, (l) => l.startsWith(\"VmHWM\")));
    return 0;
}
";
    let copied = moved.replace("    print([total, pairs.length,", "    print(ks[0]);\n    print([total, pairs.length,");
    let run = |src: &str, name: &str, args: &[&str]| -> (String, u64) {
        let bin = compile(src, name);
        checked_clean_bin(&bin, args, None);
        let r = run_bin(&bin, args);
        assert_eq!(r.code, 0, "{}", r.stderr);
        let mut ls: Vec<&str> = r.stdout.lines().collect();
        let hwm = ls.pop().unwrap_or("").chars().filter(|c| c.is_ascii_digit()).collect::<String>().parse().unwrap_or_else(|_| panic!("{}", r.stdout));
        (ls.last().unwrap_or(&"").to_string(), hwm)
    };
    let (out_m, most_m) = run(moved, "texts_moved", &["300000", "300000"]);
    let (out_c, most_c) = run(&copied, "texts_copied", &["300000", "300000"]);
    assert_eq!(out_m, out_c);
    assert!(most_m + 8 * 1024 < most_c, "moved {} KiB, copied {} KiB", most_m, most_c);
    // the keys up to a count, the others freed with the block
    let (out, _) = run(moved, "texts_moved_part", &["1000", "300"]);
    assert_eq!(out, "(34890, 300, 55240)", "{}", out);
}

#[test]
fn tuples_stored_in_narrower_types() {
    // D687: the values of a `map` followed through the conditions of its
    // `if`s (`x < c`, `x == x`, of floats and integers), the bounds of
    // operations and conversions (a float saturated, NaN giving 0); each
    // part of a tuple of integers stored in its narrowest type, read back
    // converted. The bytes of a color as rtiow makes them (a float clamped,
    // then made an integer by adding 2^52), NaN and infinities among its
    // inputs; a part not bounded left as it is; `%` and `&` of integers;
    // a branch that cannot be taken. The results of the interpreter.
    let src = "function byte(x: f64): i64 {
    const s0 = x * 255.0;
    const s = s0 != s0 ? 255.0 : (s0 < 255.0 ? s0 : 255.0);
    const f = s + 4503599627370496.0;
    return f < 4503599627370496.0 ? 0 : i64(f - 4503599627370496.0);
}
action main(args: string[k]): i32 {
    const n = parseI64(args[0]);
    const xs = map(iota(n), (i) => i % 7 == 0 ? 0.0 / 0.0 : (i % 11 == 0 ? 1.0 / 0.0 : f64(i) / f64(n) * 1.5 - 0.2));
    const colors = map(xs, (x) => [byte(x), byte(x * x), byte(1.0 - x)] as const);
    print(fold(iota(colors.length), 0, (a, i) => {
        const [r, g, b] = colors[i];
        return a + r * 3 + g * 5 + b * 7;
    }));
    const [r0, g0, b0] = n > 0 ? colors[n / 2] : [0, 0, 0] as const;
    print(`${r0} ${g0} ${b0}`);
    const mixed = map(iota(n), (i) => {
        const k = i % 100;
        return [k, i * 1000, (i * 37) & 4095, k > 200 ? 70000 : -3] as const;
    });
    print(fold(iota(mixed.length), 0, (a, i) => {
        const [p, q, m, k] = mixed[i];
        return a + p + q + m + k;
    }));
    const unbounded = map(xs, (x) => [byte(x), i64(x * 1000.0)] as const);
    print(fold(iota(unbounded.length), 0, (a, i) => {
        const [c, d] = unbounded[i];
        return a + c + d;
    }));
    return 0;
}
";
    for n in ["0", "1", "1000"] {
        let want = interp_ok(src, &[n]);
        let bin = compile(src, "narrow_tuples");
        let run = run_bin(&bin, &[n]);
        assert_eq!((run.code, run.stdout.as_str()), (0, want.as_str()), "{}", run.stderr);
    }
    let p = iscc::frontend("narrow_tuples.isc", src).unwrap();
    let ty = |name: &str| p.vars.iter().find(|v| v.name == name).map(|v| isc_ir::print::type_str(&v.ty)).unwrap_or_default();
    assert_eq!(ty("colors"), "[u8, u8, u8][]");
    // (the sign of `i` not followed: `i % 100` from -99 to 99; `k > 200`
    // never true)
    assert_eq!(ty("mixed"), "[i8, i64, u16, i8][]");
    assert_eq!(ty("unbounded"), "[u8, i64][]");
    clean(src, "narrow_tuples_vg", &["1000"]);
}

/// D316: the last 2 MiB of a region of medium blocks are kept out of huge
/// pages (its sentinel is written there). Blocks of about 16 MiB up to that
/// end and into a second region (17 of them, 272 MB), every element written
/// and read, freed, then a second wave of blocks of 8 MiB in the memory
/// they gave back; with the allocator of Isocron and glibc's, on 1 and 4 threads.
#[test]
fn medium_blocks_to_the_end_of_a_region() {
    let src = "action main(args: string[k]): i32 {
    const k = parseI64(args[0]);
    const parts = map(iota(k), (j) => fill(2097144, j + 1));
    print(sum(map(parts, (p) => sum(p) + p[p.length - 1])));
    const again = map(iota(2 * k), (j) => fill(1048572, 2 * j));
    print(sum(map(again, (p) => sum(p) + p[0])));
    return 0;
}
";
    let k = 17i64;
    let first: i64 = (0..k).map(|j| 2097144 * (j + 1) + (j + 1)).sum();
    let second: i64 = (0..2 * k).map(|j| 1048572 * (2 * j) + 2 * j).sum();
    let want = format!("{}\n{}\n", first, second);
    let bin = compile(src, "heap_region_end");
    for alloc in [None, Some("glibc")] {
        for threads in ["1", "4"] {
            let mut cmd = std::process::Command::new(&bin);
            cmd.arg(k.to_string()).env("ISC_THREADS", threads);
            match alloc {
                Some(a) => cmd.env("ISC_ALLOC", a),
                None => cmd.env_remove("ISC_ALLOC"),
            };
            let o = cmd.output().unwrap();
            assert_eq!((o.status.code(), String::from_utf8_lossy(&o.stdout).to_string()), (Some(0), want.clone()), "{:?}, {} threads: {}", alloc, threads, String::from_utf8_lossy(&o.stderr));
        }
    }
}

/// D533: `concat(acc, x)` whose left array is read there for the last time
/// takes its block and grows it (in place when it has room, else a block
/// of twice its size), the right one copied after it: an accumulation by a
/// recursive function, of bytes and of integers, a left array still read
/// after (copied, not taken), a left array made there; the same outputs as
/// the interpreter, nothing lost under Valgrind, and the block grown (the
/// call of the runtime that grows it in the code).
#[test]
fn concat_grows_the_array_it_takes() {
    let src = "function grow(acc: i64[], i: i64, n: i64): i64[] {
    if (i == n) {
        return acc;
    }
    return grow(concat(acc, [i, i * 2]), i + 1, n);
}

function bytes3(acc: u8[], i: i64, n: i64): u8[] {
    if (i >= n) {
        return acc;
    }
    return bytes3(concat(acc, map(iota(i % 7), (k) => u8(k + i))), i + 1, n);
}

action main(args: string[k]): i32 {
    const n = parseI64(args[0]);
    const b = grow([], 0, n);
    print([b.length, sum(b), b[b.length - 1]] as const);
    const e = bytes3([], 0, n);
    print([e.length, sum(map(e, (x) => i64(x)))] as const);
    const c = map(iota(5), (i) => i + 1);
    const d = concat(c, [7, 8]);
    print(d);
    print(c);
    const f = concat(map(iota(n), (i) => f64(i) / 2.0), [0.5]);
    print([f.length, sum(f)] as const);
    return 0;
}
";
    let bin = compile(src, "concat_grows");
    for a in ["1", "10", "700"] {
        let want = interp_ok(src, &[a]);
        let r = run_bin(&bin, &[a]);
        assert_eq!((r.code, r.stdout.as_str()), (0, want.as_str()), "{}: {}", a, r.stderr);
    }
    let ll = std::fs::read_to_string(bin.with_extension("ll")).unwrap();
    assert!(ll.matches("call ptr @isc_str_reserve(").count() >= 2, "the blocks of the accumulators not grown");
    clean(src, "concat_grows_vg", &["700"]);
}

/// D548: the gather of a permutation of an owned array read for the last
/// time (`sort`, `sortBy` of texts, of tuples holding texts) moves its
/// elements: no copy of a text in the code (its mark, D592), the source
/// freed alone; the results as the interpreter's; no leak and no fault
/// under Valgrind and with the allocator checked. A source read again after
/// the sort keeps its texts (copies in the result).
#[test]
fn sorting_an_array_read_for_the_last_time_moves_its_elements() {
    let src = "action main(args: string[k]): i32 {
    const n = parseI64(args[0]);
    const xs = map(iota(n), (i) => toString((i * 7919) % 1000));
    const ys = sort(xs);
    print(ys[0]);
    print(ys[n / 2]);
    print(ys[n - 1]);
    const ps = map(iota(n), (i) => [toString(i % 97), i] as const);
    const qs = sortBy(ps, (p) => {
        const [t, k] = p;
        return t.length;
    });
    print(qs[n - 1]);
    const zs = map(iota(n), (i) => toString(i * 31 % 101));
    const ws = sortBy(zs, (s) => s.length);
    print(ws[0]);
    print(zs[0]);
    print(ws[n - 1]);
    print(zs[n - 1]);
    return 0;
}
";
    let bin = compile(src, "sort_moves");
    for a in ["1", "10", "700"] {
        let want = interp_ok(src, &[a]);
        let r = run_bin(&bin, &[a]);
        assert_eq!((r.code, r.stdout.as_str()), (0, want.as_str()), "{}: {}", a, r.stderr);
    }
    let ll = std::fs::read_to_string(bin.with_extension("ll")).unwrap();
    // no text copied: the sorts of `xs` and `ps` move their elements, that
    // of `zs`, read again after it, makes views of its texts (D555)
    assert_eq!(ll.matches("; text copy").count(), 0, "the elements of the sorts copied");
    clean(src, "sort_moves_vg", &["700"]);
}

/// A tail call that gives a part of a borrowed parameter again (the tail
/// of a list) is a jump of the loop (D561): a list of 3 million nodes
/// summed by its loop; a function whose place also receives a new list
/// (owned by the loop, freed at a later jump) keeps its calls. The same
/// results as the interpreter, every node freed once (the checked
/// allocator).
#[test]
fn tails_of_borrowed_lists_given_to_loops() {
    let src = "type List = Nil | Cons(i64, List);

function build(n: i64, acc: List): List {
    return n == 0 ? acc : build(n - 1, Cons(n, acc));
}

function sumOf(l: List, acc: i64): i64 {
    return match (l) {
        Nil => acc,
        Cons(x, rest) => sumOf(rest, acc + x),
    };
}

function mixed(l: List, n: i64): i64 {
    if (n == 0) {
        return sumOf(l, 0);
    }
    return match (l) {
        Nil => n,
        Cons(x, rest) => n % 3 == 0 ? mixed(Cons(x + 1, rest), n - 1) : mixed(rest, n - 1),
    };
}

action main(args: string[k]): i32 {
    const n = parseI64(args[0]);
    const l = build(n, Nil);
    print(sumOf(l, 0));
    print(mixed(build(1000, Nil), 500));
    return 0;
}
";
    let bin = compile(src, "lent_tails");
    let want = interp_ok(src, &["1000"]);
    let r = run_bin(&bin, &["1000"]);
    assert_eq!((r.code, r.stdout.as_str()), (0, want.as_str()), "{}", r.stderr);
    let n: i64 = 3_000_000;
    let r = run_bin(&bin, &["3000000"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(r.stdout.lines().next(), Some((n * (n + 1) / 2).to_string().as_str()));
    checked_clean(src, "lent_tails", &["100000"]);
}

/// The texts of a large array sorted by moves (D548), so out of the order of
/// their places, freed from the highest place down (D557): 200000 texts of
/// blocks of 128 bytes and more, literals among them, sorted, freed, made
/// again three times; the same output as the interpreter, every block freed
/// once (the checked allocator).
#[test]
fn texts_of_a_sorted_array_freed_in_the_order_of_their_places() {
    let src = "action main(args: string[k]): i32 {
    const n = parseI64(args[0]);
    print(fold(iota(3), 0, (acc, r) => {
        const xs = map(iota(n), (i) => i % 7 == 0 ? \"seven\" : `${(i * 7919 + r) % 100003}-${i % 13}`);
        const ys = sort(xs);
        return acc + ys[n / 2].length + ys[n - 1].length;
    }));
    return 0;
}
";
    let want = interp_ok(src, &["200000"]);
    let bin = compile(src, "freed_by_places");
    let r = run_bin(&bin, &["200000"]);
    assert_eq!((r.code, r.stdout.as_str()), (0, want.as_str()), "{}", r.stderr);
    checked_clean(src, "freed_by_places", &["200000"]);
}

/// The maps whose elements are views of a text that lives longer, fused
/// into their consumers (D697): a `fold` over the words found by a
/// `filter` of their starts, a `fold` that keeps one of them, a `reduce`
/// that joins them, one read at an index in a loop and kept, a `count`.
/// The same output as the interpreter, each block freed once (the checked
/// allocator, valgrind), and no array of the words made but for the
/// `reduce` (which owns its elements, D134).
#[test]
fn views_read_where_they_are_by_their_consumers() {
    let src = include_str!("data/views_fused.isc");
    let bin = compile(src, "views_fused");
    for a in ["0", "1", "7", "300"] {
        let want = interp_ok(src, &[a]);
        let r = run_bin(&bin, &[a]);
        assert_eq!((r.code, r.stdout.as_str()), (0, want.as_str()), "{}: {}", a, r.stderr);
        checked_clean_bin(&bin, &[a], None);
    }
    // no `let` of an array of the words (a `map` of slices, views or
    // copies) but for the `reduce`, which owns its elements (D134)
    fn arrays_of_slices(e: &isc_ir::Expr, out: &mut Vec<u32>) {
        let slice = |e: &isc_ir::Expr| matches!(e.kind, isc_ir::ExprKind::StrOp(isc_ir::StrOp::Slice, _));
        if let isc_ir::ExprKind::Let { init, .. } = &e.kind {
            if let isc_ir::ExprKind::Map(_, l) = &init.kind {
                if *init.ty.elem() == isc_ir::Type::Str && (slice(&l.body) || matches!(&l.body.kind, isc_ir::ExprKind::Copy(x) if slice(x))) {
                    out.push(init.span.line);
                }
            }
        }
        for c in e.children() {
            arrays_of_slices(c, out);
        }
    }
    let p = iscc::frontend("views_fused.isc", src).unwrap();
    let mut made = Vec::new();
    arrays_of_slices(&p.body, &mut made);
    let reduce = src.lines().position(|l| l.contains("print(reduce(")).unwrap() as u32 + 1;
    assert_eq!(made, vec![reduce], "the lines of the arrays of words made");
    clean(src, "views_fused_vg", &["300"]);
}

/// `countBy` whose keys are texts (D698): counted by the table of the
/// prelude, the keys in the order of their first element, each counted as
/// often as it comes (checked by the program against a count made another
/// way), past the first sizes of the table (512 keys, 1024 slots), with an
/// empty key, keys holding a zero byte, keys of more than 16 bytes that
/// share their first 8; keys made by the function of `countBy`. The same
/// output as the interpreter, each block freed once.
#[test]
fn counts_of_texts_by_a_table_of_the_prelude() {
    let src = include_str!("data/counts_of_texts.isc");
    let bin = compile(src, "counts_of_texts");
    for a in ["0", "1", "40", "3000"] {
        let want = interp_ok(src, &[a]);
        let r = run_bin(&bin, &[a]);
        assert_eq!((r.code, r.stdout.as_str()), (0, want.as_str()), "{}: {}", a, r.stderr);
        if a != "0" {
            assert!(want.lines().nth(2) == Some("true"), "{}: {}", a, want);
        }
        checked_clean_bin(&bin, &[a], None);
    }
    clean(src, "counts_of_texts_vg", &["1200"]);
}

/// Arrays whose texts are views of the texts of a value that lives longer
/// (D555): the sorts of an array read again after them, of such a view, of
/// the lines of a text kept, of a parameter of a recursive function (the
/// view freed before the jump of its tail call), in a loop, in a branch,
/// read by the threads of the pool, printed whole, of a function inlined;
/// and those that must own their texts: an array given back by a branch,
/// kept in a tuple, the text a loop keeps, a source read for the last time
/// (the moves of D548). The same output as the interpreter, each block
/// freed once (the checked allocator, valgrind), a copy in the memory
/// report only where a text must be owned (lines 29, 32, 38, 43 and 71),
/// and where the loop `longest` is given its text `best` (D644): the
/// literal at its call (line 65) and the text it chooses before its tail
/// call (line 8), copied before on entry of each call, as many copies.
#[test]
fn arrays_whose_texts_are_views_of_texts_that_live_longer() {
    let src = include_str!("data/views_of_texts.isc");
    let bin = compile(src, "views_of_texts");
    for a in ["3", "10", "700"] {
        let want = interp_ok(src, &[a]);
        let r = run_bin(&bin, &[a]);
        assert_eq!((r.code, r.stdout.as_str()), (0, want.as_str()), "{}: {}", a, r.stderr);
    }
    let p = iscc::frontend("views_of_texts.isc", src).unwrap();
    let mut copies: Vec<u32> = isc_ir::report::entries(&p).iter().filter(|e| e.kind == "copy").map(|e| e.line).collect();
    copies.sort();
    copies.dedup();
    // (not 43: the first of a sort of `xs` read where it is, a view of a
    // text of `xs`; nor 71: the sort read by its last element alone in the
    // statement that prints it, D701, never made)
    assert_eq!(copies, vec![8, 29, 32, 38, 65], "the lines whose texts are copied");
    clean(src, "views_of_texts_vg", &["700"]);
}

/// The rows of a `map` of rows (D593). A borrowed row the generated code
/// did not read where it is (an `if` or a `match` between two arrays, the
/// bytes of a text, the parameter a function gives back) was taken for a
/// new one and freed after its copy: its block freed twice. A `map` of
/// rows computed, read by its consumer without being made, lost every row.
/// Now: the same output as the interpreter, each block freed once (the
/// checked allocator, valgrind).
#[test]
fn rows_of_a_map_borrowed_computed_or_moved() {
    let src = include_str!("data/rows_of_maps.isc");
    let bin = compile(src, "rows_of_maps");
    for a in ["1", "3", "7"] {
        let want = interp_ok(src, &[a]);
        let r = run_bin(&bin, &[a]);
        assert_eq!((r.code, r.stdout.as_str()), (0, want.as_str()), "{}: {}", a, r.stderr);
        checked_clean_bin(&bin, &[a], None);
    }
    clean(src, "rows_of_maps_vg", &["7"]);
}

/// A row of another length than the first (D154): the panic of the
/// compiled program at the place the interpreter gives, the one of the
/// whole row, through its `let`s and the branches of its `if`s (D593).
#[test]
fn a_row_of_another_length_panics_at_the_place_of_the_row() {
    let rows = [
        "i % 2 == 0 ? a : a.slice(0, 2)",
        "{ const z = i % 2 == 0 ? a : a.slice(0, 2); return z; }",
        "{ const q = i + 1; return q % 2 == 1 ? a : a.slice(0, 2); }",
        "i == 0 ? a : i == 1 ? a.slice(0, 2) : a",
        "a.slice(0, 3 - i % 2)",
        "map(iota(3 - i % 2), (j) => j)",
        "{ const w = 3 - i % 2; return map(iota(w), (j) => j + w); }",
    ];
    for (k, row) in rows.iter().enumerate() {
        let src = format!(
            "action main(args: string[k]): i32 {{\n    const a = map(iota(3), (i) => i + k);\n    print(map(iota(4), (i) => {}));\n    return 0;\n}}\n",
            row
        );
        let want = interp_panic(&src, &["x"]);
        assert!(want.contains("size mismatch: 2 != 3"), "{}: {}", row, want);
        let bin = compile(&src, &format!("row_length_{}", k));
        let r = run_bin(&bin, &["x"]);
        assert_eq!(r.code, 101, "{}: {}", row, r.stderr);
        assert_eq!(r.stderr.trim(), want.trim(), "{}", row);
    }
}

/// What the code generator has no code for yet is refused by it with its
/// name (D593), where it gave code that LLVM refused: a `scan` of rows, a
/// `filter` of rows kept as an array. The interpreter runs both; counted
/// or read by a consumer, a `filter` of rows makes no array and compiles.
#[test]
fn scan_and_kept_filter_of_rows_refused() {
    let head = "action main(args: string[k]): i32 {\n    const a = map(iota(3), (i) => i + k);\n    const m = [a, a, a];\n";
    let scan = format!("{}    print(scan(m, (x, y) => map(iota(3), (j) => x[j] + y[j]), map(iota(3), (j) => 0)));\n    return 0;\n}}\n", head);
    let kept = format!("{}    print(filter(m, (r) => r[1] > 0));\n    return 0;\n}}\n", head);
    let cases = [
        (&scan, "a `scan` whose elements are arrays is not supported in the POC", "[[1, 2, 3], [2, 4, 6], [3, 6, 9]]\n"),
        (&kept, "a `filter` of rows kept as an array is not supported in the POC", "[[1, 2, 3], [1, 2, 3], [1, 2, 3]]\n"),
    ];
    for (k, (src, message, want)) in cases.iter().enumerate() {
        assert_eq!(interp_ok(src, &["x"]), *want, "interpreted");
        let p = iscc::frontend("t.isc", src).unwrap();
        let opts = iscc::BuildOptions {
            output: tmp(&format!("rows_refused_{}", k)),
            emit_llvm: None,
            emit_asm: None,
            debug: false,
            codegen: isc_codegen::Options::default(),
            link: Vec::new(),
            glibc: false,
            icon: None,
        };
        let e = iscc::build(&p, &opts).unwrap_err();
        assert!(e.to_string().contains(message), "{}", e);
    }
    let counted = format!("{}    print(count(m, (r) => r[1] > 0));\n    return 0;\n}}\n", head);
    let bin = compile(&counted, "rows_filter_counted");
    let r = run_bin(&bin, &["x"]);
    assert_eq!((r.code, r.stdout.as_str()), (0, "3\n"), "{}", r.stderr);
}

#[test]
fn parameters_given_to_called_functions() {
    // D637: a function called rather than inlined (`@noInline` here; too
    // large to inline in a program, D336) whose body consumes an array or a
    // string parameter on every path is given it: its caller moves its
    // argument at its last use, copies it otherwise (refused under
    // `requireUnique`), as the inlined body would; a function that gives it
    // in a tail call to another one given it too reuses its frame. One that
    // consumes it on some paths only (a tail call lending it to another
    // function, a scalar returned first) is lent it when a call keeps its
    // argument (`touch(w, true)`, `w` read after): it copies it there
    // alone; given it when every call gives a new value (`firstOr`, D693).
    // The results of the interpreter, nothing lost or freed twice.
    let src = "@noInline
function bump(a: i64[n], i: i64): i64[n] {
    return requireUnique(a.with(i, a[i] + 1));
}
@noInline
function plain(a: i64[n], i: i64): i64[n] {
    return a.with(i, a[i] * 2);
}
@noInline
function count(a: i64[n]): i64 {
    return sum(a);
}
@noInline
function touch(a: i64[n], c: boolean): i64 {
    if (c) {
        return sum(a.with(0, 100));
    }
    return count(a);
}
@noInline
function firstOr(a: i64[n], i: i64): i64 {
    if (i < 0) {
        return 0;
    }
    return sum(a.with(i, 0));
}
@noInline
function relay(a: i64[n], i: i64): i64[n] {
    return bump(a, i);
}
@noInline
function shout(s: string): string {
    return s + \"!\";
}
@noInline
function reads(a: i64[n]): i64 {
    return a[0] + a.length;
}
action main(args: string[k]): i32 {
    const x = fold(iota(1000), fill(10, 0), (acc, i) => bump(acc, i % 10));
    print(x);
    const y = fill(3, 7);
    const z = plain(y, 0);
    print([y[0], z[0]] as const);
    print(touch(fill(4, 1), true));
    print(touch(fill(4, 1), false));
    print([firstOr(fill(3, 2), -1), firstOr(fill(3, 2), 1)] as const);
    print(shout(\"hey\"));
    print(relay(fill(2, 5), 1));
    const w = fill(5, 3);
    print(map(iota(4), (i) => sum(plain(w, i))));
    print(touch(w, true));
    print(reads(w) + reads(fill(2, 9)));
    return 0;
}
";
    let want = interp_ok(src, &[]);
    assert_eq!(want, "[100, 100, 100, 100, 100, 100, 100, 100, 100, 100]\n(7, 14)\n103\n4\n(0, 4)\nhey!\n[5, 6]\n[18, 18, 18, 18]\n112\n19\n");
    let bin = compile(src, "given");
    let run = run_bin(&bin, &[]);
    assert_eq!((run.code, run.stdout.as_str()), (0, want.as_str()), "{}", run.stderr);
    let p = iscc::frontend("given.isc", src).unwrap();
    let given = |n: &str| p.fns.iter().find(|f| f.name == n).map(|f| f.given.first() == Some(&true)).unwrap_or(false);
    assert!(given("bump") && given("plain") && given("relay") && given("shout") && given("firstOr"), "{:?}", p.fns.iter().map(|f| (&f.name, &f.given)).collect::<Vec<_>>());
    // read only, or consumed on some paths only and kept by a call: lent
    assert!(!given("count") && !given("reads") && !given("touch"), "{:?}", p.fns.iter().map(|f| (&f.name, &f.given)).collect::<Vec<_>>());
    // the fold moves its accumulator into each call: no copy there; the
    // copies: `y` read after its call, `w` read by every task
    let copies: Vec<u32> = p.copy_reasons.iter().map(|(s, _)| s.line).collect();
    assert!(!copies.contains(&40) && copies.contains(&43) && copies.contains(&51), "{:?}", p.copy_reasons);
    clean(src, "given_vg", &[]);
}

#[test]
fn require_unique_holds_through_a_call() {
    // D637: `requireUnique` on a parameter of a function that is called:
    // satisfied inside it, and refused at a call that would copy its
    // argument, as when the function is inlined.
    let head = "@noInline
function bump(a: i64[n], i: i64): i64[n] {
    return requireUnique(a.with(i, a[i] + 1));
}
";
    let ok = format!("{}action main(args: string[k]): i32 {{\n    print(fold(iota(30), fill(3, 0), (acc, i) => bump(acc, i % 3)));\n    return 0;\n}}\n", head);
    assert_eq!(interp_ok(&ok, &[]), "[10, 10, 10]\n");
    let bin = compile(&ok, "unique_call");
    let r = run_bin(&bin, &[]);
    assert_eq!((r.code, r.stdout.as_str()), (0, "[10, 10, 10]\n"), "{}", r.stderr);
    let again = format!("{}action main(args: string[k]): i32 {{\n    const y = fill(3, 7);\n    const z = bump(y, 0);\n    print([y[0], z[0]] as const);\n    return 0;\n}}\n", head);
    let e = iscc::frontend("again.isc", &again).err().expect("refused");
    assert!(e.contains("requireUnique not satisfied (line 7): `y` is used again at line 8"), "{}", e);
}

#[test]
fn concat_moves_the_elements_of_arrays_that_die() {
    // D638: `concat` of arrays whose elements own memory (texts, records
    // holding arrays, values of a sum type holding one), both sides read
    // by it alone then freed: the elements moved (the columns copied by
    // their bytes), each side's block given back alone; a side read again
    // keeps its elements (copied); an accumulator grown by `concat` in a
    // loop, a chain of `concat`s. The results of the interpreter, nothing
    // lost or freed twice.
    let src = include_str!("data/concat_moves.isc");
    let bin = compile(src, "concat_moves");
    for a in ["0", "1"] {
        let want = interp_ok(src, &[a]);
        let r = run_bin(&bin, &[a]);
        assert_eq!((r.code, r.stdout.as_str()), (0, want.as_str()), "{}", r.stderr);
    }
    clean(src, "concat_moves_vg", &["1"]);
}

#[test]
fn a_value_freed_at_once_is_not_freed_again_by_a_tail_call() {
    // D639: a value the rest of its scope never reads is freed at once
    // (D117); a tail call of the function to itself, or to another one,
    // after it freed it again before its jump (the temporaries of the scope,
    // D102): a block freed twice. Found by the library of graphical
    // interfaces (the descriptors of a read of Wayland closed by a `map`
    // whose result was not read, in its loop). An array new or empty, in a
    // loop and before a tail call to another function; the results of the
    // interpreter, nothing lost or freed twice.
    let src = "@noInline
function other(n: i64): i64 {
    return n * 2;
}
action loop(round: i64, total: i64): i64 {
    if (round >= 4) {
        return total;
    }
    const fds = round == 1 ? [1000i32, 1001i32] : [];
    const closed = map(fds, (fd) => fd + 1i32);
    return loop(round + 1, total + 1);
}
function hop(n: i64): i64 {
    const unused = map(iota(n), (i) => i * 3);
    return other(n + 1);
}
action main(args: string[k]): i32 {
    print(loop(0, 0));
    print(hop(parseI64(args[0])));
    return 0;
}
";
    let want = interp_ok(src, &["5"]);
    assert_eq!(want, "4\n12\n");
    let bin = compile(src, "freed_once");
    let r = run_bin(&bin, &["5"]);
    assert_eq!((r.code, r.stdout.as_str()), (0, want.as_str()), "{}", r.stderr);
    clean(src, "freed_once_vg", &["5"]);
}

/// D644: the array of a loop (a recursive function that changes it at each
/// turn or gives it back, D351) given by its callers: a loop called once per
/// run with the whole array (as an inflater's `codes` once per block) takes
/// it as it comes, no copy on entry; a caller that reads its array again
/// copies it at the call (the line in the memory report), `requireUnique`
/// in the loop holds. 300000 elements in runs of 7 (42858 calls of the loop:
/// a copy of the array at each would write 100 GB); the same output as the
/// interpreter, each block freed once (the checked allocator, valgrind).
#[test]
fn a_loop_given_its_array() {
    let src = "// fills `k` places of `out` from `o` with `v`
function fillRun(out: i64[n], o: i64, k: i64, v: i64): i64[n] {
    if (k == 0) {
        return out;
    }
    return fillRun(requireUnique(out.with(o, v)), o + 1, k - 1, v);
}

// the runs of `r` places one after the other, each by a call of the loop
function runs(out: i64[n], o: i64, r: i64): i64[n] {
    if (o >= n) {
        return out;
    }
    const k = min(r, n - o);
    return runs(fillRun(out, o, k, o), o + k, r);
}

action main(args: string[k]): i32 {
    const n = parseI64(args[0]);
    const a = runs(fill(n, 0), 0, 7);
    print([a[0], a[7], a[n - 1], sum(a)] as const);
    const b = fill(n, 1);
    const c = fillRun(b, 0, 3, 9);
    print([b[0], c[0], c[3], c[n - 1]] as const);
    return 0;
}
";
    let bin = compile(src, "loop_given");
    for a in ["10", "300000"] {
        let want = interp_ok(src, &[a]);
        let r = run_bin(&bin, &[a]);
        assert_eq!((r.code, r.stdout.as_str()), (0, want.as_str()), "{}: {}", a, r.stderr);
    }
    let p = iscc::frontend("loop_given.isc", src).unwrap();
    let copies: Vec<u32> = isc_ir::report::entries(&p).iter().filter(|e| e.kind == "copy").map(|e| e.line).collect();
    assert_eq!(copies, vec![23], "the lines whose arrays are copied");
    clean(src, "loop_given_vg", &["3000"]);
}

/// D647: a `concat` that takes the block of its left array (D533, D638) in
/// one branch of an `if`, the array read or given in the other: its binding
/// bound again in that branch (what is left of it to free) read by the other
/// branch, a value of the first in the second (invalid code: `Instruction
/// does not dominate all uses`). A loop growing its array in one branch
/// alone, of scalars and of a sum type holding texts, the branches of a
/// `match`, a `concat` in each branch, one before an `if` that reads the
/// array after; the same output as the interpreter, every block freed once
/// (the checked allocator, valgrind).
#[test]
fn a_concat_that_takes_its_array_in_one_branch() {
    let src = "type Ev = A(i64) | B(f32, f32) | C | S(string);
type In = P(i64) | Q(i64, boolean) | R;

function grow(xs: i64[n], i: i64, out: i64[]): i64[] {
    if (i >= n) {
        return out;
    }
    if (xs[i] > 0) {
        return grow(xs, i + 1, concat(out, [xs[i]]));
    }
    return grow(xs, i + 1, out);
}

function events(xs: In[n], i: i64, out: Ev[], m: i64): [Ev[], i64] {
    if (i >= n) {
        return [out, m] as const;
    }
    return match (xs[i]) {
        P(a) => events(xs, i + 1, concat(out, [a % 2 == 0 ? A(a) : S(`s${a}`)]), m),
        Q(a, d) => {
            if (a == 7) {
                return events(xs, i + 1, concat(out, [C]), m + 1);
            }
            return events(xs, i + 1, d ? concat(out, [B(1.0f32, f32(a))]) : out, m);
        },
        _ => events(xs, i + 1, out, m),
    };
}

function both(t: string[n], k: i64): string[] {
    const u = map(t, (s) => s + \"!\");
    const v = k > 2 ? concat(u, [`k${k}`]) : concat(u, [\"none\"]);
    return v;
}

action main(args: string[k]): i32 {
    const n = parseI64(args[0]);
    print(grow(map(iota(n), (i) => i % 3 - 1), 0, []));
    const xs: In[] = map(iota(n), (i) => i % 4 == 0 ? P(i) : (i % 4 == 1 ? Q(i, i % 3 == 0) : (i % 4 == 2 ? Q(7, true) : R)));
    const [out, m] = events(xs, 0, [], 0);
    print([out.length, m] as const);
    print(map(out, (e) => match (e) {
        A(a) => a,
        B(x, y) => i64(y),
        C => -1,
        S(s) => s.length,
    }).sum());
    print(both(map(iota(n), (i) => `t${i}`), n));
    return 0;
}
";
    let bin = compile(src, "concat_one_branch");
    for a in ["0", "3", "40"] {
        let want = interp_ok(src, &[a]);
        let r = run_bin(&bin, &[a]);
        assert_eq!((r.code, r.stdout.as_str()), (0, want.as_str()), "{}: {}", a, r.stderr);
    }
    clean(src, "concat_one_branch_vg", &["40"]);
}

/// D653 to D657: the state of a program, a record of arrays, given from
/// function to function through functions that call each other (a small
/// evaluator: `eval` and `exec`), each of its arrays updated in place
/// (`requireUnique` holds): a part of it read before it is made anew
/// (D653), an `if` one branch of which is new (D653), two fields updated at
/// once (D655), a scalar of it read by an argument or a component after it
/// (D656), the state of a loop given by its caller (D657). A million steps
/// in about 0.1 s (a copy of the state at each step was quadratic); the
/// copies of the memory report are those of the texts of its log and of the
/// literal of its start alone; the output of the interpreter, Valgrind
/// clean.
#[test]
fn a_state_given_through_functions_that_call_each_other() {
    let src = include_str!("data/state_given.isc");
    let bin = compile(src, "state_given");
    let r = run_bin(&bin, &["300000"]);
    assert_eq!((r.code, r.stdout.as_str()), (0, "(90000600000, 51, 50, 1)\n[machine: failed]\n"), "{}", r.stderr);
    for a in ["10", "500"] {
        let want = interp_ok(src, &[a]);
        let r = run_bin(&bin, &[a]);
        assert_eq!((r.code, r.stdout.as_str()), (0, want.as_str()), "{}: {}", a, r.stderr);
    }
    let p = iscc::frontend("state_given.isc", src).unwrap();
    let copies: Vec<u32> = isc_ir::report::entries(&p).iter().filter(|e| e.kind == "copy").map(|e| e.line).collect();
    assert!(copies.iter().all(|l| *l == 10 || *l == 64), "the lines whose values are copied: {:?}", copies);
    clean(src, "state_given_vg", &["300"]);
}

/// D658: the accumulator of a `fold` of the type of its result, not of its
/// first value: a record whose array starts empty (`{ ks: [], n: 0 }`, its
/// length fixed at 0 by the type of the literal, D204) and grows at each
/// step was read at each step with that length, its elements lost
/// (`[3]` for `[0, 1, 2, 3]`); the code of chunks of Lua compiled by
/// isocron-ui lost the constants of all its functions but the last. The
/// same output as the interpreter, Valgrind clean.
#[test]
fn a_fold_whose_array_grows_from_an_empty_one() {
    let src = "type R = { ks: i64[], names: string[], n: i64 };
function addRec(k0: R, m: i64): R {
    return fold(iota(m), k0, (acc, j) => ({ ks: concat(acc.ks, [j]), names: concat(acc.names, [`n${j}`]), n: acc.n + 1 }));
}
function emptyR(): R {
    return { ks: [], names: [], n: 0 };
}
action main(args: string[k]): i32 {
    print(addRec(emptyR(), parseI64(args[0])));
    return 0;
}
";
    let want = "{ks: [0, 1, 2, 3], n: 4, names: [n0, n1, n2, n3]}\n";
    assert_eq!(interp_ok(src, &["4"]), want);
    let bin = compile(src, "fold_grows");
    let r = run_bin(&bin, &["4"]);
    assert_eq!((r.code, r.stdout.as_str()), (0, want), "{}", r.stderr);
    clean(src, "fold_grows_vg", &["4"]);
}

/// D659: a machine of registers whose state, a record of arrays, is given
/// from function to function in a cycle (`run`, `enter`, `lib`, `run`
/// again), as the machine of Lua of isocron-ui: an element of a part read
/// by another component of the new state, an argument of a function
/// inlined read after its first one, elements of the state read by the
/// arguments of a call after the state itself, texts of the state compared
/// beside it, the state made anew at each step of a `fold`. Each of its
/// arrays updated in place: seven million steps on registers of 65536
/// elements in milliseconds (a copy of them at each step took minutes);
/// the copies of the memory report are those of the literals of `main`
/// alone; the output of the interpreter, Valgrind clean.
#[test]
fn a_machine_whose_state_is_given_through_a_cycle() {
    let src = include_str!("data/machine_given.isc");
    let bin = compile(src, "machine_given");
    let start = std::time::Instant::now();
    let r = run_bin(&bin, &["1000000"]);
    let took = start.elapsed();
    assert_eq!((r.code, r.stdout.as_str()), (0, "[0, 1999998000000, 1, 1000000, 1000000, 1000000, 1000000, 999999000000, 0, 14]\n"), "{}", r.stderr);
    assert!(took.as_secs() < 5, "seven million steps in {:?}", took);
    for a in ["1", "10"] {
        let want = interp_ok(src, &[a]);
        let r = run_bin(&bin, &[a]);
        assert_eq!((r.code, r.stdout.as_str()), (0, want.as_str()), "{}: {}", a, r.stderr);
    }
    let p = iscc::frontend("machine_given.isc", src).unwrap();
    let copies: Vec<u32> = isc_ir::report::entries(&p).iter().filter(|e| e.kind == "copy").map(|e| e.line).collect();
    assert!(copies.iter().all(|l| *l == 91 || *l == 92), "the lines whose values are copied: {:?}", copies);
    clean(src, "machine_given_vg", &["30"]);
}

/// D660: a state whose array is grown by a function before the state is
/// made anew (`growTo(s.xs, s.n + 1)`, one branch of which gives back the
/// array itself, the other a longer copy), all its fields replaced, in a
/// loop: its parts taken where the block that reads them by its parts
/// begins, an argument of a function inlined read as the part itself, the
/// array moved and updated in place. A million steps in milliseconds (a
/// copy of the array at each step: minutes); the copies of the memory
/// report those of the literal of `main` alone; the output of the
/// interpreter, Valgrind clean.
#[test]
fn a_state_whose_array_is_grown_before_it_is_made_anew() {
    let src = include_str!("data/state_grown.isc");
    let bin = compile(src, "state_grown");
    let start = std::time::Instant::now();
    let r = run_bin(&bin, &["1000000"]);
    let took = start.elapsed();
    assert_eq!((r.code, r.stdout.as_str()), (0, "[1000000, 1499998500000, 2999997, 2]\n"), "{}", r.stderr);
    assert!(took.as_secs() < 5, "a million steps in {:?}", took);
    for a in ["1", "40"] {
        let want = interp_ok(src, &[a]);
        let r = run_bin(&bin, &[a]);
        assert_eq!((r.code, r.stdout.as_str()), (0, want.as_str()), "{}: {}", a, r.stderr);
    }
    let p = iscc::frontend("state_grown.isc", src).unwrap();
    let copies: Vec<u32> = isc_ir::report::entries(&p).iter().filter(|e| e.kind == "copy").map(|e| e.line).collect();
    assert!(copies.iter().all(|l| *l == 28), "the lines whose values are copied: {:?}", copies);
    clean(src, "state_grown_vg", &["300"]);
}

/// D661: the state of a loop taken apart (D206) whose new state reads a
/// part that another of its components keeps as it is (`z`, updated from
/// `b[i % 1000]`, written after `b` in the tuple): its components computed
/// first, the parts moved after them. Before, `b` (100 000 elements) was
/// copied at each step (20 000 steps in 0.45 s; the fast core of the
/// machine of Lua of isocron-ui copied its cells at each upvalue read). A
/// million steps in milliseconds; the copies of the memory report those of
/// the literal of `main` alone; the output of the interpreter, Valgrind
/// clean.
#[test]
fn a_loop_state_whose_new_state_reads_a_part_it_keeps() {
    let src = include_str!("data/loop_state_parts.isc");
    let bin = compile(src, "loop_state_parts");
    let start = std::time::Instant::now();
    let r = run_bin(&bin, &["1000000"]);
    let took = start.elapsed();
    assert_eq!((r.code, r.stdout.as_str()), (0, "[1000000, 374625000, 100000]\n"), "{}", r.stderr);
    assert!(took.as_secs() < 5, "a million steps in {:?}", took);
    for a in ["1", "20"] {
        let want = interp_ok(src, &[a]);
        let r = run_bin(&bin, &[a]);
        assert_eq!((r.code, r.stdout.as_str()), (0, want.as_str()), "{}: {}", a, r.stderr);
    }
    let p = iscc::frontend("loop_state_parts.isc", src).unwrap();
    let copies: Vec<u32> = isc_ir::report::entries(&p).iter().filter(|e| e.kind == "copy").map(|e| e.line).collect();
    assert!(copies.iter().all(|l| *l == 16), "the lines whose values are copied: {:?}", copies);
    clean(src, "loop_state_parts_vg", &["50"]);
}

/// D662: a new tuple returned by a function, read by its parts to make
/// another state anew (`const h = step(...); { ...s, xs: h.xs, n: h.n }`):
/// its parts taken where the expression that reads them begins (a block,
/// a tuple, an `if`), the array moved, not copied (a million rounds over an
/// array of 100 000 elements: 23 s, now milliseconds); and a component
/// that is the state itself (`[g, s]` taken apart, `anyReg` of the compiler
/// of Lua of isocron-ui), the state read after it: not taken apart there
/// (the memory check refused the program). No copy in `step` or `round`;
/// the output of the interpreter for both forms, Valgrind clean.
#[test]
fn a_new_tuple_read_by_its_parts_to_make_a_state() {
    let src = include_str!("data/hot_parts.isc");
    let bin = compile(src, "hot_parts");
    let start = std::time::Instant::now();
    let r = run_bin(&bin, &["1000000"]);
    let took = start.elapsed();
    assert_eq!(
        (r.code, r.stdout.as_str()),
        (0, "[1000000, 62499937515, 100000]\n{free: 11, k: [1, 2], names: [x, y, z]}\n{free: 14, k: [1, 2, 5], names: [x, y, z]}\n"),
        "{}",
        r.stderr
    );
    assert!(took.as_secs() < 5, "a million rounds in {:?}", took);
    for a in ["1", "30"] {
        let want = interp_ok(src, &[a]);
        let r = run_bin(&bin, &[a]);
        assert_eq!((r.code, r.stdout.as_str()), (0, want.as_str()), "{}: {}", a, r.stderr);
    }
    let p = iscc::frontend("hot_parts.isc", src).unwrap();
    let copies: Vec<u32> = isc_ir::report::entries(&p).iter().filter(|e| e.kind == "copy").map(|e| e.line).collect();
    assert!(copies.iter().all(|l| *l > 25), "the lines whose values are copied: {:?}", copies);
    clean(src, "hot_parts_vg", &["40"]);
}

#[test]
fn the_payload_of_a_node_made_anew() {
    let src = include_str!("data/node_payload.isc");
    let bin = compile(src, "node_payload");
    for a in ["0", "3", "40"] {
        let want = interp_ok(src, &[a]);
        let r = run_bin(&bin, &[a]);
        assert_eq!((r.code, r.stdout.as_str()), (0, want.as_str()), "{}: {}", a, r.stderr);
    }
    let r = run_bin(&bin, &["2"]);
    assert_eq!(r.stdout, "root.0.1[top](leaf2[c1](a2),b,leaf2[c1](a2))\nroot[](leaf2[](a2),b,leaf2[own](a2))\nleaf2[](a2)\n");
    clean(src, "node_payload_vg", &["5"]);
}

/// D667: a machine of a million numbers in an application kept through
/// 2 000 rounds: run from a block that read a row of the application first,
/// put back from a tuple a call returns, held and let go within the
/// application (a part of a part made anew); each a copy of the whole
/// machine at each round before (4.7 s, now milliseconds); no copy of it in
/// the report; the output of the interpreter, Valgrind clean
#[test]
fn a_machine_given_with_its_application() {
    let src = include_str!("data/parts_given.isc");
    let bin = compile(src, "parts_given");
    let start = std::time::Instant::now();
    let r = run_bin(&bin, &["1000000", "2000"]);
    let took = start.elapsed();
    assert_eq!((r.code, r.stdout.as_str()), (0, "(4, true, 0, 0)\n"), "{}", r.stderr);
    assert!(took.as_millis() < 500, "2 000 rounds in {:?}", took);
    for a in [["10", "1"], ["10", "4"]] {
        let want = interp_ok(src, &a);
        let r = run_bin(&bin, &a);
        assert_eq!((r.code, r.stdout.as_str()), (0, want.as_str()), "{:?}: {}", a, r.stderr);
    }
    let p = iscc::frontend("parts_given.isc", src).unwrap();
    let copies: Vec<(u32, String)> = isc_ir::report::entries(&p).iter().filter(|e| e.kind == "copy" && e.what.contains("[f64[], i64, i64]")).map(|e| (e.line, e.what.clone())).collect();
    assert!(copies.is_empty(), "the copies: {:?}", copies);
    clean(src, "parts_given_vg", &["20", "3"]);
}

/// D666: a state of a million numbers kept through 2 000 rounds by four
/// forms, each a copy of the whole state at each call before: a number read
/// from it before it is made anew, with a path that gives it back whole; a
/// view of it (a row) given beside it to a function that takes it; a
/// function that makes a frame anew from its parts, the part it keeps read
/// before; the state made anew as the component of a tuple
#[test]
fn a_state_given_beside_its_views() {
    let src = include_str!("data/given_views.isc");
    let bin = compile(src, "given_views");
    let start = std::time::Instant::now();
    let r = run_bin(&bin, &["1000000", "2000"]);
    let took = start.elapsed();
    assert_eq!((r.code, r.stdout.as_str()), (0, "(4003, y, 2000, 2000)\n"), "{}", r.stderr);
    assert!(took.as_millis() < 500, "2 000 rounds in {:?}", took);
    for a in [["3", "1"], ["10", "4"]] {
        let want = interp_ok(src, &a);
        let r = run_bin(&bin, &a);
        assert_eq!((r.code, r.stdout.as_str()), (0, want.as_str()), "{:?}: {}", a, r.stderr);
    }
    // (the rows and the strings copied, small; never the machine, the
    // application or the frame, whose types hold `[f64[], i64`)
    let p = iscc::frontend("given_views.isc", src).unwrap();
    let copies: Vec<(u32, String)> = isc_ir::report::entries(&p).iter().filter(|e| e.kind == "copy" && e.what.contains("[f64[], i64")).map(|e| (e.line, e.what.clone())).collect();
    assert!(copies.is_empty(), "the copies: {:?}", copies);
    clean(src, "given_views_vg", &["20", "3"]);
}

/// D665: a value read before a state is given, a parameter held whole by a
/// new tuple given: the results of a call read out of a state given back
/// with them, a length read after the state is made anew, the state given
/// whole into a new record; the state of 200 000 strings never copied
/// (2 000 rounds of 50 calls in milliseconds, more than two minutes
/// before); no copy in the report; the output of the interpreter, Valgrind
/// clean.

#[test]
fn a_state_read_before_it_is_given() {
    let src = include_str!("data/host_state.isc");
    let bin = compile(src, "host_state");
    let start = std::time::Instant::now();
    let r = run_bin(&bin, &["200000", "2000"]);
    let took = start.elapsed();
    assert_eq!((r.code, r.stdout.as_str()), (0, "[2550000, 200000, 147480550000]\n"), "{}", r.stderr);
    assert!(took.as_secs() < 3, "2 000 rounds in {:?}", took);
    for a in [["3", "1"], ["10", "4"]] {
        let want = interp_ok(src, &a);
        let r = run_bin(&bin, &a);
        assert_eq!((r.code, r.stdout.as_str()), (0, want.as_str()), "{:?}: {}", a, r.stderr);
    }
    let p = iscc::frontend("host_state.isc", src).unwrap();
    let copies: Vec<(u32, String)> = isc_ir::report::entries(&p).iter().filter(|e| e.kind == "copy").map(|e| (e.line, e.what.clone())).collect();
    assert!(copies.is_empty(), "the copies: {:?}", copies);
    clean(src, "host_state_vg", &["20", "3"]);
}

/// D693 to D696: a state in the shape of the editor of isocron-ui (tabs, the
/// document of the active one with its history) changed at each step by
/// functions that are called, without copying its history: the tab made
/// anew by a `map` over the places of the array written as that element
/// updated (D693); the document taken from the state for the function that
/// changes it, its parts that nothing reads again (its history, its text)
/// moved rather than copied, zeros left in the state (D693); a parameter
/// consumed on some paths only given when no call keeps its argument, the
/// callers' own parameters so given counted as owned (D695); the name of the
/// tab shown before, read after the state is given, computed first (D695)
/// and copied rather than the state (D696); an argument read again by the
/// argument after it given all the same (D694). The same results as the
/// interpreter, nothing lost or freed twice.
#[test]
fn states_updated_in_place_through_calls() {
    let src = "type Snap = { stamp: i64, notes: string[] };
type Doc = { version: i64, cursor: i64, undo: Snap[], text: string };
type Tab = { name: string, doc: Doc };
type State = { tabs: Tab[], active: i64, log: string[], moves: i64 };

function capped(u: Snap[n]): Snap[] {
    return n > 50 ? map(iota(50), (k) => u[n - 50 + k]) : u;
}

@noInline
function edited(d: Doc, c: i64): Doc {
    const undo = c % 3 != 0 ? d.undo : concat(d.undo, [{ stamp: d.version, notes: [`n${c}`] }]);
    return { ...d, version: d.version + 1, cursor: (d.cursor + c) % 1000, undo: capped(undo) };
}

@noInline
function changed(m: State, d: Doc): State {
    const moved = m.active < 0 || m.active >= m.tabs.length || m.tabs[m.active].doc.cursor != d.cursor;
    const tabs = map(iota(m.tabs.length), (k) => k == m.active ? { ...m.tabs[k], doc: d } : m.tabs[k]);
    return { ...m, tabs: tabs, moves: m.moves + (moved ? 1 : 0) };
}

@noInline
function once(m: State, c: i64): [State, i64] {
    if (c < 0) {
        return [{ tabs: [], active: 0, log: [], moves: m.moves }, 0] as const;
    }
    const d = m.tabs[m.active].doc;
    return [changed(m, edited(d, c)), 1] as const;
}

function step(m: State, c: i64): State {
    const [m1, n] = once(m, c);
    const before = m.active >= 0 && m.active < m.tabs.length ? m.tabs[m.active].name : \"\";
    const shown = m1.active >= 0 && m1.active < m1.tabs.length ? m1.tabs[m1.active].name : \"\";
    return shown != before ? { ...m1, moves: m1.moves + 100 } : m1;
}

function steps(m: State, k: i64, n: i64): State {
    return k >= n ? m : steps(step(m, k), k + 1, n);
}

@noInline
function stamped(d: Doc, n: i64): Doc {
    return { ...d, version: n };
}

action main(args: string[k]): i32 {
    const doc = { version: 0, cursor: 0, undo: [], text: \"a\" };
    const m0 = { tabs: [{ name: \"a.lua\", doc: doc }, { name: \"b.lua\", doc: doc }], active: 1, log: [\"ready\"], moves: 0 };
    const m = steps(m0, 0, 400);
    const d = m.tabs[1].doc;
    print(`${d.version} ${d.cursor} ${d.undo.length} ${d.undo[0].stamp} ${d.undo[49].notes[0]} ${d.text.length} ${m.moves} ${m.log.length} ${m.tabs[0].doc.version}`);
    const d3 = edited({ ...d, text: \"b\" }, 4);
    const d4 = stamped(d3, d3.version + d3.undo.length);
    print(d4.version);
    return 0;
}
";
    let want = interp_ok(src, &[]);
    assert_eq!(want, "400 800 50 252 n399 1 399 1 0\n451\n");
    let bin = compile(src, "in_place_states");
    let run = run_bin(&bin, &[]);
    assert_eq!((run.code, run.stdout.as_str()), (0, want.as_str()), "{}", run.stderr);
    let p = iscc::frontend("in_place.isc", src).unwrap();
    let given = |n: &str| p.fns.iter().find(|f| f.name == n).map(|f| f.given.clone()).unwrap_or_default();
    let names: Vec<_> = p.fns.iter().map(|f| (&f.name, &f.given)).collect();
    assert_eq!((given("once"), given("changed"), given("edited"), given("stamped")), (vec![true, false], vec![true, true], vec![true, false], vec![true, false]), "{:?}", names);
    let line = |m: &str| src.lines().position(|l| l.contains(m)).unwrap() as u32 + 1;
    let at = |l: u32| p.copy_reasons.iter().filter(|(s, _)| s.line == l).map(|(_, r)| r.clone()).collect::<Vec<_>>();
    // the copies of the program, those that take parts with their parts
    fn copies(e: &isc_ir::Expr, out: &mut Vec<(u32, Option<Vec<Vec<u32>>>)>) {
        match &e.kind {
            isc_ir::ExprKind::Copy(_) => out.push((e.span.line, None)),
            isc_ir::ExprKind::CopyTaking(_, ps) => out.push((e.span.line, Some(ps.clone()))),
            _ => {}
        }
        for c in e.children() {
            copies(c, out);
        }
    }
    let mut all = Vec::new();
    for f in p.fns.iter().filter(|f| !f.name.starts_with('$')) {
        copies(&f.body, &mut all);
    }
    copies(&p.body, &mut all);
    let of = |l: u32| all.iter().filter(|(k, _)| *k == l).map(|(_, t)| t.clone()).collect::<Vec<_>>();
    // the document taken from the state: its text and history moved
    assert_eq!(of(line("return [changed(m, edited(d, c)), 1]")), vec![Some(vec![vec![1], vec![2]])], "{:?}", all);
    // the tab made anew: no copy of the others, nor of the new document;
    // the name of the tab moved out of the old one
    assert_eq!(of(line("const tabs = map(iota")), vec![Some(vec![vec![]])], "{:?}", all);
    // the name of the tab before copied, not the state
    let before = at(line("const before = m.active"));
    assert!(before.len() == 1 && before[0].contains("(D696)"), "{:?}", p.copy_reasons);
    assert!(of(line("const [m1, n] = once(m, c)")).is_empty() && of(line("const d4 = stamped(d3")).is_empty(), "{:?}", all);
    clean(src, "in_place_states_vg", &[]);
}

/// D717: the gather of the permutation of a sort found on the way the body
/// of the binding of its array is evaluated (here in the text that `print`
/// writes, as in countwords): the texts of the pairs moved, not copied; a
/// program that reads the pairs again after copies them. The `Map` whose
/// entries alone are read computes no index of the hashes of its keys.
#[test]
fn sorted_pairs_moved_where_printed() {
    if memcheck() || std::env::var("ISC_ALLOC").is_ok() {
        return;
    }
    let moved = "action main(args: string[k]): i32 {
    const n = parseI64(args[0]);
    const words = map(iota(n), (i) => `${i}-0123456789012345678901234567890`);
    const counts = countBy(words, (w) => w);
    const es = counts.entries();
    print(join(map(sortBy(es, (e) => {
        const [w, c] = e;
        return -w.length;
    }), (e) => {
        const [w, c] = e;
        return `${w.length} ${c}`;
    }), \"\\n\"));
    const status = lines(readText(\"/proc/self/status\"));
    print(filter(status, (l) => l.startsWith(\"VmHWM\")));
    return 0;
}
";
    let copied = moved.replace("    const status =", "    print(es[0]);\n    const status =");
    let run = |src: &str, name: &str, args: &[&str]| -> (String, u64) {
        let bin = compile(src, name);
        checked_clean_bin(&bin, args, None);
        let r = run_bin(&bin, args);
        assert_eq!(r.code, 0, "{}", r.stderr);
        let mut ls: Vec<&str> = r.stdout.lines().collect();
        let hwm = ls.pop().unwrap_or("").chars().filter(|c| c.is_ascii_digit()).collect::<String>().parse().unwrap_or_else(|_| panic!("{}", r.stdout));
        (ls[..3].join("\n"), hwm)
    };
    let (out_m, most_m) = run(moved, "pairs_moved", &["200000"]);
    let (out_c, most_c) = run(&copied, "pairs_copied", &["200000"]);
    assert_eq!(out_m, "38 1\n38 1\n38 1");
    assert_eq!(out_m, out_c);
    // (half the bytes of the keys at least: 200000 of 38 bytes, blocks of
    // 48 bytes since D718)
    assert!(most_m + 4 * 1024 < most_c, "moved {} KiB, copied {} KiB", most_m, most_c);
    // the interpreter's output
    let small = |src: &str| src.replace("    const status = lines(readText(\"/proc/self/status\"));\n    print(filter(status, (l) => l.startsWith(\"VmHWM\")));\n", "");
    let bin = compile(&small(moved), "pairs_moved_small");
    assert_eq!(run_bin(&bin, &["50"]).stdout, interp_ok(&small(moved), &["50"]));
    // no index of hashes computed
    fn hash_indexes(e: &isc_ir::Expr) -> usize {
        usize::from(matches!(e.kind, isc_ir::ExprKind::StrOp(isc_ir::StrOp::HashIndex, _))) + e.children().into_iter().map(hash_indexes).sum::<usize>()
    }
    let p = iscc::frontend("pairs.isc", moved).unwrap();
    assert_eq!(hash_indexes(&p.body), 0);
}
