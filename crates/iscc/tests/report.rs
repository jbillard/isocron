//! The memory report (SPEC section 8, `--report`, DECISIONS D98): every
//! allocation with its rule, every copy with its reason, the run-time tests.

use isc_ir::report::{entries, json, text, Entry};

fn report(src: &str) -> Vec<Entry> {
    let p = iscc::frontend("r.isc", src).unwrap_or_else(|e| panic!("{}", e));
    entries(&p)
}

fn find<'a>(es: &'a [Entry], kind: &str, line: u32) -> Vec<&'a Entry> {
    es.iter().filter(|e| e.kind == kind && e.line == line).collect()
}

fn dump(es: &[Entry]) -> String {
    text("r.isc", &[], es)
}

const RULES: &str = "action main(args: string[k]): i32 {
    const a = readF64(args[0]);
    const x = map(a, (v) => v * 2.0);
    print(sum(x) + max(x));
    const s = scan(iota(100), (+), 0);
    print(s[99]);
    const h = counts(iota(10));
    print(h);
    const y = map(a, (v) => v + 1.0);
    const t = y.with(0, 5.0);
    print(sum(t) + sum(y));
    print(sum(map(a, (v) => v * v)));
    return 0;
}

function counts(x: i64[n]): i64[4] {
    return fold(x, zeros(), (c, i) => c.with(i % 4, c[i % 4] + 1));
}
";

#[test]
fn each_allocation_has_its_rule() {
    let es = report(RULES);
    let d = dump(&es);
    // read from a file: an input, no rule
    assert!(find(&es, "alloc", 2)[0].why.contains("read from a file"), "{}", d);
    // read once by `sum` and `max` together (F3), but a statement of an
    // action: computed before the next statement (section 12)
    let x = find(&es, "alloc", 3);
    assert!(x[0].why.contains("section 12"), "{}", d);
    // used twice: M1
    let y = find(&es, "alloc", 9);
    assert_eq!(y[0].rule, Some("M1"), "{}", d);
    assert!(y[0].why.contains("`y` is used 2 times, at lines 10, 11"), "{}", d);
    // scan: M2
    assert_eq!(find(&es, "alloc", 5)[0].rule, Some("M2"), "{}", d);
    // the accumulator of a fold, in the function that has it: M2, its size
    // known at compilation
    let acc = find(&es, "alloc", 17);
    assert_eq!(acc[0].rule, Some("M2"), "{}", d);
    assert_eq!(acc[0].bytes, Some(32), "{}", d);
    assert_eq!(acc[0].what, "32 bytes", "{}", d);
    // `y` used again after the update: a copy, with the line of the use
    let c = find(&es, "copy", 10);
    assert_eq!(c.len(), 1, "{}", d);
    assert!(c[0].why.contains("`y` is used again at line 11"), "{}", d);
    // fused: no allocation at line 12
    assert!(find(&es, "alloc", 12).is_empty(), "{}", d);
    // run-time tests: the indexings of `args`, the update and the read of
    // the fold
    assert_eq!(find(&es, "bounds", 17).len(), 2, "{}", d);
    assert!(find(&es, "bounds", 2)[0].what == "args[0]", "{}", d);
}

#[test]
fn actions_and_views() {
    let es = report(
        "action main(args: string[k]): i32 {
    const x = map(iota(20), (i) => f64(i));
    print(map(x, (v) => v + 1.0));
    const s = x.slice(2, 8);
    print(sum(s));
    const p = [x.slice(0, 2), 7] as const;
    print(p);
    return 0;
}
",
    );
    let d = dump(&es);
    assert_eq!(find(&es, "alloc", 3)[0].rule, Some("M3"), "{}", d);
    // a slice is a view: nothing allocated at line 4
    assert!(find(&es, "alloc", 4).is_empty() && find(&es, "copy", 4).is_empty(), "{}", d);
    // kept in a tuple: copied
    assert!(find(&es, "copy", 6)[0].why.contains("a part of `x`"), "{}", d);
}

