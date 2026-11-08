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
    let vg = std::path::PathBuf::from("/usr/bin/valgrind");
    if vg.is_file() {
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
    let vg = std::path::PathBuf::from("/usr/bin/valgrind");
    if vg.is_file() {
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
