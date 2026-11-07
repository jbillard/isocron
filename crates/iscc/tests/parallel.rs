//! Block parallelism (stage 2, phase 6): results do not depend on the number
//! of threads (SPEC section 10).

mod common;
use common::*;

const THREADS: [usize; 5] = [1, 2, 3, 4, 8];

#[test]
fn poc_programs_do_not_depend_on_threads() {
    let n = 300_007;
    let a = random_f64s(41, n);
    let b = random_f64s(42, n);
    let pa = write_f64s("p_a.f64", &a);
    let pb = write_f64s("p_b.f64", &b);
    let mut r = Rng(43);
    let bytes: Vec<u8> = (0..n).map(|_| (r.next() % 256) as u8).collect();
    let pu = tmp("p_u.u8");
    std::fs::write(&pu, &bytes).unwrap();
    let pu = pu.to_string_lossy().to_string();
    for (name, src, inputs) in [
        ("sum", SUM, vec![pa.clone()]),
        ("dot", DOT, vec![pa.clone(), pb.clone()]),
        ("std_dev", STD, vec![pa.clone()]),
        ("histogram", HIST, vec![pu.clone()]),
    ] {
        let bin = compile(src, &format!("p_{}", name));
        let reference = tmp(&format!("p_{}_interp.f64", name)).to_string_lossy().to_string();
        let mut args: Vec<&str> = inputs.iter().map(|s| s.as_str()).collect();
        args.push(&reference);
        args.push("1");
        interp_ok(src, &args);
        let want = std::fs::read(&reference).unwrap();
        for t in THREADS {
            let out = tmp(&format!("p_{}_{}.f64", name, t)).to_string_lossy().to_string();
            let mut args: Vec<&str> = inputs.iter().map(|s| s.as_str()).collect();
            args.push(&out);
            args.push("2");
            let run = run_bin_threads(&bin, &args, Some(t));
            assert_eq!(run.code, 0, "{} with {} threads: {}", name, t, run.stderr);
            assert_eq!(std::fs::read(&out).unwrap(), want, "{} with {} threads", name, t);
        }
    }
}

#[test]
fn maps_and_multi_level_reductions_do_not_depend_on_threads() {
    // three levels of blocks, a materialized map and its reduction
    let n: i64 = 4097 * 4096 + 5;
    let src = format!(
        "function f(): i64 {{\n    return reduce(iota({n}), (a, b) => a * 3 - b, 1);\n}}\n\
         function g(): f64 {{\n    return sum(map(iota({n}), (i) => 1.0 / f64(i + 1)));\n}}\n\
         action main(args: string[k]): i32 {{\n    \
         const m = map(iota(100003), (i) => f64(i) * 0.5 - 7.0);\n    \
         writeF64(args[0], m);\n    \
         print(f());\n    \
         print(g());\n    \
         print(reduce(m, (a, b) => a - b, 0.25));\n    \
         return 0;\n}}\n"
    );
    let bin = compile(&src, "p_levels");
    let mut outs = Vec::new();
    for t in THREADS {
        let f = tmp(&format!("p_levels_{}.f64", t)).to_string_lossy().to_string();
        let run = run_bin_threads(&bin, &[&f], Some(t));
        assert_eq!(run.code, 0, "{}", run.stderr);
        outs.push((run.stdout, std::fs::read(&f).unwrap()));
    }
    for (i, o) in outs.iter().enumerate() {
        assert_eq!(o, &outs[0], "{} threads differ from 1 thread", THREADS[i]);
    }
    let xs: Vec<i64> = (0..n).collect();
    let want_i = fixed_reduce(&xs, 1i64, &|a: i64, b: i64| a.wrapping_mul(3).wrapping_sub(b));
    assert!(outs[0].0.starts_with(&format!("{}\n", want_i)), "{}", outs[0].0);
}

