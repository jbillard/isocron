//! Strings (DECISIONS D134): literals and their escapes, `++`, the order by
//! bytes, `len`, `bytes`, `stringOf`, `toString`, `parseF64`; strings in
//! arrays, tuples, sum and recursive types, in `fold`, `reduce` and `scan`.
//! The interpreter and the compiled program print the same text, and the
//! compiled program frees every string (Valgrind, when installed).

mod common;
use common::*;

fn valgrind_clean(src: &str, name: &str, args: &[&str]) {
    checked_clean(src, name, args);
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
    assert!(!ll.contains("call i32 @memcmp("), "a comparison with a short literal left to `memcmp`");
    let r = run_bin(&bin, &["ab", "", "abcdefghijklmnop"]);
    assert_eq!(r.stdout, "[[0, 1, 1, 0], [1, 0, 1, 0], [0, 0, 1, 1]]\n");
}

/// D348: `toString(x).padStart(n, c)` (or `padEnd`) of an integer and a
/// fill of one byte written in one block (`pad_int`, D606), the text of `x`
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
    assert!(ll.matches("; int pad").count() >= 5, "integers padded in one block");
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
    // one text of an integer made: that of `${v}` alone, not a concatenation,
    // by the function of the prelude (D588: its count of digits by
    // `llvm.ctlz`), not the runtime
    assert!(!ll.contains("call void @isc_to_string_i64("), "a text of an integer made by the runtime");
    // (the functions that give a text: the digits of a padded integer are
    // lowered beside them, D606, and not called here)
    let texts: Vec<&str> = ll
        .lines()
        // (their type: the fast paths of the allocator, inlined too, return
        // a word, D605)
        .filter(|l| l.starts_with("define internal tailcc { ptr, i64 } ") && l.contains(" alwaysinline "))
        .filter_map(|l| l.split('@').nth(1).and_then(|r| r.split('(').next()))
        .collect();
    let calls: usize = texts.iter().map(|f| ll.matches(&format!("call tailcc {{ ptr, i64 }} @{}(", f)).count()).sum();
    assert_eq!((texts.len(), calls), (2, 1), "the text of `${{v}}` alone not made once by the prelude");
    assert!(!ll.contains("; int pad"), "a padded text of an integer made");
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
    // the copy left is not of a literal: the accumulator of `grow` at its
    // entry (rule A, D351); the elements `sort` puts in its array are moved
    // from its source, read for the last time (D548)
    assert_eq!(ll.matches("; text copy").count(), 1, "a literal copied");
    // the literals of at most 12 bytes are values, their bytes in them
    // (D530); the one longer kept in the program's data
    assert!(ll.matches("@.olit.").count() >= 1, "the literals not kept where they are");
}

/// D368: a concatenation with a literal is written by the compiled code
/// (the literals stored as constants, the texts by moves or `memcpy`), not
/// by the general concatenation (D592); the copies of 33 to 64 bytes by two moves that
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
    assert!(!ll.contains("; text concat"), "a concatenation with a literal made by the general one");
    // each text by moves here, a long one by `memcpy` of its length (D509)
    let copies = ll.lines().filter(|l| l.contains("call void @llvm.memcpy.p0.p0.i64(") && l.trim_end().ends_with("i1 false)") && l.contains(", i64 %")).count();
    assert!(copies >= 5, "the texts not written by the compiled code");
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