#[test]
fn a_length_test_does_not_materialize() {
    // the sizes of the zip are not proved equal: the test reads the length
    // of `map(a, ...)`, which is the length of `a`
    let es = report(
        "action main(args: string[k]): i32 {
    const a = readF64(args[0]);
    const b = readF64(args[1]);
    print(sum(map(zip(map(a, (v) => v * 2.0), b), (p) => {
        const [u, w] = p;
        return u * w;
    })));
    return 0;
}
",
    );
    let d = dump(&es);
    assert!(find(&es, "alloc", 4).is_empty(), "{}", d);
    let size = find(&es, "size", 4);
    assert_eq!(size[0].what, "length of a = length of b", "{}", d);
}

#[test]
fn a_function_of_the_prelude_is_reported_where_it_is_called() {
    let es = report(
        "action main(args: string[k]): i32 {
    const a = readF64(args[0]);
    print(sum(map(iota(3), (i) => a[i])));
    return 0;
}
",
    );
    let d = dump(&es);
    let b = find(&es, "bounds", 3);
    assert_eq!(b[0].what, "a[i]", "{}", d);
    assert!(b[0].why.contains("loop at line 3"), "{}", d);
    assert!(es.iter().all(|e| e.line <= 4), "{}", d);
}

#[test]
fn the_report_as_text_and_json() {
    let es = report(RULES);
    let t = text("r.isc", &[], &es);
    assert!(t.starts_with("Memory report — r.isc\n"), "{}", t);
    assert!(t.contains("  line 17   alloc       32 bytes      the accumulator of a `fold` (rule M2)"), "{}", t);
    let j = json("r.isc", &[], &es);
    assert!(j.starts_with("{\"file\": \"r.isc\", \"entries\": [{\"line\": 2, \"column\": "), "{}", j);
    assert!(j.contains("\"kind\": \"alloc\", \"what\": \"32 bytes\", \"bytes\": 32, \"rule\": \"M2\""), "{}", j);
    // balanced and escaped
    assert_eq!(j.matches('{').count(), j.matches('}').count(), "{}", j);
    assert!(!j.contains('\n') || j.ends_with("]}\n"), "{}", j);
    let none = text("e.isc", &[], &[]);
    assert!(none.contains("no allocation, no copy, no run-time test"), "{}", none);
}

#[test]
fn build_writes_the_json_next_to_the_executable() {
    let dir = std::env::temp_dir().join(format!("isc_report_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("r.isc");
    std::fs::write(&src, RULES).unwrap();
    let out = dir.join("r");
    let r = std::process::Command::new(env!("CARGO_BIN_EXE_iscc"))
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap(), "--report"])
        .output()
        .unwrap();
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let printed = String::from_utf8_lossy(&r.stdout);
    assert!(printed.starts_with("Memory report — "), "{}", printed);
    let j = std::fs::read_to_string(dir.join("r.report.json")).unwrap();
    assert!(j.contains("\"rule\": \"M1\""), "{}", j);
    assert!(out.is_file());
}

/// The example of `docs/language.md` gives the report the compiler prints.
#[test]
fn the_example_of_the_guide() {
    let doc = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/language.md")).unwrap();
    let sec = &doc[doc.find("### The memory report").unwrap()..];
    let prog_start = sec.find("```isc-check\n").unwrap() + "```isc-check\n".len();
    let prog = &sec[prog_start..prog_start + sec[prog_start..].find("```").unwrap()];
    let text_start = sec.find("```text\n").unwrap() + "```text\n".len();
    let shown = &sec[text_start..text_start + sec[text_start..].find("```").unwrap()];
    let p = iscc::frontend("stats.isc", prog).unwrap_or_else(|e| panic!("{}", e));
    assert_eq!(text("stats.isc", &p.files, &entries(&p)), shown);
}