/// Rule P (privatized accumulation, DECISIONS D50): same results, same panic.
#[test]
fn privatized_folds_match_the_interpreter() {
    let mut r = Rng(77);
    let bytes: Vec<u8> = (0..50_001).map(|_| (r.next() % 256) as u8).collect();
    let pu = tmp("p_rule_p.u8");
    std::fs::write(&pu, &bytes).unwrap();
    let pu = pu.to_string_lossy().to_string();
    let cases = [
        // non-zero initial value, variable and wrapping increments (u8)
        "function f(x: u8[n]): u8[4] {\n    return fold(x, [5u8, 250u8, 0u8, 7u8], (c, v) => c.with(v % 4u8, c[v % 4u8] + v));\n}\n",
        // the increment first, a converted index, i64 counters
        "function f(x: u8[n]): i64[16] {\n    return fold(x, zeros(), (c, v) => c.with(i64(v) / 16, 3 - i64(v) + c[i64(v) / 16]));\n}\n",
        // an index out of bounds: the panic is the same (200 counters: a u8
        // index may exceed them, so every index is checked)
        "function f(x: u8[n]): u32[200] {\n    return fold(x, zeros(), (c, v) => c.with(v, c[v] + 1u32));\n}\n",
        // 256 counters: a u8 index cannot exceed them, the loop has no check
        "function f(x: u8[n]): u32[256] {\n    return fold(x, zeros(), (c, v) => c.with(v, c[v] + 2u32));\n}\n",
    ];
    for (i, f) in cases.iter().enumerate() {
        let src = format!("{}\naction main(args: string[k]): i32 {{\n    print(f(readBytes(args[0])));\n    return 0;\n}}\n", f);
        let bin = compile(&src, &format!("p_rule_p{}", i));
        let ll = std::fs::read_to_string(bin.with_extension("ll")).unwrap();
        assert!(ll.contains("@isc_thread_count"), "case {}: rule P not applied", i);
        let want = interp(&src, &[&pu]);
        for t in [1, 4] {
            let run = run_bin_threads(&bin, &[&pu], Some(t));
            match &want {
                Outcome::Exit(code, out) => assert_eq!((run.code, &run.stdout), (*code, out), "case {} T={}", i, t),
                Outcome::Panic(m, _) => {
                    assert_eq!(run.code, 101, "case {} T={}", i, t);
                    assert_eq!(run.stderr.trim_end(), m, "case {} T={}", i, t);
                }
            }
        }
    }
}

#[test]
fn filters_and_divisions_of_the_index_do_not_depend_on_threads() {
    // filters of more than one block (by blocks in parallel, D178):
    // integers, the bytes of a slice, tuples (columns), strings (owned); a
    // map whose index is divided by constants (the quotients and remainders
    // carried by the loop, D180)
    let src = r#"function strs(n: i64): string[k] {
    return filter(map(iota(n), (i) => String(i)), (s) => s.length == 2 || s.length == 5);
}
function firsts(t: [i64, f64][k]): i64[k] {
    return map(t, (p) => {
        const [i, x] = p;
        return i;
    });
}
function seconds(t: [i64, f64][k]): f64[k] {
    return map(t, (p) => {
        const [i, x] = p;
        return x;
    });
}
action main(args: string[k]): i32 {
    const n = 3 * 4096 + 17;
    const a = filter(iota(5 * n), (i) => i % 7 == 3 || i == 5 * n - 1);
    print([a.length, sum(a), a[0], a[a.length - 1]] as const);
    const b = map(iota(4 * n), (i) => u8(i * 37 % 251));
    const c = b.slice(5, 4 * n - 3).filter((x) => x != 10u8 && x < 200u8);
    print([c.length, i64(sum(map(c, (x) => u64(x))))] as const);
    const t = filter(map(iota(n), (i) => [i, f64(i) * 0.5] as const), (p) => {
        const [i, x] = p;
        return i % 3 == 1;
    });
    print([t.length, sum(firsts(t)), sum(seconds(t))] as const);
    const s = strs(n);
    print([s.length, sum(map(s, (x) => x.length))] as const);
    print(s[s.length - 1]);
    const d = map(iota(5 * n), (q) => q % 61 == 60 ? i64(b[q % 4096]) : q / 61 * 1000 + q % 61 + q % 7 * 100000 + q / 4096);
    print(sum(d));
    print([d[4095], d[4096], d[5 * n - 1]] as const);
    print(filter(iota(10), (i) => i % 2 == 0));
    return 0;
}
"#;
    let want = "(8790, 270419953, 3, 61524)\n(39016, 3899678)\n(4102, 25237555, 12618777.5)\n(2395, 11705)\n12304\n48645906552\n(67008, 167010, 1108051)\n[0, 2, 4, 6, 8]\n";
    assert_eq!(interp_ok(src, &[]), want);
    let bin = compile(src, "p_filters");
    let ll = std::fs::read_to_string(bin.with_extension("ll")).unwrap();
    assert!(ll.contains("srem i64 %t") && ll.contains(", 61\n"), "the remainder by 61 is not carried by the loop");
    for t in THREADS {
        let run = run_bin_threads(&bin, &[], Some(t));
        assert_eq!((run.code, run.stdout.as_str()), (0, want), "{} threads: {}", t, run.stderr);
    }
}

