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
        if in_task && line.contains("call void @isc_parallel_for_dynamic") {
            nested += 1;
        }
    }
    assert!(nested >= 2, "no loop shared from inside a task");
    for t in THREADS {
        let run = run_bin_threads(&bin, &[], Some(t));
        assert_eq!((run.code, run.stdout.as_str()), (0, want.as_str()), "{} threads: {}", t, run.stderr);
    }
}
