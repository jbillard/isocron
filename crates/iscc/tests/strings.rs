//! Strings (DECISIONS D134): literals and their escapes, `++`, the order by
//! bytes, `len`, `bytes`, `stringOf`, `toString`, `parseF64`; strings in
//! arrays, tuples, sum and recursive types, in `fold`, `reduce` and `scan`.
//! The interpreter and the compiled program print the same text, and the
//! compiled program frees every string (Valgrind, when installed).

mod common;
use common::*;

fn valgrind_clean(src: &str, name: &str, args: &[&str]) {
    let vg = std::path::PathBuf::from("/usr/bin/valgrind");
    if !vg.is_file() {
        eprintln!("valgrind not installed: {} skipped", name);
        return;
    }
    let opts = isc_codegen::Options { isa: isc_codegen::Isa::Avx2, ..isc_codegen::Options::default() };
    let bin = compile_with(src, &format!("{}_vg", name), opts);
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

/// The interpreter and the compiled program print `expected`, and the
/// compiled program frees everything.
fn agree(src: &str, name: &str, args: &[&str], expected: &str) {
    assert_eq!(interp_ok(src, args), expected, "interpreter");
    let bin = compile(src, name);
    let r = run_bin(&bin, args);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(r.stdout, expected, "compiled");
    valgrind_clean(src, name, args);
}

const OPS: &str = r#"function greet(name: string): string {
    return "Bonjour, " + name + " !";
}

function label(x: f64): string {
    return "x = " + toString(x);
}

// `+` of strings is the primitive whatever the functions of the program
function concat(a: string, b: string): string {
    return b;
}

action main(args: string[k]): i32 {
    print(greet("monde"));
    print(label(1.5));
    print(len("héllo"));
    print("abc" < "abd");
    print("abc" == "ab" + "c");
    print("b" >= "abc");
    print("Z" < "a");
    print("" < "a");
    print(bytes("AZ"));
    print(stringOf(bytes("ok\tfin")));
    print(toString(42) + "/" + toString(-7i8) + "/" + toString(true) + "/" + toString(255u8) + "/" + toString(0.1f32));
    print("" + "");
    print(len(""));
    print(parseF64("2.5e-3"));
    print("☺ \"citation\" \\ 😀");
    print(map(args, (a) => len(a)));
    print(map(args, (a) => a + "!"));
    print(filter(args, (a) => a > "f"));
    print("a" + "b" == concat("a", "b"));
    return 0;
}
"#;

#[test]
fn string_operations() {
    agree(
        OPS,
        "str_ops",
        &["un", "deux", "trois"],
        "Bonjour, monde !\nx = 1.5\n6\ntrue\ntrue\ntrue\ntrue\ntrue\n[65, 90]\nok\tfin\n42/-7/true/255/0.1\n\n0\n0.0025\n\u{263A} \"citation\" \\ \u{1F600}\n[2, 4, 5]\n[un!, deux!, trois!]\n[un, trois]\nfalse\n",
    );
}

const VALUES: &str = r#"type Opt = Some(string) | None;

type Tree = Leaf | Node(Tree, string, Tree);

function pick(b: boolean, x: string, y: string): string {
    if (b) {
        return x;
    } else {
        return y;
    }
}

function first(xs: string[n]): Opt {
    if (n > 0) {
        return Some(xs[0]);
    } else {
        return None;
    }
}

function show(o: Opt): string {
    return match (o) {
        Some(s) => concat(concat("Some(", s), ")"),
        None => "None",
    };
}

function insert(t: Tree, s: string): Tree {
    return match (t) {
        Leaf => Node(Leaf, s, Leaf),
        Node(l, v, r) => s < v ? Node(insert(l, s), v, r) : Node(l, v, insert(r, s)),
    };
}

function walk(t: Tree): string {
    return match (t) {
        Leaf => "",
        Node(l, v, r) => concat(concat(concat(walk(l), v), " "), walk(r)),
    };
}

