//! Actions as values (DECISIONS D352, SPEC section 12): the type `action
//! (T) => U`, an action or a function given where it is expected, a lambda
//! that is an action, generic actions, `forEach`; the same output
//! interpreted and compiled; the uses refused at compile time.

mod common;
use common::*;

fn error(src: &str) -> String {
    match iscc::frontend("test.isc", src) {
        Ok(_) => panic!("expected an error"),
        Err(e) => e,
    }
}

const ACTIONS: &str = r#"action twice(f: action (i64) => i64, x: i64): i64 {
    const a = f(x);
    return f(a);
}

action shown(x: i64): i64 {
    print(`shown ${x}`);
    return x + 1;
}

function doubled(x: i64): i64 {
    return 2 * x;
}

action eachIndex<T>(x: T[n], f: action (T, i64) => void, i: i64): void {
    if (i < x.length) {
        f(x[i], i);
        eachIndex(x, f, i + 1);
    }
}

action retry(f: action (i64) => boolean, tries: i64, i: i64): i64 {
    if (i == tries) {
        return -1;
    }
    return f(i) ? i : retry(f, tries, i + 1);
}

action main(args: string[k]): i32 {
    print(twice(shown, 1));
    print(twice(doubled, 5));
    const pick = args.length > 0 ? shown : doubled;
    print(pick(10));
    eachIndex(["a", "b"], (s, i) => {
        print(`${i}: ${s}`);
    }, 0);
    forEach([1.5, 2.5], (v) => print(v));
    print(retry((i) => {
        print(`try ${i}`);
        return i == 2;
    }, 5, 0));
    print(map([1, 2, 3], doubled));
    return 0;
}
"#;

/// The uses allowed: the same output interpreted and compiled, on 1 and 4
/// threads, an action chosen at run time among two (`pick`).
#[test]
fn actions_given_as_values() {
    let want = "shown 1\nshown 2\n3\n20\nshown 10\n11\n0: a\n1: b\n1.5\n2.5\ntry 0\ntry 1\ntry 2\n2\n[2, 4, 6]\n";
    assert_eq!(interp_ok(ACTIONS, &["x"]), want, "interpreter");
    let bin = compile(ACTIONS, "actions_values");
    for t in [1, 4] {
        let r = run_bin_threads(&bin, &["x"], Some(t));
        assert_eq!((r.code, r.stdout.as_str()), (0, want), "compiled, {} threads: {}", t, r.stderr);
    }
    let r = run_bin(&bin, &[]);
    assert!(r.stdout.contains("\n20\n20\n0: a"), "the other action of `pick`: {}", r.stdout);
}

/// The uses refused, each with its place and its reason.
#[test]
fn actions_refused_where_they_would_have_an_effect() {
    let main = "\naction main(args: string[k]): i32 {\n    return 0;\n}\n";
    let cases: [(&str, &str); 6] = [
        (
            "action show(x: i64): i64 {\n    print(x);\n    return x;\n}\naction go(): void {\n    print(map([1, 2], show));\n}\n",
            "expected `(_) => _`, found `action (i64) => i64`",
        ),
        ("action go(): void {\n    print(map([1, 2], (x) => {\n        print(x);\n        return x;\n    }));\n}\n", "(`print` is one): this lambda is pure"),
        ("function apply(f: action (i64) => i64, x: i64): i64 {\n    return x;\n}\n", "the parameter `f` of the `function` `apply` is an action"),
        ("action show(x: i64): i64 {\n    return x;\n}\nfunction pick(): i64 {\n    const f = show;\n    return 1;\n}\n", "the action `show` is a value only inside an `action`"),
        ("action twice(f: action (i64) => i64, xs: i64[n]): void {\n    print(map(xs, (x) => f(x)));\n}\n", "the action `f` cannot be called inside a lambda that is not an action"),
        ("action run(f: action (i64)): void {\n}\n", "expected `=>` after the parameters of the type of an action"),
    ];
    for (src, want) in cases {
        let e = error(&format!("{}{}", src, main));
        assert!(e.contains(want), "{}\n--- gives ---\n{}", src, e);
    }
    // a function that prints: the action named, the way to effects given
    let e = error("function hello(x: i64): i64 {\n    print(x);\n    return x;\n}\naction main(args: string[k]): i32 {\n    return 0;\n}\n");
    assert!(e.contains("(`print` is one): a `function` computes a value and has no effect") && e.contains("an action given to `http.serve`"), "{}", e);
}

