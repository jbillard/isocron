//! Modules (DECISIONS D139, D150): `import * as m from "./m";` reads `m.isc`
//! relative to the importing file, a bare `"m"` in `$ISC_PATH`; what a
//! module exports is `m.f`, `m.T`, `m.C`, or imported by name (`import { f,
//! g as h } from "./m";`), after the local variables; inside the module,
//! its own names first; each file read once, whatever the path that names
//! it; cycles, missing modules, private names and a `main` in a module are
//! errors that name the right file.

mod common;
use common::*;

fn dir(name: &str) -> std::path::PathBuf {
    let d = tmp(name);
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn write(d: &std::path::Path, name: &str, src: &str) -> String {
    let p = d.join(name);
    std::fs::write(&p, src).unwrap();
    p.to_string_lossy().to_string()
}

const GEOM: &str = "// shapes
export type Shape = Circle(f64) | Square(f64);

function sq(x: f64): f64 {
    return x * x;
}

export function area(s: Shape): f64 {
    return match (s) {
        Circle(r) => 3.0 * sq(r),
        Square(c) => sq(c),
    };
}

// the module's own `sum` (private): the prelude's elsewhere
function sum(x: f64[n]): f64 {
    return 100.0;
}

export function total(x: f64[n]): f64 {
    return sum(x);
}

export function unit(): Shape {
    return Square(1.0);
}
";

const STATS: &str = "import * as geom from \"./geom\";

export function both(a: geom.Shape, b: geom.Shape): f64 {
    return geom.area(a) + geom.area(b);
}

export function is_square(s: geom.Shape): boolean {
    return match (s) {
        geom.Square(c) => true,
        _ => false,
    };
}
";

const MAIN: &str = "import * as geom from \"./geom\";
import * as stats from \"./stats\";

function area(x: i64): i64 {
    return x * 10;
}

action main(args: string[k]): i32 {
    const c: geom.Shape = geom.Circle(2.0);
    print(geom.area(c));
    print(stats.both(c, geom.unit()));
    print(stats.is_square(geom.Square(3.0)));
    print(stats.is_square(c));
    print(area(4));
    print(sum([1.0, 2.0]));
    print(geom.total([1.0, 2.0]));
    match (c) {
        geom.Circle(r) => print(r),
        geom.Square(s) => print(s),
    };
    return 0;
}
";

#[test]
fn modules_and_qualified_names() {
    let d = dir("modules_ok");
    write(&d, "geom.isc", GEOM);
    write(&d, "stats.isc", STATS);
    let main = write(&d, "main.isc", MAIN);
    let expected = "12\n13\ntrue\nfalse\n40\n3\n100\n2\n";
    let p = iscc::frontend(&main, MAIN).unwrap();
    let mut out = Vec::new();
    assert_eq!(isc_interp::run(&p, &[], &mut out).unwrap(), 0);
    assert_eq!(String::from_utf8(out).unwrap(), expected, "interpreter");
    let bin = tmp("modules_ok_bin");
    let opts = iscc::BuildOptions { output: bin.clone(), ..Default::default() };
    iscc::build(&p, &opts).unwrap();
    let r = run_bin(&bin, &[]);
    assert_eq!(r.stdout, expected, "compiled");
}

fn error(d: &std::path::Path, name: &str, src: &str) -> String {
    let path = write(d, name, src);
    match iscc::frontend(&path, src) {
        Ok(_) => panic!("{}: expected an error", name),
        Err(e) => e,
    }
}

#[test]
fn module_errors() {
    let d = dir("modules_err");
    write(&d, "a.isc", "import * as b from \"./b\";\n\nexport function f(): i64 {\n    return 1;\n}\n");
    write(&d, "b.isc", "import * as a from \"./a\";\n\nexport function g(): i64 {\n    return 2;\n}\n");
    let e = error(&d, "cyc.isc", "import * as a from \"./a\";\n\naction main(args: string[k]): i32 {\n    print(a.f());\n    return 0;\n}\n");
    assert!(e.contains("b.isc:1:20: error: import cycle: a -> b -> a"), "{}", e);
    let e = error(&d, "nf.isc", "import * as nowhere from \"./nowhere\";\n\naction main(args: string[k]): i32 {\n    return 0;\n}\n");
    assert!(e.contains("nf.isc:1:26: error: module \"./nowhere\" not found"), "{}", e);
    write(&d, "withmain.isc", "action main(args: string[k]): i32 {\n    return 0;\n}\n");
    let e = error(&d, "usemain.isc", "import * as withmain from \"./withmain\";\n\naction main(args: string[k]): i32 {\n    return 0;\n}\n");
    assert!(e.contains("withmain.isc:1:8: error: the module `withmain` declares `main`"), "{}", e);
    let e = error(&d, "late.isc", "function h(): i64 {\n    return 1;\n}\nimport * as a from \"./a\";\n");
    assert!(e.contains("late.isc:4:1: error: `import` comes before the other declarations"), "{}", e);
    write(&d, "broken.isc", "export function bad(): i64 {\n    return 1.5 + true;\n}\n");
    let e = error(&d, "usebroken.isc", "import * as broken from \"./broken\";\n\naction main(args: string[k]): i32 {\n    print(broken.bad());\n    return 0;\n}\n");
    assert!(e.contains("broken.isc:2:18: error:"), "{}", e);
    // what a module does not export (D150)
    write(&d, "priv.isc", "function hidden(): i64 {\n    return 1;\n}\n\nexport function shown(): i64 {\n    return hidden();\n}\n");
    let e = error(&d, "usepriv.isc", "import * as priv from \"./priv\";\n\naction main(args: string[k]): i32 {\n    print(priv.hidden());\n    return 0;\n}\n");
    assert!(e.contains("usepriv.isc:4:11: error: `priv.hidden`: the module \"./priv\" declares it but does not export it"), "{}", e);
    let e = error(&d, "usepriv2.isc", "import { shown, hidden } from \"./priv\";\n\naction main(args: string[k]): i32 {\n    return 0;\n}\n");
    assert!(e.contains("usepriv2.isc:1:17: error: `hidden` cannot be imported: the module \"./priv\" declares it but does not export it"), "{}", e);
    let e = error(&d, "usepriv3.isc", "import { shown } from \"./priv\";\n\nfunction shown(): i64 {\n    return 2;\n}\n\naction main(args: string[k]): i32 {\n    return 0;\n}\n");
    assert!(e.contains("usepriv3.isc:1:10: error: `shown` is both imported and declared in this file"), "{}", e);
    // a bare name is looked for in $ISC_PATH only
    let e = error(&d, "bare.isc", "import { shown } from \"priv\";\n\naction main(args: string[k]): i32 {\n    return 0;\n}\n");
    assert!(e.contains("module \"priv\" not found in $ISC_PATH; the file next to this one is \"./priv\""), "{}", e);
    // a name of the module that it does not declare
    write(&d, "small.isc", "export function one(): i64 {\n    return 1;\n}\n");
    let e = error(&d, "missing.isc", "import * as small from \"./small\";\n\naction main(args: string[k]): i32 {\n    print(small.two());\n    return 0;\n}\n");
    assert!(e.contains("small.two"), "{}", e);
}

#[test]
fn modules_found_through_isc_path() {
    let d = dir("modules_path");
    let lib = d.join("lib");
    std::fs::create_dir_all(&lib).unwrap();
    write(&lib, "util.isc", "export function twice(x: i64): i64 {\n    return 2 * x;\n}\n");
    let src = "import * as util from \"util\";\n\naction main(args: string[k]): i32 {\n    print(util.twice(21));\n    return 0;\n}\n";
    let main = write(&d, "main.isc", src);
    assert!(iscc::frontend(&main, src).is_err());
    // the only test of this file that reads `$ISC_PATH`
    std::env::set_var("ISC_PATH", lib.to_string_lossy().to_string());
    let p = iscc::frontend(&main, src).unwrap();
    std::env::remove_var("ISC_PATH");
    let mut out = Vec::new();
    assert_eq!(isc_interp::run(&p, &[], &mut out).unwrap(), 0);
    assert_eq!(String::from_utf8(out).unwrap(), "42\n");
}

/// Names imported by name, renamed, shadowed by a local variable; modules
/// in subdirectories; one file named by two paths is one module (D150).
#[test]
fn named_imports_and_subdirectories() {
    let d = dir("modules_named");
    std::fs::create_dir_all(d.join("lib/deep")).unwrap();
    write(&d, "geom.isc", GEOM);
    write(&d, "lib/deep/core.isc", "export function twice(x: i64): i64 {\n    return 2 * x;\n}\n");
    write(&d, "lib/util.isc", "import { twice as two } from \"./deep/core\";\nimport { Shape, unit } from \"../geom\";\n\nexport function twice(x: i64): i64 {\n    return two(x);\n}\n\nexport function one(): Shape {\n    return unit();\n}\n");
    let src = "import { area, Shape, Circle as Disc, Square } from \"./geom\";
import * as util from \"./lib/util\";
import * as g from \"./lib/../geom\";

function scale(s: Shape, k: f64): Shape {
    return match (s) {
        Disc(r) => Disc(r * k),
        Square(c) => Square(c * k),
    };
}

action main(args: string[k]): i32 {
    const c: Shape = Disc(1.0);
    print(area(scale(c, 2.0)));
    print(g.area(util.one()));
    print(util.twice(21));
    // the same type through both paths
    print(area(g.unit()) == g.area(Square(1.0)));
    const area = 5;
    print(area);
    return 0;
}
";
    let main = write(&d, "main.isc", src);
    let expected = "12\n1\n42\ntrue\n5\n";
    let p = iscc::frontend(&main, src).unwrap();
    let mut out = Vec::new();
    assert_eq!(isc_interp::run(&p, &[], &mut out).unwrap(), 0);
    assert_eq!(String::from_utf8(out).unwrap(), expected, "interpreter");
    let bin = tmp("modules_named_bin");
    let opts = iscc::BuildOptions { output: bin.clone(), ..Default::default() };
    iscc::build(&p, &opts).unwrap();
    assert_eq!(run_bin(&bin, &[]).stdout, expected, "compiled");
}