action main(args: string[k]): i32 {
    const xs = ["pomme", "kiwi", "abricot"];
    print(xs);
    print(pick(true, xs[0], xs[1]));
    print(show(first(xs)));
    print(show(first(filter(xs, (a) => len(a) > 10))));
    const t = fold(xs, Leaf, (acc, s) => insert(acc, s));
    print(walk(t));
    print(fold(xs, "", (acc, s) => concat(acc, s)));
    const ys = xs.with(1, "figue");
    print(ys);
    const p = [xs[2], 3] as const;
    print(p);
    const [s, n] = p;
    print(concat(s, toString(n)));
    print(map(iota(5), (i) => toString(i * i)));
    print(xs == ["pomme", "kiwi", "abricot"]);
    print(xs == ys);
    print(t == insert(insert(insert(Leaf, "pomme"), "kiwi"), "abricot"));
    print(map(xs, (a) => [a, len(a)] as const));
    return 0;
}
"#;

#[test]
fn strings_in_values() {
    agree(
        VALUES,
        "str_values",
        &[],
        "[pomme, kiwi, abricot]\npomme\nSome(pomme)\nNone\nabricot kiwi pomme \npommekiwiabricot\n[pomme, figue, abricot]\n(abricot, 3)\nabricot3\n[0, 1, 4, 9, 16]\ntrue\nfalse\ntrue\n[(pomme, 5), (kiwi, 4), (abricot, 7)]\n",
    );
}

const REDUCTIONS: &str = r#"type List = Nil | Cons(i64, List);

function smaller(a: string, b: string): string {
    if (a < b) {
        return a;
    } else {
        return b;
    }
}

function second(a: List, b: List): List {
    return b;
}

action main(args: string[k]): i32 {
    const big = map(iota(10000), (i) => toString(i * 7919 % 10007));
    const z = "~";
    print(reduce(big, smaller, z));
    print(reduce(big, (a, b) => len(a) >= len(b) ? a : b, ""));
    print(len(reduce(filter(big, (s) => len(s) == 2), (a, b) => concat(a, b), "")));
    print(reduce(["a", "b", "c", "d", "e"], (a, b) => concat(a, b), ""));
    print(scan(filter(args, (s) => s != "x"), (a, b) => concat(a, b), z));
    const ws = map(filter(big, (s) => len(s) < 2), (s) => concat(s, "!"));
    print(ws);
    print(fold(big, 0, (acc, s) => acc + len(s)));
    print(reduce(map(big, (s) => len(s)), (+), 0));
    const ls = map(iota(5), (i) => Cons(i, Nil));
    print(reduce(ls, second, Nil));
    print(z);
    return 0;
}
"#;

#[test]
fn reductions_of_strings() {
    // the fixed form of section 10, independently
    let big: Vec<String> = (0..10000i64).map(|i| ((i * 7919) % 10007).to_string()).collect();
    let refs: Vec<&str> = big.iter().map(|s| s.as_str()).collect();
    let smallest = fixed_reduce(&refs, "~", &|a, b| if a < b { a } else { b });
    let longest = fixed_reduce(&refs, "", &|a, b| if a.len() >= b.len() { a } else { b });
    let total: usize = big.iter().map(|s| s.len()).sum();
    let two: usize = big.iter().filter(|s| s.len() == 2).map(|s| s.len()).sum();
    let expected = format!(
        "{}\n{}\n{}\naecbd\n[~a, ~ab]\n[0!, 9!, 8!, 7!, 6!, 5!, 4!, 3!, 2!, 1!]\n{}\n{}\nNil\n~\n",
        smallest, longest, two, total, total
    );
    agree(REDUCTIONS, "str_reductions", &["a", "x", "b"], &expected);
}

#[test]
fn string_panics() {
    let bad = "action main(args: string[k]): i32 {\n    print(stringOf(map(bytes(args[0]), (b) => b == 65u8 ? 255u8 : b)));\n    return 0;\n}\n";
    let m = interp_panic(bad, &["xxA"]);
    assert!(m.contains("stringOf: the bytes are not UTF-8 (invalid from byte 2)"), "{}", m);
    let bin = compile(bad, "str_utf8");
    let r = run_bin(&bin, &["xxA"]);
    assert_ne!(r.code, 0);
    assert!(r.stderr.contains("stringOf: the bytes are not UTF-8 (invalid from byte 2)"), "{}", r.stderr);
    let nan = "action main(args: string[k]): i32 {\n    print(parseF64(args[0]));\n    return 0;\n}\n";
    let m = interp_panic(nan, &["1,5"]);
    assert!(m.contains("parseF64: `1,5` is not a valid number"), "{}", m);
    let bin = compile(nan, "str_parse");
    let r = run_bin(&bin, &["1,5"]);
    assert!(r.stderr.contains("parseF64: `1,5` is not a valid number"), "{}", r.stderr);
}