const NESTED_SRC: &str = r#"type Tree = Leaf | Node(Tree, Tree);
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
function steps(x: i64, k: i64): i64 {
    if (x <= 1) return k;
    return steps(x % 2 == 0 ? x / 2 : 3 * x + 1, k + 1);
}
action main(args: string[k]): i32 {
    print(map(iota(9), (d) => sum(map(iota(1 << (12 - d)), (i) => check(bottomUp(d))))));
    print(sum(map(iota(5), (a) => sum(map(iota(40), (b) => sum(map(iota(30), (c) => steps(a * 1200 + b * 30 + c + 1, 0))))))));
    print(map(iota(4), (a) => reduce(map(iota(5000), (i) => steps(a * 5000 + i + 1, 0)), (x, y) => Math.max(x, y), 0)));
    print(map(iota(3), (a) => reduce(map(iota(3000), (i) => steps(a * 3000 + i + 1, 0)), (x, y) => Math.min(x, y), 1000000)));
    print(map(iota(3), (n) => sum(map(iota(n), (i) => steps(i + 7, 0)))));
    print(map(iota(3), (a) => reduce(map(iota(5000), (i) => steps(i + a + 1, 0)), (x, y) => x + y, 5)));
    return 0;
}
"#;

#[test]
fn loops_in_tasks_are_shared_and_do_not_depend_on_threads() {
    // D201: an exact reduce (D183) of a map whose elements are loops is
    // shared by chunks, in a task too, where the threads that have finished
    // their share take them: three levels, `min` and `max`, ranges of 0, 1
    // and 2 elements; a reduce whose z is not neutral keeps the fixed form.
    // The results of the interpreter for any number of threads.
    let want = interp_ok(NESTED_SRC, &[]);
    let bin = compile(NESTED_SRC, "p_nested");
    let ll = std::fs::read_to_string(bin.with_extension("ll")).unwrap();
    let mut in_task = false;
    let mut nested = 0;
    for line in ll.lines() {
        if line.starts_with("define") {
            in_task = line.contains("@isc_task_");
        }
        // a loop of chunks taken in turn: mode 1 of its site (D357)
        if in_task && line.contains("call void @isc_parallel_for_site(") && line.ends_with("i32 1)") {
            nested += 1;
        }
    }
    assert!(nested >= 2, "no loop shared from inside a task");
    for t in THREADS {
        let run = run_bin_threads(&bin, &[], Some(t));
        assert_eq!((run.code, run.stdout.as_str()), (0, want.as_str()), "{} threads: {}", t, run.stderr);
    }
}

