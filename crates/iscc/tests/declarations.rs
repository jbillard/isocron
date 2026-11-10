//! Type aliases and constants of the top level (D159): `type X = <type>;`
//! is the type itself, as in TypeScript (generic, of a record, a tuple, a
//! function, a `Map`, an array whose size variable becomes one of the
//! signature); `const X = literal;` and `const X: T = value;` are values,
//! exported and imported as functions; the errors.

mod common;
use common::*;

const DECL: &str = r##"type Point = { x: f64, y: f64 };
type Pair<T> = [T, T];
type Op = (a: f64, b: f64) => f64;
type Counts = Map<string, i64>;
type Row = f64[n];

const N = 3;
const HALF = 0.5;
const NAME = "isocron";
const ON = true;
const PRIMES = [2, 3, 5, 7];
const ROOT2: f64 = Math.sqrt(2.0);
const ORIGIN: Point = { x: 0.0, y: 0.0 };

function norm(p: Point): f64 {
    return Math.sqrt(p.x * p.x + p.y * p.y);
}

function swap<T>(p: Pair<T>): Pair<T> {
    const [a, b] = p;
    return [b, a] as const;
}

function apply(f: Op, a: f64, b: f64): f64 {
    return f(a, b);
}

function scale(r: Row, k: f64): Row {
    return r * k;
}

function total(c: Counts): i64 {
    return sum(c.values());
}

action main(args: string[k]): i32 {
    print(norm({ x: 3.0, y: 4.0 }));
    print(norm(ORIGIN));
    print(swap([1, 2] as const));
    print(swap(["a", "b"] as const));
    print(apply((a, b) => a * b + HALF, 2.0, 3.0));
    print(scale([1.0, 2.0], 2.0));
    print(total(countBy(["a", "b", "a"], (w) => w)));
    print(fill(N, NAME));
    print(ON && PRIMES[3] == 7);
    print(sum(PRIMES));
    print(ROOT2 * ROOT2);
    const N = 10;
    print(N);
    return 0;
}
"##;

const SHAPES: &str = r##"export type Point = { x: f64, y: f64 };
export const ORIGIN: Point = { x: 0.0, y: 0.0 };
export const UNIT = 1.0;

export function dist(a: Point, b: Point): f64 {
    return Math.sqrt((a.x - b.x) ** 2.0 + (a.y - b.y) ** 2.0);
}
"##;

const MAIN: &str = r##"import { Point, ORIGIN, dist } from "./shapes";
import * as shapes from "./shapes";

type Path = Point[n];

function length(p: Path): f64 {
    return sum(map(iota(p.length - 1), (i) => dist(p[i], p[i + 1])));
}

action main(args: string[k]): i32 {
    const p: Point = { x: 3.0, y: 4.0 };
    print(dist(ORIGIN, p));
    print(shapes.UNIT + shapes.dist(shapes.ORIGIN, p));
    print(length([ORIGIN, p, { x: 3.0, y: 0.0 }]));
    return 0;
}
"##;

fn valgrind_clean(bin: &std::path::Path) {
    checked_clean_bin(bin, &[], None);
    let Some(vg) = valgrind_path() else {
        return;
    };
    let out = std::process::Command::new(vg)
        .args(["--error-exitcode=9", "--leak-check=full", "--errors-for-leak-kinds=definite,indirect", "-q"])
        .arg(bin)
        .env("ISC_ALLOC", "glibc")
        .env("ISC_THREADS", "2")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "valgrind: {}", String::from_utf8_lossy(&out.stderr));
}

#[test]
fn aliases_and_constants() {
    let want = "5\n0\n(2, 1)\n(b, a)\n6.5\n[2, 4]\n3\n[isocron, isocron, isocron]\ntrue\n17\n2.0000000000000004\n10\n";
    assert_eq!(interp_ok(DECL, &[]), want, "interpreter");
    let bin = compile(DECL, "declarations");
    assert_eq!(run_bin(&bin, &[]).stdout, want, "compiled");
    // Valgrind knows AVX2, not AVX-512
    let opts = isc_codegen::Options { isa: isc_codegen::Isa::Avx2, ..isc_codegen::Options::default() };
    valgrind_clean(&compile_with(DECL, "declarations_vg", opts));
}

#[test]
fn aliases_and_constants_of_modules() {
    let d = tmp("declarations_modules");
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("shapes.isc"), SHAPES).unwrap();
    let main = d.join("main.isc");
    std::fs::write(&main, MAIN).unwrap();
    let p = iscc::frontend(&main.to_string_lossy(), MAIN).unwrap();
    let mut out = Vec::new();
    assert_eq!(isc_interp::run(&p, &[], &mut out).unwrap(), 0);
    assert_eq!(String::from_utf8(out).unwrap(), "5\n6\n9\n");
}

#[test]
fn declaration_errors() {
    for (decls, want) in [
        ("const X = Math.sqrt(2.0);", "1:7: error: give `X` its type: `const X: T = ...` (only a literal, or an array of literals, has a type of its own)"),
        ("const X: i64 = Y;\nconst Y: i64 = X;", "1:7: error: the constant `X` depends on itself"),
        ("type A = [B, i64];\ntype B = { a: A };", "1:10: error: the type alias `A` contains itself"),
        ("type P<T> = [T, T];\nfunction f(p: P): i64 {\n    return 1;\n}", "2:15: error: the type `P` takes 1 type argument(s), but 0 were given"),
        ("let X = 1;", "1:1: error: a program declares functions, actions, types and constants (`const`); `let` goes inside a body"),
        ("const X: string = 3;", "1:19: error: expected an integer literal and a numeric type (class Num), found `string`"),
    ] {
        let src = format!("{}\n\naction main(args: string[k]): i32 {{\n    return 0;\n}}\n", decls);
        match iscc::frontend("t.isc", &src) {
            Ok(_) => panic!("accepted: {}", decls),
            Err(e) => assert!(e.contains(want), "{}\n{}", decls, e),
        }
    }
}
