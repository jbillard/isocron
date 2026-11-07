//! The standard library of v1 (D155): the functions of `Math`, seeded
//! random numbers, sorting. The interpreter and the compiled program print
//! the same text, with 1 and 4 threads, and free everything (Valgrind).

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

fn agree(src: &str, name: &str, args: &[&str], expected: &str) {
    assert_eq!(interp_ok(src, args), expected, "interpreter");
    let bin = compile(src, name);
    for t in [1, 4] {
        let r = run_bin_threads(&bin, args, Some(t));
        assert_eq!(r.code, 0, "{}", r.stderr);
        assert_eq!(r.stdout, expected, "compiled, {} threads", t);
    }
    valgrind_clean(src, name, args);
}

const MATH: &str = r#"action main(args: string[k]): i32 {
    print([Math.round(2.5), Math.round(-2.5), Math.round(-0.25), Math.round(0.49999999999999994)]);
    print([Math.floor(-1.5), Math.ceil(-1.5), Math.trunc(-1.5), Math.abs(-3.5)]);
    print([Math.abs(-7), Math.abs(-9223372036854775808), abs(-3i8), abs(250u8)] as const);
    print([Math.tan(1.0), Math.atan(1.0) * 4.0, Math.atan2(1.0, -1.0), Math.asin(1.0), Math.acos(-1.0)]);
    print(map([-1.5f32, 2.5f32], Math.round));
    print([-1.0, 2.0].map(abs));
    return 0;
}
"#;

/// `Math.round` as TypeScript (the ECMAScript standard: a half toward
/// +inf, -0 kept), `floor`, `ceil`, `trunc`, `abs` of integers (modulo
/// 2^bits) and floats, `tan`, `atan`, `atan2`, `asin`, `acos`.
#[test]
fn functions_of_math() {
    agree(MATH, "std_math", &[], "[3, -2, -0, 0]\n[-2, -1, -1, 3.5]\n(7, -9223372036854775808, 3, 250)\n[1.5574077246549023, 3.141592653589793, 2.356194490192345, 1.5707963267948966, 3.141592653589793]\n[-1, 3]\n[1, 2]\n");
}

const SORT: &str = r#"action main(args: string[k]): i32 {
    print(map(iota(3), (i) => random(42, i)));
    print(map(iota(8), (i) => randomInt(7, i, 10)));
    print([3, 1, 2].toReversed());
    print(sort([5, -1, 3, 3, -7]));
    print(sort([2.5, -0.0, 0.0, -1.0, 0.0 / 0.0, -3.5]));
    print(sort(["pomme", "kiwi", "Abricot", "kiwi", ""]));
    print(sort([200u8, 3u8, 255u8]) == [3u8, 200u8, 255u8]);
    print(sort([18446744073709551615u64, 1u64]));
    print(sort([true, false, true]));
    const words = ["ccc", "a", "bb", "dd", "e"];
    print(words.sortBy((w) => w.length));
    print(words.sortBy((w) => -w.length));
    const xs = map(iota(1000), (i) => random(1, i));
    const s = sort(xs);
    print(fold(iota(999), true, (ok, i) => ok && s[i] <= s[i + 1]));
    print(sum(s) == sum(xs));
    return 0;
}
"#;

/// SplitMix64 on a counter, written independently.
fn reference_random(seed: i64, i: i64) -> f64 {
    fn mix(z: u64) -> u64 {
        let z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        let z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        z ^ (z >> 31)
    }
    let x = mix(mix(seed as u64).wrapping_add((i as u64 + 1).wrapping_mul(0x9e3779b97f4a7c15)));
    (x >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
}

/// Seeded random numbers and stable sorting (D155).
#[test]
fn random_numbers_and_sorting() {
    let expected = "[0.5961188718302076, 0.1603653875985772, 0.16639780398145976]\n[5, 3, 9, 8, 6, 3, 3, 6]\n[2, 1, 3]\n[-7, -1, 3, 3, 5]\n[-3.5, -1, -0, 0, 2.5, NaN]\n[, Abricot, kiwi, kiwi, pomme]\ntrue\n[1, 18446744073709551615]\n[false, true, true]\n[a, e, bb, dd, ccc]\n[ccc, bb, dd, a, e]\ntrue\ntrue\n";
    agree(SORT, "std_sort", &[], expected);
    let first = format!("[{}, {}, {}]", reference_random(42, 0), reference_random(42, 1), reference_random(42, 2));
    assert_eq!(expected.lines().next().unwrap(), first);
}

const LENGTHS: &str = r#"action main(args: string[k]): i32 {
    const n = parseI64(args[0]);
    print(sum(iota(n).map((i) => i * i)));
    print(fill(n, "ab"));
    const m = 3;
    print(iota(m));
    print(iota(args.length + 1));
    return 0;
}
"#;

/// `iota(n)` and `fill(n, v)` of any `i64` (D155): a size known at run
/// time, a panic when it is negative.
#[test]
fn lengths_known_at_run_time() {
    agree(LENGTHS, "std_lengths", &["4"], "14\n[ab, ab, ab, ab]\n[0, 1, 2]\n[0, 1]\n");
    let want = "panic at line 3, column 19: negative size: -2";
    assert_eq!(interp_panic(LENGTHS, &["-2"]), want);
    let bin = compile(LENGTHS, "std_lengths_neg");
    let r = run_bin(&bin, &["-2"]);
    assert_eq!(r.code, 101);
    assert!(r.stderr.contains(want), "{}", r.stderr);
}

const JS: &str = r#"action main(args: string[k]): i32 {
    print(["42", " 2.5e3 ", "", "12abc", "0x1F", "0b101", "-Infinity", ".5", "5.", "1_000", "-0x10", "\u{A0}7\u{FEFF}"].map(Number));
    print(["12.5abc", "abc", "  -.5e3x", "1e", "Infinityx", "+3"].map(parseFloat));
    print([parseInt("42px"), parseInt("ff", 16), parseInt("0x1A"), parseInt("101", 2), parseInt("z", 37), parseInt("-0"), parseInt("123456789012345678901234567890")]);
    print([Number(7i32), Number(2.5f32)]);
    return 0;
}
"#;

const PARSING: &str = r#"function total(t: string): f64 {
    return sum(lines(t).map(parseF64));
}

action main(args: string[k]): i32 {
    print(args.map((a) => parseI64(a)));
    print(total("1.5\n2.25\n"));
    print(args.map((a) => match (tryParseF64(a)) {
        Ok(v) => v,
        Err(e) => -1.0,
    }));
    return 0;
}
"#;

/// `Number(s)`, `parseFloat(s)`, `parseInt(s, radix)` as ECMAScript (D156):
/// a `f64`, NaN when the text is not a number; `parseI64` and `parseF64`
/// are functions, usable in a lambda.
#[test]
fn conversions_of_texts() {
    agree(JS, "std_js", &[], "[42, 2500, 0, NaN, 31, 5, -inf, 0.5, 5, NaN, NaN, 7]\n[12.5, NaN, -500, 1, inf, 3]\n[42, 255, 26, 5, NaN, -0, 123456789012345680000000000000]\n[7, 2.5]\n");
    agree(PARSING, "std_parse", &["7", "8"], "[7, 8]\n3.75\n[7, 8]\n");
    assert_eq!(interp_panic(PARSING, &["7", "x"]), "panic at line 6, column 35: parseI64: `x` is not a valid integer");
}

const BIG: &str = r#"function fact(n: i64, acc: bigint): bigint {
    if (n <= 1) return acc;
    return fact(n - 1, acc * BigInt(n));
}

action main(args: string[k]): i32 {
    const a = 123456789012345678901234567890n;
    const b = -98765432109876543210n;
    print(a + b);
    print([a * b, a / b, a % b, -a] as const);
    print(2n ** 100n);
    print(fact(30, 1n));
    print([a > b, a == a + 0n, b < 0n, 0n == BigInt(0)] as const);
    print(`${a}`.length);
    print([i64(2n ** 64n + 5n), i64(-7n), Number(2n ** 70n)] as const);
    print(BigInt("  -42 ") + 0xffn);
    print(sort([5n, -3n, 2n ** 80n, 0n]));
    print(reduce([1n, 2n, 3n], (+), 0n));
    print(sum([10n, 20n]));
    return 0;
}
"#;