/// The arrays of a `map` and of a `filter` of more than two huge pages, and
/// the chunks of a printed `map` (D254): each thread fills a contiguous
/// range of their blocks (`isc_parallel_for_fill`); new arrays (the first
/// round) and arrays from the cache of the allocator (the second); the
/// same elements and output for any number of threads.
#[test]
fn arrays_filled_by_ranges_do_not_depend_on_threads() {
    let src = r#"function round(r: i64): i64[3] {
    const n = 1500017 + r;
    const a = map(iota(n), (i) => (i * 7919 + r) % 1000003);
    const b = filter(a, (x) => x % 3 != 0);
    const c = map(iota(6 * n), (i) => u8((i * 37 + r) % 251));
    const d = c.filter((x) => x != 10u8);
    return [sum(b) + b[b.length - 1], i64(sum(map(d, (x) => u64(x)))), b.length * 1000000 + d.length % 1000000];
}
action main(args: string[k]): i32 {
    print(round(0));
    print(round(5));
    const text = map(iota(3000017), (i) => i % 61 == 60 ? 10u8 : u8(65 + (i * 7) % 26));
    printBytes(map(iota(text.length), (i) => text[text.length - 1 - i]));
    return 0;
}
"#;
    let mut want = String::new();
    for r in [0i64, 5] {
        let n = 1500017 + r;
        let b: Vec<i64> = (0..n).map(|i| (i * 7919 + r) % 1000003).filter(|x| x % 3 != 0).collect();
        let d: Vec<u8> = (0..6 * n).map(|i| ((i * 37 + r) % 251) as u8).filter(|&x| x != 10).collect();
        let sb: i64 = b.iter().sum::<i64>() + b[b.len() - 1];
        let sd: i64 = d.iter().map(|&x| x as i64).sum();
        want += &format!("[{}, {}, {}]\n", sb, sd, b.len() as i64 * 1000000 + d.len() as i64 % 1000000);
    }
    let text: Vec<u8> = (0..3000017i64).map(|i| if i % 61 == 60 { 10 } else { 65 + ((i * 7) % 26) as u8 }).collect();
    want += &String::from_utf8(text.iter().rev().copied().collect()).unwrap();
    let bin = compile(src, "p_new_arrays");
    let ll = std::fs::read_to_string(bin.with_extension("ll")).unwrap();
    // a range of blocks for each thread: mode 2 of its site (D357)
    let fills = ll.lines().filter(|l| l.contains("call void @isc_parallel_for_site(") && l.ends_with("i32 2)")).count();
    assert!(fills >= 6, "the arrays are not filled by ranges (mode 2 of isc_parallel_for_site)");
    for t in THREADS {
        let run = run_bin_threads(&bin, &[], Some(t));
        assert_eq!((run.code, run.stdout.as_str()), (0, want.as_str()), "{} threads: {}", t, run.stderr);
    }
}

/// A `filter` that keeps few elements (D255): the blocks that keep nothing
/// are not read again; the elements kept at the edges of the blocks, in the
/// last block, none at all, and a filter that keeps almost everything; the
/// same results for any number of threads.
#[test]
fn sparse_filters_skip_the_empty_blocks() {
    let src = r#"action main(args: string[k]): i32 {
    const n = 40 * 4096 + 123;
    const marks = map(iota(n), (i) => i == 0 || i == 4095 || i == 4096 || i == 9 * 4096 + 17 || i == 31 * 4096 - 1 || i == n - 1 ? 62u8 : 65u8);
    print(filter(iota(n), (i) => marks[i] == 62u8));
    print(filter(iota(n), (i) => marks[i] == 10u8).length);
    const most = filter(marks, (b) => b != 62u8);
    print([most.length, i64(sum(map(most, (b) => u64(b))))] as const);
    const late = filter(iota(n), (i) => i >= n - 3);
    print(late);
    return 0;
}
"#;
    let n: i64 = 40 * 4096 + 123;
    let want = format!(
        "[0, 4095, 4096, {}, {}, {}]\n0\n({}, {})\n[{}, {}, {}]\n",
        9 * 4096 + 17,
        31 * 4096 - 1,
        n - 1,
        n - 6,
        (n - 6) * 65,
        n - 3,
        n - 2,
        n - 1
    );
    let bin = compile(src, "p_sparse_filters");
    for t in THREADS {
        let run = run_bin_threads(&bin, &[], Some(t));
        assert_eq!((run.code, run.stdout.as_str()), (0, want.as_str()), "{} threads: {}", t, run.stderr);
    }
}