/// A fused filter allocates nothing; one read by index is materialized.
#[test]
fn fused_filters_allocate_nothing() {
    let es = report(
        "function s(x: f64[n]): f64 {
    return sum(filter(x, (v) => v > 0.25));
}

function t(x: f64[n]): f64 {
    return filter(x, (v) => v > 0.25)[0];
}

action main(args: string[k]): i32 {
    const x = readF64(args[0]);
    print(s(x) + t(x));
    return 0;
}
",
    );
    let d = dump(&es);
    assert!(find(&es, "alloc", 2).is_empty(), "{}", d);
    assert_eq!(find(&es, "alloc", 6).len(), 1, "{}", d);
}

/// A slice of a producer allocates nothing when its consumer reads it; a
/// window of a producer materializes it (M1).
#[test]
fn views_of_producers() {
    let es = report(
        "function mid(x: f64[n]): f64 {
    return sum(map(x, (v) => v * 3.0).slice(2, n - 2));
}

function wsum(x: f64[n]): f64 {
    return sum(map(window(map(x, (v) => v * 0.5), 3), (r) => sum(r)));
}

action main(args: string[k]): i32 {
    const x = readF64(args[0]);
    print(mid(x) + wsum(x));
    return 0;
}
",
    );
    let d = dump(&es);
    assert!(find(&es, "alloc", 2).is_empty(), "{}", d);
    let w = find(&es, "alloc", 6);
    assert_eq!(w[0].rule, Some("M1"), "{}", d);
    assert!(w[0].why.contains("read through a window"), "{}", d);
}

/// Recursion (SPEC section 11): a call that is not a tail call uses the
/// stack, a direct tail call is a loop, an array result is M2.
#[test]
fn recursion_in_the_report() {
    let es = report(
        "function fib(n: i64): i64 {
    if (n < 2) {
        return n;
    } else {
        return fib(n - 1) + fib(n - 2);
    }
}

function walk(x: f64[n], i: i64, acc: f64): f64 {
    if (i == n) {
        return acc;
    } else {
        return walk(x, i + 1, acc + x[i]);
    }
}

function up(x: f64[n], k: i64): f64[n] {
    if (k == 0) {
        return x;
    } else {
        return map(up(x, k - 1), (v) => v + 1.0);
    }
}

action main(args: string[k]): i32 {
    const x = readF64(args[0]);
    print(fib(10) + i64(walk(x, 0, 0.0)) + i64(sum(up(x, 3))));
    return 0;
}
",
    );
    let d = dump(&es);
    // `fib(n - 1)` and `fib(n - 2)`
    assert_eq!(find(&es, "stack", 5).len(), 2, "{}", d);
    assert_eq!(find(&es, "loop", 13)[0].what, "`walk`", "{}", d);
    let up = find(&es, "alloc", 21);
    assert!(up.iter().any(|e| e.rule == Some("M2") && e.why.contains("the function `up`, called")), "{}", d);
}

/// A node of a recursive type is a `refcount` entry; one more owner of a
/// node is a count incremented, not a copy.
#[test]
fn recursive_types_in_the_report() {
    let es = report(
        "type List = Nil | Cons(i64, List);

function tail_of(l: List): List {
    return match (l) {
        Nil => Nil,
        Cons(h, t) => t,
    };
}

action main(args: string[k]): i32 {
    const l = Cons(1, Cons(2, Nil));
    print(tail_of(l));
    return 0;
}
",
    );
    let d = dump(&es);
    // `Cons`, `Cons` and `Nil`: three nodes
    assert!(find(&es, "refcount", 11).iter().all(|e| e.what == "`List`"), "{}", d);
    assert_eq!(find(&es, "refcount", 11).len(), 3, "{}", d);
    assert!(es.iter().any(|e| e.kind == "refcount" && e.why.starts_with("one owner more")), "{}", d);
    assert!(es.iter().all(|e| e.kind != "copy"), "{}", d);
}