/// The output of `actions_parallel.isc` cut at each line printed outside
/// `mapParallel`: the lines of its tasks (in any order: they run at the same
/// time) sorted, then that line.
fn parallel_segments(out: &str) -> Vec<(Vec<String>, String)> {
    let (mut segs, mut cur) = (Vec::new(), Vec::new());
    for l in out.lines() {
        if (l.starts_with('[') && l.matches(',').count() == 11) || l.starts_with("word ") {
            cur.push(l.to_string());
        } else {
            cur.sort();
            segs.push((std::mem::take(&mut cur), l.to_string()));
        }
    }
    segs
}

/// D373: `mapParallel`: 16 actions that print 300 lines each at the same
/// time (no line cut by another), an action given by its name, a pure
/// function, `mapParallel` inside a task, an empty array; the same lines as
/// the interpreter (which runs the tasks one after the other) and the
/// results in order, compiled on 1 and 4 threads.
#[test]
fn actions_in_parallel() {
    let src = include_str!("data/actions_parallel.isc");
    let want = parallel_segments(&interp_ok(src, &[]));
    let tails: Vec<&str> = want.iter().map(|s| s.1.as_str()).collect();
    assert_eq!(tails, ["sum 1240", "[ok 1, ok 2, long ccc]", "[1, 4, 9]", "[5, 365]", "0"]);
    assert_eq!(want.iter().map(|s| s.0.len()).collect::<Vec<_>>(), [4800, 3, 0, 1800, 0]);
    let bin = compile(src, "actions_parallel");
    for t in [1, 4] {
        let r = run_bin_threads(&bin, &[], Some(t));
        assert_eq!(r.code, 0, "{} threads: {}", t, r.stderr);
        assert!(parallel_segments(&r.stdout) == want, "compiled, {} threads: other lines", t);
    }
}