/// `bigint` (D156): literals `123n`, `+ - * / % **` (`/` toward zero, `%` of
/// the sign of the dividend), comparisons, `BigInt(x)`, conversions modulo
/// 2^64 and to the nearest `f64`, its text, sorting, the generic `sum`;
/// the values checked against Python's integers.
#[test]
fn big_integers() {
    agree(BIG, "std_big", &[], "123456788913580246791358024680\n(-12193263113702179522496570642237463801111263526900, -1249999988, 60185185207253086410, -123456789012345678901234567890)\n1267650600228229401496703205376\n265252859812191058636308480000000\n(true, true, true, true)\n30\n(5, -7, 1180591620717411300000)\n213\n[-3, 0, 5, 1208925819614629174706176]\n6\n30\n");
    for (body, want) in [
        ("print(1n / 0n)", "panic at line 2, column 14: integer division by zero"),
        ("print(2n ** BigInt(args.length - 1))", "panic at line 2, column 14: negative integer exponent: -1"),
        ("print(BigInt(\"1.5\"))", "panic at line 2, column 17: BigInt: `1.5` is not an integer"),
    ] {
        let src = format!("action main(args: string[k]): i32 {{\n    {};\n    return 0;\n}}\n", body);
        assert_eq!(interp_panic(&src, &[]), want, "{}", body);
        let bin = compile(&src, "std_big_panic");
        let r = run_bin(&bin, &[]);
        assert_eq!(r.code, 101, "{}", body);
        assert!(r.stderr.contains(want), "{}: {}", body, r.stderr);
    }
    // a literal negative exponent: a compile error (D162)
    match iscc::frontend("t.isc", "action main(args: string[k]): i32 {\n    print(2n ** -1n);\n    return 0;\n}\n") {
        Ok(_) => panic!("accepted"),
        Err(e) => assert!(e.contains("t.isc:2:14: error: negative integer exponent"), "{}", e),
    }
    // no mixing with the other numbers, as in TypeScript
    match iscc::frontend("t.isc", "action main(args: string[k]): i32 {\n    print(1n + 1.5);\n    return 0;\n}\n") {
        Ok(_) => panic!("accepted"),
        Err(e) => assert!(e.contains("found `bigint`"), "{}", e),
    }
}

const REGEX: &str = r##"function isWord(s: string): boolean {
    return /^[a-z]+$/i.test(s);
}

action main(args: string[k]): i32 {
    const dna = "GGCCaggGTAAAtttACCCTtaggGTAAA";
    print(countMatches(dna, /agggtaaa|tttaccct/gi));
    print(countMatches(dna, RegExp("[cgt]gggtaaa|tttaccc[acg]", "i")));
    print(dna.replace(/a/gi, "<$&>"));
    print(dna.replace(/a/i, "-"));
    print("2024-11-03".replace(/(\d+)-(\d+)-(\d+)/, "$3/$2/$1"));
    print("x1y22z".replaceAll(/\d+/g, "#"));
    print(matches("a1b22c333", /\d+/g));
    print(["abc", "a1", "Zed"].map(isWord));
    print("hello world".search(/o w/));
    print("a.b.c".replaceAll(".", "$$"));
    print("abc".replace("b", "[$&]"));
    print(10 / 2 / 5);
    const rs = [/a/, /b+/];
    print(rs.map((r) => r.test("bbb")));
    return 0;
}
"##;

/// Regular expressions (D157): literals `/source/flags` and `RegExp(s, f)`,
/// `test`, `search`, `countMatches`, `matches`, `replace` and `replaceAll`
/// of a `RegExp` or a text (the `$` patterns of ECMAScript); an invalid
/// literal is a compile error, a text a panic.
#[test]
fn regular_expressions() {
    agree(REGEX, "std_regex", &[], "3\n0\nGGCC<a>ggGT<A><A><A>ttt<A>CCCTt<a>ggGT<A><A><A>\nGGCC-ggGTAAAtttACCCTtaggGTAAA\n03/11/2024\nx#y#z\n[1, 22, 333]\n[true, false, true]\n4\na$b$c\na[b]c\n1\n[false, true]\n");
    match iscc::frontend("t.isc", "action main(args: string[k]): i32 {\n    print(/a(?=b)/.test(\"ab\"));\n    return 0;\n}\n") {
        Ok(_) => panic!("accepted"),
        Err(e) => assert!(e.contains("t.isc:2:11: error: RegExp: invalid regular expression `a(?=b)`: look-around, including look-ahead and look-behind, is not supported"), "{}", e),
    }
    match iscc::frontend("t.isc", "action main(args: string[k]): i32 {\n    print(/a/);\n    return 0;\n}\n") {
        Ok(_) => panic!("accepted"),
        Err(e) => assert!(e.contains("a RegExp cannot be printed"), "{}", e),
    }
    // flags known at compile time: checked there
    match iscc::frontend("t.isc", "action main(args: string[k]): i32 {\n    print(\"ab\".replaceAll(/a/, \"x\"));\n    return 0;\n}\n") {
        Ok(_) => panic!("accepted"),
        Err(e) => assert!(e.contains("t.isc:2:26: error: replaceAll: the RegExp must have the flag g"), "{}", e),
    }
    // flags computed at run time: a panic
    let src = "action main(args: string[k]): i32 {\n    print(RegExp(args[0]).test(\"ab\"));\n    print(\"ab\".replaceAll(RegExp(\"a\", args[0].slice(0, 0)), \"x\"));\n    return 0;\n}\n";
    for (arg, want) in [("(", "panic at line 2, column 17: RegExp: invalid regular expression `(`: unclosed group"), ("a", "panic at line 3, column 26: replaceAll: the RegExp must have the flag g")] {
        assert_eq!(interp_panic(src, &[arg]), want);
        let bin = compile(src, "std_regex_panic");
        let r = run_bin(&bin, &[arg]);
        assert_eq!(r.code, 101);
        assert!(r.stderr.contains(want), "{}", r.stderr);
    }
}

const MAP: &str = r##"action main(args: string[k]): i32 {
    const m = Map([["b", 2] as const, ["a", 1] as const, ["c", 3] as const, ["a", 10] as const]);
    print(m.size);
    print(m.keys());
    print(m.values());
    print(m.entries());
    print(m.has("a"));
    print(m.has("z"));
    print(m.get("a"));
    print(m.get("z"));
    const m2 = m.set("d", 4).set("b", 20).delete("c");
    print(m2.entries());
    const c = countBy(["x", "y", "x", "z", "x"], (s) => s);
    print(c.entries());
    return 0;
}
"##;

const MAP_COUNT: &str = r##"function total(m: Map<string, i64>, ws: string[n]): i64 {
    return sum(map(ws, (w) => m.get(w) ?? 0));
}

action main(args: string[k]): i32 {
    const words = "the cat and the dog and the bird".split(" ");
    const counts = countBy(words, (w) => w);
    print(counts.entries());
    print(counts.get("the") ?? 0);
    print(counts.get("fish") ?? 0);
    print(counts.get("fish") ?? counts.get("cat") ?? -1);
    print(total(counts, ["the", "and", "fish"]));
    const sorted = sortBy(counts.entries(), (e) => {
        const [w, c] = e;
        return -c;
    });
    print(sorted);
    const byLen = Map(map(words, (w) => [w.length, w] as const));
    print(byLen.entries());
    const values = 7;
    print(counts.values());
    print(values);
    return 0;
}
"##;

/// `Map<K, V>` (D158): built from its entries (each key at its first
/// insertion, with its last value), `size`, `get` with `??`, `has`, `keys`,
/// `values`, `entries`, `set`, `delete`, `countBy`; a method is not hidden
/// by a local variable of the same name.
#[test]
fn maps() {
    agree(MAP, "std_map", &[], "3\n[b, a, c]\n[2, 10, 3]\n[(b, 2), (a, 10), (c, 3)]\ntrue\nfalse\nSome(10)\nNone\n[(b, 20), (a, 10), (d, 4)]\n[(x, 3), (y, 1), (z, 1)]\n");
    agree(MAP_COUNT, "std_map_count", &[], "[(the, 3), (cat, 1), (and, 2), (dog, 1), (bird, 1)]\n3\n0\n1\n5\n[(the, 3), (and, 2), (cat, 1), (dog, 1), (bird, 1)]\n[(3, the), (4, bird)]\n[3, 1, 2, 1, 1]\n7\n");
    valgrind_clean(MAP, "std_map", &[]);
    valgrind_clean(MAP_COUNT, "std_map_count", &[]);
}

/// What a `Map` refuses, at compile time: a float key, printing, equality,
/// an array of maps; `new` of anything but `Map` and `RegExp`.
#[test]
fn map_errors() {
    for (body, want) in [
        ("const m = Map([[1.5, 2] as const]);\n    print(m.size);", "2:19: error: a float cannot be a key of a Map (class Key: an integer, a boolean, a string or a bigint)"),
        ("const x = 1.5;\n    const m = Map([[x, 2] as const]);\n    print(m.size);", "3:19: error: a float cannot be a key of a Map"),
        ("const m = Map([[1, 2] as const]);\n    print(m);", "3:11: error: a Map cannot be printed: print `m.entries()`"),
        ("const m = Map([[1, 2] as const]);\n    print(m == m);", "error: a Map cannot be compared"),
        ("const m = Map([[1, 2] as const]);\n    print([m, m].length);", "error: a Map cannot be stored here"),
        ("const p = new Foo(1);", "2:19: error: `new Foo`: `new` makes only a `Map` or a `RegExp`"),
    ] {
        let src = format!("action main(args: string[k]): i32 {{\n    {}\n    return 0;\n}}\n", body);
        match iscc::frontend("t.isc", &src) {
            Ok(_) => panic!("accepted: {}", body),
            Err(e) => assert!(e.contains(want), "{}\n{}", body, e),
        }
    }
}

/// A part of a tuple that is not memory is read after the parts that are
/// memory are taken out (the memory check refused it).
#[test]
fn scalar_part_after_taken_parts() {
    let src = "action main(args: string[k]): i32 {\n    const x = [\"a\"];\n    const p = [3, x, 4] as const;\n    const [r, q, s] = p;\n    print(r + s);\n    print(q);\n    return 0;\n}\n";
    agree(src, "tuple_scalar_part", &[], "7\n[a]\n");
    valgrind_clean(src, "tuple_scalar_part", &[]);
}

