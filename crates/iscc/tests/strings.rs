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
    let bin = compile_glibc(src, &format!("{}_vg", name), opts);
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
        ("print(\"ab\".repeat(args.length - 1))", "panic at line 2, column 22: repeat: the count is negative: -1"),
        ("print(\"é\".slice(0, 1))", "panic at line 2, column 20: slice: the index 1 is inside a character of UTF-8"),
        ("print(\"abc\".slice(2, 5))", "panic at line 2, column 22: slice out of bounds: [2..5] of a string of 3 bytes"),
        ("print((1.5).toFixed(args.length + 101))", "panic at line 2, column 24: toFixed: the number of digits must be between 0 and 100, got 101"),
    ] {
        let src = format!("action main(args: string[k]): i32 {{\n    {};\n    return 0;\n}}\n", body);
        let m = interp_panic(&src, &[]);
        assert!(m.starts_with(want), "{}: {}", body, m);
        let bin = compile(&src, "str_ts_panic");
        let r = run_bin(&bin, &[]);
        assert_eq!(r.code, 101, "{}", body);
        assert!(r.stderr.starts_with(want), "{}: {}", body, r.stderr);
    }
    // with literals: compile errors (D162)
    for (body, want) in [
        ("print(\"ab\".repeat(-1))", "2:22: error: repeat: the count is negative: -1"),
        ("print((1.5).toFixed(101))", "2:24: error: toFixed: the number of digits must be between 0 and 100, got 101"),
    ] {
        let src = format!("action main(args: string[k]): i32 {{\n    {};\n    return 0;\n}}\n", body);
        match iscc::frontend("t.isc", &src) {
            Ok(_) => panic!("accepted: {}", body),
            Err(e) => assert!(e.contains(want), "{}: {}", body, e),
        }
    }
}

#[test]
fn global_searches_split_among_threads_find_the_matches_of_one_search() {
    // a text of several parts of 256 KiB (D189): counts, matches and
    // replacements of global regular expressions, searched by part in
    // parallel then merged, against the interpreter; empty matches, matches
    // across the cuts, characters of several bytes, `$&`
    let src = "action main(args: string[k]): i32 {
    const unit = \"aN<tB|Y>é\\n>xa|t\\nBYaS<|>\";
    const s = unit.repeat(80000);
    print(s.length);
    print([countMatches(s, /a[NSt]|BY/g), countMatches(s, /<[^>]*>/g), countMatches(s, /x*/g), countMatches(s, /^>.*\\n|\\n/gm)] as const);
    const m = matches(s, /\\|[^|][^|]*\\|/g);
    print([m.length, m[0], m[m.length - 1]] as const);
    const r = s.replace(/^>.*\\n|\\n/gm, \"\").replace(/<[^>]*>/g, \"[$&]\").replace(/é|Y/g, \"-\").replace(/x*/g, \"_\");
    print([r.length, r.slice(0, 40), r.slice(r.length - 40, r.length)] as const);
    return 0;
}
";
    let want = interp_ok(src, &[]);
    let bin = compile(src, "rx_split");
    for t in [1, 2, 3, 4] {
        let run = run_bin_threads(&bin, &[], Some(t));
        assert_eq!((run.code, run.stdout.as_str()), (0, want.as_str()), "{} threads: {}", t, run.stderr);
    }
}