/// Rule R (D292): an affine recurrence modulo a constant written as a
/// `fold` over `iota` computed in parts from powers of its step. The same
/// integers as the interpreter for every form of the step (the product and
/// the sum in either order, without the sum, without the product, a
/// multiplier above the modulus, the largest modulus allowed), for the
/// paths that keep the loop as written (fewer than 64 steps, a first value
/// outside `0 .. m`, a modulus too large for the powers, a negative
/// multiplier, an update out of bounds and its panic), in a task, and for
/// any number of threads.
#[test]
fn affine_recurrences_match_the_interpreter() {
    let fold = |n: &str, init: &str, step: &str| format!("fold(iota({}), {}, (v, t) => v.with(t + 1, {}))", n, init, step);
    let cases = [
        // the generator of fasta: the parallel parts
        (fold("139967", "fill(139968, 42)", "(v[t] * 3877 + 29573) % 139968"), true),
        (fold("200003", "fill(200010, 7)", "(29573 + 3877 * v[t]) % 139968"), true),
        (fold("50021", "fill(50022, 1)", "(v[t] * 48271) % 2147483647"), true),
        (fold("30011", "fill(30012, 5)", "(v[t] + 999) % 1000003"), true),
        (fold("70001", "fill(70002, 3)", "(v[t] * 1000000000000 + 12345) % 1000003"), true),
        (fold("90001", "fill(90002, 2)", "(v[t] * 3 + 1) % 3037000499"), true),
        // the first value far above m: v[1] in 0 .. m, then the parts
        (fold("40000", "fill(40001, 1000000000000)", "(v[t] * 3 + 1) % 1000003"), true),
        // the loop as written, chosen when the program runs: 63 steps; v[1]
        // negative; the update of v[n] and its panic
        (fold("63", "fill(64, 9)", "(v[t] * 3877 + 29573) % 139968"), true),
        (fold("40000", "fill(40001, -6)", "(v[t] * 3 + 1) % 7"), true),
        (fold("40000", "fill(40000, 5)", "(v[t] * 3 + 1) % 1000003"), true),
        // the loop alone, the rule refused at compilation: m * (m - 1) too
        // large; a negative multiplier
        (fold("40000", "fill(40001, 5)", "(v[t] * 2 + 1) % 4000000000"), false),
        (fold("40000", "fill(40001, 5)", "(v[t] * -3 + 1) % 1000003"), false),
    ];
    for (i, (expr, rule)) in cases.iter().enumerate() {
        let src = format!(
            "function f(): i64[k] {{\n    return {};\n}}\n\naction main(args: string[k]): i32 {{\n    const v = f();\n    print(v);\n    return 0;\n}}\n",
            expr
        );
        let bin = compile(&src, &format!("p_rule_r{}", i));
        let ll = std::fs::read_to_string(bin.with_extension("ll")).unwrap();
        // the parts of rule R: a loop of mode 0 (D357)
        let plain = ll.lines().any(|l| l.contains("call void @isc_parallel_for_site(") && l.ends_with("i32 0)"));
        assert_eq!(plain, *rule, "case {}: rule R applied or not", i);
        let want = interp(&src, &[]);
        for t in THREADS {
            let run = run_bin_threads(&bin, &[], Some(t));
            match &want {
                Outcome::Exit(code, out) => assert_eq!((run.code, &run.stdout), (*code, out), "case {} T={}", i, t),
                Outcome::Panic(m, _) => {
                    assert_eq!(run.code, 101, "case {} T={}", i, t);
                    assert_eq!(run.stderr.trim_end(), m, "case {} T={}", i, t);
                }
            }
        }
    }
    // in the tasks of a `map`: the parts on the thread of each task
    let src = format!(
        "function g(r: i64): i64 {{\n    const v = {};\n    return v[1] + v[5000 + r] * 3 + v[20000 + r];\n}}\n\naction main(args: string[k]): i32 {{\n    print(map(iota(40), (r) => g(r)));\n    return 0;\n}}\n",
        fold("20000 + r", "fill(20001 + r, r)", "(v[t] * 3877 + 29573) % 139968")
    );
    let bin = compile(&src, "p_rule_r_tasks");
    let want = interp_ok(&src, &[]);
    for t in THREADS {
        let run = run_bin_threads(&bin, &[], Some(t));
        assert_eq!((run.code, run.stdout.as_str()), (0, want.as_str()), "tasks T={}: {}", t, run.stderr);
    }
}