/// A panic that the literals of an operation make certain is a compile
/// error with the message of the panic (D162); the same operation on
/// values known at run time keeps its panic.
#[test]
fn certain_panics_are_compile_errors() {
    for (body, want) in [
        ("print(1 / 0)", "2:13: error: integer division by zero"),
        ("print(args.length % 0)", "2:23: error: integer division by zero"),
        ("print([1, 2][5])", "2:17: error: index out of bounds: the index is 5 but the length is 2"),
        ("print([1, 2][-1])", "2:17: error: index out of bounds: the index is -1 but the length is 2"),
        ("print(2 ** -1)", "2:13: error: negative integer exponent"),
        ("print(iota(-1))", "2:15: error: negative size: -1"),
        ("print(fill(-2, 0.5))", "2:15: error: negative size: -2"),
        ("print(\"ab\".replaceAll(RegExp(\"a\", \"i\"), \"x\"))", "2:26: error: replaceAll: the RegExp must have the flag g"),
    ] {
        let src = format!("action main(args: string[k]): i32 {{\n    {};\n    return 0;\n}}\n", body);
        match iscc::frontend("t.isc", &src) {
            Ok(_) => panic!("accepted: {}", body),
            Err(e) => assert!(e.contains(want), "{}: {}", body, e),
        }
    }
    // values of run time: panics, and nothing refused that may not panic
    let src = "action main(args: string[k]): i32 {\n    const z = args.length;\n    print([2.0 ** -1.0, 1.0 / 0.0]);\n    print([1, 2][z]);\n    print(10 / z);\n    return 0;\n}\n";
    assert_eq!(interp_panic(src, &[]), "panic at line 5, column 14: integer division by zero");
    let bin = compile(src, "certain_panics");
    let r = run_bin(&bin, &[]);
    assert_eq!(r.code, 101);
    assert!(r.stdout.starts_with("[0.5, inf]\n1\n"), "{}", r.stdout);
}

const MAP_GROUPS: &str = r##"action main(args: string[k]): i32 {
    const n = 200000;
    const ints = countBy(iota(n), (i) => (i * 7919) % 1237);
    print(ints.size);
    print(ints.keys().slice(0, 5));
    print(ints.get(0) ?? -1);
    print(ints.get(1236) ?? -1);
    const words = countBy(iota(n), (i) => `w${i % 3001}`);
    print(words.size);
    print(words.get("w3000") ?? -1);
    print(words.keys().slice(0, 3));
    const bools = countBy(iota(10), (i) => i % 3 == 0);
    print(bools.entries());
    const bigs = Map(map(iota(1000), (i) => [BigInt(i % 10) * 10n ** 30n, i] as const));
    print(bigs.size);
    print(bigs.get(7n * 10n ** 30n) ?? -1);
    return 0;
}
"##;

/// `Map` and `countBy` by a table of hashes (D167): many keys (the table
/// grows), integers, strings, booleans and `bigint`, in the order of their
/// first occurrence; the counts and last values checked here.
#[test]
fn map_groups_by_hashes() {
    let n = 200000i64;
    let c0 = (0..n).filter(|i| (i * 7919) % 1237 == 0).count();
    let c1236 = (0..n).filter(|i| (i * 7919) % 1237 == 1236).count();
    let first5: Vec<String> = (0..5).map(|i: i64| ((i * 7919) % 1237).to_string()).collect();
    let w3000 = (0..n).filter(|i| i % 3001 == 3000).count();
    let want = format!(
        "1237\n[{}]\n{}\n{}\n3001\n{}\n[w0, w1, w2]\n[(true, 4), (false, 6)]\n10\n997\n",
        first5.join(", "),
        c0,
        c1236,
        w3000
    );
    agree(MAP_GROUPS, "std_map_groups", &[], &want);
}

const GROUPS_AS_MADE: &str = r##"function key(i: i64): i64 {
    const r = (i * 7919) % 1237;
    return i % 2 == 0 ? r : -r * 100003;
}

action main(args: string[k]): i32 {
    const n = 1500000;
    const ks = countBy(iota(n), key);
    print(ks.size);
    print(ks.keys().slice(0, 6));
    print([ks.get(0) ?? -1, ks.get(1236) ?? -1, ks.get(-123603708) ?? -1, ks.get(5) ?? -1]);
    const small = countBy(iota(n), (i) => u8(i % 200));
    print([small.size, small.get(199u8) ?? -1, small.get(0u8) ?? -1]);
    const words = countBy(iota(n), (i) => i32(i % 7) - 3i32);
    print(words.entries());
    print(map([3, 5], (m) => countBy(iota(n), (i) => i % m * 70001).size));
    return 0;
}
"##;

/// `countBy` of a `map` of integer keys: the keys grouped as they are made,
/// by blocks, never stored all (D244), in parallel parts merged in their
/// order; small keys counted by their value and large ones hashed in the
/// same tables, negative keys, bytes, `i32`, inside the tasks of a `map`;
/// the `iota` read twice (the keys and the first element of each group)
/// never made. The same groups, in the order of their first occurrence.
#[test]
fn keys_grouped_as_they_are_made() {
    let n = 1500000i64;
    let key = |i: i64| {
        let r = (i * 7919) % 1237;
        if i % 2 == 0 { r } else { -r * 100003 }
    };
    let mut order: Vec<i64> = Vec::new();
    let mut counts: std::collections::HashMap<i64, i64> = std::collections::HashMap::new();
    for i in 0..n {
        let k = key(i);
        let c = counts.entry(k).or_insert(0);
        if *c == 0 {
            order.push(k);
        }
        *c += 1;
    }
    let get = |k: i64| counts.get(&k).copied().unwrap_or(-1);
    let first6: Vec<String> = order[..6].iter().map(|k| k.to_string()).collect();
    let small199 = (0..n).filter(|i| i % 200 == 199).count();
    let small0 = (0..n).filter(|i| i % 200 == 0).count();
    let words: Vec<String> = (0..7).map(|r| format!("({}, {})", r - 3, (0..n).filter(|i| i % 7 == r).count())).collect();
    let want = format!(
        "{}\n[{}]\n[{}, {}, {}, {}]\n[200, {}, {}]\n[{}]\n[3, 5]\n",
        order.len(),
        first6.join(", "),
        get(0),
        get(1236),
        get(-123603708),
        get(5),
        small199,
        small0,
        words.join(", ")
    );
    agree(GROUPS_AS_MADE, "std_groups_as_made", &[], &want);
    let p = iscc::frontend("g.isc", GROUPS_AS_MADE).unwrap_or_else(|e| panic!("{}", e));
    let ir = isc_ir::print::program(&p);
    assert!(!ir.lines().zip(ir.lines().skip(1)).any(|(a, b)| a.contains(": i64[] =") && b.trim() == "iota"), "an iota bound to a variable:\n{}", ir);
}

const GROUPS_SCATTERED: &str = r##"function key(i: i64): i64 {
    const r = (i * 7919) % 200003;
    return i % 11 == 0 ? r % 1000 : (i % 13 == 0 ? -r - 5 : r * 1000003 + 70000);
}

action main(args: string[k]): i32 {
    const n = parseI64(args[0]);
    const ks = countBy(iota(n), key);
    const keys = ks.keys();
    const m = keys.length;
    print(m);
    print(keys.slice(0, 6));
    print(sum(map(iota(m), (j) => (j + 1) * (keys[j] % 1000003))));
    print(sum(map(iota(m), (j) => (j % 97) * (ks.get(keys[j]) ?? 0))));
    print([ks.get(70000) ?? -1, ks.get(-5) ?? -1, ks.get(999) ?? -1]);
    return 0;
}
"##;

/// `countBy` with many distinct large keys (D253): the tables of the parts
/// scatter their keys by their hash once large, counted by rounds list by
/// list; with small keys (counted by their value), negative ones (too large
/// to fit with their index in 64 bits: kept in the tables) among them. The
/// groups in the order of their first occurrence, their counts, on 1 and 4
/// threads; and 40 million keys on 16 threads, 16 parts whose rounds of
/// 2^20 entries (D295) end twice in each.
#[test]
fn keys_scattered_by_their_hash() {
    let bin = compile(GROUPS_SCATTERED, "std_groups_scattered");
    for (n, threads) in [(3_000_000i64, &[1, 4][..]), (40_000_000, &[16][..])] {
    let key = |i: i64| {
        let r = (i * 7919) % 200003;
        if i % 11 == 0 {
            r % 1000
        } else if i % 13 == 0 {
            -r - 5
        } else {
            r * 1000003 + 70000
        }
    };
    let mut order: Vec<i64> = Vec::new();
    let mut counts: std::collections::HashMap<i64, i64> = std::collections::HashMap::new();
    for i in 0..n {
        let k = key(i);
        let c = counts.entry(k).or_insert(0);
        if *c == 0 {
            order.push(k);
        }
        *c += 1;
    }
    let m = order.len() as i64;
    let first6: Vec<String> = order[..6].iter().map(|k| k.to_string()).collect();
    let weighted: i64 = order.iter().enumerate().map(|(j, k)| (j as i64 + 1) * (k % 1000003)).sum();
    let by_count: i64 = order.iter().enumerate().map(|(j, k)| (j as i64 % 97) * counts[k]).sum();
    let get = |k: i64| counts.get(&k).copied().unwrap_or(-1);
    let want = format!("{}\n[{}]\n{}\n{}\n[{}, {}, {}]\n", m, first6.join(", "), weighted, by_count, get(70000), get(-5), get(999));
    for &t in threads {
        let r = run_bin_threads(&bin, &[&n.to_string()], Some(t));
        assert_eq!(r.code, 0, "{}", r.stderr);
        assert_eq!(r.stdout, want, "{} keys, {} threads", n, t);
    }
    }
}