/// Stage 2, phase 3: "histogramme sans allocation". The update of the fold
/// is in place (no copy); the only arrays are the input, the accumulator
/// (rule M2, once per call) and the array written (M3).
#[test]
fn histogram_without_allocation() {
    let src = include_str!("../../../examples/poc/histogram.isc");
    let p = iscc::frontend("histogram.isc", src).unwrap();
    let es = entries(&p);
    let d = dump(&es);
    assert!(es.iter().all(|e| e.kind != "copy"), "{}", d);
    let allocs: Vec<&Entry> = es.iter().filter(|e| e.kind == "alloc").collect();
    assert_eq!(allocs.len(), 3, "{}", d);
    let acc: Vec<&&Entry> = allocs.iter().filter(|e| e.rule == Some("M2")).collect();
    assert_eq!((acc.len(), acc[0].bytes), (1, Some(1024)), "{}", d);
    assert!(allocs.iter().any(|e| e.why.contains("read from a file")), "{}", d);
    assert!(allocs.iter().any(|e| e.rule == Some("M3")), "{}", d);
}

const FOLD_OF_TUPLES: &str = "function walk(a: f64[n], b: f64[n]): [f64[n], f64[n]] {
    return fold(iota(n), [map(a, (x) => x), map(b, (x) => x)] as const, (acc, k) => {
        const [x, y] = acc;
        return [x.with(k, x[k] + 1.0), y.with(k, y[k] * 2.0)] as const;
    });
}

function record(v: f64[n]): { x: f64[n], v: f64[n] } {
    const w = map(v, (e) => e);
    return { x: w * 2.0, v: w };
}

action main(args: string[k]): i32 {
    const [a, b] = walk([1.0, 2.0], [3.0, 4.0]);
    print(a);
    print(b);
    const r = record([5.0, 6.0]);
    print(r.x);
    print(r.v);
    return 0;
}
";

/// The accumulator of a `fold`, a tuple taken apart at the top of the body,
/// gives its components their own values: each `with` updates in place
/// (D164); the fields of a record are evaluated in the order of the
/// program, so that `w` is read for `x` before it is moved into `v`.
#[test]
fn tuples_of_folds_and_records_without_copies() {
    let es = report(FOLD_OF_TUPLES);
    let copies: Vec<&Entry> = es.iter().filter(|e| e.kind == "copy").collect();
    assert!(copies.is_empty(), "{}", dump(&es));
    let mut out = Vec::new();
    let p = iscc::frontend("r.isc", FOLD_OF_TUPLES).unwrap();
    assert_eq!(isc_interp::run(&p, &[], &mut out).unwrap(), 0);
    assert_eq!(String::from_utf8(out).unwrap(), "[2, 3]\n[6, 8]\n[10, 12]\n[5, 6]\n");
}

/// D383: the arrays the code generator makes by parts, never whole, are
/// reported so: the lines of reverse-complement made by blocks of 1 MiB for
/// `printBytes` (D225), its bases made by windows (D293); neither an
/// allocation of the whole array.
#[test]
fn arrays_made_by_parts_are_reported_so() {
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/benchmarks_game/revcomp.isc")).unwrap();
    let es = report(&src);
    let line_of = |needle: &str| src.lines().position(|l| l.contains(needle)).unwrap() as u32 + 1;
    let (lines, bases) = (line_of("return map(iota(length)"), line_of("const bases = "));
    assert!(!find(&es, "blocks", lines).is_empty(), "{}", dump(&es));
    assert!(!find(&es, "windows", bases).is_empty(), "{}", dump(&es));
    assert!(find(&es, "alloc", lines).is_empty() && find(&es, "alloc", bases).is_empty(), "{}", dump(&es));
}