/// D321: the tasks of a parallel fill write at least 16 KiB (16384 bytes,
/// 8192 elements of 2 bytes, B elements of 4 bytes and more), the results
/// of the reductions still by blocks of B: arrays of bytes, `u16` and
/// `f64` around those sizes (one task, two, a partial last one), rows of 61
/// bytes cut by the tasks (lines written by blocks to the output), a sum of
/// floats whose bits fix B; as the interpreter on 1, 3 and 4 threads.
#[test]
fn fills_by_tasks_of_16_kib_do_not_depend_on_threads() {
    let src = "action main(args: string[k]): i32 {
    const n = parseI64(args[0]);
    const bs = map(iota(n), (i) => u8((i * 7 + i / 61) % 251));
    const ws = map(iota(n), (i) => u16((i * 13) % 65521));
    const fs = map(iota(n), (i) => f64(i % 1000) * 0.001 + 1.0 / f64(i + 1));
    print([sum(map(bs, (b) => i64(b))), sum(map(ws, (w) => i64(w))), bs.length]);
    print(sum(fs));
    printBytes(map(iota(n), (q) => q % 61 == 60 || q == n - 1 ? 10u8 : u8(65 + (q / 61 * 60 + q % 61) % 26)));
    return 0;
}
";
    let bin = compile(src, "fills_16k");
    for n in ["16383", "16384", "16385", "32769", "100003", "1048577"] {
        let want = interp_ok(src, &[n]);
        for t in [1, 3, 4] {
            let r = run_bin_threads(&bin, &[n], Some(t));
            assert_eq!((r.code, r.stdout.as_str()), (0, want.as_str()), "n = {}, {} threads: {}", n, t, r.stderr);
        }
    }
}

/// D371: the small blocks of the first 16 classes move between the caches
/// of the threads and the global lists by whole batches (a full list kept
/// in reserve, a reserve given to all the threads in one step): blocks made
/// by the threads of a `map`, freed by `main`, made again by the threads,
/// 40 rounds; nothing wrong in their contents with 1, 2 and 4 threads.
#[test]
fn small_blocks_move_between_threads_by_batches() {
    let src = include_str!("data/heap_batches.isc");
    let bin = compile(src, "heap_batches");
    for t in [1, 2, 4] {
        let r = run_bin_threads(&bin, &["40", "20000"], Some(t));
        assert_eq!((r.code, r.stdout.as_str()), (0, "0\n"), "{} threads: {}", t, r.stderr);
    }
}

/// D372: the TLS block of a compiled program aligned on a page of 4 KiB
/// (the segment `PT_TLS`, its alignment 4096), so that the cache of the
/// allocator of each thread is at the same place of its page in every
/// program, whatever the static data of the runtime.
#[test]
fn thread_caches_at_a_fixed_place_of_their_page() {
    let bin = compile("action main(args: string[k]): i32 {\n    print(sum(map(iota(100000), (i) => `${i}`.length)));\n    return 0;\n}\n", "tls_page");
    let b = std::fs::read(&bin).unwrap();
    let rd = |at: usize, n: usize| -> u64 { b[at..at + n].iter().rev().fold(0u64, |a, &x| (a << 8) | x as u64) };
    assert_eq!(&b[0..4], b"\x7fELF");
    let (phoff, phentsize, phnum) = (rd(0x20, 8) as usize, rd(0x36, 2) as usize, rd(0x38, 2) as usize);
    let tls = (0..phnum).map(|i| phoff + i * phentsize).find(|&ph| rd(ph, 4) == 7).expect("no PT_TLS");
    assert_eq!(rd(tls + 0x30, 8), 4096, "the TLS segment not aligned on a page");
    for t in [1, 4] {
        let r = run_bin_threads(&bin, &[], Some(t));
        assert_eq!((r.code, r.stdout.as_str()), (0, "488890\n"), "{}", r.stderr);
    }
}

/// D460: on one thread a parallel loop runs at once, without the choice of
/// its site (no clock read, nothing written to its words): loops of 0, 1, 2
/// and 1000 elements, nested, with an inner loop chosen by a site, on 1 and
/// 4 threads; the interpreter's output.
#[test]
fn loops_on_one_thread_run_at_once() {
    let src = "function inner(i: i64): i64 {
    return sum(map(iota(i % 7), (j) => (i * j) % 11));
}

action main(args: string[k]): i32 {
    print(map([0, 1, 2, 1000], (n) => sum(map(iota(n), (i) => inner(i) + sum(map(iota(i % 3), (j) => j * i))))));
    return 0;
}
";
    let Outcome::Exit(0, want) = interp(src, &[]) else { panic!("the interpreter failed") };
    let bin = compile(src, "one_thread_loops");
    for t in [1, 4] {
        for _ in 0..3 {
            let r = run_bin_threads(&bin, &[], Some(t));
            assert_eq!((r.code, r.stdout.as_str()), (0, want.as_str()), "T={}: {}", t, r.stderr);
        }
    }
}