const GROUPS_SMALL: &str = r##"action main(args: string[k]): i32 {
    const n = parseI64(args[0]);
    const ks = countBy(iota(n), (i) => i < n - 3000 ? (i / 7) % 60000 : (i % 3 == 0 ? i * 1000003 : i % 5));
    const keys = ks.keys();
    const m = keys.length;
    print(m);
    print(keys.slice(0, 6));
    print(sum(map(iota(m), (j) => (j + 1) * (keys[j] % 1000003))));
    print(sum(map(iota(m), (j) => (j % 97) * (ks.get(keys[j]) ?? 0))));
    print([ks.get(0) ?? -1, ks.get(59999) ?? -1, ks.get(4) ?? -1]);
    return 0;
}
"##;

/// D296: `countBy` of small keys (counted by their value), the blocks whose
/// keys are all small in one loop, the table grown first to the largest key
/// of the block (the keys grow along the indices); then blocks with large
/// keys among them. The groups in the order of their first occurrence and
/// their counts, on 1 and 4 threads.
#[test]
fn small_keys_counted_in_one_loop() {
    let bin = compile(GROUPS_SMALL, "std_groups_small");
    for n in [70_000i64, 2_000_000] {
        let key = |i: i64| if i < n - 3000 { (i / 7) % 60000 } else if i % 3 == 0 { i * 1000003 } else { i % 5 };
        let mut order: Vec<i64> = Vec::new();
        let mut counts: std::collections::HashMap<i64, i64> = std::collections::HashMap::new();
        for i in 0..n {
            let k = key(i);
            let c = counts.entry(k).or_insert(0);
            if *c == 0 {
                order.push(k);
            }
            *c += 1;
        }
        let m = order.len() as i64;
        let first6: Vec<String> = order[..6].iter().map(|k| k.to_string()).collect();
        let weighted: i64 = order.iter().enumerate().map(|(j, k)| (j as i64 + 1) * (k % 1000003)).sum();
        let by_count: i64 = order.iter().enumerate().map(|(j, k)| (j as i64 % 97) * counts[k]).sum();
        let get = |k: i64| counts.get(&k).copied().unwrap_or(-1);
        let want = format!("{}\n[{}]\n{}\n{}\n[{}, {}, {}]\n", m, first6.join(", "), weighted, by_count, get(0), get(59999), get(4));
        for t in [1, 4] {
            let r = run_bin_threads(&bin, &[&n.to_string()], Some(t));
            assert_eq!(r.code, 0, "{}", r.stderr);
            assert_eq!(r.stdout, want, "{} keys, {} threads", n, t);
        }
    }
}

const TAGGED: &str = r##"type List = Nil | Cons(i64, List);
type Tree = Leaf | Empty | Node(Tree, i64, Tree);

function range(a: i64, b: i64): List {
    if (a >= b) return Nil;
    return Cons(a, range(a + 1, b));
}

function total(l: List): i64 {
    return match (l) {
        Nil => 0,
        Cons(x, r) => x + total(r),
    };
}

function build(d: i64): Tree {
    if (d == 0) return d % 2 == 0 ? Leaf : Empty;
    return Node(build(d - 1), d, d == 2 ? Empty : build(d - 1));
}

function count(t: Tree): i64 {
    return match (t) {
        Leaf => 1,
        Empty => 10,
        Node(l, v, r) => v + count(l) + count(r),
    };
}

action main(args: string[k]): i32 {
    const l = range(0, 5);
    print(total(l));
    print(l);
    print([Nil, Cons(1, Nil), Nil]);
    print(Nil == Nil);
    print(range(0, 3) == Cons(0, Cons(1, Cons(2, Nil))));
    print(range(0, 3) == range(0, 2));
    const t = build(3);
    print(count(t));
    const shared = [t, t, Leaf, Empty];
    print(map(shared, count));
    print(Leaf == Empty);
    return 0;
}
"##;

/// A constructor without payload of a recursive type is no node: the odd
/// value 2 * tag + 1 (D169), read, compared, printed, shared and dropped
/// like any node.
#[test]
fn constructors_without_payload_are_not_allocated() {
    agree(TAGGED, "std_tagged", &[], "10\nCons(0, Cons(1, Cons(2, Cons(3, Cons(4, Nil)))))\n[Nil, Cons(1, Nil), Nil]\ntrue\ntrue\nfalse\n33\n[33, 33, 1, 10]\nfalse\n");
}

const CONSTANT_ARRAYS: &str = r##"const T = [3, 1, 2];

function bump(a: i64[n]): i64[n] {
    return a.with(0, 9);
}

action main(args: string[k]): i32 {
    print(T.with(1, 5));
    print(T);
    print(bump(T));
    print(T);
    print(fold(iota(3), [0.0, 0.0], (acc, i) => acc.with(i % 2, acc[i % 2] + 1.0)));
    print(sort([5, 3, 4]));
    print([1, 2] == [1, 2]);
    print([1.5, 2.5] * 2.0);
    print(map(iota(10000), (i) => [1, 2, 3][i % 3]).sum());
    print([true, false, true].filter((b) => b).length);
    print([7u8, 255u8][1]);
    print([1.0f32, -0.0f32]);
    return 0;
}
"##;

/// An array literal of scalar constants is data of the program (D173):
/// read without an allocation, copied when a value is made of it (`with`,
/// the accumulator of a `fold`), never freed nor changed.
#[test]
fn arrays_of_constants() {
    agree(CONSTANT_ARRAYS, "std_const_arrays", &[], "[3, 5, 2]\n[3, 1, 2]\n[9, 1, 2]\n[3, 1, 2]\n[2, 1]\n[3, 4, 5]\ntrue\n[3, 5]\n19999\n2\n255\n[1, -0]\n");
}

const CONSTANT_STRINGS: &str = r##"const NAMES = ["zero", "one", "two", ""];

function first(a: string[n]): string {
    return a[0];
}

function bumped(a: string[n]): string[n] {
    return a.with(0, "nine");
}

function named(i: i64): string {
    const days = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    return days[i % 7];
}

function longer(i: i64): string {
    const ws = ["a", "bb", "ccc"];
    return ws[i % 3] + "!";
}

function fresh(): string[] {
    return ["made", "here"];
}

action main(args: string[k]): i32 {
    print(NAMES.with(1, "uno"));
    print(NAMES);
    print(bumped(NAMES));
    print(NAMES);
    print(first(NAMES));
    print(join(map(iota(10), named), ","));
    print(join(map(iota(5), longer), " "));
    print(sum(map(iota(10000), (i) => ["x", "yy", "zzz"][i % 3].length)));
    print(sort(["pear", "apple", "fig"]));
    print(["a", "b"] == ["a", "b"]);
    print(fold(iota(3), ["", ""], (acc, i) => acc.with(i % 2, acc[i % 2] + "i")));
    print(map(["é", "日本", ""], (s) => s.length));
    print(join(["a", "b", "c"], "-"));
    print(["solo"][0].toUpperCase());
    const f = fresh();
    print(f.with(1, "there"));
    print(`${named(3)}${longer(2)}${NAMES[2]}`);
    return 0;
}
"##;

/// An array literal of string literals is data of the program (D401):
/// read without an allocation (an index, a length, `join`, a concatenation
/// with a part of it), copied when a value is made of it (`with`, `sort`,
/// the accumulator of a `fold`, returned, a constant of the top level given
/// to a function), never freed nor changed; Valgrind clean.
#[test]
fn arrays_of_string_literals() {
    agree(CONSTANT_STRINGS, "std_const_strings", &[], "[zero, uno, two, ]\n[zero, one, two, ]\n[nine, one, two, ]\n[zero, one, two, ]\nzero\nMon,Tue,Wed,Thu,Fri,Sat,Sun,Mon,Tue,Wed\na! bb! ccc! a! bb!\n19999\n[apple, fig, pear]\ntrue\n[ii, i]\n[2, 6, 0]\na-b-c\nSOLO\n[made, there]\nThuccc!two\n");
}

/// D239: the product of a big integer by a word, by 4 words in assembly
/// (`mulx` and one chain of `adc`) then the rest: numbers of 1 to 41 words
/// and a partial last word, words of all ones (every carry), signs, the
/// products by small and large words; the interpreter's results.
#[test]
fn big_integers_times_a_word() {
    let src = "action main(args: string[k]): i32 {
    const n = parseI64(args[0]);
    const ones = 2n ** BigInt(64 * n) - 1n;
    print(ones * 3n);
    print(ones * 18446744073709551615n);
    print(-ones * 7n);
    print((ones + 12345n) * 4294967296n);
    print((3n ** BigInt(40 * n + 7)) * 1000000007n);
    print(BigInt(n) * 10n);
    return 0;
}
";
    let bin = compile(src, "big_times_word");
    for n in ["1", "3", "4", "5", "8", "9", "16", "41"] {
        let want = interp_ok(src, &[n]);
        let r = run_bin(&bin, &[n]);
        assert_eq!((r.code, r.stdout.as_str()), (0, want.as_str()), "n {}: {}", n, r.stderr);
    }
}

