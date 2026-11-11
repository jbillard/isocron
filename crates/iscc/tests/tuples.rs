//! Arrays of tuples, stored in columns (SPEC section 4; DECISIONS D69):
//! compiled programs give the interpreter's output, natively with 1, 3 and 8
//! threads, and as WebAssembly.

mod common;
use common::*;

const PROG: &str = "function pick(a: [f64, i64], b: [f64, i64]): [f64, i64] {
    const [va, ia] = a;
    const [vb, ib] = b;
    if (vb < va) {
        return b;
    } else {
        return a;
    }
}

function indexed(x: f64[n]): [f64, i64][n] {
    return map(iota(n), (i) => [x[i], i] as const);
}

function add3(a: [i64, f64, boolean], b: [i64, f64, boolean]): [i64, f64, boolean] {
    const [i, x, p] = a;
    const [j, y, q] = b;
    return [i + j, x + y, p && q] as const;
}

action main(args: string[k]): i32 {
    const x = readF64(args[0]);
    const pairs = indexed(x);
    print(reduce(pairs, pick, [1e308, -1] as const));
    const mixed = map(x, (v) => [u8(v * 100.0), v, v > 0.0] as const);
    print(mixed[0]);
    print(mixed[3 * 4096 + 1]);
    const [c, s, all] = reduce(map(mixed, (m) => {
        const [a, b, p] = m;
        return [i64(a), b, p] as const;
    }), add3, [0, 0.0, true] as const);
    print([c, s, all] as const);
    print(fold(mixed, 0, (acc, m) => {
        const [a, b, p] = m;
        if (p) {
            return acc + i64(a);
        } else {
            return acc - 1;
        }
    }));
    const changed = mixed.with(7, [255u8, -2.5, false] as const);
    print(changed[7]);
    print(changed == mixed);
    print(mixed == mixed);
    return 0;
}
";

#[test]
fn arrays_of_tuples_match_interpreter() {
    let n = 3 * 4096 + 5;
    let x: Vec<f64> = random_f64s(41, n).into_iter().map(|v| v.abs()).collect();
    let px = write_f64s("tup_x.f64", &x);
    let want = interp_ok(PROG, &[&px]);
    assert!(want.lines().count() == 8, "{}", want);
    let bin = compile(PROG, "tup_prog");
    for t in [1, 3, 8] {
        let r = run_bin_threads(&bin, &[&px], Some(t));
        assert_eq!(r.code, 0, "{}", r.stderr);
        assert_eq!(r.stdout, want, "{} threads", t);
    }
    if let Some(runner) = wasi_runner() {
        let w = compile_wasm(PROG, "tup_prog");
        let r = run_wasm(&runner, &w, &[&px]);
        assert_eq!(r.code, 0, "{}", r.stderr);
        assert_eq!(r.stdout, want, "WebAssembly");
    }
}