fn error(src: &str) -> String {
    match iscc::frontend("test.isc", src) {
        Ok(_) => panic!("expected an error"),
        Err(e) => e,
    }
}

#[test]
fn string_errors() {
    let e = error("action main(args: string[k]): i32 {\n    print(\"abc);\n    return 0;\n}\n");
    assert!(e.contains("unterminated string literal"), "{}", e);
    let e = error("action main(args: string[k]): i32 {\n    print(\"a\\qb\");\n    return 0;\n}\n");
    assert!(e.contains("unknown escape `\\q`"), "{}", e);
    let e = error("action main(args: string[k]): i32 {\n    print(\"\\u{110000}\");\n    return 0;\n}\n");
    assert!(e.contains("invalid escape"), "{}", e);
    let e = error("action main(args: string[k]): i32 {\n    print(\"a\" - \"b\");\n    return 0;\n}\n");
    assert!(e.contains("string"), "{}", e);
    let e = error("action main(args: string[k]): i32 {\n    print(\"a\" < 1);\n    return 0;\n}\n");
    assert!(e.contains("string"), "{}", e);
    let e = error("action main(args: string[k]): i32 {\n    print(concat(1, 2));\n    return 0;\n}\n");
    assert!(e.contains("string"), "{}", e);
    // parsing has no effect: a function may parse (D156)
    let src = "function f(s: string): f64 {\n    return parseF64(s);\n}\n\naction main(args: string[k]): i32 {\n    print(f(\"1\"));\n    return 0;\n}\n";
    assert_eq!(interp_ok(src, &[]), "1\n");
}

const TEMPLATES: &str = r#"function label(name: string, x: f64): string {
    return `${name} = ${x}`;
}

action main(args: string[k]): i32 {
    const n = 3;
    print(`n = ${n}, n * n = ${n * n}, ${n > 2}, ${-7i8}, ${255u8}`);
    print(label(args[0], 3.25));
    print(`${args[0]}`);
    print(``);
    print(`two
lines, braces { } and \` \${x}`);
    print(`nested: ${`[${n + 1}]`} ${{ a: 1 }.a}`);
    print(map(iota(3), (i) => `#${i}`));
    return 0;
}
"#;

/// Template literals (D150): the texts and the insertions concatenated,
/// each insertion written as `print` writes it.
#[test]
fn template_literals() {
    agree(
        TEMPLATES,
        "str_templates",
        &["pi"],
        "n = 3, n * n = 9, true, -7, 255\npi = 3.25\npi\n\ntwo\nlines, braces { } and ` ${x}\nnested: [4] 1\n[#0, #1, #2]\n",
    );
    let e = error("action main(args: string[k]): i32 {\n    print(`a ${[1, 2]}`);\n    return 0;\n}\n");
    assert!(e.contains("test.isc:2:16: error: expected a scalar type"), "{}", e);
    let e = error("action main(args: string[k]): i32 {\n    print(`a ${1 + 2`);\n    return 0;\n}\n");
    assert!(e.contains("unterminated template literal"), "{}", e);
}

const ROWS: &str = r#"function shout(r: string[n]): string[n] {
    return r.map((s) => s + "!");
}

action main(args: string[k]): i32 {
    const rows = lines(args[0]).map((l) => l.split(","));
    print(rows);
    print(rows[1][2]);
    print(rows.map((r) => r.join("-")));
    const lit = [["a", "b"], ["c", args[1]]];
    print(lit);
    print(lit == [["a", "b"], ["c", "x"]]);
    const loud = lit.map((r) => shout(r));
    print(loud);
    const same = rows.map((r) => r);
    print(same[0]);
    const firsts = rows.map((r) => r[0]);
    print(firsts);
    print(map(iota(2), (i) => fill(3, toString(i))));
    const z = "z";
    print(fill(2, z));
    print(z);
    return 0;
}
"#;