/// D389: the quotient of a multiply-add of bigints, `(a * k + b) / c`,
/// in one call of the runtime, from the leading words when they decide it:
/// numbers of 1 to 40 words, `k` of one word first or second, `b` first or
/// second, sums that are exact multiples of `c` and one less (the bounds
/// then apart: the whole computation), negative operands, a `k` of two
/// words, small numbers; the call in the code; the interpreter's results;
/// a division by zero, the panic of the division.
#[test]
fn big_quotients_of_a_multiply_add() {
    let src = "function quotient(a: bigint, k: bigint, b: bigint, c: bigint): bigint {
    return (a * k + b) / c;
}

function swapped(a: bigint, k: bigint, b: bigint, c: bigint): bigint {
    return (b + k * a) / c;
}

action main(args: string[k]): i32 {
    const n = parseI64(args[0]);
    forEach(iota(n), (i) => {
        const w = i + 1;
        const c = 3n ** BigInt(40 * w + 1) + BigInt(w);
        const a = 7n ** BigInt(33 * w + 3);
        const k = BigInt(w * 1000003 + 3);
        const b = 5n ** BigInt(41 * w);
        const q = quotient(a, k, b, c);
        const e = q * c - a * k;
        print([q, swapped(a, k, b, c), quotient(k, a, b, c), quotient(a, k, e, c), quotient(a, k, e - 1n, c)]);
        print([quotient(-a, k, b, c), quotient(a, k, -b, c), quotient(a, k, b, -c), quotient(a, 2n ** 70n + 1n, b, c), quotient(BigInt(w), 3n, 4n, 7n)]);
    });
    return 0;
}
";
    let bin = compile(src, "big_muladd_div");
    let ll = std::fs::read_to_string(bin.with_extension("ll")).unwrap();
    // the two functions inlined at their 10 calls: one call of the runtime each
    assert_eq!(ll.matches("call void @isc_big_muladd_div").count(), 10, "a quotient is not one call");
    for n in ["1", "3", "40"] {
        let want = interp_ok(src, &[n]);
        let r = run_bin(&bin, &[n]);
        assert_eq!((r.code, r.stdout.as_str()), (0, want.as_str()), "n {}: {}", n, r.stderr);
    }
    let zero = "action main(args: string[k]): i32 {\n    const a = 3n ** 200n;\n    print((a * 3n + a) / BigInt(args.length - 1));\n    return 0;\n}\n";
    let bin = compile(zero, "big_muladd_div_zero");
    let r = run_bin(&bin, &["x"]);
    assert_eq!(r.code, 101, "{}", r.stderr);
    // the panic of the interpreter and of the code before (D389)
    assert!(r.stderr.contains("panic at line 3, column 24: integer division by zero"), "{}", r.stderr);
}

const COUNTS_BY_LINES: &str = r##"function key(i: i64): i64 {
    if (i % 1009 == 0) {
        return -(i % 50) - 1;
    }
    if (i % 97 == 0) {
        return i % 1000;
    }
    return (i * 2654435761) % 200003 + 70000;
}

action main(args: string[k]): i32 {
    const n = parseI64(args[0]);
    const ks = countBy(iota(n), key);
    print(ks.size);
    print(ks.keys().slice(0, 8));
    print([ks.get(0) ?? -1, ks.get(97) ?? -1, ks.get(-1) ?? -1, ks.get(70000) ?? -1, ks.get(270002) ?? -1]);
    print(sum(map(ks.entries(), (e) => {
        const [k, c] = e;
        return (k % 1000003) * c;
    })));
    return 0;
}
"##;

/// D317: `countBy` by tables of counts only (`$groupCounts`), each key
/// with its count in one word, found in its line of 8 slots: about 200000
/// distinct keys (the table grows from 1024 lines), small keys among them
/// (counted by their value, inside the loop of the hashed ones) and
/// negative ones (too large to share a word with a count: a `WordGroups`
/// of their own), in parallel parts (n = 1200000) and in one part without
/// the table of small keys (n = 60000: the key 0 hashed); the groups in the
/// order of their first occurrence, as the interpreter, on 1, 3 and 8
/// threads; Valgrind on the smaller one.
#[test]
fn counts_by_lines_of_slots() {
    let key = |i: i64| -> i64 {
        if i % 1009 == 0 {
            -(i % 50) - 1
        } else if i % 97 == 0 {
            i % 1000
        } else {
            (i * 2654435761) % 200003 + 70000
        }
    };
    let bin = compile(COUNTS_BY_LINES, "std_counts_by_lines");
    for n in [60000i64, 1200000] {
        let mut order: Vec<i64> = Vec::new();
        let mut counts: std::collections::HashMap<i64, i64> = std::collections::HashMap::new();
        for i in 0..n {
            let c = counts.entry(key(i)).or_insert(0);
            if *c == 0 {
                order.push(key(i));
            }
            *c += 1;
        }
        let get = |k: i64| counts.get(&k).copied().unwrap_or(-1);
        let first: Vec<String> = order[..8].iter().map(|k| k.to_string()).collect();
        let check: i64 = order.iter().map(|&k| (k % 1000003) * counts[&k]).sum();
        let want = format!("{}\n[{}]\n[{}, {}, {}, {}, {}]\n{}\n", order.len(), first.join(", "), get(0), get(97), get(-1), get(70000), get(270002), check);
        let arg = n.to_string();
        assert_eq!(interp_ok(COUNTS_BY_LINES, &[&arg]), want, "interpreter, n = {}", n);
        for t in [1, 3, 8] {
            let r = run_bin_threads(&bin, &[&arg], Some(t));
            assert_eq!((r.code, r.stdout.as_str()), (0, want.as_str()), "n = {}, {} threads: {}", n, t, r.stderr);
        }
    }
    valgrind_clean(COUNTS_BY_LINES, "std_counts_by_lines", &["60000"]);
}

const SIZED_PAYLOADS: &str = r##"type Shape = Poly(f64[k]) | Dot(f64);
type Tree = Leaf(i64) | Branch(Tree[k]);

function area(s: Shape): f64 {
    return match (s) {
        Poly(xs) => sum(xs),
        Dot(x) => x,
    };
}

function total(t: Tree): i64 {
    return match (t) {
        Leaf(v) => v,
        Branch(ts) => sum(map(ts, total)),
    };
}

function build(d: i64): Tree {
    return d == 0 ? Leaf(1) : Branch(map(iota(d), (i) => build(i)));
}

function width(t: Tree): i64 {
    return match (t) {
        Leaf(_) => 0,
        Branch(ts) => ts.length,
    };
}

action main(args: string[k]): i32 {
    print([area(Poly([1.0, 2.0, 3.5])), area(Dot(4.0)), area(Poly(map(iota(5), (i) => f64(i))))]);
    print(total(build(6)));
    const ts = map(iota(5), (d) => build(d));
    print(map(ts, total));
    print(map(ts, width));
    print(build(2) == Branch([Leaf(1), Branch([Leaf(1)])]));
    return 0;
}
"##;

/// D335: a payload holds an array of its own size (`Poly(f64[k])`,
/// `Branch(Tree[k])`): any at a construction, a new existential size where
/// a pattern takes it apart; an array of a recursive type whose nodes hold
/// arrays (the nodes are pointers in it, D130); equality node by node; the
/// same in the interpreter and compiled; Valgrind.
#[test]
fn payloads_hold_arrays_of_their_own_size() {
    agree(SIZED_PAYLOADS, "std_sized_payloads", &[], "[6.5, 4, 10]\n32\n[1, 1, 2, 4, 8]\n[0, 1, 2, 3, 4]\ntrue\n");
}

/// D335: the module `json`: an object with arrays, numbers, literals and
/// escapes (a surrogate pair) read and written again; the errors of an
/// array with a comma at its end, a member without colon, an unknown
/// escape, two values without comma, a word that is not a literal; empty
/// and nested containers; the same in the interpreter and compiled;
/// Valgrind.
#[test]
fn json_read_and_written() {
    agree(include_str!("data/std_json.isc"), "std_json", &[], include_str!("data/std_json.out"));
}

const SIZES_IN_PARAMETERS: &str = r##"function total(r: { name: string, xs: f64[k] }): f64 {
    return sum(r.xs) + f64(k);
}

function pair(t: [i64, f64[n]], ys: f64[n]): f64 {
    const [a, xs] = t;
    return sum(xs * ys) + f64(a);
}

function both(r: { xs: f64[k], ys: f64[k] }): f64 {
    return sum(r.xs * r.ys);
}

function make(n: i64): { name: string, xs: f64[k] } {
    return { name: "a", xs: map(iota(n), (i) => f64(i)) };
}

function rows(t: f64[r][c]): i64 {
    return r * 100 + (r > 0 ? t[0].length : 0);
}

function sums(t: f64[r][c]): f64[r] {
    return map(t, (row) => sum(row));
}