/// D598: what a function of the prelude does inside itself is not in the
/// report of the program, as the functions of the runtime it replaces were
/// not: neither the checks of its own arrays (`b[0]` of `parseI64`, the
/// bytes of the needle of `indexOf`), nor its calls of functions that only
/// the prelude can name (`$parseRest`, `$findOne`). The checks of the
/// program stay, also those of its code run by a function of the prelude
/// (the predicate of `findIndex`), and its own calls.
#[test]
fn the_inside_of_the_prelude_is_not_in_the_report() {
    let es = report(
        "action main(args: string[k]): i32 {
    print(parseI64(\"12\") + 1);
    return 0;
}
",
    );
    assert!(es.is_empty(), "{}", dump(&es));
    let es = report(
        "function depth(n: i64): i64 {
    return n == 0 ? 0 : 1 + depth(n - 1);
}

action main(args: string[k]): i32 {
    const ys = map(iota(4), (i) => i + depth(1));
    print(args[0].indexOf(\"ab\") + f64(args.length).toString().length);
    print(findIndex(iota(3), (j) => { const t = ys.slice(j, 4); return t[1] > 2; }));
    print(args[0].trim().includes(\"a long needle of 20\") ? 1 : 0);
    return 0;
}
",
    );
    let d = dump(&es);
    assert!(es.iter().all(|e| !e.what.contains('$')), "a name of the prelude: {}", d);
    let bounds: Vec<(u32, &str)> = es.iter().filter(|e| e.kind == "bounds").map(|e| (e.line, e.what.as_str())).collect();
    assert_eq!(bounds, vec![(7, "args[0]"), (8, "t[1]"), (9, "args[0]")], "{}", d);
    let stack: Vec<(u32, &str)> = es.iter().filter(|e| e.kind == "stack").map(|e| (e.line, e.what.as_str())).collect();
    assert_eq!(stack, vec![(2, "`depth`"), (6, "`depth`")], "{}", d);
}

/// D598: the message of `tryParseF64` and of `tryParseI64` is the result of
/// a function that only the prelude can name: it has no line in the report
/// of a program, as when the runtime made it.
#[test]
fn the_result_of_a_function_of_the_prelude_is_not_listed() {
    let es = report(
        "action main(args: string[k]): i32 {
    print(match (tryParseF64(args[0])) { Ok(v) => v, Err(e) => 0.0 });
    print(match (tryParseI64(args[0])) { Ok(v) => v, Err(e) => 0 });
    print(parseF64(args[0]));
    return 0;
}
",
    );
    let d = dump(&es);
    assert!(!d.contains('$'), "a name of the prelude: {}", d);
    assert!(es.iter().all(|e| e.kind != "alloc" && e.line <= 4), "{}", d);
    assert_eq!(find(&es, "bounds", 2).len(), 1, "{}", d);
}

/// D659: the entries of a module of the program at its file and line
/// (`parts.isc:3`), not at the line of the program that imports it; those
/// of the prelude and of the standard modules at the line of the program
/// that calls them. The JSON names the file of each (`source`).
#[test]
fn the_entries_of_a_module_at_their_file_and_line() {
    let dir = std::env::temp_dir().join(format!("isc_report_modules_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("parts.isc"),
        "export function bumped(xs: i64[n]): i64[] {
    const ys = xs.with(0, xs[0] + 1);
    return concat(ys, xs);
}
",
    )
    .unwrap();
    std::fs::write(
        dir.join("main.isc"),
        "import * as parts from \"./parts\";
import * as path from \"path\";

action main(args: string[k]): i32 {
    const xs = map(iota(parseI64(args[0])), (i) => i * 2);
    print(parts.bumped(xs));
    print(path.join(args[0], \"x.txt\"));
    return 0;
}
",
    )
    .unwrap();
    let out = dir.join("main");
    let r = std::process::Command::new(env!("CARGO_BIN_EXE_iscc"))
        .current_dir(&dir)
        .args(["build", "main.isc", "-o", out.to_str().unwrap(), "--report"])
        .output()
        .unwrap();
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let printed = String::from_utf8_lossy(&r.stdout);
    assert!(printed.lines().any(|l| l.starts_with("  parts.isc:2 copy")), "{}", printed);
    assert!(!printed.contains("std:"), "{}", printed);
    assert!(!printed.contains("<prelude>"), "{}", printed);
    let j = std::fs::read_to_string(dir.join("main.report.json")).unwrap();
    assert!(j.contains("\"source\": \"parts.isc\""), "{}", j);
    assert!(j.contains("\"source\": null"), "{}", j);
    let _ = std::fs::remove_dir_all(&dir);
}