/// D434: the value of a `let` that is a component owning memory of a new
/// tuple that dies with it (a field read out of a record a function made:
/// `textResponse(200, s).body`): the component moved out without a copy,
/// the others freed on their own. Components of bytes, of an array of
/// tuples of strings, of an array of integers, of a string; scalars beside; the
/// component read in a parallel `map`, in a recursive action and kept in an
/// array; the interpreter's output on 1, 3 and 8 threads; nothing lost or
/// freed twice (valgrind).
#[test]
fn a_component_read_out_of_a_new_tuple_is_moved() {
    let src = "type Resp = { status: i64, headers: [string, string][], body: u8[], rows: i64[], note: string };

function make(i: i64, s: string): Resp {
    return { status: 200 + i % 3, headers: [[\"Content-Type\", \"text/plain\"] as const, [\"X-Id\", `${i}`] as const], body: bytes(s), rows: [i, i + 1, i * 2], note: `note ${i}` };
}

function bodyOf(i: i64, s: string): u8[] {
    return make(i, s).body;
}

function noteOf(i: i64): string {
    return make(i, \"x\").note;
}

function rowsOf(i: i64): i64[] {
    return make(i, \"y\").rows;
}

action walk(i: i64, n: i64, acc: i64): i64 {
    if (i == n) {
        return acc;
    }
    const b = bodyOf(i, `walk ${i}`);
    return walk(i + 1, n, acc + b.length + i64(b[0]));
}

action main(args: string[k]): i32 {
    const n = parseI64(args[0]);
    const texts = map(iota(n), (i) => `hello number ${1000000 + i}`);
    print(sum(map(iota(n), (i) => make(i, texts[i]).body.length)));
    print(sum(map(iota(n), (i) => bodyOf(i, texts[i]).length * (i % 7 + 1))));
    const kept = map(iota(n), (i) => bodyOf(i, texts[i]));
    print(sum(map(kept, (b) => i64(b[b.length - 1]))));
    print(join(map(iota(5), (i) => noteOf(i)), \",\"));
    print(sum(map(iota(n), (i) => sum(rowsOf(i)))));
    print(walk(0, n, 0));
    return 0;
}
";
    let ir = iscc::frontend("take.isc", src).unwrap();
    let text = format!("{:?}", ir);
    assert!(text.contains("Take("), "no component taken out of a tuple");
    for n in ["0", "1", "5", "5000"] {
        let Outcome::Exit(0, want) = interp(src, &[n]) else { panic!("the interpreter failed for {}", n) };
        let bin = compile(src, "take_component");
        for t in [1, 3, 8] {
            let r = run_bin_threads(&bin, &[n], Some(t));
            assert_eq!(r.code, 0, "n {} T={}: {}", n, t, r.stderr);
            assert_eq!(r.stdout, want, "n {} T={}", n, t);
        }
    }
    let vg = valgrind_path();
    if let Some(vg) = vg {
        let opts = isc_codegen::Options { isa: isc_codegen::Isa::Avx2, ..isc_codegen::Options::default() };
        let bin = compile_glibc(src, "take_component_vg", opts);
        let out = std::process::Command::new(vg)
            .args(["--error-exitcode=9", "--leak-check=full", "--errors-for-leak-kinds=definite,indirect", "-q"])
            .arg(&bin)
            .arg("300")
            .env("ISC_ALLOC", "glibc")
            .env("ISC_THREADS", "2")
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(0), "under valgrind:\n{}", String::from_utf8_lossy(&out.stderr));
    }
}

/// D504: a tuple made by an `if` whose two branches make new tuples (a call,
/// a tuple literal) is new memory: taken apart (`Take`) where it is
/// destructured, each component owned on its own, instead of borrowed from
/// the tuple and copied where it is kept (the field of a record carried by
/// the loop of the decoder of `json`, copied at each member). A text carried
/// by a recursive function and replaced on some turns; an array, a text and
/// an integer out of an `if` in a `map`; the interpreter's output on 1, 3
/// and 8 threads; nothing lost or freed twice (valgrind).
#[test]
fn a_tuple_made_by_an_if_is_taken_apart() {
    let src = "function word(i: i64): [string, i64] {
    return [`word ${i}`, i * 3] as const;
}

function cells(i: i64): [i64[], string, i64] {
    return [[i, i + 1, i + 2], `cell ${i}`, i] as const;
}

function carry(i: i64, n: i64, name: string, total: i64): [string, i64] {
    if (i == n) {
        return [name, total] as const;
    }
    const [s, k] = i % 3 == 0 ? word(i) : [name, -1] as const;
    const t = total + k + s.length;
    return carry(i + 1, n, s, t);
}

function k0(i: i64): i64 {
    return i * 7;
}

function pick(i: i64): string {
    const [xs, s, k] = i % 2 == 0 ? cells(i) : [[k0(i)], `odd ${i}`, i] as const;
    return `${s}:${sum(xs)}:${k}`;
}

action main(args: string[k]): i32 {
    const n = parseI64(args[0]);
    const [last, total] = carry(0, n, \"none\", 0);
    print(`${last} ${total}`);
    print(join(map(iota(n % 50), (i) => pick(i)), \",\"));
    print(sum(map(iota(n), (i) => pick(i).length)));
    return 0;
}
";
    let ir = iscc::frontend("if_tuple.isc", src).unwrap();
    let carry = ir.fns.iter().find(|f| f.name == "carry").expect("no function carry");
    assert!(format!("{:?}", carry.body).contains("Take("), "the tuple of the `if` of `carry` is not taken apart");
    // `pick` inlined in both `map`s of `main`: its three components taken
    // twice, beside the two of `[last, total]`
    let takes = format!("{:?}", ir.body).matches("Take(").count();
    assert!(takes >= 8, "{} parts taken in `main`: the tuples of the `if` of `pick` are not taken apart", takes);
    for n in ["0", "1", "7", "5000"] {
        let Outcome::Exit(0, want) = interp(src, &[n]) else { panic!("the interpreter failed for {}", n) };
        let bin = compile(src, "if_tuple");
        for t in [1, 3, 8] {
            let r = run_bin_threads(&bin, &[n], Some(t));
            assert_eq!(r.code, 0, "n {} T={}: {}", n, t, r.stderr);
            assert_eq!(r.stdout, want, "n {} T={}", n, t);
        }
    }
    let vg = valgrind_path();
    if let Some(vg) = vg {
        let opts = isc_codegen::Options { isa: isc_codegen::Isa::Avx2, ..isc_codegen::Options::default() };
        let bin = compile_glibc(src, "if_tuple_vg", opts);
        let out = std::process::Command::new(vg)
            .args(["--error-exitcode=9", "--leak-check=full", "--errors-for-leak-kinds=definite,indirect", "-q"])
            .arg(&bin)
            .arg("300")
            .env("ISC_ALLOC", "glibc")
            .env("ISC_THREADS", "2")
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(0), "under valgrind:\n{}", String::from_utf8_lossy(&out.stderr));
    }
}

/// D585: a bracket literal first in a choice whose place says neither
/// array nor tuple (a `const` without a type, a destructuring, the
/// `return` of a lambda) takes the form of the other branch, as it does
/// second: a tuple where the call is first and where it is second, in a
/// nested choice, in a choice of literals, destructured, in the early
/// `return` of a lambda; arrays where the other branch is an array; the
/// interpreter's output on 1, 3 and 8 threads; branches that disagree
/// refused.
#[test]
fn a_bracket_literal_first_in_a_choice_takes_the_other_form() {
    let src = "function g(n: i64): [u64, u64] {
    return [u64(n) * 3u64, u64(n) + 7u64] as const;
}

function h(n: i64): [u64, u64] {
    return [u64(n) + 100u64, 5u64] as const;
}

function g2(n: i64): [i64, u8] {
    return [n * 10, u8(n % 200)] as const;
}

function show(p: [u64, u64]): string {
    const [a, b] = p;
    return `${a}:${b}`;
}

function row(i: i64): string {
    const t1 = i == 0 ? [0u64, 1u64] : g(i);
    const t2 = i != 0 ? g(i) : [0u64, 1u64];
    const t3 = i == 0 ? [0u64, 1u64] : i % 2 == 0 ? g(i) : h(i);
    const t4 = i < 2 ? (i == 0 ? [0u64, 1u64] : [1u64, 1u64]) : h(i);
    const [p, q] = i % 3 == 0 ? [1, 2u8] : g2(i);
    const early = (j: i64) => {
        if (j % 4 == 1) {
            const z = u64(j);
            return [z, z];
        }
        return h(j);
    };
    const xs = i == 0 ? [1, 2, 3] : iota(i);
    const ys = i > 1 ? [i, 2 * i] : [7];
    return `${show(t1)} ${show(t2)} ${show(t3)} ${show(t4)} ${p}:${q} ${show(early(i))} ${sum(xs)} ${sum(ys)}`;
}

action main(args: string[k]): i32 {
    const n = parseI64(args[0]);
    print(join(map(iota(n), (i) => row(i)), \"\\n\"));
    return 0;
}
";
    let want = interp_ok(src, &["6"]);
    let first = want.lines().next().unwrap_or("");
    assert_eq!(first, "0:1 0:1 0:1 0:1 1:2 100:5 6 7", "{}", want);
    assert_eq!(want.lines().count(), 6, "{}", want);
    let bin = compile(src, "choice_literal_first");
    for n in ["0", "1", "6", "40"] {
        let want = interp_ok(src, &[n]);
        for t in [1, 3, 8] {
            let r = run_bin_threads(&bin, &[n], Some(t));
            assert_eq!(r.code, 0, "n {} T={}: {}", n, t, r.stderr);
            assert_eq!(r.stdout, want, "n {} T={}", n, t);
        }
    }
    // branches that disagree: refused, the literal first or the call first
    for (choice, want) in [
        ("n == 3 ? [1, 2u8] : x", "type mismatch: expected `u8[2]`, found `f64`"),
        ("n == 3 ? g2(n) : x", "type mismatch: expected `[i64, u8]`, found `f64`"),
        ("n == 3 ? [1, 2u8, 3] : g2(n)", "type mismatch: expected `u8[3]`, found `[i64, u8]`"),
    ] {
        let bad = format!(
            "function g2(n: i64): [i64, u8] {{\n    return [n * 10, u8(n % 200)] as const;\n}}\n\naction main(args: string[k]): i32 {{\n    const n = parseI64(args[0]);\n    const x = parseF64(args[0]);\n    const v = {};\n    return 0;\n}}\n",
            choice
        );
        let e = iscc::frontend("bad.isc", &bad).unwrap_err();
        assert!(e.contains(want), "wanted `{}` in:\n{}", want, e);
    }
}

/// D634: a tuple of scalars made and read inside the element computation of
/// a `map` (returned by a function inlined there, chosen by a condition,
/// built and taken apart) no longer keeps the loop scalar: each part is a
/// vector (the roots of the IR are vector instructions); the results are
/// those of the interpreter, with 1 and 8 threads.
#[test]
fn tuples_inside_vector_code() {
    let src = "function polar(x: f32, y: f32): [f32, f32] {\n    return [sqrt(x * x + y * y), min(x, y)] as const;\n}\n\naction main(args: string[k]): i32 {\n    const n = i64(args.length);\n    const r = map(iota(4096), (j) => {\n        const px = f32(j % 64 + n) + 0.5f32;\n        const py = f32(j / 64) + 0.25f32;\n        const [a, b] = px > 30.0f32 ? polar(px, py) : [py, 1.0f32] as const;\n        const t = [a * 2.0f32, b - 1.0f32, a > b] as const;\n        const [u, v, w] = t;\n        return w ? u + v : u - v;\n    });\n    print(sum(r));\n    print(r[4095]);\n    print(map(iota(8), (i) => r[i * 511]));\n    return 0;\n}\n";
    let want = interp_ok(src, &["a"]);
    let bin = compile(src, "tuples_vector");
    for t in [1, 8] {
        let r = run_bin_threads(&bin, &["a"], Some(t));
        assert_eq!((r.code, r.stdout.as_str()), (0, want.as_str()), "compiled, {} threads: {}", t, r.stderr);
    }
    let p = iscc::frontend("t.isc", src).unwrap_or_else(|e| panic!("{}", e));
    let ir = isc_codegen::emit_with(&p, &isc_codegen::Options::default()).unwrap();
    assert!(ir.contains("@llvm.sqrt.v") && ir.contains("x float> }"), "the tuple of the element computation in vectors");
}

/// D642: `{ ...r, f: v }` whose new array `v` has another length than the
/// field of `r` (a record made by a function inlined, its array a literal
/// whose length the types fixed, D204): the record holds `v` whole, read
/// with its own length (the tuple of the update had the type of `r`, its
/// length fixed: the new array read as one of the old length). A record of
/// a variable, a literal of another length, a record of records, a record
/// literal whose type has the length of its array (refused before: `size
/// mismatch`), a tuple of arrays; the results of the interpreter, for
/// lengths below, at and above the old one.
#[test]
fn a_field_replaced_by_an_array_of_another_length() {
    let src = "type M = { items: i64[], draft: string, n: i64 };
type Outer = { inner: M, tag: string };
function initial(): M {
    return { items: [1, 2], draft: \"\", n: 0 };
}
action main(args: string[k]): i32 {
    const k = parseI64(args[0]);
    const m = { ...initial(), items: map(iota(k), (i) => i * 10) };
    print(m.items);
    print(m.items.length);
    const a = { ...initial(), items: [7, 8, 9] };
    print(a.items);
    const base = initial();
    const b = { ...base, items: map(iota(k), (i) => i + 1) };
    print([b.items.length, base.items.length] as const);
    const o: Outer = { inner: initial(), tag: \"t\" };
    const o2 = { ...o, inner: { ...o.inner, items: fill(k, 4) } };
    print(o2.inner.items);
    const r = { items: [1, 2], n: 0 };
    const s = { ...r, items: [1, 2, 3] };
    const t = { ...r, items: map(iota(k), (i) => i) };
    print([s.items.length, t.items.length, r.items.length] as const);
    const p = { pair: [r.items, [5, 6, 7]] as const, n: 1 };
    const q = { ...p, pair: [[9], fill(k, 0)] as const };
    print(q.pair);
    return 0;
}
";
    let bin = compile(src, "field_other_length");
    for a in ["0", "1", "2", "5"] {
        let want = interp_ok(src, &[a]);
        let r = run_bin(&bin, &[a]);
        assert_eq!((r.code, r.stdout.as_str()), (0, want.as_str()), "k = {}: {}", a, r.stderr);
    }
    assert_eq!(
        interp_ok(src, &["5"]),
        "[0, 10, 20, 30, 40]\n5\n[7, 8, 9]\n(5, 2)\n[4, 4, 4, 4, 4]\n(3, 5, 2)\n([9], [0, 0, 0, 0, 0])\n"
    );
}