function firstRow(t: f64[r][c]): f64[c] {
    return t[0];
}

function inner(t: { xs: f64[k], name: string }[n]): string[n] {
    return map(t, (x) => `${x.name}:${x.xs.length}`);
}

function mk(i: i64): { xs: f64[k], name: string } {
    return { xs: map(iota(2), (j) => f64(i * 10 + j)), name: `n${i}` };
}

action main(args: string[k]): i32 {
    print(total({ name: "a", xs: [1.0, 2.0] }));
    print(total(make(4)));
    print(pair([3, [1.0, 2.0]] as const, [10.0, 100.0]));
    print(both({ xs: [1.0, 2.0], ys: [3.0, 4.0] }));
    const m = make(args.length + 2);
    print(both({ xs: m.xs, ys: map(m.xs, (x) => x * 2.0) }));
    const t = [[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]];
    print([rows(t), rows(map(iota(0), (i) => [1.0, 2.0])), rows(map(iota(2), (i) => [f64(i), 2.0]))]);
    print(sums(t));
    print(firstRow(t) + [10.0, 20.0, 30.0]);
    print(firstRow(map(iota(3), (i) => [f64(i), f64(i * i)])));
    const ms = map(iota(3), (i) => mk(i));
    print(ms);
    print(inner(ms));
    return 0;
}
"##;

/// D336: a size of a function bound by an array in a record or a tuple of
/// a parameter (`r: { xs: f64[k] }`, its value the length of `r.xs`), or by
/// the rows of an array of a parameter (`t: f64[r][c]`, `c` known to the
/// types only); two arrays of one size in a record tested at the call;
/// rows of a `map` given to a function, built or constant (a row of a
/// producer is no longer computed alone, D336); an array of records
/// holding arrays; the interpreter, compiled, Valgrind.
#[test]
fn sizes_bound_by_arrays_in_records_tuples_and_rows() {
    agree(
        SIZES_IN_PARAMETERS,
        "sizes_in_parameters",
        &[],
        "5\n10\n213\n11\n2\n[203, 0, 202]\n[6, 15]\n[11, 22, 33]\n[0, 0]\n[{name: n0, xs: [0, 1]}, {name: n1, xs: [10, 11]}, {name: n2, xs: [20, 21]}]\n[n0:2, n1:2, n2:2]\n",
    );
    // the sizes of two arrays of a record: tested where the function is
    // called when only the run time knows them
    let mismatch = "function both(r: { xs: f64[k], ys: f64[k] }): f64 {\n    return sum(r.xs * r.ys);\n}\n\naction main(args: string[k]): i32 {\n    const xs = map(iota(args.length + 2), (i) => f64(i));\n    print(both({ xs: xs, ys: filter(xs, (x) => x > 0.5) }));\n    return 0;\n}\n";
    assert_eq!(interp_panic(mismatch, &[]), "panic at line 7, column 15: size mismatch: 2 != 1");
    let r = run_bin(&compile(mismatch, "sizes_in_records_mismatch"), &[]);
    assert_eq!((r.code, r.stderr.trim()), (101, "panic at line 7, column 15: size mismatch: 2 != 1"));
    // the size of rows is not a value of the function
    let e = match iscc::frontend("rows.isc", "function w(t: f64[r][c]): i64 {\n    return c;\n}\n\naction main(args: string[k]): i32 {\n    print(w([[1.0]]));\n    return 0;\n}\n") {
        Ok(_) => panic!("the size of rows read as a value"),
        Err(e) => format!("{}", e),
    };
    assert!(e.contains("2:12") && e.contains("known to the types only"), "{}", e);
}

const OWN_SIZES: &str = r##"import * as json from "json";

// records holding arrays of their own size (D341)
type Request = { path: string, headers: [string, string][], body: u8[] };
type Response = { status: i64, body: u8[] };

// a function of requests, applied by `map` to a batch of them
action serveAll(rs: Request[n], handler: (Request) => Response, rounds: i64): i64 {
    if (rounds == 0) {
        return 0;
    }
    const out = map(rs, handler);
    print(map(out, (r) => `${r.status}:${stringOf(r.body)}`));
    return serveAll(rs, handler, rounds - 1);
}

function hello(r: Request): Response {
    return { status: r.path == "/" ? 200 : 404, body: bytes(`${r.path} ${r.headers.length} ${r.body.length}`) };
}

// a parameter `T[]`, a tuple holding one, a result `T[]`
function total(xs: u8[], t: [u8[], i64]): i64 {
    const [a, n] = t;
    return xs.length + a.length + n;
}

function longer(r: Request, by: i64): u8[] {
    return concat(r.body, fill(by, 33u8));
}

type Tagged = { name: string, tags: i64[] };

action main(args: string[k]): i32 {
    const rs: Request[] = [{ path: "/", headers: [], body: [] }, { path: "/x", headers: [["a", "b"] as const], body: bytes("body") }, { path: "/y", headers: [], body: bytes("a longer body") }];
    serveAll(rs, hello, 1);
    print(total(bytes("abc"), [bytes("de"), 7] as const));
    print(stringOf(longer(rs[1], args.length + 3)));
    print([rs[1] == rs[1], rs[0] == rs[1], rs[1] == rs[2]] as const);
    // a record made with arrays of any size, kept where `T[]` is expected
    const made = map(iota(4), (i) => ({ status: i, body: map(iota(i), (j) => u8(65 + j)) }));
    const kept: Response[] = made;
    print(map(kept, (r) => stringOf(r.body)));
    const one = args.length == 0 ? rs[2] : rs[0];
    print(one.body.length);
    // JSON: arrays of their own size in the records of an array
    match (json.decode<Tagged[]>("[{\"name\": \"a\", \"tags\": [1]}, {\"name\": \"b\", \"tags\": [1, 2, 3]}, {\"name\": \"c\", \"tags\": []}]")) {
        Ok(ts) => {
            print(map(ts, (t) => `${t.name}:${sum(t.tags)}/${t.tags.length}`));
            print(json.encode(ts));
        },
        Err(e) => print(json.message(e)),
    };
    return 0;
}
"##;

/// D341: arrays of their own size (`T[]`) in records, tuples and function
/// types: a batch of requests of every size given to a function by `map`,
/// a parameter and a result `T[]`, a tuple taken apart, equality, records
/// made with any sizes and kept as `T[]`, a conditional, JSON decoded into
/// records of arrays of different lengths and encoded; compiled (1 and 4
/// threads) and interpreted, the same outputs; Valgrind.
#[test]
fn arrays_of_their_own_size() {
    let want = "[200:/ 0 0, 404:/x 1 4, 404:/y 0 13]\n12\nbody!!!\n(true, false, false)\n[, A, AB, ABC]\n13\n[a:1/1, b:6/3, c:0/0]\n[{\"name\":\"a\",\"tags\":[1]},{\"name\":\"b\",\"tags\":[1,2,3]},{\"name\":\"c\",\"tags\":[]}]\n";
    agree(OWN_SIZES, "own_sizes", &[], want);
    let bin = compile(OWN_SIZES, "own_sizes_4");
    let r = run_bin_threads(&bin, &[], Some(4));
    assert_eq!((r.code, r.stdout.as_str()), (0, want), "4 threads: {}", r.stderr);
    // refused at compile time: rows of their own size, a function that
    // takes only arrays of 4 where any size is passed
    let prelude = "type Request = { path: string, body: u8[] };\ntype Response = { status: i64, body: u8[] };\n";
    let refused = [
        ("function rows(x: u8[4][]): i64 {\n    return x.length;\n}\n", "    print(1);\n", "the rows of an array of arrays have one size"),
        (
            "function four(r: { path: string, body: u8[4] }): Response {\n    return { status: i64(r.body[3]), body: [] };\n}\n\naction serve(rs: Request[n], h: (Request) => Response): void {\n    print(map(rs, h).length);\n}\n",
            "    const rs: Request[] = [{ path: \"/\", body: bytes(\"a\") }];\n    serve(rs, four);\n",
            "size mismatch: expected `({body: u8[], path: string}) => {body: u8[], status: i64}`, found `({body: u8[4], path: string}) => {body: u8[], status: i64}`",
        ),
    ];
    for (decl, body, want) in refused {
        let src = format!("{}\n{}\naction main(args: string[k]): i32 {{\n{}    return 0;\n}}\n", prelude, decl, body);
        match iscc::frontend("own.isc", &src) {
            Ok(_) => panic!("accepted:\n{}", src),
            Err(e) => assert!(e.contains(want), "{}", e),
        }
    }
    // an array of its own size given to a size of a function with another
    // array: tested where it is called, as a size known at run time
    let mismatch = format!("{}\nfunction both(r: {{ path: string, body: u8[n] }}, x: u8[n]): i64 {{\n    return sum(map(r.body + x, (a) => i64(a)));\n}}\n\naction main(args: string[k]): i32 {{\n    const r: Request = {{ path: \"/\", body: bytes(\"abc\") }};\n    print(both(r, bytes(\"x\")));\n    return 0;\n}}\n", prelude);
    let want = interp_panic(&mismatch, &[]);
    assert!(want.contains("size mismatch: 1 != 3"), "{}", want);
    let r = run_bin(&compile(&mismatch, "own_sizes_mismatch"), &[]);
    assert_eq!((r.code, r.stderr.trim()), (101, want.trim()));
}

