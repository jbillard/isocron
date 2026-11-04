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