/// Arrays of arrays of strings (D154): rows in one block, each string owned
/// by the array; a row of another length is a panic (an array of arrays is
/// regular); `fill` of a string copies it into each element.
#[test]
fn arrays_of_arrays_of_strings() {
    agree(
        ROWS,
        "str_rows",
        &["a,b,c\nd,e,f", "x"],
        "[[a, b, c], [d, e, f]]\nf\n[a-b-c, d-e-f]\n[[a, b], [c, x]]\ntrue\n[[a!, b!], [c!, x!]]\n[a, b, c]\n[a, d]\n[[0, 0, 0], [1, 1, 1]]\n[z, z]\nz\n",
    );
    let m = interp_panic(ROWS, &["a,b,c\nd,e", "x"]);
    assert_eq!(m, "panic at line 6, column 51: size mismatch: 2 != 3");
    let bin = compile(ROWS, "str_rows_ragged");
    let r = run_bin(&bin, &["a,b,c\nd,e", "x"]);
    assert_eq!(r.code, 101);
    assert!(r.stderr.contains("panic at line 6, column 51: size mismatch: 2 != 3"), "{}", r.stderr);
}

const TS_STRINGS: &str = r#"action main(args: string[k]): i32 {
    const s = "Hello, World, hello";
    print([s.indexOf("World"), s.indexOf("x"), "".indexOf("")] as const);
    print([s.includes("lo, W"), s.startsWith("Hell"), s.endsWith("llo"), s.endsWith("x")] as const);
    print(s.replaceAll("llo", "LLO"));
    print("abc".replaceAll("", "-"));
    print(s.toUpperCase() + " " + "ÉTÉ straße".toLowerCase() + " " + "straße".toUpperCase());
    print("[" + "7".padStart(3, "0") + "|" + "ab".padEnd(5) + "|" + "x".padStart(6, "ab") + "|" + "abc".padStart(2) + "]");
    print("ab".repeat(3) + "|" + "  x  ".trimStart() + "|" + "  x  ".trimEnd() + "|");
    print(s.slice(7, 12));
    print([(1.005).toFixed(2), (0.125).toFixed(2), (2.5).toFixed(0), (-2.5).toFixed(0), (-0.001).toFixed(2), (123.456).toFixed(10), (1e-7).toFixed(10)]);
    print([(0.0).toFixed(3), (-0.0).toFixed(1), (99.995).toFixed(2), (0.5).toFixed(0), (1.45).toFixed(1), (1e21).toFixed(2), (12.5f32).toFixed(3)]);
    print(map(["a", "b"], toUpperCase));
    return 0;
}
"#;

/// The functions of strings of TypeScript (D155), on bytes of UTF-8;
/// `toFixed` as JavaScript (the exact value, a half away from zero: the
/// double nearest 99.995 is above it, so `100.00`).
#[test]
fn typescript_string_functions() {
    agree(TS_STRINGS, "str_ts", &[], "(7, -1, 0)\n(true, true, true, false)\nHeLLO, World, heLLO\n-a-b-c-\nHELLO, WORLD, HELLO été straße STRASSE\n[007|ab   |ababax|abc]\nababab|x  |  x|\nWorld\n[1.00, 0.13, 3, -3, -0.00, 123.4560000000, 0.0000001000]\n[0.000, 0.0, 100.00, 1, 1.4, 1000000000000000000000, 12.500]\n[A, B]\n");
    for (body, want) in [
        ("print(\"ab\".repeat(-1))", "panic at line 2, column 22: repeat: the count is negative: -1"),
        ("print(\"é\".slice(0, 1))", "panic at line 2, column 20: slice: the index 1 is inside a character of UTF-8"),
        ("print(\"abc\".slice(2, 5))", "panic at line 2, column 22: slice out of bounds: [2..5] of a string of 3 bytes"),
        ("print((1.5).toFixed(101))", "panic at line 2, column 24: toFixed: the number of digits must be between 0 and 100, got 101"),
    ] {
        let src = format!("action main(args: string[k]): i32 {{\n    {};\n    return 0;\n}}\n", body);
        let m = interp_panic(&src, &[]);
        assert!(m.starts_with(want), "{}: {}", body, m);
        let bin = compile(&src, "str_ts_panic");
        let r = run_bin(&bin, &[]);
        assert_eq!(r.code, 101, "{}", body);
        assert!(r.stderr.starts_with(want), "{}: {}", body, r.stderr);
    }
}