/// D277: global searches by masks inside the tasks of a parallel `map` (9
/// patterns of a large text, as the counts of regex-redux) are cut into
/// parts too, shared with the threads that have finished theirs: the
/// counts and the matches of one search, with matches across the cuts,
/// against the interpreter, 1 to 4 threads.
#[test]
fn searches_inside_tasks_are_shared_with_idle_threads() {
    let src = "action main(args: string[k]): i32 {
    const s = \"agggtaaattTACCCTaNaggctaaatttaccctBY\\n\".repeat(40000);
    const pats = [\"agggtaaa|tttaccct\", \"[cgt]gggtaaa|tttaccc[acg]\", \"a[act]ggtaaa|tttacc[agt]t\", \"ag[act]gtaaa|tttac[agt]ct\", \"aa\", \"a[NSt]|BY\", \"t\", \"ct|at\", \"aggct\"];
    print(pats.map((p) => countMatches(s, RegExp(p, \"ig\"))));
    print(pats.map((p) => matches(s, RegExp(p, \"g\")).length));
    return 0;
}
";
    let want = interp_ok(src, &[]);
    let bin = compile(src, "rx_in_tasks");
    for t in [1, 2, 3, 4] {
        let run = run_bin_threads(&bin, &[], Some(t));
        assert_eq!((run.code, run.stdout.as_str()), (0, want.as_str()), "{} threads: {}", t, run.stderr);
    }
}

/// D219: a global search of a large text (4 MiB and more) takes its engine,
/// with the prefilter of its literals or without, from a sample: the same
/// matches either way. Counts of dense patterns (the automaton alone wins),
/// a replacement and the matches of rare literals (the prefilter wins),
/// patterns that can match the empty string; short texts too, inside a
/// parallel `map` (one search per task). The interpreter's results.
#[test]
fn searches_choose_their_engine_from_a_sample() {
    let src = "action main(args: string[k]): i32 {
    const n = parseI64(args[0]);
    const s = \"aaXbbaaa-AAxagggtaaaTTTACCCTacgtacgtca\".repeat(n);
    print(countMatches(s, /a*/g));
    print(countMatches(s, /a+/g));
    print(countMatches(s, /AA/gi));
    print(countMatches(s, /agggtaaa|tttaccct/gi));
    print(map([\"ab\", \"\", \"a[act]\", \"(x|b)+\"], (p) => countMatches(s, RegExp(p, \"gi\"))));
    const r = s.replace(/X|-A/g, \"<1>\").replace(/<[^>]*>/g, \"|\");
    print(r.length);
    print(countMatches(r, /\\|/g));
    print(matches(s, /xag+t/gi).length);
    return 0;
}
";
    let bin = compile(src, "rx_engines");
    for n in ["3", "120000"] {
        let want = interp_ok(src, &[n]);
        for t in [1, 4] {
            let r = run_bin_threads(&bin, &[n], Some(t));
            assert_eq!((r.code, r.stdout.as_str()), (0, want.as_str()), "n {} {} threads: {}", n, t, r.stderr);
        }
    }
}

/// D224: the patterns of two simple shapes, alternations of sequences of
/// ASCII classes of one length and `head [^c]* c`, are searched by masks of
/// 64 bytes; the others, and those whose classes have members outside
/// ASCII (`k` and `s` ignoring case: the Kelvin sign, the long s), by the
/// automaton. The matches of the interpreter (the automaton), for counts,
/// lists and replacements: dense and rare matches, patterns that overlap
/// themselves (`aa`), heads without a stop after them, `[^|]+` between
/// stops side by side, text outside ASCII, short and long texts (parts in
/// parallel and their merge), 1 and 4 threads.
#[test]
fn simple_shapes_found_by_masks() {
    let src = "action main(args: string[k]): i32 {
    const n = parseI64(args[0]);
    const base = \"aaXbbaaaa-AAxagggtaaaTTTACCCT<2>at||a|bc|tHaNaND<x>caNéHaSWaSBY|q<<k>Kks\".repeat(n);
    const s = base + \"tail <open|and|\";
    const pats = [\"aa\", \"agggtaaa|tttaccct\", \"a[NSt]|BY\", \"aND|caN|Ha[DS]|WaS\", \"tHa[Nt]\", \"<[^>]*>\", \"\\\\|[^|][^|]*\\\\|\", \"a{3}\", \"(ab|ba)x\", \"k\", \"ks\", \"[a-c]\", \"é\", \"a*\", \"<[^>]+>\", \"x[^y]*y\"];
    print(pats.map((p) => `${p} ${countMatches(s, RegExp(p, \"g\"))} ${countMatches(s, RegExp(p, \"gi\"))} ${matches(s, RegExp(p, \"g\")).join(\",\").length} ${s.replace(RegExp(p, \"gi\"), \"<>\").length} ${s.replace(RegExp(p, \"g\"), \"#\").length}`).join(\"\\n\"));
    const r = s.replace(/tHa[Nt]/g, \"<4>\").replace(/aND|caN|Ha[DS]|WaS/g, \"<3>\").replace(/a[NSt]|BY/g, \"<2>\").replace(/<[^>]*>/g, \"|\").replace(/\\|[^|][^|]*\\|/g, \"-\");
    print(r.length);
    print(r.slice(0, r.length < 80 ? r.length : 80));
    return 0;
}
";
    // the runtime of the host (AVX-512 on a machine that has it, D241) and
    // the one for AVX2
    let bin = compile(src, "simple_shapes");
    let opts = isc_codegen::Options { isa: isc_codegen::Isa::Avx2, ..isc_codegen::Options::default() };
    let bin2 = compile_with(src, "simple_shapes_avx2", opts);
    for n in ["1", "3", "9000"] {
        let want = interp_ok(src, &[n]);
        for b in [&bin, &bin2] {
            for t in [1, 4] {
                let r = run_bin_threads(b, &[n], Some(t));
                assert_eq!((r.code, r.stdout.as_str()), (0, want.as_str()), "n {} {} threads {:?}: {}", n, t, b, r.stderr);
            }
        }
    }
}

/// D236: a replacement without `$` of many matches (16384 or more) written
/// by every thread, each its part of the result at its place; with `$&`, or
/// few matches, as before. The texts of the interpreter, whole, on 1, 4 and
/// 16 threads.
#[test]
fn replacements_written_in_parallel() {
    let src = "action main(args: string[k]): i32 {
    const n = parseI64(args[0]);
    const s = \"aaXbbaaaa-AAxagggtaaaTTTACCCT<2>at||a|bc|tHaNaND<x>caNéHaSWaSBY|q<<k>Kks\".repeat(n);
    print(s.replace(/a/g, \"<1>\"));
    print(s.replace(/a[NSt]|BY/g, \"\"));
    print(s.replace(/<[^>]*>/g, \"|\").replace(/\\|[^|][^|]*\\|/g, \"-\"));
    print(s.replace(/é|aa/g, \"$&$&\"));
    print(s.replace(/q/g, \"Q\"));
    return 0;
}
";
    let bin = compile(src, "replace_parallel");
    for n in ["1", "300", "20000"] {
        let want = interp_ok(src, &[n]);
        for t in [1, 4, 16] {
            let r = run_bin_threads(&bin, &[n], Some(t));
            assert!(r.code == 0 && r.stdout == want, "n {} {} threads: {} bytes, {} expected: {}", n, t, r.stdout.len(), want.len(), r.stderr);
        }
    }
}

/// D279: a replacement of a large text (1 MiB and more) written by every
/// thread whatever its number of matches: the text cut into parts of about
/// the same bytes, a cut inside a match moved to its end. Few matches, one
/// match longer than a part (across the cuts), empty matches, `$&` (not in
/// parallel), on 1 to 16 threads: the texts of the interpreter, whole.
#[test]
fn replacements_of_large_texts_cut_by_bytes() {
    let src = "action main(args: string[k]): i32 {
    const n = parseI64(args[0]);
    const s = \"acgt\".repeat(n) + \"<\" + \"tHaN\".repeat(n) + \">q\" + \"gt\".repeat(n);
    print(s.replace(/q/g, \"QQ\").length);
    print(s.replace(/tHa[Nt]/g, \"<4>\").slice(4 * n - 3, 4 * n + 12));
    const r = s.replace(/<[^>]*>/g, \"|\");
    print([r.length, r.slice(4 * n - 2, 4 * n + 4)] as const);
    print(s.replace(/x*/g, \"_\").length);
    print(s.replace(/q|>/g, \"[$&]\").length);
    const t = s.replace(/acgt|gt/g, \"\");
    print([t.length, t.slice(0, 12)] as const);
    return 0;
}
";
    let bin = compile(src, "replace_by_bytes");
    for n in ["10", "300000"] {
        let want = interp_ok(src, &[n]);
        for t in [1, 2, 3, 4, 16] {
            let r = run_bin_threads(&bin, &[n], Some(t));
            assert_eq!((r.code, r.stdout.as_str()), (0, want.as_str()), "n {} {} threads: {}", n, t, r.stderr);
        }
    }
}

/// D280: the texts between the matches and the replacements copied by
/// moves that overlap below 32 bytes: every length of text from 0 to 64
/// between the matches, replacements of 0 to 33 bytes, in a large text
/// (written in parallel). The interpreter's texts, 1 and 4 threads.
#[test]
fn short_copies_of_a_replacement() {
    let src = "action main(args: string[k]): i32 {
    const unit = map(iota(65), (n) => \"acgtacgtacgtacgtacgtacgtacgtacgtacgtacgtacgtacgtacgtacgtacgtacgtacgt\".slice(0, n) + \"Q\").join(\"\");
    const s = unit.repeat(500);
    print(s.replace(/Q/g, \"\"));
    print(s.replace(/Q/g, \"a\"));
    print(s.replace(/Q/g, \"<2>\"));
    print(s.replace(/Q/g, \"abcdefg\"));
    print(s.replace(/Q/g, \"abcdefgh\"));
    print(s.replace(/Q/g, \"abcdefghijklmno\"));
    print(s.replace(/Q/g, \"abcdefghijklmnop\"));
    print(s.replace(/Q/g, \"abcdefghijklmnopqrstuvwxyz012345\"));
    print(s.replace(/Q/g, \"abcdefghijklmnopqrstuvwxyz0123456\"));
    return 0;
}
";
    let bin = compile(src, "short_copies");
    let want = interp_ok(src, &[]);
    for t in [1, 4] {
        let r = run_bin_threads(&bin, &[], Some(t));
        assert!(r.code == 0 && r.stdout == want, "{} threads: {} bytes, {} expected: {}", t, r.stdout.len(), want.len(), r.stderr);
    }
}

/// D237: an alternation of simple shapes of other lengths and kinds, at
/// the start of a line or anywhere, searched by masks: at a place the first
/// alternative that matches (`a|ab` matches `a`), as the automaton of the
/// interpreter. Counts, lists and replacements, short and long texts, 1 and
/// 4 threads.
#[test]
fn alternations_of_shapes_found_by_masks() {
    let src = "action main(args: string[k]): i32 {
    const n = parseI64(args[0]);
    const s = \">ONE head\\naaXbbaa>ab\\n>x y\\nagg<1>tt|q|ab\\nabc\\n\".repeat(n) + \">tail\";
    const pats = [\"^>.*\\\\n|\\\\n\", \"a|ab\", \"ab|a\", \"ab|c|>\", \"^a|t\", \"<[^>]*>|a\", \"x[^y]*y|q|<[^>]+>\", \"^>[^\\\\n]*\"];
    print(pats.map((p) => `${p} ${countMatches(s, RegExp(p, \"mg\"))} ${matches(s, RegExp(p, \"mg\")).join(\",\").length} ${s.replace(RegExp(p, \"mg\"), \"\").length} ${s.replace(RegExp(p, \"mg\"), \"[$&]\").length}`).join(\"\\n\"));
    print(s.replace(/^>.*\\n|\\n/mg, \"\").length);
    return 0;
}
";
    let bin = compile(src, "shape_alternations");
    for n in ["1", "3", "40000"] {
        let want = interp_ok(src, &[n]);
        for t in [1, 4] {
            let r = run_bin_threads(&bin, &[n], Some(t));
            assert_eq!((r.code, r.stdout.as_str()), (0, want.as_str()), "n {} {} threads: {}", n, t, r.stderr);
        }
    }
}

/// D301: `startsWith` and `endsWith` against literals of 0 to 17 bytes (up
/// to 16 compared in the generated code, as one or two integers), strings
/// shorter, as long and longer, of several bytes a character; `includes`
/// and `indexOf` by needles of 1 byte (`memchr`), 2 to 16 (their first
/// byte, then the others) and more (`memmem`): at the start, at the end,
/// overlapping candidates, none, empty. The text of the interpreter.
#[test]
fn searches_of_short_needles() {
    let src = r#"function affixes(s: string): i64[21] {
    return [s.startsWith("") ? 1 : 0, s.startsWith("a") ? 1 : 0, s.startsWith("ab") ? 1 : 0, s.startsWith("abc") ? 1 : 0,
        s.startsWith("abcdefg") ? 1 : 0, s.startsWith("abcdefgh") ? 1 : 0, s.startsWith("abcdefghi") ? 1 : 0,
        s.startsWith("abcdefghijklmno") ? 1 : 0, s.startsWith("abcdefghijklmnop") ? 1 : 0, s.startsWith("abcdefghijklmnopq") ? 1 : 0,
        s.startsWith("é") ? 1 : 0, s.startsWith("abd") ? 1 : 0,
        s.endsWith("") ? 1 : 0, s.endsWith("q") ? 1 : 0, s.endsWith("pq") ? 1 : 0, s.endsWith("jklmnopq") ? 1 : 0,
        s.endsWith("ijklmnopq") ? 1 : 0, s.endsWith("bcdefghijklmnopq") ? 1 : 0, s.endsWith("abcdefghijklmnopq") ? 1 : 0,
        s.endsWith("ù") ? 1 : 0, s.endsWith("xq") ? 1 : 0];
}

function finds(s: string, t: string): i64[2] {
    return [s.indexOf(t), s.includes(t) ? 1 : 0];
}

action main(args: string[k]): i32 {
    const texts = ["", "a", "ab", "abc", "abcdefg", "abcdefgh", "abcdefghi", "abcdefghijklmnop", "abcdefghijklmnopq",
        "abcdefghijklmnopqr", "éabc", "abcù", "xabcdefghijklmnopq", "aaab", "abababac"];
    for_each(texts);
    const needles = ["a", "q", "é", "ù", "ab", "aab", "abac", "bab", "pq", "defghijklmnopq", "abcdefghijklmnopq",
        "bcdefghijklmnopqr", "zz", "", "aaab"];
    print(map(texts, (s) => map(iota(needles.length * 2), (k) => finds(s, needles[k / 2])[k % 2])));
    return 0;
}

action for_each(texts: string[n]): void {
    print(map(texts, affixes));
}
"#;
    let want = interp_ok(src, &[]);
    agree(src, "str_short_needles", &[], &want);
}

/// D301: a string literal given to a direct tail call is kept by its loop
/// without being owned (borrowed data of the program), between strings
/// that the loop owns and frees: a loop (a million calls), the text of the
/// interpreter, nothing lost nor freed twice (valgrind).
#[test]
fn literals_given_to_tail_calls() {
    let src = r#"function build(i: i64, n: i64, acc: string, longest: i64): i64 {
    if (i == n) return longest + acc.length;
    if (i % 7 == 0) return build(i + 1, n, "", longest > acc.length ? longest : acc.length);
    if (i % 5 == 0) return build(i + 1, n, "ab", longest);
    return build(i + 1, n, acc + "x", longest);
}

action main(args: string[k]): i32 {
    print(build(0, parseI64(args[0]), "start", 0));
    return 0;
}
"#;
    let want = interp_ok(src, &["1000000"]);
    agree(src, "str_literal_tail_calls", &["1000000"], &want);
}

/// D302: the case of ASCII texts (each byte mapped) and of others (Unicode),
/// padding by a fill of one byte (the string written once) and by longer
/// fills, the replacement of a text without `$` patterns (its places found,
/// the string written once) and with them: none, one, several, overlapping,
/// at both ends, longer, shorter and empty replacements. The text of the
/// interpreter, nothing lost (valgrind).
#[test]
fn direct_writes_of_case_padding_and_replacement() {
    let src = r#"action main(args: string[k]): i32 {
    const texts = ["", "abc", "Hello, World 42", "straße é", "aaaa", "abababab", "xabx"];
    print(texts.map((s) => s.toUpperCase() + "|" + s.toLowerCase()).join(";"));
    print(texts.map((s) => s.padStart(0, "-") + "|" + s.padStart(3, "0") + "|" + s.padStart(12, "*") + "|" + s.padEnd(12, ".") + "|" + s.padStart(9, "ab") + "|" + s.padEnd(10, "é")).join(";"));
    print(texts.map((s) => s.replace("a", "<>") + "|" + s.replaceAll("a", "") + "|" + s.replaceAll("ab", "xyz") + "|" + s.replaceAll("aa", "b") + "|" + s.replace("x", "") + "|" + s.replaceAll("é", "e") + "|" + s.replaceAll("a", "[$&]") + "|" + s.replaceAll("", "_")).join(";"));
    return 0;
}
"#;
    let want = interp_ok(src, &[]);
    agree(src, "str_direct_writes", &[], &want);
}

/// D303: a slice or the bytes of a string read where it is (a variable, a
/// field, an element, a parameter, another slice, a literal) are a view of
/// it, neither copied nor freed; a consumer that keeps one gets a copy: an
/// array, a field, a result, a variable that outlives its source, a
/// concatenation, a parameter of a tail call, the accumulator of a fold, a
/// map read in parallel. The text of the interpreter, nothing lost nor read
/// after its end (valgrind); a bound out of the string or inside a
/// character stops the program as the interpreter, at the same place.
#[test]
fn parts_of_strings_read_where_they_are() {
    let src = r#"type Entry = { name: string, n: i64 };

function head(s: string): string {
    return s.slice(0, 2);
}

function count(s: string, c: u8): i64 {
    return sum(map(bytes(s), (b) => b == c ? 1 : 0));
}

function shrink(s: string, k: i64): i64 {
    if (s.length <= 1) return k;
    return shrink(s.slice(1, s.length), k + i64(bytes(s)[0]));
}

action main(args: string[k]): i32 {
    const s = args[0];
    const e: Entry = { name: s + "/" + s, n: 3 };
    const words = ["alpha", "béta", "gamma", "", "δέλτα"];
    const inner = s.slice(1, 6).slice(1, 3);
    print(inner + "|" + e.name.slice(2, 5) + "|" + "constant".slice(2, 5));
    print([s.slice(0, 1), e.name.slice(0, 3), head(s)]);
    const kept: Entry = { name: s.slice(2, 4), n: e.name.slice(0, 4).length };
    print(kept.name + " " + toString(kept.n));
    const fresh = (s + "!").slice(1, 4);
    print(fresh);
    print(words.map((w) => w.slice(0, w.length > 4 ? 4 : w.length).length + count(w, 97u8)));
    print(words.map((w) => w.length > 4 && w.slice(0, 4) == "alph"));
    print(fold(iota(5), "start", (acc, i) => acc.slice(0, 3) + toString(i)));
    print(shrink(e.name, 0));
    const bs = bytes(s);
    print(bs.length + i64(bs[0]) + count(e.name, 108u8));
    print(stringOf(bytes(e.name)) == e.name);
    print(sum(map(iota(200000), (i) => i64(bytes(words[i % 5]).length) + words[i % 3].slice(1, 3).length)));
    print(words[1].slice(parseI64(args[1]), parseI64(args[2])));
    return 0;
}
"#;
    let args = ["hello-world", "1", "3"];
    let want = interp_ok(src, &args);
    agree(src, "str_parts_views", &args, &want);
    let bin = compile(src, "str_parts_views_bad");
    for (a, b) in [("3", "2"), ("-1", "2"), ("2", "20"), ("1", "2"), ("2", "4")] {
        let args = ["hello-world", a, b];
        let m = interp_panic(src, &args);
        let r = run_bin(&bin, &args);
        assert_eq!(r.code, 101, "{} {}: {}", a, b, r.stderr);
        assert_eq!(r.stderr.trim_end(), m.trim_end(), "slice({}, {})", a, b);
    }
}

/// D304: a reduction of integers by `+`, `*`, `min` or `max` from its
/// neutral value, over elements the vector loops do not take (strings):
/// one accumulator instead of the V of the fixed form, the same value for
/// any order (modular, associative, commutative). Several blocks of B,
/// sums and products that wrap, narrow integers, signed and unsigned; from
/// another value the fixed form (V copies of it in each block). The value
/// of the interpreter on 1, 3 and 8 threads.
#[test]
fn integer_reductions_of_strings_in_one_accumulator() {
    let src = r#"action main(args: string[k]): i32 {
    const n = parseI64(args[0]);
    const ws = map(iota(n), (i) => toString(i * 7919 % 100003) + "x");
    print(sum(map(ws, (w) => i64(bytes(w)[0]) * 1000000000000000)));
    print(count(ws, (w) => w.startsWith("1")));
    print(reduce(map(ws, (w) => w.length * 7 + 1), (a, b) => a * b, 1));
    print(reduce(map(ws, (w) => u8(w.length * 37)), (a, b) => a + b, 0u8));
    print(reduce(map(ws, (w) => i32(bytes(w)[0]) * 100000000i32), (a, b) => a + b, 0i32));
    print(reduce(map(ws, (w) => i64(bytes(w)[1 % w.length]) - 70), (a, b) => min(a, b), 9223372036854775807));
    print(reduce(map(ws, (w) => u64(w.length)), (a, b) => max(a, b), 0u64));
    print(reduce(map(ws, (w) => i16(w.length) - 9i16), (a, b) => max(a, b), -32768i16));
    print(reduce(map(ws, (w) => w.length), (a, b) => a + b, 5));
    print(reduce(map(ws, (w) => w.length), (a, b) => min(a, b), 3));
    return 0;
}
"#;
    let bin = compile(src, "str_int_reductions");
    for n in ["1", "100", "4096", "10000", "300000"] {
        let want = interp_ok(src, &[n]);
        for threads in [1, 3, 8] {
            let r = run_bin_threads(&bin, &[n], Some(threads));
            assert_eq!((r.code, r.stdout.as_str()), (0, want.as_str()), "{} elements, {} threads: {}", n, threads, r.stderr);
        }
    }
}

/// D305: the lines of a text and the parts of a split in one block, freed
/// with the last of them; an array of strings freed at once by the runtime,
/// those of one block counted down together. Empty texts, empty lines and
/// parts, `\r\n`, no final `\n`, separators of one and several bytes, at
/// both ends, one after the other, characters of several bytes; parts kept
/// after their array is gone, sorted, filtered, two blocks in one array
/// (`concat`), strings of a block among others; small texts and one of 3
/// MiB (in parallel); on 1, 3 and 8 threads, as the interpreter; nothing
/// lost, nothing freed twice (valgrind).
#[test]
fn lines_and_parts_in_one_block() {
    let src = r#"action kept(t: string, sep: string): string[k] {
    const parts = t.split(sep);
    print(parts.length);
    return filter(parts, (p) => p.length % 3 != 1);
}

action main(args: string[k]): i32 {
    const n = parseI64(args[0]);
    const big = join(map(iota(n), (i) => i % 9 == 0 ? "" : toString(i * 7919) + (i % 11 == 0 ? "\r" : "") + (i % 13 == 0 ? "é,;" : ",")), "\n");
    const texts = ["", "\n", "a", "a\n", "\n\nb\r\n\r\nc", "x,,y,", ",a,b,", "é;;é;", big];
    print(texts.map((t) => lines(t).length));
    print(texts.map((t) => lines(t).map((l) => toString(l.length)).join(".")).join("|").length);
    print(texts.map((t) => t.split(",").join("+")).join("|").length);
    print(texts.map((t) => t.split(";;").length));
    print(texts.map((t) => t.split("é").map((p) => toString(p.length)).join(".")).join("|").length);
    const a = kept(big, ",");
    const b = kept(big, "\n");
    const both = concat(a, b);
    const sorted = sortBy(both, (s) => s);
    print([both.length, sum(map(both, (s) => s.length))]);
    print([sorted[0], sorted[sorted.length - 1], sorted[sorted.length / 2]]);
    const mixed = concat(lines("p\nq\nr"), [big.slice(0, 3) + "!", "s"]);
    print(mixed);
    return 0;
}
"#;
    let bin = compile(src, "str_block_parts");
    for n in ["40", "400000"] {
        let want = interp_ok(src, &[n]);
        for threads in [1, 3, 8] {
            let r = run_bin_threads(&bin, &[n], Some(threads));
            assert_eq!((r.code, r.stdout.as_str()), (0, want.as_str()), "{} numbers, {} threads: {}", n, threads, r.stderr);
        }
    }
    valgrind_clean(src, "str_block_parts", &["40"]);
    valgrind_clean(src, "str_block_parts", &["200000"]);
}

/// D315: the text of a replacement by a regular expression moved into it
/// at its last use: written where it is when each match is replaced by as
/// many bytes (a new text, a text made in a lambda, no match at all), freed
/// by it otherwise (a shorter and a longer replacement in a row); a text
/// used after its replacement is not given and keeps its bytes. Short and
/// long texts (more than 1 MiB and 65536 matches: written by the threads),
/// on 1, 3 and 8 threads, as the interpreter; Valgrind finds no leak and no
/// invalid access.
#[test]
fn texts_given_to_their_replacement() {
    let src = r###"action main(args: string[k]): i32 {
    const n = parseI64(args[0]);
    const base = join(map(iota(n), (i) => "xaNDyHaSzWaS".slice(0, i % 13) + "caN"), "-");
    const a = (base + "!").replace(/aND|caN|Ha[DS]|WaS/g, "<3>");
    const c = (base + "!").replace(/aND|caN|Ha[DS]|WaS/g, "<3>").replace(/<3>/g, "#").replace(/#/g, "##");
    const d = (base + "?").replace(/zz/g, "yy");
    const e = map(iota(3), (i) => (base + toString(i)).replace(/caN/g, "CAN").length);
    const f = base.replace(/caN/g, "CAN");
    print([base.length, a.length, c.length, d.length, f.length]);
    print(e);
    print([base.slice(0, 40), a.slice(0, 40), c.slice(0, 40), d.slice(d.length - 20, d.length), f.slice(0, 40)]);
    print(sum(map(bytes(a), (x) => i64(x))) + 3 * sum(map(bytes(c), (x) => i64(x))) + 5 * sum(map(bytes(f), (x) => i64(x))) + 7 * sum(map(bytes(base), (x) => i64(x))));
    return 0;
}
"###;
    let bin = compile(src, "str_given_replace");
    for n in ["40", "200000"] {
        let want = interp_ok(src, &[n]);
        for threads in [1, 3, 8] {
            let r = run_bin_threads(&bin, &[n], Some(threads));
            assert_eq!((r.code, r.stdout.as_str()), (0, want.as_str()), "n = {}, {} threads: {}", n, threads, r.stderr);
        }
    }
    valgrind_clean(src, "str_given_replace", &["40"]);
    valgrind_clean(src, "str_given_replace", &["2000"]);
}

/// D313: the parts of a search and of a replacement in parallel, several
/// per thread, each thread its own range first, then the ends of the
/// others': a text of more than 1 MiB whose first half has no match and
/// whose second half has one every few bytes (the threads of the first
/// parts finish early and take parts of the others); a count, replacements
/// longer, shorter and empty; on 1, 2, 3, 5 and 8 threads and on 65 (more
/// than the 64 ranges: the tasks only taken in turn), as the interpreter.
/// D314: the result written by pieces, the runs of the search and the
/// copies of the long texts without match, at the start and at the end.
#[test]
fn uneven_parts_finished_by_the_other_threads() {
    let src = r#"action main(args: string[k]): i32 {
    const n = parseI64(args[0]);
    const text = "xyzw".repeat(n) + join(map(iota(n), (i) => "aN".repeat(i % 3 + 1) + "xyzw".slice(0, i % 5)), "") + "BY";
    const a = text.replace(/a[NSt]|BY/g, "<2>");
    const b = text.replace(/a[NSt]|BY/g, "");
    const c = a.replace(/<[^>]*>/g, "|");
    const d = (text + "xyzw".repeat(n)).replace(/a[NSt]|BY/g, "<2>");
    print([countMatches(text, /a[NSt]|BY/g), a.length, b.length, c.length, d.length]);
    print([a.slice(a.length - 30, a.length), c.slice(c.length / 2, c.length / 2 + 40), d.slice(d.length - 4 * n - 10, d.length - 4 * n + 10)]);
    print(sum(map(bytes(a), (x) => i64(x))) + 3 * sum(map(bytes(b), (x) => i64(x))) + 5 * sum(map(bytes(c), (x) => i64(x))) + 7 * sum(map(bytes(d), (x) => i64(x))));
    return 0;
}
"#;
    let bin = compile(src, "str_uneven_parts");
    for n in ["30", "300000"] {
        let want = interp_ok(src, &[n]);
        for threads in [1, 2, 3, 5, 8, 65] {
            let r = run_bin_threads(&bin, &[n], Some(threads));
            assert_eq!((r.code, r.stdout.as_str()), (0, want.as_str()), "n = {}, {} threads: {}", n, threads, r.stderr);
        }
    }
}

/// D308: a replacement written in parallel, the text between two matches
/// moved by 32 bytes where the text and the part of the result have the
/// room: gaps of 0 to 40 bytes and longer, matches at both ends of the text
/// and at the ends of the parts of the threads, replacements longer, shorter
/// and empty, a text of more than 1 MiB (written in parallel) and a short
/// one; on 1, 3 and 8 threads, as the interpreter.
#[test]
fn replacements_written_by_moves_of_32_bytes() {
    let src = r#"action main(args: string[k]): i32 {
    const n = parseI64(args[0]);
    const gaps = map(iota(n), (i) => (i * 7919 + i / 13) % 45);
    const text = "ab" + join(map(iota(n), (i) => "xyzw".repeat(gaps[i] / 4 + 1).slice(0, gaps[i]) + (i % 3 == 0 ? "ab" : "aC")), "") + "ab";
    const a = text.replace(/a[bC]/g, "<<long replacement>>");
    const b = text.replace(/a[bC]/g, "-");
    const c = text.replace(/a[bC]/g, "");
    const d = text.replace(/ab|zw/g, "Q");
    print([a.length, b.length, c.length, d.length]);
    print([a.slice(0, 60), b.slice(b.length - 40, b.length), c.slice(1000, 1040), d.slice(d.length / 2, d.length / 2 + 50)]);
    print(sum(map(bytes(a), (x) => i64(x))) + 3 * sum(map(bytes(b), (x) => i64(x))) + 5 * sum(map(bytes(c), (x) => i64(x))) + 7 * sum(map(bytes(d), (x) => i64(x))));
    return 0;
}
"#;
    let bin = compile(src, "str_replace_room");
    for n in ["50", "120000"] {
        let want = interp_ok(src, &[n]);
        for threads in [1, 3, 8] {
            let r = run_bin_threads(&bin, &[n], Some(threads));
            assert_eq!((r.code, r.stdout.as_str()), (0, want.as_str()), "{} matches, {} threads: {}", n, threads, r.stderr);
        }
    }
}

/// D346: `==` and `!=` against a literal of at most 16 bytes compiled in
/// place (its length, then its bytes as one or two integers), on either
/// side; longer literals and two variables by the runtime. The same
/// results as the interpreter, every length around 0, 8 and 16.
#[test]
fn equality_with_short_literals() {
    let src = r#"function kinds(s: string): i64[8] {
    return [s == "" ? 1 : 0, s == "/" ? 1 : 0, "ab" == s ? 1 : 0, s != "abcdefgh" ? 1 : 0, s == "abcdefghi" ? 1 : 0, s == "abcdefghijklmnop" ? 1 : 0, s != "é/." ? 1 : 0, s == "abcdefghijklmnopq" ? 1 : 0];
}

action main(args: string[k]): i32 {
    const words = ["", "/", "//", "a", "ab", "abc", "abcdefg", "abcdefgh", "abcdefgi", "abcdefghi", "abcdefghj", "bbcdefghi", "abcdefghijklmnop", "abcdefghijklmnoq", "xbcdefghijklmnop", "abcdefghijklmnopq", "é/.", "é/", args[0]];
    print(map(words, (w) => kinds(w)));
    print(map(words, (w) => w == args[0] ? 1 : 0));
    return 0;
}
"#;
    let want = "[[1, 0, 0, 1, 0, 0, 1, 0], [0, 1, 0, 1, 0, 0, 1, 0], [0, 0, 0, 1, 0, 0, 1, 0], [0, 0, 0, 1, 0, 0, 1, 0], [0, 0, 1, 1, 0, 0, 1, 0], [0, 0, 0, 1, 0, 0, 1, 0], [0, 0, 0, 1, 0, 0, 1, 0], [0, 0, 0, 0, 0, 0, 1, 0], [0, 0, 0, 1, 0, 0, 1, 0], [0, 0, 0, 1, 1, 0, 1, 0], [0, 0, 0, 1, 0, 0, 1, 0], [0, 0, 0, 1, 0, 0, 1, 0], [0, 0, 0, 1, 0, 1, 1, 0], [0, 0, 0, 1, 0, 0, 1, 0], [0, 0, 0, 1, 0, 0, 1, 0], [0, 0, 0, 1, 0, 0, 1, 1], [0, 0, 0, 1, 0, 0, 0, 0], [0, 0, 0, 1, 0, 0, 1, 0], [0, 0, 1, 1, 0, 0, 1, 0]]\n[0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]\n";
    agree(src, "short_literal_equality", &["ab"], want);
    // only literals of at most 16 bytes: no comparison left to the runtime
    let short = "function kinds(s: string): i64[4] {\n    return [s == \"\" ? 1 : 0, \"ab\" == s ? 1 : 0, s != \"abcdefgh\" ? 1 : 0, s == \"abcdefghijklmnop\" ? 1 : 0];\n}\n\naction main(args: string[k]): i32 {\n    print(map(args, (w) => kinds(w)));\n    return 0;\n}\n";
    let bin = compile(short, "short_literal_equality_ir");
    let ll = std::fs::read_to_string(bin.with_extension("ll")).unwrap();
    assert!(!ll.contains("call i32 @isc_str_cmp("), "a comparison with a short literal left to the runtime");
    let r = run_bin(&bin, &["ab", "", "abcdefghijklmnop"]);
    assert_eq!(r.stdout, "[[0, 1, 1, 0], [1, 0, 1, 0], [0, 0, 1, 1]]\n");
}

/// D348: `toString(x).padStart(n, c)` (or `padEnd`) of an integer and a
/// fill of one byte written in one block (`isc_pad_int`), the text of `x`
/// never made; another fill by the text made, padded, then freed; a text
/// read twice as before. Negative numbers padded as JavaScript does
/// (`0000-7`), unsigned ones, a text already long enough, in templates.
#[test]
fn integers_padded_in_one_block() {
    let src = r#"function pad(x: i64, n: i64): string {
    return toString(x).padStart(n, "0");
}

function right(x: u8, n: i64): string {
    return toString(x).padEnd(n, "*");
}

function two(x: i32, n: i64): string {
    return toString(x).padStart(n, "ab");
}

function twice(x: i64): string {
    const t = toString(x);
    return t.padStart(4, "0") + t;
}

action main(args: string[k]): i32 {
    const v = parseI64(args[0]);
    print([pad(v, 4), pad(v, 1), pad(-v, 6), pad(v, 0), right(u8(v % 256), 5), two(i32(v), 7), twice(v)] as const);
    print(`${pad(v % 60, 2)}:${pad(v % 7, 2)}`);
    return 0;
}
"#;
    agree(src, "padded_integers", &["7"], "(0007, 7, 0000-7, 7, 7****, ababab7, 00077)\n07:00\n");
    agree(src, "padded_integers", &["2026"], "(2026, 2026, 0-2026, 2026, 234**, aba2026, 20262026)\n46:03\n");
    agree(src, "padded_integers", &["0"], "(0000, 0, 000000, 0, 0****, ababab0, 00000)\n00:00\n");
    let bin = compile(src, "padded_integers_ir");
    let ll = std::fs::read_to_string(bin.with_extension("ll")).unwrap();
    // the pads of the tuple in one block each; those of the template
    // written by the concatenation itself (D359)
    assert!(ll.matches("call void @isc_pad_int(").count() >= 5, "integers padded in one block");
    assert!(ll.contains("call void @isc_put_int("), "the parts of the template not written in its block");
}

/// D351: `acc + x` where `acc` is the accumulator of a tail recursion
/// (owned, at its last use): `x` written after it in its block when the
/// block has room, else in a block of twice its length; the same text as
/// the interpreter, nothing lost or freed twice (Valgrind, the allocator of
/// glibc: every append in a new block), the IR calling `isc_str_append`.
#[test]
fn strings_grown_in_place() {
    let src = include_str!("data/strings_grown.isc");
    let want = "114286\n0,1234567,8901234,5678901,2345\n9012345,6789\nab!ab\nx0,120,1\n";
    agree(src, "strings_grown", &["100000"], want);
    let bin = compile(src, "strings_grown_ir");
    let ll = std::fs::read_to_string(bin.with_extension("ll")).unwrap();
    assert!(ll.contains("call void @isc_str_append"), "no text grown in place");
}

/// D359: the texts of integers that are parts of a concatenation (a
/// template, a chain), padded or not, signed or not, never made: the
/// concatenation writes them in its one block (`isc_str_build`), more than
/// 16 parts too, the left part of an accumulator given (its text continued
/// in its block, D351), fills and texts of 0 to 39 bytes; the same text as
/// the interpreter, Valgrind clean, no text of an integer made; the
/// parts written by the compiled code (D363).
#[test]
fn integers_written_in_the_block_of_their_template() {
    let src = include_str!("data/strings_template_parts.isc");
    let bin = compile(src, "template_parts");
    for a in ["7", "-42", "0", "-9223372036854775807"] {
        let want = interp_ok(src, &[a]);
        let r = run_bin(&bin, &[a]);
        assert_eq!((r.code, r.stdout.as_str()), (0, want.as_str()), "{}: {}", a, r.stderr);
    }
    valgrind_clean(src, "template_parts", &["5"]);
    let ll = std::fs::read_to_string(bin.with_extension("ll")).unwrap();
    // each integer written where it goes (D363), the literals between them
    // stored as constants (`memcpy` of a constant length, inlined), not
    // given to the runtime
    assert!(!ll.contains("isc_str_build"), "a table of parts given to the runtime");
    assert!(ll.matches("call void @isc_put_int(").count() >= 18, "templates not written in their blocks");
    assert!(ll.matches(", i64 1, i1 false)").count() >= 17, "the literals of a template not stored as constants");
    // one text of an integer made: that of `${v}` alone, not a concatenation
    assert_eq!(ll.matches("call void @isc_to_string_i64(").count(), 1, "a text of an integer made");
    assert!(!ll.contains("call void @isc_pad_int("), "a padded text of an integer made");
    // the accumulators give their left text to the concatenation
    assert!(ll.contains("call ptr @isc_str_reserve("), "the left text of an accumulator not given");
}

/// D361: a literal kept as a string (returned, stored in a record or an
/// array, the start of an accumulator, made by the threads of a `map`) is
/// not copied into a block: its bytes where they are in the program, never
/// freed, never written (a replacement or a text grown from it makes a new
/// block); the text of the interpreter with 1 and 4 threads, Valgrind clean,
/// no copy of a literal in the IR.
#[test]
fn literals_kept_without_a_copy() {
    let src = include_str!("data/strings_literals_kept.isc");
    let bin = compile(src, "literals_kept");
    for a in ["0", "1", "2"] {
        let want = interp_ok(src, &[a]);
        for t in [1, 4] {
            let r = run_bin_threads(&bin, &[a], Some(t));
            assert_eq!((r.code, r.stdout.as_str()), (0, want.as_str()), "{} on {} threads: {}", a, t, r.stderr);
        }
    }
    valgrind_clean(src, "literals_kept", &["1"]);
    let ll = std::fs::read_to_string(bin.with_extension("ll")).unwrap();
    // the copies left are not of literals: the accumulator of `grow` at its
    // entry (rule A, D351) and the elements `sort` puts in its array (twice)
    assert_eq!(ll.matches("call void @isc_str_new(").count(), 3, "a literal copied");
    assert!(ll.matches("@.olit.").count() >= 16, "the literals not kept where they are");
}

/// D368: a concatenation with a literal is written by the compiled code
/// (the literals stored as constants, the texts by `isc_put_text`), not
/// by `isc_str_concat`; the copies of 33 to 64 bytes by two moves that
/// overlap: texts of 0 to 80 bytes around literals, an accumulator grown by
/// literals, replacements whose matches are 0 to 80 bytes apart; the text
/// of the interpreter with 1 and 4 threads, Valgrind clean.
#[test]
fn concatenations_with_literals_written_by_the_code() {
    let src = include_str!("data/strings_concat_literals.isc");
    let bin = compile(src, "concat_literals");
    for a in ["0", "3"] {
        let want = interp_ok(src, &[a]);
        for t in [1, 4] {
            let r = run_bin_threads(&bin, &[a], Some(t));
            assert_eq!((r.code, r.stdout.as_str()), (0, want.as_str()), "{} on {} threads: {}", a, t, r.stderr);
        }
    }
    valgrind_clean(src, "concat_literals", &["1"]);
    let ll = std::fs::read_to_string(bin.with_extension("ll")).unwrap();
    assert!(!ll.contains("call void @isc_str_concat"), "a concatenation with a literal given to the runtime");
    assert!(ll.matches("call void @isc_put_text(").count() >= 5, "the texts not written by the compiled code");
}

/// D369: `padStart` and `padEnd` of a text by a fill of one byte write the
/// text and the fill by moves: fills of 0 to 40 bytes before and after
/// texts of 0 to 20 bytes, a fill of several bytes (the other way), the
/// text of the interpreter.
#[test]
fn texts_padded_by_moves() {
    let src = "function text(n: i64): string {\n    return stringOf(map(iota(n), (j) => u8(97 + (n + j) % 26)));\n}\naction main(args: string[k]): i32 {\n    const v = parseI64(args[0]);\n    const ts = map(iota(21), (n) => text(n));\n    print(map(iota(21 * 41), (i) => ts[i % 21].padStart(i / 21 + v, \"*\")).join(\"|\"));\n    print(map(iota(21 * 41), (i) => ts[i % 21].padEnd(i / 21 + v, \".\")).join(\"|\"));\n    print(map(iota(30), (i) => ts[i % 5].padStart(i, \"ab\")).join(\"|\"));\n    return 0;\n}\n";
    agree(src, "texts_padded", &["0"], &interp_ok(src, &["0"]));
    agree(src, "texts_padded", &["3"], &interp_ok(src, &["3"]));
}

/// D441: the integers of a template of one or two digits, not negative,
/// written by their bytes (a pair of a table of `00` to `99`), the others
/// by the runtime: every value from -120 to 120, around 999 and 1000, the
/// extremes of `i64`, integers of 8 and 32 bits, several in one template,
/// one padded (`padStart`, the runtime); padded to 2 by zeros at the
/// start from -20 to 129 (D446: two digits of the table below 100), by
/// another fill, at the end; padded to 3 and 4 by zeros, from -70 to about
/// 64000, both sides of 1000 and 10000 (D477: a digit and a pair, two
/// pairs); the interpreter's text.
#[test]
fn small_integers_in_templates() {
    let src = "action main(args: string[k]): i32 {
    print(map(iota(241), (i) => `${i - 120}|${(i - 120) * 7}`).join(\",\"));
    print([`${999}${1000}`, `${-9223372036854775807 - 1}`, `${9223372036854775807}`, `${u8(255)}:${i32(-5)}:${u8(7)}`]);
    print(map(iota(30), (i) => `${i / 10}${i % 10}-${toString(i).padStart(3, \"0\")}`).join(\" \"));
    print(map(iota(130), (i) => `${toString(i - 20).padStart(2, \"0\")}:${toString(i).padStart(2, \"0\")}|${toString(i).padStart(2, \"*\")}|${toString(i).padEnd(2, \"0\")}`).join(\" \"));
    print(map(concat(iota(130), map(iota(40), (k) => 980 + k * 233)), (i) => `${toString(i - 20).padStart(3, \"0\")}:${toString(i * 11 - 50).padStart(4, \"0\")}|${toString(i).padStart(3, \"0\")}${toString(i * 7).padStart(4, \"0\")}`).join(\" \"));
    return 0;
}
";
    let Outcome::Exit(0, want) = interp(src, &[]) else { panic!("the interpreter failed") };
    assert!(want.contains("-120|-840,") && want.contains("120|840"), "{}", want);
    for (name, isa) in [("small_ints", isc_codegen::Isa::Native), ("small_ints_avx2", isc_codegen::Isa::Avx2)] {
        let bin = compile_with(src, name, isc_codegen::Options { isa, ..isc_codegen::Options::default() });
        let r = run_bin_threads(&bin, &[], Some(1));
        assert_eq!(r.code, 0, "{}", r.stderr);
        assert_eq!(r.stdout, want, "{}", name);
    }
}
