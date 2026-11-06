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
    assert!(e.contains("(`print` is one): a `function` computes a value and has no effect") && e.contains("http.serveAction"), "{}", e);
}