const RULES_OF_THE_SERVER: &str = r##"// the rules of the compiler found with the server of HTTP (D342)
type P = { id: i64, buf: u8[] };
type A = { id: i64, bytes: u8[] };
type Frame = { length: i64, fault: i64 };
type Msg = { name: string, headers: [string, string][] };
type R = { status: i64, body: u8[] };
type E = Bad(string) | Worse(i64);
type H = { eols: i64[], a: i64 };

// a type parameter fixed by the array before the lambda that reads fields
function fresh(ps: P[n], xs: A[m]): i64 {
    return filter(xs, (a) => a.bytes.length > 0 && count(ps, (p) => p.id == a.id) == 0).length;
}

function step(n: i64): Result<i64, string> {
    return n > 3 ? Err("big") : Ok(n);
}

// an early return of a construction, then a `match` of the same type
function f(n: i64, hs: [string, string][h]): Result<R, E> {
    if (n >= 5) {
        return Ok({ status: n, body: [] });
    }
    return match (step(n)) {
        Ok(b) => b == 0 ? Err(Bad("zero")) : f(n + 1, hs),
        Err(e) => Err(Worse(n)),
    };
}

function h(k: i64): H {
    return { eols: map(iota(k + 1), (j) => j * 10), a: k };
}

// a slice of records that hold arrays: each a copy of its own
function cut(n: i64, take: i64): H[] {
    const all = map(iota(n), (k) => h(k));
    return all.slice(0, take);
}

action main(args: string[k]): i32 {
    const ps: P[] = [{ id: 1, buf: [] }, { id: 3, buf: bytes("x") }];
    const xs: A[] = [{ id: 2, bytes: bytes("y") }, { id: 3, bytes: bytes("z") }, { id: 4, bytes: [] }];
    print(fresh(ps, xs));
    print(count(ps, (p) => p.id > 1));
    const fr: Frame[] = [{ length: 7, fault: 0 }, { length: 0, fault: 400 }];
    print(map(fr, (x) => x.length + x.fault));
    const ms: Msg[] = [{ name: "a", headers: [["k", "v"] as const] }, { name: "b", headers: [] }];
    print(map(ms, (m) => `${m.name}:${m.headers.length}`));
    print(match (f(args.length + 1, [])) {
        Ok(r) => r.status,
        Err(e) => -1,
    });
    const hs = cut(4, 2);
    const noise = map(iota(5), (i) => map(iota(100), (j) => j + i));
    print(hs);
    print(noise[4][99]);
    const n = args.length + 3;
    print("a" + toString(n) + "b" + toString(n + 1) + "c");
    print(`x ${n} y ${n * 2} z ${n + 1} end`);
    return 0;
}
"##;

/// D342: the rules found with the server of HTTP: a type parameter that
/// the array fixes known to the lambda after it (`count(ps, (p) => p.id
/// ...)`, behind `&&`), a field `length` read by `.length`, arrays of
/// tuples of strings in the records of an array, an early return of a
/// construction before a `match`, a slice of records that hold arrays (each
/// a copy: they were freed twice), chains of strings in one block (the
/// interpreter: all the parts); compiled and interpreted, Valgrind.
#[test]
fn rules_found_with_the_server() {
    agree(
        RULES_OF_THE_SERVER,
        "rules_of_the_server",
        &[],
        "1\n1\n[7, 400]\n[a:1, b:0]\n-1\n[{a: 0, eols: [0]}, {a: 1, eols: [0, 10]}]\n103\na3b4c\nx 3 y 6 z 4 end\n",
    );
}

const TYPE_ARGUMENTS: &str = r##"function first<T>(xs: T[n]): T {
    return xs[0];
}

function two<A, B>(a: A, b: B): [A, B] {
    return [a, b] as const;
}

function none<T>(): Option<T> {
    return None;
}

action main(args: string[k]): i32 {
    print(first<f64>([2.5, 1.0]));
    print(first<string>(["a"]));
    print(two<i32, string>(3, "x"));
    print(none<i64>() ?? 7);
    const a = 1;
    const b = 2;
    const c = 3;
    print(a < b);
    print([a < b, b > c, (a < b) == (c > b)]);
    return 0;
}
"##;

/// D336: the type arguments of a call, `f<T>(x)` as in TypeScript: read as
/// types when types closed by `>` and followed by `(` come after `<`,
/// else a comparison; their number checked.
#[test]
fn type_arguments_of_calls() {
    agree(TYPE_ARGUMENTS, "type_arguments", &[], "2.5\na\n(3, x)\n7\ntrue\n[true, false, true]\n");
    let e = match iscc::frontend("ta.isc", "function id<T>(x: T): T {\n    return x;\n}\n\naction main(args: string[k]): i32 {\n    print(id<i64, f64>(1));\n    return 0;\n}\n") {
        Ok(_) => panic!("two type arguments given to one parameter"),
        Err(e) => format!("{}", e),
    };
    assert!(e.contains("takes 1 type argument(s), but 2 were given"), "{}", e);
    let e = match iscc::frontend("ta.isc", "function id<T>(x: T): T {\n    return x;\n}\n\naction main(args: string[k]): i32 {\n    print(id<f64>(1u8));\n    return 0;\n}\n") {
        Ok(_) => panic!("a type argument not followed"),
        Err(e) => format!("{}", e),
    };
    assert!(e.contains("mismatch"), "{}", e);
}

/// D336: the text of a float as long as it is (5e-324: 326 bytes), the
/// same compiled as in the interpreter (a buffer of 64 bytes cut it).
#[test]
fn texts_of_long_floats() {
    agree(
        "action main(args: string[k]): i32 {\n    const xs = [5e-324, 1e21, 1.5e300, -1.7976931348623157e308, 1e-7];\n    print(map(xs, (x) => toString(x).length));\n    print(toString(1e-45f32).length);\n    print(toString(-1.7976931348623157e308).slice(0, 18));\n    return 0;\n}\n",
        "texts_of_long_floats",
        &[],
        "[326, 22, 301, 310, 9]\n47\n-17976931348623157\n",
    );
}

/// D336: `json.decode<T>(text)` and `json.encode(x)`: a record with an
/// array of records holding an `Option`, fields unknown to the type
/// skipped, the path of each fault (a field missing, of another type, an
/// integer out of range, a bracket of the other kind) with its line and
/// column; the integers of every width at the ends of their ranges, `f32`;
/// arrays of arrays and arrays of records holding arrays, whose rows of
/// other lengths are faults; fixed sizes, tuples, `Option` at the top, the
/// last of two members of one name, escapes in keys; NaN, the infinities,
/// -0 and the exponents written as JavaScript does; the brackets checked
/// level by level; escapes and surrogate pairs read by `filter`, `map`
/// and `join` (20000 escapes in one string), their faults at their place,
/// a missing comma at the word after the value; the interpreter, compiled,
/// Valgrind.
#[test]
fn json_decoded_into_types_and_encoded() {
    agree(include_str!("data/std_json_types.isc"), "std_json_types", &[], include_str!("data/std_json_types.out"));
    agree(include_str!("data/std_json_decode.isc"), "std_json_decode", &[], include_str!("data/std_json_decode.out"));
    agree(include_str!("data/std_json_text.isc"), "std_json_text", &[], include_str!("data/std_json_text.out"));
    agree(include_str!("data/std_json_strings.isc"), "std_json_strings", &[], include_str!("data/std_json_strings.out"));
}

/// D336: a type that JSON cannot carry, refused at compile time with the
/// place of the part that it cannot carry.
#[test]
fn json_types_refused_at_compile_time() {
    let cases: &[(&str, &str)] = &[
        ("json.decode<{ m: Map<string, i64> }>(\"{}\")", "a `Map`"),
        ("json.decode<{ a: { b: bigint } }>(\"{}\")", "a `bigint` (JSON numbers are read as `f64` or integers of 64 bits) at `.a.b`"),
        ("json.decode<{ f: (i64) => i64 }>(\"{}\")", "a function at `.f`"),
        ("json.decode<Option<Option<i64>>>(\"1\")", "an `Option` of an `Option`"),
        ("json.decode<Shape>(\"1\")", "the sum type `Shape`"),
        ("json.decode(\"1\")", "`void`"),
        ("json.encode(None)", "`void`"),
        ("json.decode<Option<f64[k]>[m]>(\"[]\")", "an `Option` holding an array, in an array"),
    ];
    for (call, want) in cases {
        let src = format!("import * as json from \"json\";\n\ntype Shape = Dot(f64) | Line(f64, f64);\n\naction main(args: string[k]): i32 {{\n    const r = {};\n    return 0;\n}}\n", call);
        let e = match iscc::frontend("bad.isc", &src) {
            Ok(_) => panic!("accepted: {}", call),
            Err(e) => format!("{}", e),
        };
        assert!(e.contains("JSON cannot carry the type of") && e.contains(want) && e.contains("bad.isc:6:"), "{}: {}", call, e);
    }
    // a type parameter: the type must be known where it is called
    let generic = "import * as json from \"json\";\n\nfunction load<T>(s: string): Result<T, json.JsonError> {\n    return json.decode<T>(s);\n}\n\naction main(args: string[k]): i32 {\n    print(match (load<f64>(\"1\")) {\n        Ok(x) => x,\n        Err(e) => 0.0,\n    });\n    return 0;\n}\n";
    let e = match iscc::frontend("bad.isc", generic) {
        Ok(_) => panic!("decode of a type parameter accepted"),
        Err(e) => format!("{}", e),
    };
    assert!(e.contains("a type parameter (the type must be known where it is called)") && e.contains("bad.isc:4:"), "{}", e);
}