/// D478: the texts of a template of at most 64 bytes copied in the code by
/// moves that overlap (32, 16, 8, 4 bytes, or three bytes), the runtime
/// beyond: parts of 0 to 70 bytes and of 100, both sides of 4, 8, 16, 32
/// and 64, two
/// in a template with literals and integers between them, on the stack
/// and in a block (a result kept); the interpreter's text, for the machine
/// and for AVX2.
#[test]
fn short_texts_of_templates_by_masks() {
    let src = "action main(args: string[k]): i32 {
    const t = stringOf(map(iota(200), (j) => u8(65 + j % 26)));
    const lens = concat(iota(71), [100]);
    print(map(lens, (n) => `[${t.slice(0, n)}]${n}<${t.slice(200 - n, 200)}>`));
    const kept = map(lens, (n) => `\"${t.slice(n % 7, n % 7 + n)}\"`);
    print(kept.join(\" \"));
    return 0;
}
";
    let Outcome::Exit(0, want) = interp(src, &[]) else { panic!("the interpreter failed") };
    for (name, isa) in [("short_parts", isc_codegen::Isa::Native), ("short_parts_avx2", isc_codegen::Isa::Avx2)] {
        let bin = compile_with(src, name, isc_codegen::Options { isa, ..isc_codegen::Options::default() });
        let ll = std::fs::read_to_string(bin.with_extension("ll")).unwrap();
        let long = ll.lines().any(|l| l.contains("call void @llvm.memcpy.p0.p0.i64(") && l.contains(", i64 %"));
        assert!(ll.contains("load i128, ptr") && long, "{}: moves here, `memcpy` beyond", name);
        let r = run_bin_threads(&bin, &[], Some(1));
        assert_eq!(r.code, 0, "{}", r.stderr);
        assert_eq!(r.stdout, want, "{}", name);
    }
}

/// D482: `s.lastIndexOf(t)`, the byte index of the last `t` (-1 if none,
/// the length of `s` for an empty `t`): a byte in texts of 0 to 70 bytes at
/// every place (the windows of 16 from the end, then `memrchr`), needles
/// of several bytes, overlapping ones, characters of two bytes; the
/// interpreter's values.
#[test]
fn last_index_of_texts() {
    let src = "action main(args: string[k]): i32 {
    const t = stringOf(map(iota(70), (j) => u8(97 + j % 5)));
    print(map(iota(71), (n) => t.slice(0, n).lastIndexOf(\"c\")));
    print(map(iota(71), (n) => t.slice(0, n).lastIndexOf(\"cd\") * 100 + t.slice(n % 7, n).lastIndexOf(\"\")));
    print([\"aaa\".lastIndexOf(\"aa\"), \"abcabc\".lastIndexOf(\"abc\"), \"\u{e9}\u{e9}\".lastIndexOf(\"\u{e9}\"), \"a\u{e9}b\".lastIndexOf(\"b\"), \"\".lastIndexOf(\"a\"), \"\".lastIndexOf(\"\")] as const);
    return 0;
}
";
    let Outcome::Exit(0, want) = interp(src, &[]) else { panic!("the interpreter failed") };
    assert!(want.ends_with("(1, 3, 2, 3, -1, 0)\n"), "{}", want);
    let r = run_bin(&compile(src, "last_index_of"), &[]);
    assert_eq!((r.code, r.stdout.as_str()), (0, want.as_str()), "{}", r.stderr);
}

/// D487: `toFixed` of 2000 doubles from about 1e-9 to 1e20, of both signs,
/// with 0 to 20 decimals (the short path up to 18, the exact value beyond):
/// the compiled program writes what the interpreter writes.
#[test]
fn to_fixed_of_many_doubles() {
    let src = "action main(args: string[k]): i32 {
    const xs = map(iota(2000), (i) => (i % 2 == 0 ? 1.0 : -1.0) * f64(i * 7919 % 100003 + 1) / 7.0 * [1e-9, 1e-3, 1.0, 1e3, 1e9, 1e15, 1e19][i % 7]);
    print(join(map(xs, (x) => join(map(iota(21), (d) => x.toFixed(d)), \" \")), \"\\n\"));
    return 0;
}
";
    let Outcome::Exit(0, want) = interp(src, &[]) else { panic!("the interpreter failed") };
    assert_eq!(want.lines().count(), 2000);
    let r = run_bin(&compile(src, "to_fixed_many"), &[]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(r.stdout == want, "{:?}", r.stdout.lines().zip(want.lines()).find(|(a, b)| a != b));
}

/// D508, D509: a text grown by appends of parts of every length, the
/// interpreter's text from 0 to 2000 appends, on 1 and 4 threads. `acc +
/// part` (`isc_str_append`, `append_parts`: a part of at most 16 bytes by
/// one move of 16, D508), parts of 0 to 40 bytes; `acc + part + sep(i)`,
/// texts only after the text given (written by the compiled code after
/// `isc_str_reserve`, D509: no `isc_str_append_n`), parts of 0 to 100
/// bytes (beyond 64 by `memcpy`).
#[test]
fn appends_of_parts_of_every_length() {
    let two = "function grow(i: i64, n: i64, acc: string, src: string): string {
    if (i == n) {
        return acc;
    }
    const len = i % 41;
    const at = (i * 7) % (src.length - 41);
    return grow(i + 1, n, acc + src.slice(at, at + len), src);
}

action main(args: string[k]): i32 {
    const n = parseI64(args[0]);
    const src = join(map(iota(200), (i) => `${i * 7919 % 1000}`), \"-\");
    const r = grow(0, n, \"<\", src);
    print(r.length);
    print(r.slice(0, Math.min(300, r.length)));
    print(r.slice(Math.max(0, r.length - 300), r.length));
    print(sum(map(bytes(r), (c) => i64(c))));
    return 0;
}
";
    let three = "function sep(i: i64): string {
    return i % 3 == 0 ? \"|\" : (i % 3 == 1 ? \"<>\" : \"\");
}

function grow(i: i64, n: i64, acc: string, src: string): string {
    if (i == n) {
        return acc;
    }
    const len = i % 101;
    const at = (i * 7) % (src.length - 101);
    return grow(i + 1, n, acc + src.slice(at, at + len) + sep(i), src);
}

action main(args: string[k]): i32 {
    const n = parseI64(args[0]);
    const src = join(map(iota(200), (i) => `${i * 7919 % 1000}`), \"-\");
    const r = grow(0, n, \"<\", src);
    print(r.length);
    print(r.slice(0, Math.min(300, r.length)));
    print(r.slice(Math.max(0, r.length - 300), r.length));
    print(sum(map(bytes(r), (c) => i64(c))));
    return 0;
}
";
    for (name, src, call, not) in [("two", two, "@isc_str_append(", "@isc_str_reserve("), ("three", three, "@isc_str_reserve(", "@isc_str_append_n(")] {
        let bin = compile(src, &format!("appends_every_length_{}", name));
        let ll = std::fs::read_to_string(bin.with_extension("ll")).unwrap();
        assert!(ll.contains(&format!("call void {}", call)) || ll.contains(&format!("call ptr {}", call)), "{}: no call of {}", name, call);
        assert!(!ll.contains(&format!("call void {}", not)) && !ll.contains(&format!("call ptr {}", not)), "{}: a call of {}", name, not);
        for n in ["0", "1", "17", "50", "2000"] {
            let want = interp_ok(src, &[n]);
            for t in [1, 4] {
                let r = run_bin_threads(&bin, &[n], Some(t));
                assert_eq!((r.code, r.stdout.as_str()), (0, want.as_str()), "{} n {} {} threads: {}", name, n, t, r.stderr);
            }
        }
    }
}

/// D518: a text freed by the compiled code as a part of a tuple (taken out
/// of it, or the tuple freed) tests first whether it is a literal of the
/// program kept as a string (D361: 4 bytes past a multiple of 16, where no
/// block starts), which the runtime never frees, and does not call
/// `isc_free` then. Literals and texts of the heap in tuples, records,
/// accumulators and the branches of `if`s, freed at the end of their
/// scopes: the interpreter's output on 1 and 4 threads, nothing lost or
/// freed twice under valgrind, the test before the call in the LLVM.
#[test]
fn literals_kept_as_strings_not_given_to_isc_free() {
    let src = "type Item = { label: string, n: i64 };

function item(i: i64): Item {
    return { label: i % 3 == 0 ? \"zero\" : (i % 3 == 1 ? `one ${i}` : \"\"), n: i };
}

function why(i: i64): [i64, string] {
    return i % 2 == 0 ? [i, \"\"] as const : [-1, `odd ${i}`] as const;
}

function grow(i: i64, n: i64, acc: string): string {
    if (i == n) {
        return acc;
    }
    const [k, w] = why(i);
    return grow(i + 1, n, k >= 0 ? acc : acc + w);
}

action main(args: string[k]): i32 {
    const n = parseI64(args[0]);
    print(sum(map(iota(n), (i) => item(i).label.length)));
    print(sum(map(iota(n), (i) => {
        const [k, w] = why(i);
        return k + w.length;
    })));
    print(grow(0, n, \"\").length);
    print(grow(0, 7, \"start\"));
    return 0;
}
";
    let bin = compile(src, "literals_not_freed");
    let ll = std::fs::read_to_string(bin.with_extension("ll")).unwrap();
    // the low bits of the pointer of a text tested against 4 before a call
    // of `isc_free`
    let lines: Vec<&str> = ll.lines().collect();
    let tested = lines.iter().enumerate().any(|(k, l)| {
        l.contains("call void @isc.late.free(ptr") && lines[k.saturating_sub(8)..k].iter().any(|p| p.contains("icmp eq i64") && p.trim_end().ends_with(", 4"))
    });
    assert!(tested, "a text freed without the test of a literal");
    for n in ["0", "1", "10", "1000"] {
        let want = interp_ok(src, &[n]);
        for t in [1, 4] {
            let r = run_bin_threads(&bin, &[n], Some(t));
            assert_eq!((r.code, r.stdout.as_str()), (0, want.as_str()), "n {} {} threads: {}", n, t, r.stderr);
        }
    }
    valgrind_clean(src, "literals_not_freed", &["300"]);
}

/// D519: a text taken out of a tuple is tested as a literal wherever it is
/// freed, before the jump of a tail call too (the loop of the check of the
/// members of a decoder of `json`: no literal given to `isc_free` there):
/// every `isc_free` of the loop that takes such texts apart preceded by the
/// test; the interpreter's output, valgrind clean.
#[test]
fn texts_taken_out_of_tuples_tested_where_they_are_freed() {
    let src = "function check(i: i64): [i64, string] {
    return i % 5 == 0 ? [-1, `bad ${i}`] as const : [i, \"\"] as const;
}

function walk(i: i64, n: i64, acc: i64): i64 {
    if (i == n) {
        return acc;
    }
    const [k, why] = check(i);
    const t = acc + k + why.length;
    return walk(i + 1, n, t);
}

action main(args: string[k]): i32 {
    print(walk(0, parseI64(args[0]), 0));
    return 0;
}
";
    let ir = iscc::frontend("taken_texts.isc", src).unwrap();
    let wi = ir.fns.iter().position(|f| f.name == "walk").expect("no function walk");
    let bin = compile(src, "taken_texts");
    let ll = std::fs::read_to_string(bin.with_extension("ll")).unwrap();
    let start = ll.find(&format!("@isc_fn_{}(", wi)).expect("no function walk in the LLVM");
    let body = &ll[start..start + ll[start..].find("\n}\n").unwrap()];
    let lines: Vec<&str> = body.lines().collect();
    let frees: Vec<usize> = (0..lines.len()).filter(|&k| lines[k].contains("call void @isc.late.free(ptr")).collect();
    assert!(!frees.is_empty(), "no text freed in walk");
    for k in frees {
        assert!(lines[k.saturating_sub(8)..k].iter().any(|p| p.contains("icmp eq i64") && p.trim_end().ends_with(", 4")), "a text of walk freed without the test of a literal: {}", lines[k]);
    }
    for n in ["0", "1", "10", "1000"] {
        let want = interp_ok(src, &[n]);
        let r = run_bin(&bin, &[n]);
        assert_eq!((r.code, r.stdout.as_str()), (0, want.as_str()), "n {}", n);
    }
    valgrind_clean(src, "taken_texts", &["300"]);
}

/// D531: empty texts of every form (a slice of nothing, `""`, a text made
/// empty, the zero of a field of another variant) added to a text given or
/// not, copied into an array, sorted, compared. The representation tried
/// by D530 copied a short text by one load of 16 bytes where they stay in
/// their page, and read so at the address Rust gives an empty slice (1): a
/// fault of segmentation (`time.format`, a field after an empty one). Kept
/// for every read past the bytes of a text.
#[test]
fn empty_texts_of_every_form() {
    let src = "type T = A(string) | B(i64);

function grow(acc: string, s: string, i: i64): string {
    if (i == 6) {
        return acc;
    }
    return grow(`${acc}${s.slice(i, i)}${i}`, s, i + 1);
}

function add(acc: string, s: string, i: i64): string {
    if (i == 6) {
        return acc;
    }
    return add(acc + s.slice(i, i), s, i + 1);
}

function pieces(s: string, i: i64): string[] {
    return map(iota(i), (k) => s.slice(k, k));
}

function text(t: T): string {
    return match (t) {
        A(x) => x,
        B(n) => \"\",
    };
}

action main(args: string[k]): i32 {
    const s = \"abcdefgh\";
    const n = parseI64(args[0]);
    print(grow(\"\", s, 0));
    print(grow(\"start\", s, 0));
    print(add(\"\", s, 0).length);
    print(add(`a${n}`, s, 0));
    print(add(\"a text longer than twelve\", s, 0));
    const e = s.slice(n, n);
    const g = `${s.slice(0, n)}`;
    print([e.length, g.length, (e + \"\").length, (\"\" + e).length] as const);
    print(e == \"\" && g == e && text(B(1)) == e && text(A(\"\")) == g);
    const ps = sort(concat(pieces(s, 5), [\"b\", \"\", \"a\"]));
    print(ps);
    print(map(ps, (p) => `[${p}]`).join(\"\"));
    print(`${e}${e}${g}x${e}`);
    return 0;
}
";
    let bin = compile(src, "empty_texts");
    for a in ["0", "3", "8"] {
        let want = interp_ok(src, &[a]);
        let r = run_bin(&bin, &[a]);
        assert_eq!((r.code, r.stdout.as_str()), (0, want.as_str()), "{}: {}", a, r.stderr);
    }
    valgrind_clean(src, "empty_texts", &["3"]);
}

/// D532: texts of 0 to 20 bytes that differ at each place, or not, or only
/// by their length, compared (`==`, `!=`, `<`, `<=`, `>`, `>=`), sorted and
/// used as keys of a map: their first 8 bytes decide without a call when
/// they differ, the runtime otherwise; the same answers as the interpreter.
#[test]
fn comparisons_of_texts_by_their_first_bytes() {
    let src = "function text(n: i64, at: i64, c: i64): string {
    return stringOf(map(iota(n), (j) => j == at ? u8(c) : u8(97 + j % 3)));
}

action main(args: string[k]): i32 {
    const m = parseI64(args[0]);
    const ts = concat(map(iota(m * 21), (k) => text(k / 21, k % 21, 98)), map(iota(m), (n) => text(n, -1, 0)));
    const n = ts.length;
    print(count(iota(n * n), (k) => ts[k / n] == ts[k % n]));
    print(count(iota(n * n), (k) => ts[k / n] != ts[k % n]));
    print(count(iota(n * n), (k) => ts[k / n] < ts[k % n]));
    print(count(iota(n * n), (k) => ts[k / n] <= ts[k % n]));
    print(count(iota(n * n), (k) => ts[k / n] > ts[k % n]));
    print(sum(map(iota(n * n), (k) => ts[k / n] >= ts[k % n] ? k % 7 : 0)));
    const s = sort(ts);
    print(s.slice(0, 30));
    print(sum(map(iota(n), (k) => s[k].length * (k % 5))));
    const keys = new Map(map(iota(n), (k) => [ts[k], k] as const));
    print(sum(map(ts, (t) => keys.get(t) ?? -1)));
    print(keys.get(\"abcabcabc\") ?? -1);
    return 0;
}
";
    let bin = compile(src, "text_comparisons");
    for a in ["21"] {
        let want = interp_ok(src, &[a]);
        let r = run_bin(&bin, &[a]);
        assert_eq!((r.code, r.stdout.as_str()), (0, want.as_str()), "{}: {}", a, r.stderr);
    }
}

/// `readWord(b, at, n)` (D580): every place and size of a text of 16 bytes,
/// little-endian, interpreted and compiled the same (the word of the
/// runtime core, `read_word`); a size out of 1 to 8 and bytes past the end
/// or before the start: the same panics.
#[test]
fn words_read_from_bytes() {
    let src = "action main(args: string[k]): i32 {
    const b = bytes(\"0123456789abcdef\");
    print(map(iota(8), (n) => sum(map(iota(17 - (n + 1)), (at) => readWord(b, at, n + 1) % u64(1000003)))));
    print(map(iota(9), (at) => readWord(b, at, 8)));
    print(map(iota(3), (n) => readWord(b, 16 - (n + 1), n + 1)));
    return 0;
}
";
    let mut want = String::new();
    let b = b"0123456789abcdef";
    let word = |at: usize, n: usize| (0..n).fold(0u64, |w, i| w | (b[at + i] as u64) << (8 * i));
    let sums: Vec<String> = (1..=8).map(|n| (0..=16 - n).map(|at| word(at, n) % 1000003).sum::<u64>().to_string()).collect();
    want.push_str(&format!("[{}]\n", sums.join(", ")));
    let eights: Vec<String> = (0..9).map(|at| word(at, 8).to_string()).collect();
    want.push_str(&format!("[{}]\n", eights.join(", ")));
    let ends: Vec<String> = (1..=3).map(|n| word(16 - n, n).to_string()).collect();
    want.push_str(&format!("[{}]\n", ends.join(", ")));
    agree(src, "read_word", &[], &want);
    let bad = "action main(args: string[k]): i32 {
    const b = bytes(\"0123456789abcdef\");
    print(readWord(b, parseI64(args[0]), parseI64(args[1])));
    return 0;
}
";
    let bin = compile(bad, "read_word_bad");
    for (args, msg) in [
        (["16", "1"], "index out of bounds: the index is 16 but the length is 16"),
        (["9", "8"], "index out of bounds: the index is 16 but the length is 16"),
        (["-1", "2"], "index out of bounds: the index is -1 but the length is 16"),
        (["0", "0"], "readWord: a word has 1 to 8 bytes, got 0"),
        (["0", "9"], "readWord: a word has 1 to 8 bytes, got 9"),
    ] {
        let m = interp_panic(bad, &args);
        assert!(m.contains(msg), "{:?} interpreted: {}", args, m);
        let r = run_bin(&bin, &args);
        assert_ne!(r.code, 0);
        assert!(r.stderr.contains(msg), "{:?} compiled: {}", args, r.stderr);
    }
}

/// D582: `parseI64` written in Isocron (the prelude, its panic `$panic`):
/// the answers of Rust's `str::parse::<i64>` (that the runtime gave),
/// valid texts of 1 to 25 characters (signs, leading zeros, the bounds of
/// `i64`) in one program, interpreted and compiled; texts that are no
/// integer (a sign alone, a character next to the digits, an overflow
/// after leading zeros), each the panic of the runtime, at the call. A
/// program cannot name `$panic`.
#[test]
fn parse_i64_written_in_isocron() {
    let mut texts: Vec<String> = [
        "0", "-0", "+0", "00", "9223372036854775807", "-9223372036854775808", "+9223372036854775807",
        "000000000000000000000009223372036854775807", "-000000000000000000000009223372036854775808",
        "00000000000000000000", "000000000000000000001", "12345678", "123456789", "1234567890123456",
        "12345678901234567", "1234567890123456789", "-1234567890123456789", "1000000000000000000",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let mut x: u64 = 582;
    for n in 1..=19 {
        for _ in 0..6 {
            let d: String = (0..n)
                .map(|_| {
                    x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                    char::from(b'0' + (x >> 59) as u8 % 10)
                })
                .collect();
            for t in [d.clone(), format!("-{}", d), format!("+{}", d), format!("000{}", d)] {
                if t.parse::<i64>().is_ok() {
                    texts.push(t);
                }
            }
        }
    }
    let src = "action main(args: string[k]): i32 {\n    print(map(args, parseI64));\n    return 0;\n}\n";
    let args: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();
    let values: Vec<String> = texts.iter().map(|s| s.parse::<i64>().unwrap().to_string()).collect();
    agree(src, "parse_i64_isocron", &args, &format!("[{}]\n", values.join(", ")));
    let one = "action main(args: string[k]): i32 {\n    print(parseI64(args[0]));\n    return 0;\n}\n";
    let bin = compile(one, "parse_i64_isocron_bad");
    for s in [
        "", "+", "-", "--1", "+-1", "1a", " 1", "1 ", "/", ":", "1/2", "١", "9223372036854775808", "-9223372036854775809",
        "99999999999999999999", "000000000000000000000009223372036854775808", "12345678901234567x",
    ] {
        assert!(s.parse::<i64>().is_err(), "{}", s);
        let msg = format!("panic at line 2, column 19: parseI64: `{}` is not a valid integer", s);
        let m = interp_panic(one, &[s]);
        assert!(m.contains(&msg), "{:?} interpreted: {}", s, m);
        let r = run_bin(&bin, &[s]);
        assert_eq!(r.code, 101, "{:?}: {}", s, r.stderr);
        assert!(r.stderr.contains(&msg), "{:?} compiled: {}", s, r.stderr);
    }
    // ten calls (D582: a function of the prelude is always inlined, its
    // panic at the place of each call, as the runtime's)
    let ten = "action main(args: string[k]): i32 {
    const a = parseI64(args[0]);
    const b = map(args, parseI64);
    const c = map(args, (s) => parseI64(s) + 1);
    const d = map(args, (s) => parseI64(s) * 2);
    const e = map(args, (s) => parseI64(s) - 3);
    const f = map(args, (s) => parseI64(s) / 2);
    const g = map(args, (s) => parseI64(s) % 7);
    const h = map(args, (s) => parseI64(s) & 255);
    const i = map(args, (s) => parseI64(s) | 1);
    const j = map(args, (s) => parseI64(s) ^ 5);
    print([a, sum(b), sum(c), sum(d), sum(e), sum(f), sum(g), sum(h), sum(i), sum(j)]);
    return 0;
}
";
    let bin10 = compile(ten, "parse_i64_ten");
    for (args, msg) in [
        (["12", "x"], "panic at line 3, column 18: parseI64: `x` is not a valid integer"),
        (["x", "1"], "panic at line 2, column 23: parseI64: `x` is not a valid integer"),
        (["7", "-"], "panic at line 3, column 18: parseI64: `-` is not a valid integer"),
    ] {
        let m = interp_panic(ten, &args);
        assert!(m.contains(msg), "{:?} interpreted: {}", args, m);
        let r = run_bin(&bin10, &args);
        assert!(r.stderr.contains(msg), "{:?} compiled: {}", args, r.stderr);
    }
    let r = run_bin(&bin10, &["-9223372036854775808", "1"]);
    assert_eq!(r.stdout, "[-9223372036854775808, -9223372036854775807, -9223372036854775805, 2, 9223372036854775803, -4611686018427387904, 0, 1, -9223372036854775806, -9223372036854775799]\n", "{}", r.stderr);
    // its panic allocates nothing (the runtime writes its three parts):
    // accepted by `requireNoAlloc`, as the runtime's was
    let no_alloc = "@requireNoAlloc
function total(xs: string[n]): i64 {
    return sum(map(xs, parseI64));
}

action main(args: string[k]): i32 {
    print(total(args));
    return 0;
}
";
    agree(no_alloc, "parse_i64_no_alloc", &["1", "22", "-3"], "20\n");
    let named = "action main(args: string[k]): i32 {\n    const x: i64 = $panic(\"boom\", \"\", \"\");\n    print(x);\n    return 0;\n}\n";
    assert!(isc_types::check_source(named).is_err());
}

/// D586: `tryParseI64` written in Isocron (the prelude, on the reading of
/// `parseI64`): `Ok` of the value of Rust's `str::parse::<i64>`, `Err` of
/// the message of the panic of `parseI64` for any other text, interpreted
/// and compiled the same.
#[test]
fn try_parse_i64_written_in_isocron() {
    let texts = [
        "0", "-0", "+7", "12345678", "-1234567", "123456789012", "-9223372036854775808", "9223372036854775807",
        "000000000000000000000042", "", "+", "-", "x", "1a", " 1", "9223372036854775808", "99999999999999999999",
    ];
    let src = "action main(args: string[k]): i32 {
    forEach(args, (s) => {
        print(match (tryParseI64(s)) { Ok(v) => `ok ${v}`, Err(e) => `err ${e}` });
    });
    return 0;
}
";
    let want: String = texts
        .iter()
        .map(|t| match t.parse::<i64>() {
            Ok(v) => format!("ok {}\n", v),
            Err(_) => format!("err parseI64: `{}` is not a valid integer\n", t),
        })
        .collect();
    agree(src, "try_parse_i64_isocron", &texts, &want);
}

/// D588: the text of an integer written in Isocron (`$rtIntText`,
/// `$rtUintText` of the prelude, called by the code generator where no
/// fusion writes it): the texts of Rust's `Display` for every width, the
/// bounds, the powers of ten and their neighbours, a `-` before the
/// negatives; interpreted (the runtime's text) and compiled the same.
#[test]
fn integer_text_written_in_isocron() {
    let src = "action main(args: string[k]): i32 {
    const p10 = map(iota(19), (i) => fold(iota(i), 1, (a, j) => a * 10));
    const xs = concat(concat([0, 1, -1, 9, 10, -10, 99, 100, -100, -9223372036854775807 - 1, 9223372036854775807],
        map(p10, (p) => p - 1)), concat(map(p10, (p) => p), map(p10, (p) => 1 - p)));
    print(join(map(xs, toString), \" \"));
    print(join(map(xs, (x) => toString(u64(x))), \" \"));
    print(join(map(xs, (x) => toString(i32(x % 2147483647))), \" \"));
    print(join(map(xs, (x) => toString(u32(x & 4294967295))), \" \"));
    print(join(map(xs, (x) => toString(i16(x % 32767))), \" \"));
    print(join(map(xs, (x) => toString(u8(x & 255))), \" \"));
    print(join(map([18446744073709551615u64, 10000000000000000000u64, 9999999999999999999u64], toString), \" \"));
    return 0;
}
";
    let p10: Vec<i64> = (0..19).map(|i| 10i64.pow(i)).collect();
    let mut xs: Vec<i64> = vec![0, 1, -1, 9, 10, -10, 99, 100, -100, i64::MIN, i64::MAX];
    xs.extend(p10.iter().map(|p| p - 1));
    xs.extend(p10.iter().copied());
    xs.extend(p10.iter().map(|p| 1 - p));
    let line = |f: &dyn Fn(i64) -> String| xs.iter().map(|&x| f(x)).collect::<Vec<_>>().join(" ");
    let want = [
        line(&|x| x.to_string()),
        line(&|x| (x as u64).to_string()),
        line(&|x| ((x % 2147483647) as i32).to_string()),
        line(&|x| ((x & 4294967295) as u32).to_string()),
        line(&|x| ((x % 32767) as i16).to_string()),
        line(&|x| ((x & 255) as u8).to_string()),
        "18446744073709551615 10000000000000000000 9999999999999999999".to_string(),
    ]
    .join("\n")
        + "\n";
    agree(src, "integer_text_isocron", &[], &want);
}

/// The text of a `f64` as the runtime wrote it: `NaN` without sign, `inf`,
/// `-inf`, else Rust's `Display` (no exponent).
fn display64(v: f64) -> String {
    if v.is_nan() {
        "NaN".to_string()
    } else if v.is_infinite() {
        (if v > 0.0 { "inf" } else { "-inf" }).to_string()
    } else {
        format!("{}", v)
    }
}

/// The same for a `f32`.
fn display32(v: f32) -> String {
    if v.is_nan() {
        "NaN".to_string()
    } else if v.is_infinite() {
        (if v > 0.0 { "inf" } else { "-inf" }).to_string()
    } else {
        format!("{}", v)
    }
}

/// Every `toString` of a float of `e` made a call of the function of the
/// prelude that the code generator calls for it; the number replaced.
fn float_texts_by_prelude(e: &mut isc_ir::Expr, f64_text: u32, f32_text: u32) -> usize {
    let mut n: usize = e.children_mut().into_iter().map(|c| float_texts_by_prelude(c, f64_text, f32_text)).sum();
    let f = match &e.kind {
        isc_ir::ExprKind::StrOp(isc_ir::StrOp::ToString, xs) if xs[0].ty == isc_ir::Type::F64 => f64_text,
        isc_ir::ExprKind::StrOp(isc_ir::StrOp::ToString, xs) if xs[0].ty == isc_ir::Type::F32 => f32_text,
        _ => return n,
    };
    let isc_ir::ExprKind::StrOp(_, xs) = std::mem::replace(&mut e.kind, isc_ir::ExprKind::Zero) else { unreachable!() };
    e.kind = isc_ir::ExprKind::Call(f, xs);
    n += 1;
    n
}

/// The text of a float written in Isocron (`$rtF64Text`, `$rtF32Text` of
/// the prelude, called by the code generator in place of the runtime's
/// `isc_to_string_f64` and `isc_to_string_f32`, removed): the texts of
/// Rust's `Display`, `NaN`, `inf` and `-inf`.
/// - The edges, as texts: zeros, what is not a number (NaN of several
///   signs and payloads), every power of two and the ends of the mantissas
///   of every exponent, the subnormal floats of few bits, the powers of ten
///   and decimals of 1 to 17 digits at every place of the point (where the
///   layout changes: the point at the ends of the digits, 16 and 17 zeros
///   after `0.`, 33 and 34 digits before the point), integers and halves
///   around 2^53; the same for `f32`. Interpreted (the runtime core's text),
///   compiled, in WebAssembly, and interpreted with each `toString` made a
///   call of the function of the prelude (its primitives `$floatBits`,
///   `$mulHigh` and `$decimalText` as the interpreter has them); all of
///   them with the checked allocator and a part under Valgrind (the words
///   written past the end of a text stay inside its block).
/// - By sums of hashes of 4096 texts, compiled: the 100000 least subnormal
///   doubles, two million doubles of random bits (those of the test of the
///   runtime core, `shortest_as_display`), a million of random mantissas
///   between 2^-70 and 2^70, the 100000 least subnormal `f32` and two
///   million `f32` of random bits.
#[test]
fn float_text_written_in_isocron() {
    let texts = r#"action main(args: string[k]): i32 {
    print(join(map(readF64(args[0]), toString), "\n"));
    print(join(map(readF32(args[1]), toString), "\n"));
    return 0;
}
"#;
    let mut d: Vec<f64> = vec![
        0.0,
        -0.0,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NAN,
        -f64::NAN,
        f64::from_bits(0x7ff0_0000_0000_0001),
        f64::from_bits(0xfff8_0000_0000_0123),
        f64::from_bits(u64::MAX),
        f64::MIN_POSITIVE,
        f64::MAX,
        f64::MIN,
        f64::EPSILON,
        0.1,
        0.2,
        0.3,
        1.0 / 3.0,
        5e-324,
        1e-323,
        1.5e-323,
        2.2250738585072009e-308,
        9007199254740992.0,
        9007199254740993.0,
        123456789012345680.0,
        1e21,
        1e22,
        1e23,
        0.5,
        1.5,
        100.0,
        1234.5678,
    ];
    let mut r = Rng(0x9e37_79b9_7f4a_7c15);
    for be in 0..2047u64 {
        let top = (1u64 << 52) - 1;
        for m in [0, 1, 2, top, top - 1, 1 << 51, (1 << 51) + 1, r.next() & top, r.next() & top, (r.next() & 0xfff) << 40] {
            d.push(f64::from_bits(be << 52 | m));
        }
        d.push(-f64::from_bits(be << 52 | (r.next() & top)));
    }
    for i in 0..52 {
        d.extend([f64::from_bits(1 << i), f64::from_bits((1 << i) + 1), f64::from_bits((3 << i) >> 1)]);
    }
    for e in -330..=310 {
        for digits in ["1", "5", "9.5", "1.2", "9.999999999999999", "1.2345678", "1.2345678901234567", "1.7976931348623157", "2.2250738585072014"] {
            let v: f64 = format!("{}e{}", digits, e).parse().unwrap();
            if v.is_finite() {
                d.extend([v, f64::from_bits(v.to_bits() + 1), f64::from_bits(v.to_bits().wrapping_sub(1) & (u64::MAX >> 1))]);
            }
        }
    }
    for nd in 1..=17usize {
        let digits = &"12345678901234567"[..nd];
        for e in -(nd as i32) - 19..=36 - nd as i32 {
            let v: f64 = format!("{}e{}", digits, e).parse().unwrap();
            d.extend([v, -v]);
        }
    }
    for i in 0..2000u64 {
        d.extend([f64::from_bits(i), (9007199254740992u64 - 1000 + i) as f64, i as f64 * 0.5, i as f64, -(i as f64) * 0.125]);
    }
    let mut s: Vec<f32> = vec![
        0.0,
        -0.0,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NAN,
        -f32::NAN,
        f32::from_bits(0x7f80_0001),
        f32::MIN_POSITIVE,
        f32::MAX,
        f32::MIN,
        f32::EPSILON,
        0.1,
        0.2,
        0.3,
        1.0 / 3.0,
        1e-45,
        3e-45,
        16777216.0,
        16777218.0,
        1e10,
        0.5,
        1.5,
        100.0,
        1234.5678,
    ];
    for be in 0..255u32 {
        let top = (1u32 << 23) - 1;
        for m in [0, 1, 2, top, top - 1, 1 << 22, r.next() as u32 & top, r.next() as u32 & top, (r.next() as u32 & 0xff) << 15] {
            s.push(f32::from_bits(be << 23 | m));
        }
        s.push(-f32::from_bits(be << 23 | (r.next() as u32 & top)));
    }
    for e in -50..=40 {
        for digits in ["1", "5", "9.5", "1.2", "9.999999", "1.2345678", "3.4028235", "1.1754944"] {
            let v: f32 = format!("{}e{}", digits, e).parse().unwrap();
            if v.is_finite() {
                s.extend([v, f32::from_bits(v.to_bits() + 1), f32::from_bits(v.to_bits().wrapping_sub(1) & (u32::MAX >> 1)), -v]);
            }
        }
    }
    for i in 0..2000u32 {
        s.extend([f32::from_bits(i), (16777216 - 1000 + i) as f32, i as f32 * 0.5, -(i as f32)]);
    }
    let file64 = |name: &str, xs: &[f64]| -> String {
        let p = tmp(name);
        std::fs::write(&p, xs.iter().flat_map(|x| x.to_bits().to_le_bytes()).collect::<Vec<u8>>()).unwrap();
        p.to_string_lossy().to_string()
    };
    let file32 = |name: &str, xs: &[f32]| -> String {
        let p = tmp(name);
        std::fs::write(&p, xs.iter().flat_map(|x| x.to_bits().to_le_bytes()).collect::<Vec<u8>>()).unwrap();
        p.to_string_lossy().to_string()
    };
    let want = |d: &[f64], s: &[f32]| -> String {
        let a: Vec<String> = d.iter().map(|&v| display64(v)).collect();
        let b: Vec<String> = s.iter().map(|&v| display32(v)).collect();
        format!("{}\n{}\n", a.join("\n"), b.join("\n"))
    };
    let (f64s, f32s) = (file64("float_text_edges.f64", &d), file32("float_text_edges.f32", &s));
    let args = [f64s.as_str(), f32s.as_str()];
    let expected = want(&d, &s);
    assert!(interp_ok(texts, &args) == expected, "interpreted: the texts of {} floats differ from `Display`", d.len() + s.len());
    let bin = compile(texts, "float_text_isocron");
    let run = run_bin(&bin, &args);
    assert_eq!(run.code, 0, "{}", run.stderr);
    for (i, (got, exp)) in run.stdout.lines().zip(expected.lines()).enumerate() {
        assert_eq!(got, exp, "compiled, the text of the float {} (after {} doubles)", i, d.len());
    }
    assert!(run.stdout == expected, "compiled: the texts differ from `Display`");
    // every layout with the allocator checked: nothing written past a block
    checked_clean_bin(&bin, &args, None);
    match wasi_runner() {
        Some(runner) => {
            let wasm = compile_wasm(texts, "float_text_isocron");
            let run = run_wasm(&runner, &wasm, &args);
            assert_eq!(run.code, 0, "{}", run.stderr);
            assert!(run.stdout == expected, "WebAssembly: the texts differ from `Display`");
        }
        None => eprintln!("wasmtime not installed: the texts of floats in WebAssembly skipped"),
    }
    // the functions of the prelude interpreted: each `toString` their call
    let mut p = iscc::frontend("test.isc", texts).unwrap_or_else(|e| panic!("{}", e));
    let (f64_text, f32_text) = p.float_text.expect("the functions of the prelude lowered");
    assert_eq!(float_texts_by_prelude(&mut p.body, f64_text, f32_text), 2);
    let mut out = Vec::new();
    let code = isc_interp::run(&p, &args.map(String::from), &mut out).unwrap_or_else(|e| panic!("the prelude interpreted: {}", e));
    assert_eq!(code, 0);
    for (i, (got, exp)) in String::from_utf8(out).unwrap().lines().zip(expected.lines()).enumerate() {
        assert_eq!(got, exp, "the prelude interpreted, the text of the float {} (after {} doubles)", i, d.len());
    }
    // a part with the allocator checked and under Valgrind
    let part64: Vec<f64> = d.iter().copied().step_by(13).collect();
    let part32: Vec<f32> = s.iter().copied().step_by(13).collect();
    let (p64, p32) = (file64("float_text_part.f64", &part64), file32("float_text_part.f32", &part32));
    valgrind_clean(texts, "float_text_isocron", &[&p64, &p32]);
    // the forms of `toString`
    let forms = "action main(args: string[k]): i32 {
    const third = 1.0 / f64(args.length + 3);
    print(`${1.5}|${-0.0}|${0.1f32}|${1e21}|${third}|${f32(third)}`);
    print(String(2.5) + \" \" + (0.25).toString() + \" \" + toString(1e-7) + \" \" + String(-1e100 * 1e300));
    return 0;
}
";
    agree(forms, "float_text_forms", &[], "1.5|-0|0.1|1000000000000000000000|0.3333333333333333|0.33333334\n2.5 0.25 0.0000001 -inf\n");
    // the sums of the hashes of 4096 texts
    let sums = r#"function hash(s: string): u64 {
    return fold(bytes(s), 0xcbf29ce484222325u64, (h, b) => (h ^ u64(b)) * 0x100000001b3u64);
}

action main(args: string[k]): i32 {
    const xs = readF64(args[0]);
    const hs = map(xs, (x) => hash(toString(x)));
    print(join(map(iota(xs.length / 4096), (c) => toString(sum(map(iota(4096), (j) => hs[c * 4096 + j])))), "\n"));
    const ys = readF32(args[1]);
    const gs = map(ys, (y) => hash(toString(y)));
    print(join(map(iota(ys.length / 4096), (c) => toString(sum(map(iota(4096), (j) => gs[c * 4096 + j])))), "\n"));
    return 0;
}
"#;
    let hash = |t: &str| t.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3));
    // a twentieth of them under Valgrind (`ISC_TEST_MEMCHECK`, D578)
    let scale = if memcheck() { 20 } else { 1 };
    let mut many64: Vec<f64> = (0..100_000u64 / scale).map(f64::from_bits).collect();
    let mut x: u64 = 0x9e37_79b9_7f4a_7c15;
    for _ in 0..2_000_000 / scale {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        many64.push(f64::from_bits(x));
    }
    for _ in 0..1_000_000 / scale {
        let m = r.next();
        many64.push(f64::from_bits((953 + m % 141) << 52 | (m >> 12) | (m & 1) << 63));
    }
    many64.truncate(many64.len() / 4096 * 4096);
    let mut many32: Vec<f32> = (0..100_000u32 / scale as u32).map(f32::from_bits).collect();
    for _ in 0..2_000_000 / scale {
        many32.push(f32::from_bits(r.next() as u32));
    }
    many32.truncate(many32.len() / 4096 * 4096);
    let (m64, m32) = (file64("float_text_many.f64", &many64), file32("float_text_many.f32", &many32));
    let bin = compile(sums, "float_text_sums");
    let run = run_bin_threads(&bin, &[&m64, &m32], Some(4));
    assert_eq!(run.code, 0, "{}", run.stderr);
    let mut got = run.stdout.lines();
    for (c, chunk) in many64.chunks(4096).enumerate() {
        let sum = chunk.iter().fold(0u64, |a, &v| a.wrapping_add(hash(&display64(v))));
        assert_eq!(got.next(), Some(sum.to_string().as_str()), "the texts of the doubles {} to {}", c * 4096, c * 4096 + 4095);
    }
    for (c, chunk) in many32.chunks(4096).enumerate() {
        let sum = chunk.iter().fold(0u64, |a, &v| a.wrapping_add(hash(&display32(v))));
        assert_eq!(got.next(), Some(sum.to_string().as_str()), "the texts of the `f32` {} to {}", c * 4096, c * 4096 + 4095);
    }
    assert_eq!(got.next(), None);
}

/// D590: `trim`, `trimStart` and `trimEnd` written in Isocron (the
/// prelude), the white space of Rust's `char::is_whitespace` at the ends
/// (every character of it, and characters of several bytes that are not
/// white space: U+200B, U+FEFF, U+180E, U+00A1), and `slice` made by the
/// code generator: the same texts as Rust, interpreted and compiled; the
/// panics of a slice out of the text and inside a character the same.
#[test]
fn trim_and_slice_written_in_isocron() {
    let spaces: Vec<char> = [9u32, 10, 11, 12, 13, 32, 0x85, 0xA0, 0x1680, 0x2000, 0x2005, 0x200A, 0x2028, 0x2029, 0x202F, 0x205F, 0x3000]
        .iter()
        .map(|&c| char::from_u32(c).unwrap())
        .collect();
    let others: Vec<char> = [0x200Bu32, 0xFEFF, 0x180E, 0xA1, 0x84, 0x2027, 0x3001].iter().map(|&c| char::from_u32(c).unwrap()).collect();
    let mut texts: Vec<String> = vec![String::new(), " ".into(), "a".into(), " a b ".into()];
    for (i, &w) in spaces.iter().enumerate() {
        let o = others[i % others.len()];
        texts.push(format!("{w}{w}x{o}y{w}"));
        texts.push(format!("{o}{w}é{w}{o}"));
        texts.push(format!("{w}{w}"));
    }
    let src = "action main(args: string[k]): i32 {
    forEach(args, (s) => {
        print(`<${trim(s)}|${s.trimStart()}|${trimEnd(s)}>`);
    });
    return 0;
}
";
    let args: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();
    let want: String = texts.iter().map(|s| format!("<{}|{}|{}>\n", s.trim(), s.trim_start(), s.trim_end())).collect();
    agree(src, "trim_isocron", &args, &want);
    let one = "action main(args: string[k]): i32 {\n    print(args[0].slice(parseI64(args[1]), parseI64(args[2])));\n    return 0;\n}\n";
    let bin = compile(one, "slice_made_here");
    for (s, a, b, want) in [("aé€b", 0, 1, "a\n"), ("aé€b", 1, 3, "é\n"), ("aé€b", 3, 7, "€b\n"), ("aé€b", 7, 7, "\n"), ("", 0, 0, "\n")] {
        let r = run_bin(&bin, &[s, &a.to_string(), &b.to_string()]);
        assert_eq!((r.code, r.stdout.as_str()), (0, want), "{} {} {}: {}", s, a, b, r.stderr);
    }
    for (s, a, b, msg) in [
        ("aé€b", 2, 3, "slice: the index 2 is inside a character of UTF-8"),
        ("aé€b", 1, 4, "slice: the index 4 is inside a character of UTF-8"),
        ("abc", -1, 2, "slice out of bounds: [-1..2] of a string of 3 bytes"),
        ("abc", 2, 1, "slice out of bounds: [2..1] of a string of 3 bytes"),
        ("abc", 0, 4, "slice out of bounds: [0..4] of a string of 3 bytes"),
    ] {
        let args = [s, &a.to_string(), &b.to_string()];
        let m = interp_panic(one, &args);
        assert!(m.contains(msg), "{:?} interpreted: {}", args, m);
        let r = run_bin(&bin, &args);
        assert_eq!(r.code, 101, "{:?}: {}", args, r.stderr);
        assert!(r.stderr.contains(msg), "{:?} compiled: {}", args, r.stderr);
    }
}

/// D591: the answers of `str::parse::<i64>` for the reading of integers of
/// the prelude (`tryParseI64`, the reading of `parseI64`, D582, D586), on
/// the texts of the test of the runtime's reader (D539), removed with it:
/// every text of up to 4 characters of an alphabet of digits, signs, bytes
/// next to the digits and characters of UTF-8; numbers of every length
/// around the powers of ten and the bounds; digits with one byte changed.
/// Interpreted and compiled on a part, compiled on all of them.
#[test]
fn parse_i64_of_the_prelude_answers_as_str_parse() {
    let src = "action main(args: string[k]): i32 {
    forEach(lines(readText(args[0])), (s) => {
        print(match (tryParseI64(s)) { Ok(v) => toString(v), Err(e) => \"x\" });
    });
    return 0;
}
";
    let texts = |random: usize| -> Vec<String> {
        let alpha = ["0", "1", "5", "9", "/", ":", "+", "-", " ", "a", "é", "\u{10348}"];
        let mut all: Vec<String> = Vec::new();
        let mut level = vec![String::new()];
        for _ in 0..4 {
            let mut next = Vec::new();
            for t in &level {
                for c in alpha {
                    next.push(format!("{}{}", t, c));
                }
            }
            all.extend(next.iter().cloned());
            level = next;
        }
        // the empty text, a line between the others
        all.insert(7, String::new());
        for k in 0..=19u32 {
            let p = 10i128.pow(k);
            for x in [p - 1, p, p + 1, -p + 1, -p, -p - 1] {
                all.extend([format!("{}", x), format!("+{}", x), format!("0{}", x), format!("-00{}", x)]);
            }
        }
        for x in [i64::MAX as i128, i64::MAX as i128 + 1, i64::MIN as i128, i64::MIN as i128 - 1, u64::MAX as i128] {
            all.push(format!("{}", x));
        }
        for n in 0..=24 {
            all.extend(["0".repeat(n), format!("-{}", "9".repeat(n)), format!("{}1", "0".repeat(n))]);
        }
        let mut r: u64 = 1;
        for _ in 0..random {
            r = r.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let n = 1 + (r >> 59) as usize % 20;
            let mut b: Vec<u8> = (0..n).map(|i| b'0' + ((r >> (i * 3 % 60)) % 10) as u8).collect();
            if r % 3 == 0 {
                let i = (r >> 7) as usize % n;
                b[i] = [b'/', b':', b' ', b'a', b'+', b'-', 0x7f, 0][(r >> 13) as usize % 8];
            }
            let t = String::from_utf8(b).unwrap();
            all.push(format!("-{}", t));
            all.push(t);
        }
        // an empty text cannot be the last line (`lines` gives none for it)
        all.push("7".to_string());
        all
    };
    let want = |ts: &[String]| -> String { ts.iter().map(|t| t.parse::<i64>().map(|v| format!("{}\n", v)).unwrap_or_else(|_| "x\n".to_string())).collect() };
    let part = texts(2_000);
    let file = tmp("parse_i64_texts_part.txt");
    std::fs::write(&file, part.join("\n")).unwrap();
    agree(src, "parse_i64_answers", &[file.to_str().unwrap()], &want(&part));
    let all = texts(200_000);
    let file = tmp("parse_i64_texts_all.txt");
    std::fs::write(&file, all.join("\n")).unwrap();
    let bin = compile(src, "parse_i64_answers_all");
    let r = run_bin(&bin, &[file.to_str().unwrap()]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(r.stdout == want(&all), "the answers of {} texts differ from `str::parse::<i64>`", all.len());
}

/// D591: `repeat` and the text of a boolean made by the generated code (a
/// block of the final size, the bytes written copied after themselves; a
/// word stored into a block of 8 bytes): every count from 0 to 70 of texts
/// of 0 to 4 bytes a character, the text of a `join` of as many; a
/// negative count the panic of the runtime; interpreted and compiled.
#[test]
fn repeat_and_boolean_text_made_by_the_generated_code() {
    let src = "action main(args: string[k]): i32 {
    print(map(args, (s) => map(iota(71), (i) => s.repeat(i)) == map(iota(71), (i) => join(fill(i, s), \"\"))));
    print(map(args, (s) => s.repeat(3)));
    print(map([true, false, 1 < 2, 2 < 1], toString));
    print(`${true}|${false}|${args.length > 2}`);
    return 0;
}
";
    let texts = ["", "a", "ab", "é€😀", "0123456789"];
    let thrice: Vec<String> = texts.iter().map(|t| t.repeat(3)).collect();
    let want = format!("[true, true, true, true, true]\n[{}]\n[true, false, true, false]\ntrue|false|true\n", thrice.join(", "));
    agree(src, "repeat_bool_text", &texts, &want);
    let bad = "action main(args: string[k]): i32 {\n    print(args[0].repeat(parseI64(args[1])).length);\n    return 0;\n}\n";
    let m = interp_panic(bad, &["ab", "-3"]);
    assert!(m.contains("repeat: the count is negative: -3"), "{}", m);
    let bin = compile(bad, "repeat_negative");
    let r = run_bin(&bin, &["ab", "-3"]);
    assert_eq!(r.code, 101, "{}", r.stderr);
    assert!(r.stderr.contains("panic at line 2, column 25: repeat: the count is negative: -3"), "{}", r.stderr);
    let r = run_bin(&bin, &["ab", "1000000"]);
    assert_eq!((r.code, r.stdout.as_str()), (0, "2000000\n"), "{}", r.stderr);
}

/// D592: the copy of a text, the concatenation of texts that are not
/// literals, the equality, the order, `startsWith` and `endsWith` of two
/// texts are made by the generated code (a block of the allocator and
/// `memcpy`; `memcmp` of the bytes both have, then the lengths), no longer
/// by functions of the runtime. Every pair of texts of 0 to 300 bytes,
/// some equal, some a start or an end of another, some of several bytes a
/// character: the text of the interpreter, each block freed once; the same
/// in WebAssembly, where `memcmp` takes its count in 32 bits. The buffer on
/// the stack of a text only read (D404) is aligned on its size: never
/// across two pages.
#[test]
fn text_blocks_made_by_the_generated_code() {
    let src = include_str!("data/strings_blocks.isc");
    let long: Vec<String> = [7, 8, 9, 15, 16, 17, 31, 32, 33, 63, 64, 65, 127, 128, 129, 300].iter().map(|k| "q".repeat(*k)).collect();
    let mut sets: Vec<Vec<&str>> = vec![
        vec!["ab", "", "abc", "b", "é", "ab€", "abcabc", "é"],
        vec!["x"],
        vec!["aa", "a", "aaa", "", "aa", "aaaaaaaa", "aaaaaaaab", "aaaaaaaa"],
    ];
    sets.push(long.iter().map(|s| s.as_str()).collect());
    let bin = compile(src, "strings_blocks");
    for args in &sets {
        let want = interp_ok(src, args);
        let r = run_bin(&bin, args);
        assert_eq!((r.code, r.stdout.as_str()), (0, want.as_str()), "{:?}: {}", &args[..1], r.stderr);
        checked_clean_bin(&bin, args, None);
    }
    let ll = std::fs::read_to_string(bin.with_extension("ll")).unwrap();
    for f in ["@isc_str_new(", "@isc_str_concat(", "@isc_str_concat_n(", "@isc_str_cmp("] {
        assert!(!ll.contains(f), "{} still called", f);
    }
    assert!(ll.contains("alloca [128 x i8], align 128"), "the buffer of a text on the stack not aligned on its size");
    valgrind_clean(src, "strings_blocks", &sets[0]);
    let Some(runner) = wasi_runner() else {
        eprintln!("no WASI engine (wasmtime): the WebAssembly program not run");
        return;
    };
    let wasm = compile_wasm(src, "strings_blocks_wasm");
    for args in &sets {
        let want = interp_ok(src, args);
        let r = run_wasm(&runner, &wasm, args);
        assert_eq!((r.code, r.stdout.as_str()), (0, want.as_str()), "WebAssembly, {:?}: {}", &args[..1], r.stderr);
    }
}

/// D595: `findIndex` among bytes tests an array of 16 to 63 bytes, and the
/// end of a longer one, by vectors of 16 (the last one over the end of the
/// one before) where it tested one byte at a time; and
/// `findIndex(iota(n), (j) => b[j] == x && b[j + 2] == y)` reads `b` at the
/// index of its loop (the element `i` of `iota(n)` is `i`), so by vectors
/// too. Arrays of every length from 0 to 200, the byte at every place:
/// no wrong answer, compiled for each set of instructions and
/// interpreted.
#[test]
fn find_index_among_bytes_by_narrow_vectors() {
    let src = include_str!("data/find_index_bytes.isc");
    assert_eq!(interp_ok(src, &[]), "0\n", "interpreted");
    for (isa, name) in [
        (isc_codegen::Isa::Native, "native"),
        (isc_codegen::Isa::Sse, "sse"),
        (isc_codegen::Isa::Avx2, "avx2"),
    ] {
        let opts = isc_codegen::Options { isa, ..isc_codegen::Options::default() };
        let bin = compile_with(src, &format!("find_index_bytes_{}", name), opts);
        let r = run_bin(&bin, &[]);
        assert_eq!((r.code, r.stdout.as_str()), (0, "0\n"), "{}: {}", name, r.stderr);
        let ll = std::fs::read_to_string(bin.with_extension("ll")).unwrap();
        assert!(ll.contains("icmp eq <16 x i8>"), "{}: no vector of 16 bytes in a search", name);
        checked_clean_bin(&bin, &[], None);
    }
}

/// D596: `s.indexOf(t)` and `s.includes(t)` are functions of the prelude,
/// written in Isocron: a byte by `findIndex`; more, the places where two
/// bytes of the needle are (by vectors), then its bytes compared; past a
/// budget of bytes compared in vain, the two-way search, of linear time.
/// Against a search one place at a time: texts over alphabets of 1 to 4
/// letters, needles cut from them or not, long repeated texts that spend
/// the budget; no wrong answer, compiled, interpreted and in WebAssembly,
/// each block freed once; the values of the documentation; no call of the
/// runtime left.
#[test]
fn index_of_and_includes_written_in_isocron() {
    let src = include_str!("data/strings_search.isc");
    assert_eq!(interp_ok(src, &["120"]), "[0, 0]\n", "interpreted");
    let bin = compile(src, "strings_search");
    let r = run_bin(&bin, &["4000"]);
    assert_eq!((r.code, r.stdout.as_str()), (0, "[0, 0]\n"), "{}", r.stderr);
    checked_clean_bin(&bin, &["400"], None);
    let ll = std::fs::read_to_string(bin.with_extension("ll")).unwrap();
    for f in ["@isc_index_of(", "@isc_str_test("] {
        assert!(!ll.contains(f), "{} still called", f);
    }
    let values = r#"action main(args: string[k]): i32 {
    const s = "Hello, World";
    const t = "héllo wörld";
    print([s.indexOf("World"), s.indexOf("x"), s.indexOf(""), s.indexOf("Hello, World!"), s.indexOf("o"), s.indexOf("o, W")] as const);
    print([t.indexOf("llo"), t.indexOf("ö"), t.indexOf("é"), "".indexOf(""), "".indexOf("a"), args[0].indexOf(args[1])] as const);
    print([s.includes("lo, Wo"), s.includes("world"), s.includes(""), t.includes("rld"), "".includes(""), args[0].includes(args[1])] as const);
    return 0;
}
"#;
    let want = "(7, -1, 0, -1, 4, 4)\n(3, 8, 1, 0, -1, 7)\n(true, false, true, true, true, true)\n";
    let args = ["a text of twenty-nine bytes..", "of twenty-nine bytes"];
    assert_eq!(interp_ok(values, &args), want, "interpreted");
    let vbin = compile(values, "strings_search_values");
    let r = run_bin(&vbin, &args);
    assert_eq!((r.code, r.stdout.as_str()), (0, want), "{}", r.stderr);
    let Some(runner) = wasi_runner() else {
        eprintln!("no WASI engine (wasmtime): the WebAssembly program not run");
        return;
    };
    let wasm = compile_wasm(src, "strings_search_wasm");
    let r = run_wasm(&runner, &wasm, &["400"]);
    assert_eq!((r.code, r.stdout.as_str()), (0, "[0, 0]\n"), "WebAssembly: {}", r.stderr);
}

/// D606: `padStart`, `padEnd` and the padded text of an integer are made by
/// the generated code (one block of the size of the padded text, the text
/// copied, the fill stored or copied after itself; the digits of an integer
/// from the words of the prelude's `$rtDigitWords`), no longer by functions
/// of the runtime. Texts of 0 to 300 bytes and of several bytes a
/// character, fills of 0, 1 and several bytes (the last copy cut at the
/// start of a character), widths below, equal to and above the lengths,
/// both sides of the moves of 4 to 64 bytes and of the buffer of a text on
/// the stack, the least `i64`; integers of every width and sign; a megabyte
/// of fill: the text of the interpreter, each block freed once (the checked
/// allocator, Valgrind), on 1 and 4 threads; the same in WebAssembly.
#[test]
fn padded_texts_made_by_the_generated_code() {
    let src = include_str!("data/strings_padded.isc");
    let texts = ["x", "ab", "é", "€uro", "😀", "abc😀déf", "0123456789", ""];
    let bin = compile(src, "strings_padded");
    let ll = std::fs::read_to_string(bin.with_extension("ll")).unwrap();
    for f in ["@isc_pad(", "@isc_pad_into(", "@isc_pad1_into(", "@isc_pad_int("] {
        assert!(!ll.contains(f), "{} still called", f);
    }
    assert!(ll.contains("; text pad\n") && ll.contains("; text pad, on the stack when it fits\n") && ll.contains("; int pad\n"), "the padded texts not made here");
    let sets: Vec<Vec<&str>> = ["kept", "read", "ints", "large"].iter().map(|m| std::iter::once(*m).chain(texts).collect()).collect();
    for args in &sets {
        let want = interp_ok(src, args);
        for t in [1, 4] {
            let r = run_bin_threads(&bin, args, Some(t));
            assert_eq!(r.code, 0, "{}: {}", args[0], r.stderr);
            assert!(r.stdout == want, "{}, {} threads: the text of the interpreter not printed", args[0], t);
        }
        checked_clean_bin(&bin, args, None);
    }
    valgrind_clean(src, "strings_padded", &sets[1]);
    let Some(runner) = wasi_runner() else {
        eprintln!("no WASI engine (wasmtime): the WebAssembly program not run");
        return;
    };
    let wasm = compile_wasm(src, "strings_padded_wasm");
    for args in &sets {
        let want = interp_ok(src, args);
        let r = run_wasm(&runner, &wasm, args);
        assert_eq!(r.code, 0, "WebAssembly, {}: {}", args[0], r.stderr);
        assert!(r.stdout == want, "WebAssembly, {}: the text of the interpreter not printed", args[0]);
    }
}

/// D607: `join` is made by the generated code (the lengths added, one block,
/// each text copied after its separator; 1 MiB or more by ranges of its
/// texts on the pool), and the join read once by a `map` of its bytes
/// (D428) has its ranges and its bytes from functions emitted with the
/// program, no longer from the runtime. Arrays of 0 to 24 texts of every
/// kind of lengths (empty, both sides of the moves of 4 to 64 bytes, up to
/// 300 bytes) by separators of 0, 1 and several bytes, of several bytes a
/// character, known to the compiler or not; joins of 6000 texts and of one
/// of 3 MB (by ranges, on 1 to 8 threads); a join
/// never made; joins of the views of a text, and inside the tasks of a
/// `map`: the text of the interpreter, each block freed once (the checked
/// allocator, Valgrind); the same in WebAssembly, with and without threads.
#[test]
fn joins_made_by_the_generated_code() {
    let src = include_str!("data/strings_joined.isc");
    let bin = compile(src, "strings_joined");
    let ll = std::fs::read_to_string(bin.with_extension("ll")).unwrap();
    for f in ["declare void @isc_join(", "declare i64 @isc_join_ranges(", "declare i64 @isc_join_next("] {
        assert!(!ll.contains(f), "{} of the runtime still called", f);
    }
    assert!(ll.contains("; text join\n") && ll.contains("; join read by a map, never made\n"), "the joins not made here");
    assert!(ll.contains("define internal void @isc_join_large(") && ll.contains("define internal i64 @isc_join_next("), "the functions of the joins not emitted");
    // (the large joins of 30000 and 90000 texts, the large text first and
    // last: `a_large_join_made_in_parallel`, `stdin.rs`)
    let sets: [&[&str]; 5] = [
        &["small", "24", "-1", ";", "é"],
        &["large", "6000", "17", "+"],
        &["large", "2", "1", "ab"],
        &["map", "3000", "1234", "+"],
        &["views", "700", "-1", "/"],
    ];
    let wants: Vec<String> = sets.iter().map(|args| interp_ok(src, args)).collect();
    for (args, want) in sets.iter().zip(&wants) {
        for t in [1, 2, 3, 4, 8] {
            let r = run_bin_threads(&bin, args, Some(t));
            assert_eq!(r.code, 0, "{:?}: {}", args, r.stderr);
            assert!(r.stdout == *want, "{:?}, {} threads: the text of the interpreter not printed", args, t);
        }
        checked_clean_bin(&bin, args, None);
    }
    valgrind_clean(src, "strings_joined", sets[0]);
    valgrind_clean(src, "strings_joined", sets[3]);
    let Some(runner) = wasi_runner() else {
        eprintln!("no WASI engine (wasmtime): the WebAssembly program not run");
        return;
    };
    let wasm = compile_wasm(src, "strings_joined_wasm");
    let threads = compile_wasm_isa(src, "strings_joined_wasm_threads", isc_codegen::Isa::Wasm32Threads);
    for k in [0, 1, 3, 4] {
        let (args, want) = (sets[k], &wants[k]);
        let r = run_wasm(&runner, &wasm, args);
        assert_eq!(r.code, 0, "WebAssembly, {:?}: {}", args, r.stderr);
        assert!(r.stdout == *want, "WebAssembly, {:?}: the text of the interpreter not printed", args);
        let r = run_wasm_threads(&runner, &threads, args, Some(4));
        assert_eq!(r.code, 0, "WebAssembly with threads, {:?}: {}", args, r.stderr);
        assert!(r.stdout == *want, "WebAssembly with threads, {:?}: the text of the interpreter not printed", args);
    }
}

/// The tables of one direction (`upper` or `lower`) of the prelude, as two
/// functions, from the text of each character in that case (`map`).
fn case_direction(name: &str, map: &dyn Fn(char) -> String, out: &mut String) {
    use std::collections::BTreeMap;
    use std::fmt::Write;
    // the characters of several bytes that have another case
    let mut cased: BTreeMap<u32, String> = BTreeMap::new();
    for c in (0x80..=0x10FFFFu32).filter_map(char::from_u32) {
        let m = map(c);
        if m != c.to_string() {
            cased.insert(c as u32, m);
        }
    }
    let top = *cased.keys().next_back().expect("a character with another case");
    assert!(top < 0x20000, "a character with another case past the 2048 blocks: U+{:X}", top);
    // the texts: the longest first, so that a shorter one is found in them
    let mut texts = String::from(" ");
    let mut by_len: Vec<&String> = cased.values().collect();
    by_len.sort_by(|a, b| b.len().cmp(&a.len()).then(a.cmp(b)));
    by_len.dedup();
    let mut place: BTreeMap<&str, usize> = BTreeMap::new();
    for t in by_len {
        let at = match texts.find(t.as_str()) {
            Some(at) => at,
            None => {
                texts.push_str(t);
                texts.len() - t.len()
            }
        };
        assert!(t.len() <= 8 && at < 8192, "a text of {} bytes at {}: no entry of 16 bits for it", t.len(), at);
        place.insert(t.as_str(), at * 8 + t.len() - 1);
    }
    let mut page_of: BTreeMap<u32, usize> = BTreeMap::new();
    for c in cased.keys() {
        let n = page_of.len() + 1;
        page_of.entry(c >> 6).or_insert(n);
    }
    assert!(page_of.len() < 256, "{} pages: more than a byte counts", page_of.len());
    let mut pages = [0u8; 2048];
    let mut entries = vec![0u16; (page_of.len() + 1) * 64];
    for (c, t) in &cased {
        let page = page_of[&(c >> 6)];
        pages[(c >> 6) as usize] = page as u8;
        entries[page * 64 + (c & 63) as usize] = place[t.as_str()] as u16;
    }
    // the words as their hexadecimal digits, 16 a word, in one literal
    let digits = |words: Vec<u64>| -> String { words.iter().map(|w| format!("{:016x}", w)).collect() };
    let pages: Vec<u64> = pages.chunks(8).map(|b| b.iter().rev().fold(0u64, |w, &x| w << 8 | x as u64)).collect();
    let entries: Vec<u64> = entries.chunks(4).map(|e| e.iter().rev().fold(0u64, |w, &x| w << 16 | x as u64)).collect();
    writeln!(out, "// {} characters of several bytes have a text in {} case, in {} pages.", cased.len(), name, page_of.len()).unwrap();
    writeln!(out, "function ${}Entry(block: i64, low: i64): i64 {{", name).unwrap();
    writeln!(out, "    const pages = $words(\"{}\");", digits(pages)).unwrap();
    writeln!(out, "    const entries = $words(\"{}\");", digits(entries)).unwrap();
    writeln!(out, "    const page = block < 2048 ? i64((pages[(block >> 3) & 255] >> u64((block & 7) * 8)) & 255u64) : 0;").unwrap();
    writeln!(out, "    const at = page * 64 + low;").unwrap();
    writeln!(out, "    return i64((entries[at >> 2] >> u64((at & 3) * 16)) & 0xFFFFu64);").unwrap();
    writeln!(out, "}}\n").unwrap();
    // ASCII letters, digits and the space as they are, the others by value
    let mut lit = String::new();
    for c in texts.chars() {
        if c.is_ascii_alphanumeric() || c == ' ' {
            lit.push(c);
        } else {
            write!(lit, "\\u{{{:x}}}", c as u32).unwrap();
        }
    }
    writeln!(out, "// Their texts: {} bytes.", texts.len()).unwrap();
    writeln!(out, "function ${}Texts(): string {{\n    return \"{}\";\n}}", name, lit).unwrap();
}

/// The tables of `toUpperCase` and `toLowerCase` of the prelude: its text
/// between their two marks, from `char::to_uppercase` and
/// `char::to_lowercase` of the Rust that builds this test.
fn case_tables() -> String {
    use std::fmt::Write;
    let (a, b, c) = char::UNICODE_VERSION;
    let mut out = String::new();
    writeln!(out, "// Generated by the test `case_tables_of_the_prelude` (Unicode {}.{}.{}): do not edit.", a, b, c).unwrap();
    case_direction("upper", &|c| c.to_uppercase().collect(), &mut out);
    writeln!(out).unwrap();
    case_direction("lower", &|c| c.to_lowercase().collect(), &mut out);
    writeln!(out, "// End of the generated tables.").unwrap();
    out
}

/// The tables of `toUpperCase` and `toLowerCase` of the prelude are those
/// of the Rust that builds the compiler, whose mappings the interpreter
/// applies (`char::to_uppercase`, `char::to_lowercase`): a Rust of another
/// version of Unicode fails here until the tables are written again, by
/// this test: with `ISC_CASE_TABLES=<file>` it writes them to the file,
/// whose text goes between the two marks of the prelude.
#[test]
fn case_tables_of_the_prelude() {
    let tables = case_tables();
    if let Some(path) = std::env::var_os("ISC_CASE_TABLES") {
        std::fs::write(&path, &tables).unwrap();
    }
    assert!(
        isc_types::PRELUDE.contains(&tables),
        "the tables of `toUpperCase` and `toLowerCase` of the prelude are not those of this Rust (Unicode {:?}): run this test with ISC_CASE_TABLES=<file>, then put the text of the file between the two marks of crates/isc-types/src/prelude.isc",
        char::UNICODE_VERSION
    );
}

/// `s` in upper or lower case as Rust writes it, each character on its own
/// (`char::to_uppercase`, `char::to_lowercase`): the answer of the
/// interpreter, and of the runtime before.
fn rust_case(s: &str, upper: bool) -> String {
    if upper {
        s.chars().flat_map(char::to_uppercase).collect()
    } else {
        s.chars().flat_map(char::to_lowercase).collect()
    }
}

/// The lines of the test of the case (none holds a `\n` or a `\r`, the last
/// one is not empty):
/// - each scalar value alone (`full`), or a part of them: those below
///   U+3000, one in 61 of the others, and every one that has another case;
/// - ASCII of every length from 0 to 300, every byte of it;
/// - each character that has another case in 17 places: between letters,
///   twice, beside another one, after 3 to 129 bytes of ASCII (the ends of
///   the vectors of 4, 8, 16 and 32 bytes, of the buffer of 128 bytes of
///   the stack, of the words of 8 bytes that the prelude passes at once);
/// - `mixes` lines at random, of 0 to 300 characters, none, a sixteenth, a
///   quarter or all of them out of ASCII: with another case, with one of
///   another length or of several characters, without a case, of 2, 3 and 4
///   bytes, marks that combine;
/// - long lines (20000 to 100000 characters): of ASCII, of letters of
///   another case, and many characters whose other case has another
///   length, the one after the other.
/// With `letters` false, no line has a letter of ASCII, and the long lines
/// are 20 times shorter.
fn case_lines(full: bool, mixes: usize, letters: bool) -> Vec<String> {
    let differs = |c: char| rust_case(&c.to_string(), true) != c.to_string() || rust_case(&c.to_string(), false) != c.to_string();
    let cased: Vec<char> = (0x80..=0x10FFFFu32).filter_map(char::from_u32).filter(|&c| differs(c)).collect();
    let mut ls: Vec<String> = Vec::new();
    for c in (0..=0x10FFFFu32).filter_map(char::from_u32) {
        let v = c as u32;
        if c != '\n' && c != '\r' && (letters || !c.is_ascii_alphabetic()) && (full || v < 0x3000 || v % 61 == 0 || cased.binary_search(&c).is_ok()) {
            ls.push(c.to_string());
        }
    }
    let ascii: Vec<u8> = (0..128u8).filter(|&b| b != b'\n' && b != b'\r' && (letters || !b.is_ascii_alphabetic())).collect();
    for len in 0..=300usize {
        ls.push((0..len).map(|j| ascii[(len * 7 + j * 13 + j * j) % ascii.len()] as char).collect());
    }
    let around: &[u8] = if letters { b"aZbYcXdW eV,fU1gT" } else { b"0 1,2;3-4_5 6.7:8" };
    let run = |n: usize, seed: usize| -> String { (0..n).map(|j| around[(seed + j * 5) % around.len()] as char).collect() };
    for (k, &c) in cased.iter().enumerate() {
        let other = cased[(k * 7919 + 1) % cased.len()];
        ls.push(format!("{}{}{}", run(1, k), c, run(1, k + 1)));
        ls.push(format!("{}{}", c, c));
        ls.push(format!("{}{}{} {}", c, other, c, other));
        ls.push(format!("{}{}{}", run(5, k), c, run(40, k)));
        for before in [3usize, 7, 14, 15, 16, 30, 31, 32, 62, 125, 126, 127, 129] {
            ls.push(format!("{}{}", run(before, k), c));
        }
    }
    let special: Vec<char> = "\u{df}\u{1e9e}\u{149}\u{1f0}\u{390}\u{3b0}\u{587}\u{1e96}\u{fb01}\u{fb03}\u{fb05}\u{1f80}\u{1f88}\u{1fb3}\u{1ffc}\u{130}\u{131}\u{17f}\u{212a}\u{3c3}\u{3c2}\u{3a3}\u{23a}\u{2c65}\u{250}\u{2c6f}\u{1c5}\u{1c6}\u{1c4}\u{65e5}\u{672c}\u{1f600}\u{1f680}\u{301}\u{308}\u{345}\u{a0}\u{2028}\u{feff}\u{10ffff}\u{80}\u{7ff}\u{800}\u{ffff}\u{10000}"
        .chars()
        .collect();
    let mut r = Rng(0x5eed_ca5e);
    for i in 0..mixes {
        let len = match r.next() % 8 {
            0 => (r.next() % 300) as usize,
            1 => (r.next() % 4) as usize,
            _ => (r.next() % 40) as usize,
        };
        // out of 16 characters, how many are not ASCII
        let rate = [0u64, 1, 4, 16][(r.next() % 4) as usize];
        let line: String = (0..len)
            .map(|_| {
                if r.next() % 16 < rate {
                    match r.next() % 3 {
                        0 => special[(r.next() % special.len() as u64) as usize],
                        _ => cased[(r.next() % cased.len() as u64) as usize],
                    }
                } else {
                    ascii[(r.next() % ascii.len() as u64) as usize] as char
                }
            })
            .collect();
        ls.push(line);
        if i % 64 == 0 {
            ls.push(String::new());
        }
    }
    // (shorter for the prelude interpreted: the interpreter copies a text
    // at each character it writes)
    let long = if letters { 20 } else { 1 };
    ls.push(run(5_000 * long, 3));
    ls.push((0..1_500 * long).map(|j| cased[(j * 31) % cased.len()]).collect());
    ls.push((0..2_500 * long).map(|j| ['\u{131}', 'i', '\u{130}', ' ', '\u{df}', '\u{fb03}', '\u{390}'][(j * j + j / 7) % 7]).collect::<String>().replace('i', if letters { "i" } else { "1" }));
    ls.push((0..1_000 * long).map(|j| if j % 97 == 0 { '\u{149}' } else { around[j % around.len()] as char }).collect());
    ls
}

/// What the program of the test prints for these lines: their upper case,
/// their lower case, then both on a line, between `<` and `>`.
fn case_expected(ls: &[String]) -> String {
    // (written into one text: more than a million lines)
    let mut out = String::new();
    for upper in [true, false] {
        for l in ls {
            out.push_str(&rust_case(l, upper));
            out.push('\n');
        }
    }
    for l in ls {
        out.push('<');
        out.push_str(&rust_case(l, true));
        out.push('|');
        out.push_str(&rust_case(l, false));
        out.push_str(">\n");
    }
    out
}

/// The bytes that a program printed are `want`; else its first line that
/// differs.
fn same_text(got: &[u8], want: &str, what: &str) {
    if got == want.as_bytes() {
        return;
    }
    let got = String::from_utf8_lossy(got);
    for (i, (g, w)) in got.split('\n').zip(want.split('\n')).enumerate() {
        assert!(g == w, "{}: the line {} is {:?}, not {:?}", what, i, g, w);
    }
    panic!("{}: {} lines, not {}", what, got.split('\n').count(), want.split('\n').count());
}

/// The exit code, the standard output and the standard error of a command.
fn raw_output(cmd: &mut std::process::Command) -> (i32, Vec<u8>, String) {
    let o = cmd.output().unwrap();
    (o.status.code().unwrap_or(-1), o.stdout, String::from_utf8_lossy(&o.stderr).to_string())
}

/// Every `toUpperCase` and `toLowerCase` of `e` made a call of the function
/// of the prelude that the code generator calls for a text that is not
/// ASCII, the text itself given as its copy (right for a text without a
/// letter of ASCII); their number.
fn case_by_prelude(e: &mut isc_ir::Expr, f: u32) -> usize {
    use isc_ir::{Expr, ExprKind, Lit, StrOp, Type};
    let mut n: usize = e.children_mut().into_iter().map(|c| case_by_prelude(c, f)).sum();
    let upper = match &e.kind {
        ExprKind::StrOp(StrOp::ToUpper, _) => true,
        ExprKind::StrOp(StrOp::ToLower, _) => false,
        _ => return n,
    };
    let ExprKind::StrOp(_, mut xs) = std::mem::replace(&mut e.kind, ExprKind::Zero) else { unreachable!() };
    let x = xs.pop().unwrap();
    let sp = x.span;
    let bytes = Expr::new(ExprKind::StrOp(StrOp::Bytes, vec![x.clone()]), Type::Array(Box::new(Type::Int(isc_ir::IntKind::U8)), isc_ir::Fixed::ANY), sp);
    e.kind = ExprKind::Call(f, vec![bytes, x, Expr::i64(0, sp), Expr::i64(0, sp), Expr::i64(0, sp), Expr::lit(Lit::Bool(upper), Type::Bool, sp)]);
    n += 1;
    n
}

const CASE_LINES: &str = r#"function both(l: string): string {
    const u = l.toUpperCase();
    const w = l.toLowerCase();
    return "<" + u + "|" + w + ">";
}

action main(args: string[k]): i32 {
    const ls = lines(readText(args[0]));
    print(join(map(ls, (l) => l.toUpperCase()), "\n"));
    print(join(map(ls, (l) => l.toLowerCase()), "\n"));
    print(join(map(ls, both), "\n"));
    return 0;
}
"#;

/// `toUpperCase` and `toLowerCase` made by the generated code (the texts of
/// ASCII, by vectors) and by the prelude (`$caseText`, `$caseSplice` and
/// the tables of Unicode), the runtime's `isc_to_case` and
/// `isc_to_case_into` removed: the texts of Rust's `char::to_uppercase` and
/// `char::to_lowercase`, each character on its own.
/// - The lines of `case_lines`: every scalar value alone, ASCII of every
///   length, each character that has another case at the ends of the
///   vectors and of the buffer of the stack, 20000 mixes at random, long
///   lines; each kept (a block) and only read in its scope (the buffer of
///   the stack up to 128 bytes), compiled for the native instructions.
/// - A part of them (every character that has another case, 2000 mixes):
///   interpreted (the runtime core's mapping), compiled for SSE and AVX2,
///   in WebAssembly, with the allocator checked, a smaller part under
///   Valgrind.
/// - The prelude interpreted on lines without a letter of ASCII: each
///   `toUpperCase` and `toLowerCase` a call of `$caseText` (its primitive
///   `$withText` as the interpreter has it).
/// - No function of the runtime is called for them: neither those removed
///   nor the check of UTF-8 of `stringOf`.
#[test]
fn case_written_in_isocron() {
    let file = |name: &str, ls: &[String]| -> String {
        let p = tmp(name);
        std::fs::write(&p, format!("{}\n", ls.join("\n"))).unwrap();
        p.to_string_lossy().to_string()
    };
    // a small part first: the forms of the calls, interpreted and compiled
    let forms = r#"function shout(s: string): string {
    return s.toUpperCase() + "!";
}

action main(args: string[k]): i32 {
    const s = "Hello, World";
    print(s.toUpperCase() + " " + s.toLowerCase() + " " + "".toUpperCase() + "|" + "".toLowerCase() + "|");
    print("stra\u{df}e \u{c9}t\u{e9} \u{3a3}\u{3c3}\u{3c2} \u{130}\u{131} \u{fb01}\u{149}".toUpperCase());
    print("STRA\u{1e9e}E \u{c9}t\u{e9} \u{3a3}\u{3c3}\u{3c2} \u{130}\u{131} \u{212a}".toLowerCase());
    print(map(args, (a) => a.toUpperCase().toLowerCase() + "/" + shout(a)));
    print(map(args, (a) => a.toLowerCase().length + a.toUpperCase().length));
    print(fold(args, "", (acc, a) => acc + a.toUpperCase()));
    print(`${args[0].toUpperCase()}-${args[1].toLowerCase()}` == "A-b" ? "same" : args[2].toUpperCase());
    return 0;
}
"#;
    let args = ["a", "B", "\u{e9}t\u{e9} \u{df}", "\u{1c5}x", "a text of forty bytes, longer than a vector", "\u{131}\u{130}\u{131}"];
    let want = "HELLO, WORLD hello, world ||\nSTRASSE \u{c9}T\u{c9} \u{3a3}\u{3a3}\u{3a3} \u{130}I FI\u{2bc}N\nstra\u{df}e \u{e9}t\u{e9} \u{3c3}\u{3c3}\u{3c2} i\u{307}\u{131} k\n".to_string()
        + &format!("[{}]\n", args.iter().map(|a| format!("{}/{}!", rust_case(&rust_case(a, true), false), rust_case(a, true))).collect::<Vec<_>>().join(", "))
        + &format!("[{}]\n", args.iter().map(|a| (rust_case(a, false).len() + rust_case(a, true).len()).to_string()).collect::<Vec<_>>().join(", "))
        + &format!("{}\nsame\n", args.iter().map(|a| rust_case(a, true)).collect::<String>());
    agree(forms, "case_forms", &args, &want);

    // every line, compiled
    let part = case_lines(false, 2000, true);
    let part_file = file("case_part.txt", &part);
    let part_want = case_expected(&part);
    let bin = compile(CASE_LINES, "case_lines");
    let (code, out, err) = raw_output(std::process::Command::new(&bin).arg(&part_file));
    assert_eq!(code, 0, "{}", err);
    same_text(&out, &part_want, "compiled, a part");
    if !memcheck() {
        let all = case_lines(true, 20000, true);
        let all_file = file("case_all.txt", &all);
        let all_want = case_expected(&all);
        drop(all);
        let (code, out, err) = raw_output(std::process::Command::new(&bin).arg(&all_file));
        assert_eq!(code, 0, "{}", err);
        same_text(&out, &all_want, "compiled");
        let (code, out, err) = raw_output(std::process::Command::new(&bin).arg(&all_file).env("ISC_THREADS", "1"));
        assert_eq!(code, 0, "{}", err);
        same_text(&out, &all_want, "compiled, one thread");
    }
    // no function of the runtime for them; both functions of the module
    let ll = std::fs::read_to_string(bin.with_extension("ll")).unwrap();
    for f in ["@isc_to_case", "call void @isc_str_adopt_bytes(", "call void @isc_str_from_bytes("] {
        assert!(!ll.contains(f), "{} still called", f);
    }
    for f in ["@isc_text_upper(ptr", "@isc_text_lower(ptr", "@isc_text_upper_into(ptr", "@isc_text_lower_into(ptr"] {
        assert!(ll.contains(&format!("call tailcc {{ ptr, i64 }} {}", f)), "{} not called", f);
    }
    // the part: interpreted, the other sets of instructions, the allocator
    // checked, WebAssembly
    assert!(interp_ok(CASE_LINES, &[part_file.as_str()]) == part_want, "interpreted: the texts differ from those of Rust");
    checked_clean_bin(&bin, &[part_file.as_str()], None);
    for (isa, name) in [(isc_codegen::Isa::Sse, "sse"), (isc_codegen::Isa::Avx2, "avx2")] {
        let opts = isc_codegen::Options { isa, ..isc_codegen::Options::default() };
        let b = compile_with(CASE_LINES, &format!("case_lines_{}", name), opts);
        let (code, out, err) = raw_output(std::process::Command::new(&b).arg(&part_file));
        assert_eq!(code, 0, "{}: {}", name, err);
        same_text(&out, &part_want, name);
    }
    match wasi_runner() {
        Some(runner) => {
            let wasm = compile_wasm(CASE_LINES, "case_lines");
            let (code, out, err) = raw_output(std::process::Command::new(&runner).arg("run").arg("--dir=/::/").arg(&wasm).arg(&part_file));
            assert_eq!(code, 0, "{}", err);
            same_text(&out, &part_want, "WebAssembly");
        }
        None => eprintln!("wasmtime not installed: the texts in another case in WebAssembly skipped"),
    }
    // a smaller part under Valgrind
    let few: Vec<String> = part.iter().step_by(41).cloned().chain(part.iter().rev().take(4).map(|l| l.chars().take(3000).collect())).collect();
    let few_file = file("case_few.txt", &few);
    valgrind_clean(CASE_LINES, "case_lines", &[few_file.as_str()]);
    // the prelude interpreted, on lines without a letter of ASCII
    let plain = case_lines(false, 2000, false);
    let plain_file = file("case_plain.txt", &plain);
    let mut p = iscc::frontend("test.isc", CASE_LINES).unwrap_or_else(|e| panic!("{}", e));
    let f = p.case_text.expect("the function of the prelude lowered");
    let calls = case_by_prelude(&mut p.body, f);
    assert!(calls >= 4, "{} calls of the function of the prelude", calls);
    let mut out = Vec::new();
    let code = isc_interp::run(&p, &[plain_file], &mut out).unwrap_or_else(|e| panic!("the prelude interpreted: {}", e));
    assert_eq!(code, 0);
    same_text(&out, &case_expected(&plain), "the prelude interpreted");
}

/// `$withText(t, at, piece)` of the prelude (here `padStart` made it in the
/// lowered program: the same operands): the bytes of `piece` over those of
/// `t` from `at`; a panic when they do not lie in `t` or when one of their
/// ends is inside a character of `t`. Interpreted and compiled, pieces of 0
/// to 80 bytes at every place of a text (each way of copying them).
#[test]
fn with_text_replaces_whole_characters() {
    fn with_text(e: &mut isc_ir::Expr) -> usize {
        let mut n: usize = e.children_mut().into_iter().map(with_text).sum();
        if let isc_ir::ExprKind::StrOp(op @ isc_ir::StrOp::PadStart, _) = &mut e.kind {
            *op = isc_ir::StrOp::WithText;
            n += 1;
        }
        n
    }
    let src = r#"action main(args: string[k]): i32 {
    const t = args[0];
    const pieces = lines(args[1]);
    print(join(map(iota(pieces.length), (i) => t.padStart(i % (t.length - pieces[i].length + 1), pieces[i])), "\n"));
    print(t.padStart(parseI64(args[2]), args[3]));
    return 0;
}
"#;
    let mut p = iscc::frontend("with_text.isc", src).unwrap_or_else(|e| panic!("{}", e));
    assert_eq!(with_text(&mut p.body) + p.fns.iter_mut().map(|f| with_text(&mut f.body)).sum::<usize>(), 2);
    let out = tmp("with_text");
    let opts = iscc::BuildOptions { output: out.clone(), emit_llvm: None, emit_asm: None, debug: false, codegen: isc_codegen::Options::default(), link: Vec::new(), glibc: false };
    iscc::build(&p, &opts).unwrap_or_else(|e| panic!("{}", e));
    let both = |args: &[&str]| -> (Result<String, String>, Run) {
        let mut o = Vec::new();
        let i = match isc_interp::run(&p, &args.iter().map(|s| s.to_string()).collect::<Vec<_>>(), &mut o) {
            Ok(_) => Ok(String::from_utf8(o).unwrap()),
            Err(e) => Err(e.to_string()),
        };
        (i, run_bin(&out, args))
    };
    // pieces of every length at every place of a text of ASCII
    let t: String = (0..200).map(|j| (b'a' + (j % 26) as u8) as char).collect();
    let pieces: Vec<String> = (0..=80usize).flat_map(|n| (0..3).map(move |k| (0..n).map(|j| (b'0' + ((j + k) % 10) as u8) as char).collect::<String>())).collect();
    let want: Vec<String> = pieces
        .iter()
        .enumerate()
        .map(|(i, piece)| {
            let at = i % (t.len() - piece.len() + 1);
            format!("{}{}{}", &t[..at], piece, &t[at + piece.len()..])
        })
        .collect();
    let joined = pieces.join("\n");
    let (i, c) = both(&[t.as_str(), joined.as_str(), "3", "XYZ"]);
    let expected = format!("{}\nabcXYZ{}\n", want.join("\n"), &t[6..]);
    assert_eq!(i.as_deref(), Ok(expected.as_str()), "interpreted");
    assert_eq!((c.code, c.stdout.as_str()), (0, expected.as_str()), "compiled: {}", c.stderr);
    // whole characters, or a panic
    let fault = "withText: a text written outside another or inside one of its characters";
    for (text, at, piece, want) in [
        ("h\u{e9}llo", "1", "ab", Some("habllo")),
        ("h\u{e9}llo", "1", "\u{c9}", Some("h\u{c9}llo")),
        ("h\u{e9}llo", "0", "H\u{c8}", Some("H\u{c8}llo")),
        ("h\u{e9}llo", "6", "", Some("h\u{e9}llo")),
        ("\u{65e5}\u{672c}", "3", "\u{e9}a", Some("\u{65e5}\u{e9}a")),
        ("", "0", "", Some("")),
        ("h\u{e9}llo", "1", "a", None),
        ("h\u{e9}llo", "2", "a", None),
        ("h\u{e9}llo", "2", "", None),
        ("h\u{e9}llo", "5", "ab", None),
        ("h\u{e9}llo", "7", "", None),
        ("h\u{e9}llo", "-1", "a", None),
        ("\u{65e5}\u{672c}", "0", "ab", None),
        ("\u{65e5}\u{672c}", "4", "ab", None),
        ("abc", "9223372036854775807", "ab", None),
    ] {
        let (i, c) = both(&[text, "", at, piece]);
        match want {
            Some(w) => {
                let line = format!("\n{}\n", w);
                assert_eq!(i.as_deref(), Ok(line.as_str()), "interpreted: {:?} {} {:?}", text, at, piece);
                assert_eq!((c.code, c.stdout.as_str()), (0, line.as_str()), "compiled: {:?} {} {:?}: {}", text, at, piece, c.stderr);
            }
            None => {
                assert!(i.as_ref().is_err_and(|m| m.contains(fault)), "interpreted: {:?} {} {:?}: {:?}", text, at, piece, i);
                assert!(c.code != 0 && c.stderr.contains(fault), "compiled: {:?} {} {:?}: {} {}", text, at, piece, c.code, c.stderr);
            }
        }
    }
}
