//! Modules (DECISIONS D139): `import m` reads `m.isc` next to the importing
//! file or in `$ISC_PATH`; its declarations are `m.f`, `m.T`, `m.C`; inside
//! the module, its own names first; each module read once; cycles, missing
//! modules and a `main` in a module are errors that name the right file.

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
type Shape = Circle(f64) | Square(f64);

function sq(x: f64): f64 {
    return x * x;
}

function area(s: Shape): f64 {
    return match (s) {
        Circle(r) => 3.0 * sq(r),
        Square(c) => sq(c),
    };
}

// the module's own `sum`: the prelude's elsewhere
function sum(x: f64[n]): f64 {
    return 100.0;
}

function total(x: f64[n]): f64 {
    return sum(x);
}

function unit(): Shape {
    return Square(1.0);
}
";

const STATS: &str = "import * as geom from \"./geom\";

function both(a: geom.Shape, b: geom.Shape): f64 {
    return geom.area(a) + geom.area(b);
}

function is_square(s: geom.Shape): boolean {
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
    write(&d, "a.isc", "import * as b from \"./b\";\n\nfunction f(): i64 {\n    return 1;\n}\n");
    write(&d, "b.isc", "import * as a from \"./a\";\n\nfunction g(): i64 {\n    return 2;\n}\n");
    let e = error(&d, "cyc.isc", "import * as a from \"./a\";\n\naction main(args: string[k]): i32 {\n    print(a.f());\n    return 0;\n}\n");
    assert!(e.contains("b.isc:1:13: error: import cycle: a -> b -> a"), "{}", e);
    let e = error(&d, "nf.isc", "import * as nowhere from \"./nowhere\";\n\naction main(args: string[k]): i32 {\n    return 0;\n}\n");
    assert!(e.contains("nf.isc:1:13: error: module `nowhere` not found"), "{}", e);
    write(&d, "withmain.isc", "action main(args: string[k]): i32 {\n    return 0;\n}\n");
    let e = error(&d, "usemain.isc", "import * as withmain from \"./withmain\";\n\naction main(args: string[k]): i32 {\n    return 0;\n}\n");
    assert!(e.contains("withmain.isc:1:8: error: the module `withmain` declares `main`"), "{}", e);
    let e = error(&d, "late.isc", "function h(): i64 {\n    return 1;\n}\nimport * as a from \"./a\";\n");
    assert!(e.contains("late.isc:4:1: error: `import` comes before the other declarations"), "{}", e);
    write(&d, "broken.isc", "function bad(): i64 {\n    return 1.5 + true;\n}\n");
    let e = error(&d, "usebroken.isc", "import * as broken from \"./broken\";\n\naction main(args: string[k]): i32 {\n    print(broken.bad());\n    return 0;\n}\n");
    assert!(e.contains("broken.isc:2:18: error:"), "{}", e);
    // a name of the module that it does not declare
    write(&d, "small.isc", "function one(): i64 {\n    return 1;\n}\n");
    let e = error(&d, "missing.isc", "import * as small from \"./small\";\n\naction main(args: string[k]): i32 {\n    print(small.two());\n    return 0;\n}\n");
    assert!(e.contains("small.two"), "{}", e);
}

#[test]
fn modules_found_through_isc_path() {
    let d = dir("modules_path");
    let lib = d.join("lib");
    std::fs::create_dir_all(&lib).unwrap();
    write(&lib, "util.isc", "function twice(x: i64): i64 {\n    return 2 * x;\n}\n");
    let src = "import * as util from \"./util\";\n\naction main(args: string[k]): i32 {\n    print(util.twice(21));\n    return 0;\n}\n";
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