const REDECLARED: &str = r##"import * as json from "json";
import * as time from "time";

// a function of the prelude declared again by the program
function count(j: json.Json): i64 {
    return match (j) {
        json.Arr(xs) => xs.length * 100,
        _ => -1,
    };
}

action main(args: string[k]): i32 {
    match (json.parse("[1, 2, 3]")) {
        Ok(j) => print(count(j)),
        Err(e) => print(json.message(e)),
    };
    match (json.decode<{ a: f64[k] }>("{\"a\": [1.5, 2.5]}")) {
        Ok(v) => print(json.encode(v)),
        Err(e) => print(json.message(e)),
    };
    print(time.formatIso(1767225600123000000));
    return 0;
}
"##;

/// D337: a function of the prelude that a program declares again (here
/// `count`, of another type) is the program's in the program, and keeps
/// its meaning in the modules of the standard library (`json` counts with
/// the `count` of the prelude); the interpreter, compiled, Valgrind.
#[test]
fn a_program_declaring_a_function_of_the_prelude_leaves_the_modules_alone() {
    agree(REDECLARED, "redeclared_prelude", &[], "300\n{\"a\":[1.5,2.5]}\n2026-01-01T00:00:00.123Z\n");
}

const BRANCHES_OF_SIZES: &str = r##"type S = A | B | C;

function pick(c: boolean, a: i64[n], b: i64[m]): i64[k] {
    if (c) {
        return a;
    }
    return b;
}

function rows(s: S): f64[k] {
    return match (s) {
        A => [1.0],
        B => [1.0, 2.0],
        C => map(iota(3), (i) => f64(i)),
    };
}

function grid(c: boolean): f64[r][q] {
    return c ? [[1.0, 2.0]] : [[1.0], [2.0], [3.0]];
}

action main(args: string[k]): i32 {
    print([pick(true, [1, 2, 3], [4]).length, pick(false, [1, 2, 3], [4]).length]);
    print(map([A, B, C], (s) => rows(s).length));
    print([grid(true).length, grid(false).length, grid(true)[0].length, grid(false)[0].length]);
    const xs = args.length > 5 ? [1, 2] : [3];
    print(xs);
    const t = args.length == 0 ? { a: [1.0], n: 1 } : { a: [1.0, 2.0], n: 2 };
    print(t);
    const z = fill(2, fill(3, 1.5));
    print(z);
    print(fill(0, fill(0, 0.0)).length);
    print(fill(2, ["a", "b"]));
    const e = ["", "x".slice(0, 0), "" + "", `${""}`];
    print(map(e, (s) => s.length));
    return 0;
}
"##;

/// D337: the branches of a conditional, or the arms of a `match`, that are
/// arrays of two sizes: a size fixed at run time, as the size of an array
/// that a call returns (through tuples and records; the type of the
/// conditional its own in the lowering, not that of its first branch);
/// `fill` of a row: an array of arrays; empty strings that own no memory;
/// the interpreter, compiled with 1 and 4 threads, Valgrind.
#[test]
fn branches_of_arrays_of_two_sizes() {
    agree(
        BRANCHES_OF_SIZES,
        "branches_of_sizes",
        &[],
        "[3, 1]\n[1, 2, 3]\n[1, 3, 2, 1]\n[3]\n{a: [1], n: 1}\n[[1.5, 1.5, 1.5], [1.5, 1.5, 1.5]]\n0\n[[a, b], [a, b]]\n[0, 0, 0, 0]\n",
    );
}

/// D338: the module `sha256` (FIPS 180-4): the vectors of the NIST (the
/// empty message, `abc`, the messages of 448 and 896 bits, a million `a`),
/// the lengths at the bounds of the blocks (55, 56, 63, 64, 65, 119, 120,
/// 128 bytes) and a text of UTF-8, as `sha256sum` gives them; the
/// interpreter, compiled with 1 and 4 threads, Valgrind.
#[test]
fn sha256_of_the_vectors_of_the_nist() {
    agree(include_str!("data/std_sha256.isc"), "std_sha256", &[], include_str!("data/std_sha256.out"));
}

/// D397: `json.quote` in one pass: each of the 32 controls (`\n`, `\r`,
/// `\t` by name, the others `\u00XY` in lower case), a quote, a backslash,
/// text that is not ASCII as it is, runs of plain bytes between them, an
/// empty text; the texts of the version of six `replaceAll` before it; the
/// interpreter and compiled.
#[test]
fn json_quotes_in_one_pass() {
    let src = "import * as json from \"json\";

action main(args: string[k]): i32 {
    print(join(map(iota(32), (c) => json.quote(stringOf([u8(65), u8(c), u8(66)]))), \" \"));
    print(json.quote(\"say \\\"hi\\\" \\\\ é日本\\n\"));
    print(json.quote(\"\"));
    print(json.quote(\"plain\"));
    return 0;
}
";
    let mut controls = Vec::new();
    for c in 0..32u8 {
        let e = match c {
            9 => "\\t".to_string(),
            10 => "\\n".to_string(),
            13 => "\\r".to_string(),
            _ => format!("\\u{:04x}", c),
        };
        controls.push(format!("\"A{}B\"", e));
    }
    let want = format!("{}\n\"say \\\"hi\\\" \\\\ é日本\\n\"\n\"\"\n\"plain\"\n", controls.join(" "));
    assert_eq!(interp_ok(src, &[]), want);
    let bin = compile(src, "json_quotes");
    let r = run_bin(&bin, &[]);
    assert_eq!((r.code, r.stdout.as_str()), (0, want.as_str()), "{}", r.stderr);
}

/// D398: `time.format` in one pass from left to right: the texts of the
/// eight `replaceAll` of before it (fields repeated or overlapping their
/// own letters, `ZZ`, lower case not fields, an empty pattern, text that is
/// not ASCII); the interpreter and compiled.
#[test]
fn time_formats_in_one_pass() {
    let src = "import * as time from \"time\";

action main(args: string[k]): i32 {
    const d = time.toDateTime(1767225600000000000 + 3 * 997123456789 + 3 * 7919, 90);
    print(join(map([\"YYYY-MM-DD HH:mm:ss.SSS Z\", \"YYYYY\", \"MMM\", \"ZZ\", \"SSSS\", \"sss\", \"\", \"plain text\", \"D DD DDD\", \"mmMMmm\", \"S SS SSS\", \"yyyy-mm-dd\", \"[YYYY]\", \"été YYYY 日本\"], (p) => time.format(d, p)), \"|\"));
    return 0;
}
";
    let want = "2026-01-01 02:19:51.370 +01:30|2026Y|01M|+01:30+01:30|370S|51s||plain text|D 01 01D|190119|S SS 370|yyyy-19-dd|[2026]|été 2026 日本\n";
    assert_eq!(interp_ok(src, &[]), want);
    let bin = compile(src, "time_formats");
    let r = run_bin(&bin, &[]);
    assert_eq!((r.code, r.stdout.as_str()), (0, want), "{}", r.stderr);
}

/// D399: `stringOf(b)` of bytes at their last use takes their block once
/// they are checked as UTF-8: texts made of new bytes, of an empty array,
/// in a recursion that makes one each turn; bytes still read after it
/// copied (not taken); bytes that are not UTF-8 the same panic at the same
/// place; the interpreter and compiled.
#[test]
fn string_of_takes_the_bytes_at_their_last_use() {
    let src = "function word(i: i64): string {
    const b = map(iota(i % 7 + 1), (j) => u8(97 + (i + j) % 26));
    return stringOf(b);
}

function words(i: i64, n: i64, acc: i64): i64 {
    return i == n ? acc : words(i + 1, n, acc + word(i).length);
}

action main(args: string[k]): i32 {
    const n = parseI64(args[0]);
    print(words(0, n, 0));
    const kept = map(iota(5), (j) => u8(65 + j));
    const t = stringOf(kept);
    print(`${t} ${kept[4]}`);
    const e: u8[] = [];
    print(`[${stringOf(e)}]`);
    print(join(map(iota(3), (i) => word(i * 5)), \",\"));
    const bad = map(iota(3), (j) => u8(n > 0 ? 255 : 65));
    print(stringOf(bad));
    return 0;
}
";
    let bin = compile(src, "string_of_takes");
    for args in [["0"], ["1000"]] {
        let want = interp(src, &args);
        let r = run_bin(&bin, &args);
        match &want {
            Outcome::Exit(code, out) => assert_eq!((r.code, r.stdout.as_str()), (*code, out.as_str()), "{:?}: {}", args, r.stderr),
            Outcome::Panic(m, out) => {
                assert_eq!((r.code, r.stdout.as_str()), (101, out.as_str()), "{:?}: {}", args, r.stderr);
                assert!(r.stderr.contains(m.as_str()), "{:?}: {} / {}", args, r.stderr, m);
            }
        }
    }
}