/// D373: two actions that wait for each other (one listens and reads, the
/// other connects and writes) run at the same time, on 2 and 4 threads.
#[test]
fn actions_in_parallel_wait_for_each_other() {
    let bin = compile(include_str!("data/actions_parallel_wait.isc"), "actions_parallel_wait");
    for t in ["2", "4"] {
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port().to_string();
        let mut child = std::process::Command::new(&bin).arg(&port).env("ISC_THREADS", t).stdout(std::process::Stdio::piped()).spawn().unwrap();
        let start = std::time::Instant::now();
        while child.try_wait().unwrap().is_none() {
            if start.elapsed() > std::time::Duration::from_secs(60) {
                child.kill().unwrap();
                panic!("{} threads: the two actions did not run at the same time", t);
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let o = child.wait_with_output().unwrap();
        assert_eq!((o.status.code(), String::from_utf8_lossy(&o.stdout).into_owned()), (Some(0), "[hello from the other task, sent]\n".to_string()), "{} threads", t);
    }
}

/// D373: the uses of `mapParallel` refused at compile time.
#[test]
fn actions_in_parallel_refused() {
    let main = "\naction main(args: string[k]): i32 {\n    return 0;\n}\n";
    let cases: [(&str, &str); 4] = [
        ("function f(xs: i64[n]): i64[n] {\n    return mapParallel(xs, (x) => x + 1);\n}\n", "a `function` cannot call the action `mapParallel`"),
        ("action f(xs: i64[n]): void {\n    print(map(xs, (x) => mapParallel([x], (y) => y).length));\n}\n", "the action `mapParallel` cannot be called inside a lambda"),
        ("action f(n: i64): void {\n    print(mapParallel(iota(n), (i) => iota(i)).length);\n}\n", "`mapParallel` over arrays of arrays is not supported in the POC"),
        (
            "import * as net from \"net\";\naction f(c: net.Conn): void {\n    print(mapParallel([1, 2], (i) => match (net.writeText(c, \"x\")) {\n        Ok(_) => 1,\n        Err(e) => 0,\n    }));\n    net.close(c);\n}\n",
            "a lambda cannot capture it",
        ),
    ];
    for (src, want) in cases {
        let e = error(&format!("{}{}", src, main));
        assert!(e.contains(want), "{}\n--- gives ---\n{}", src, e);
    }
}

/// D386: `isPure(f)` gives whether the function received is pure (a
/// `function`, a lambda without an effect: true; an action, even one
/// without an effect, a lambda that prints or calls an action: false), a
/// constant of each call; its first branch takes `f` as pure, given to a
/// `map`. The interpreter and the code compiled on 1 and 4 threads: the
/// same output.
#[test]
fn the_purity_of_the_function_received() {
    let src = include_str!("data/actions_pure_received.isc");
    let want = "true false false true true\ntrue false\nloud 1\nloud 2\nloud 3\nlambda 1\nlambda 2\nlambda 3\nloud 5\n1006 4 4 1030 2 12\n";
    assert_eq!(interp_ok(src, &[]), want);
    let bin = compile(src, "actions_pure_received");
    for t in [1, 4] {
        let r = run_bin_threads(&bin, &[], Some(t));
        assert_eq!((r.code, r.stdout.as_str()), (0, want), "{} threads: {}", t, r.stderr);
    }
}

/// D386: `isPure` refused on what is not a function; an action still one
/// outside the first branch of `if (isPure(h))` (given to `map`: refused).
#[test]
fn the_purity_test_refused() {
    let e = error("action main(args: string[k]): i32 {\n    print(`${isPure(3)}`);\n    return 0;\n}\n");
    assert!(e.contains("`isPure` takes a function"), "{}", e);
    let e = error("action f(h: action (i64) => i64, x: i64[n]): i64[] {\n    if (isPure(h)) {\n        return x;\n    }\n    return map(x, (v) => h(v));\n}\naction main(args: string[k]): i32 {\n    return 0;\n}\n");
    assert!(e.contains("cannot be called inside a lambda that is not an action"), "{}", e);
}

/// D386: `http.serve` given a pure function (by its name or a lambda
/// without an effect) answers by the `map` of `serveLoop`, never by the loop
/// of actions; given an action, by `serveActionLoop` only.
#[test]
fn serve_takes_the_way_of_its_handler() {
    let names = |src: &str| -> Vec<String> { iscc::frontend("test.isc", src).unwrap_or_else(|e| panic!("{}", e)).fns.iter().map(|f| f.name.clone()).collect() };
    let has = |ns: &[String], n: &str| ns.iter().any(|f| f.contains(n));
    let main = |handler: &str| format!("import * as http from \"http\";\n\nfunction hello(r: http.Request): http.Response {{\n    return http.textResponse(200, \"hello\");\n}}\n\naction logged(r: http.Request): http.Response {{\n    print(r.path);\n    return http.textResponse(200, \"hello\");\n}}\n\naction main(args: string[k]): i32 {{\n    return match (http.serve(8080, {})) {{\n        Ok(_) => 0,\n        Err(e) => 1,\n    }};\n}}\n", handler);
    for h in ["hello", "(r) => http.textResponse(200, r.path)"] {
        let ns = names(&main(h));
        assert!(has(&ns, "serveLoop") && !has(&ns, "serveActionLoop"), "{}: {:?}", h, ns);
    }
    for h in ["logged", "(r) => logged(r)"] {
        let ns = names(&main(h));
        assert!(has(&ns, "serveActionLoop") && !has(&ns, "serveLoop"), "{}: {:?}", h, ns);
    }
}
