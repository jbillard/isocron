//! The module of the prelude compiled once (D611): the functions of the
//! prelude that are never inlined (the rare paths of the allocator and of
//! the texts of numbers), compiled once for a compiler and a target, kept
//! in a directory, and linked with every program instead of being compiled
//! in each.

mod common;
use common::*;
use std::path::{Path, PathBuf};

/// A program that reaches the texts of floats, `toFixed`, the cases and
/// the allocator.
const TEXTS: &str = "action main(args: string[k]): i32 {
    const x = parseF64(args[0]);
    print(toString(x) + \" \" + x.toFixed(2) + \" \" + (x * 1e300 * 1e300).toFixed(1) + \" \" + args[1].toUpperCase());
    print(x / 3.0);
    print([toString(args.length).padStart(4, \"0\"), toString(toString(5e-324).length), (2.0 ** 60.0).toFixed(3), 0.1.toFixed(30)]);
    return 0;
}
";

const TEXTS_OUT: &str = "2.5 2.50 Infinity ÉTÉ\n0.8333333333333334\n[0002, 326, 1152921504606846976.000, 0.100000000000000005551115123126]\n";

/// A directory of its own for the modules of a test, empty.
fn cache(name: &str) -> PathBuf {
    let d = tmp(name);
    std::fs::create_dir_all(&d).unwrap();
    for e in std::fs::read_dir(&d).unwrap().flatten() {
        std::fs::remove_file(e.path()).unwrap();
    }
    d
}

/// The program built with its modules compiled once kept in `dir` (none:
/// every function in the program): the executable and the text of its IR.
fn built(src: &str, name: &str, dir: Option<&Path>) -> (PathBuf, String) {
    let p = iscc::frontend(&format!("{}.isc", name), src).unwrap_or_else(|e| panic!("{}", e));
    let out = tmp(name);
    let ll = out.with_extension("ll");
    let opts = iscc::BuildOptions {
        output: out.clone(),
        emit_llvm: Some(ll.clone()),
        emit_asm: None,
        debug: false,
        codegen: isc_codegen::Options::default(),
        link: Vec::new(),
        glibc: false,
    };
    iscc::build_in(&p, &opts, dir).unwrap_or_else(|e| panic!("{}: {}", name, e));
    (out, std::fs::read_to_string(&ll).unwrap())
}

/// The files of a directory of modules: the objects and their lists.
fn modules(dir: &Path) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let mut all: Vec<PathBuf> = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.path()).collect();
    all.sort();
    let of = |ext: &str| all.iter().filter(|p| p.extension().is_some_and(|e| e == ext)).cloned().collect::<Vec<_>>();
    assert_eq!(of("o").len() + of("txt").len(), all.len(), "{:?}", all);
    (of("o"), of("txt"))
}

/// The functions never inlined that a module defines itself.
fn rare_defined(ir: &str) -> Vec<&str> {
    ir.lines().filter(|l| l.starts_with("define internal ") && l.contains(" cold noinline")).collect()
}

/// The first build makes the module, the next ones read it; a program
/// declares the rare paths of the prelude and defines none; its output is
/// the one of the program that holds them all.
#[test]
fn made_once_and_read_again() {
    let dir = cache("pre_once");
    let (bin, ir) = built(TEXTS, "pre_once_a", Some(&dir));
    let (objects, lists) = modules(&dir);
    assert_eq!((objects.len(), lists.len()), (1, 1));
    let made = std::fs::metadata(&objects[0]).unwrap().modified().unwrap();
    for name in ["isc.pre.kRefill", "isc.pre.kSpill", "isc.pre.kCarve", "isc.pre.fixedRest", "isc.pre.rareF64Text", "isc.kernel.start"] {
        assert!(ir.lines().any(|l| l.starts_with("declare hidden ") && l.contains(&format!("@{}(", name))), "{} is not declared", name);
        assert!(!ir.lines().any(|l| l.starts_with("define ") && l.contains(&format!("@{}(", name))), "{} is defined", name);
    }
    assert_eq!(rare_defined(&ir), Vec::<&str>::new());
    // a panic of the allocator never returns: the program knows it
    assert!(ir.lines().any(|l| l.contains("@isc.pre.kDamaged(") && l.contains("noreturn")), "what LLVM proved of a function is not in its declaration");
    let r = run_bin(&bin, &["2.5", "été"]);
    assert_eq!((r.code, r.stdout.as_str()), (0, TEXTS_OUT), "{}", r.stderr);
    // the same program whole
    let (whole, whole_ir) = built(TEXTS, "pre_once_whole", None);
    assert!(!whole_ir.contains("@isc.pre."));
    assert!(!rare_defined(&whole_ir).is_empty());
    let w = run_bin(&whole, &["2.5", "été"]);
    assert_eq!((w.code, w.stdout.as_str()), (0, TEXTS_OUT), "{}", w.stderr);
    // another program: the module is read, not made again
    let (hello, hello_ir) = built("action main(args: string[k]): i32 {\n    print(`hello ${args[0]}`);\n    return 0;\n}\n", "pre_once_b", Some(&dir));
    let (again, _) = modules(&dir);
    assert_eq!(again, objects);
    assert_eq!(std::fs::metadata(&objects[0]).unwrap().modified().unwrap(), made);
    assert!(hello_ir.contains("declare hidden ") && hello_ir.contains("@isc.pre.kRefill("));
    assert_eq!(run_bin(&hello, &["you"]).stdout, "hello you\n");
}

/// A program takes from the module what it reaches, nothing else: the
/// executable of a hello is not larger by the texts of floats.
#[test]
fn a_program_carries_what_it_reaches() {
    let dir = cache("pre_size");
    let src = "action main(args: string[k]): i32 {\n    print(`hello ${args[0]}`);\n    return 0;\n}\n";
    let (with, _) = built(src, "pre_size_with", Some(&dir));
    let (without, _) = built(src, "pre_size_without", None);
    let (a, b) = (std::fs::metadata(&with).unwrap().len(), std::fs::metadata(&without).unwrap().len());
    // (seen once, in the pass of glibc of a suite, D623: 182 600 bytes
    // against 112 968, not again in 16 runs; what the module was then)
    let state = || -> String {
        std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| format!("{} {} bytes", e.file_name().to_string_lossy(), e.metadata().map(|m| m.len()).unwrap_or(0)))
            .collect::<Vec<_>>()
            .join(", ")
    };
    assert!(a <= b + 4096, "{} bytes with the module, {} without; the directory of the module: {}; ISC_ALLOC {:?}", a, b, state(), std::env::var("ISC_ALLOC"));
    // the tables of the texts of floats: the module's, not twice
    let (with, ir) = built(TEXTS, "pre_size_texts", Some(&dir));
    let (without, _) = built(TEXTS, "pre_size_texts_whole", None);
    assert!(ir.lines().any(|l| l.starts_with("@isc.pre.k.") && l.contains(" = linkonce_odr hidden ")), "no table under a name made from its value");
    let (a, b) = (std::fs::metadata(&with).unwrap().len(), std::fs::metadata(&without).unwrap().len());
    assert!(a <= b + 16384, "{} bytes with the module, {} without", a, b);
}

/// The names of the module are names of LLVM, not bytes of the texts of a
/// program: a text that holds one is printed as written.
#[test]
fn a_text_that_holds_a_name() {
    let dir = cache("pre_names");
    let src = "action main(args: string[k]): i32 {
    print(\"mail@.digits2 @isc_fn_3 @isc.pre.kRefill( @.pow10\");
    print(`${args[0]}@.pow10 ${parseF64(args[0]) / 3.0}`);
    return 0;
}
";
    let (bin, ir) = built(src, "pre_names_bin", Some(&dir));
    assert!(ir.contains("declare hidden "));
    let r = run_bin(&bin, &["2.5"]);
    assert_eq!((r.code, r.stdout.as_str()), (0, "mail@.digits2 @isc_fn_3 @isc.pre.kRefill( @.pow10\n2.5@.pow10 0.8333333333333334\n"), "{}", r.stderr);
}

/// No place to keep the module (a directory that cannot be made): every
/// program compiles what it reaches, as without it.
#[test]
fn a_place_that_cannot_be_written() {
    let file = tmp("pre_not_a_directory");
    std::fs::write(&file, "a file").unwrap();
    let (bin, ir) = built(TEXTS, "pre_no_place", Some(&file.join("modules")));
    assert!(!ir.contains("@isc.pre."));
    let r = run_bin(&bin, &["2.5", "été"]);
    assert_eq!((r.code, r.stdout.as_str()), (0, TEXTS_OUT), "{}", r.stderr);
}

/// A list that is not one (cut, or of another compiler): the module is made
/// again, not read.
#[test]
fn a_list_that_is_not_one() {
    let dir = cache("pre_list");
    built(TEXTS, "pre_list_a", Some(&dir));
    let (_, lists) = modules(&dir);
    let good = std::fs::read_to_string(&lists[0]).unwrap();
    assert!(isc_codegen::PreManifest::from_text(&good).is_some());
    std::fs::write(&lists[0], "kernel\tperhaps\n").unwrap();
    // (a compiler started now: what this process read before is not asked;
    // the library keeps the lists it read, so the list is damaged for
    // another directory of the same content)
    let other = cache("pre_list_other");
    for f in std::fs::read_dir(&dir).unwrap().flatten() {
        std::fs::copy(f.path(), other.join(f.file_name())).unwrap();
    }
    let (bin, ir) = built(TEXTS, "pre_list_b", Some(&other));
    assert!(ir.contains("@isc.pre.kRefill("));
    let (_, lists) = modules(&other);
    assert_eq!(std::fs::read_to_string(&lists[0]).unwrap(), good);
    let r = run_bin(&bin, &["2.5", "été"]);
    assert_eq!((r.code, r.stdout.as_str()), (0, TEXTS_OUT), "{}", r.stderr);
}

/// The texts moved to Isocron (D626): the rare paths of `replace`,
/// `replaceAll` and `lastIndexOf`, whose tasks are named from their
/// function and not from their place in a program, and the reading of a
/// CSV text (`@noInline`), the module's in a program whose own tasks come
/// first; its output that of the whole program.
#[test]
fn the_moved_texts_from_the_module() {
    let src = "action main(args: string[k]): i32 {
    const s = args[0];
    print(sum(map(iota(parseI64(args[1])), (i) => i % 7)));
    print(s.replaceAll(args[2], args[3]) + \"|\" + s.replace(args[2], args[3]));
    print(s.lastIndexOf(args[2]));
    print(Number(args[1]));
    print(parseCsvF64(args[4], 0));
    return 0;
}
";
    let dir = cache("pre_texts");
    let (bin, ir) = built(src, "pre_texts_a", Some(&dir));
    for name in ["isc.pre.replaceSlow", "isc.pre.replaceLong", "isc.pre.twoWayLast", "isc.pre.csvRead"] {
        assert!(ir.lines().any(|l| l.starts_with("declare hidden ") && l.contains(&format!("@{}(", name))), "{} is not declared", name);
        assert!(!ir.lines().any(|l| l.starts_with("define ") && l.contains(&format!("@{}(", name))), "{} is defined", name);
    }
    let (whole, _) = built(src, "pre_texts_whole", None);
    for args in [["a$b$c-$$-abcabc", "100000", "$", "[$&]", "1,2\n3,4\n"], ["xyzxyz", "12", "yz", "$'", "1, 2.5\n"]] {
        let (r, w) = (run_bin(&bin, &args), run_bin(&whole, &args));
        assert_eq!((r.code, r.stdout.as_str()), (0, w.stdout.as_str()), "{:?}: {}", args, r.stderr);
        assert_eq!(w.code, 0, "{:?}: {}", args, w.stderr);
    }
}

/// The manifest as text and back.
#[test]
fn the_list_as_text() {
    let p = iscc::pre_program().unwrap();
    let (_, m) = isc_codegen::emit_pre(&p, &isc_codegen::Options::default()).unwrap();
    assert!(m.kernel && m.fns.len() >= 10, "{:?}", m.fns.keys().collect::<Vec<_>>());
    assert_eq!(isc_codegen::PreManifest::from_text(&m.to_text()), Some(m));
    assert_eq!(isc_codegen::PreManifest::from_text(""), None);
    assert_eq!(isc_codegen::PreManifest::from_text("kernel\ttrue\nfn\tone\ttwo\n"), None);
}

/// A function that the program would make from another text than the
/// module's stays in the program; when it is one of the allocator, whose
/// words are the module's or the program's, never both, the program takes
/// nothing from the module.
#[test]
fn a_function_that_differs_stays_in_the_program() {
    let opts = isc_codegen::Options::default();
    let (_, m) = isc_codegen::emit_pre(&iscc::pre_program().unwrap(), &opts).unwrap();
    let p = iscc::frontend("pre_differs.isc", TEXTS).unwrap();
    let declared = |ir: &str, name: &str| ir.lines().any(|l| l.starts_with("declare hidden ") && l.contains(&format!("@{}(", name)));
    let (ir, using) = isc_codegen::emit_using(&p, &opts, &m).unwrap();
    assert!(using && declared(&ir, "isc.pre.fixedRest") && declared(&ir, "isc.pre.kRefill"));
    assert_eq!(rare_defined(&ir), Vec::<&str>::new());
    // a text function of another making
    let mut other = m.clone();
    other.fns.get_mut("isc.pre.fixedRest").unwrap().0.push('0');
    let (ir, using) = isc_codegen::emit_using(&p, &opts, &other).unwrap();
    assert!(using && !declared(&ir, "isc.pre.fixedRest") && declared(&ir, "isc.pre.kRefill"));
    assert_eq!(rare_defined(&ir).len(), 1, "{:?}", rare_defined(&ir));
    // a function of the allocator of another making: nothing shared
    let whole = isc_codegen::emit_with(&p, &opts).unwrap();
    for name in ["isc.pre.kRefill", "isc.kernel.start"] {
        let mut other = m.clone();
        other.fns.get_mut(name).unwrap().0.push('0');
        let (ir, using) = isc_codegen::emit_using(&p, &opts, &other).unwrap();
        assert!(!using, "{}", name);
        assert_eq!(ir, whole, "{}", name);
    }
    // a module without the allocator: the same
    let mut other = m.clone();
    other.kernel = false;
    let (ir, using) = isc_codegen::emit_using(&p, &opts, &other).unwrap();
    assert!(!using);
    assert_eq!(ir, whole);
}

/// Builds at the same time in one process, the module not made yet: made
/// once, every program right.
#[test]
fn builds_at_the_same_time() {
    let dir = cache("pre_threads");
    let runs: Vec<_> = (0..4)
        .map(|i| {
            let dir = dir.clone();
            std::thread::spawn(move || {
                let (bin, _) = built(TEXTS, &format!("pre_threads_{}", i), Some(&dir));
                run_bin(&bin, &["2.5", "été"])
            })
        })
        .collect();
    for r in runs {
        let r = r.join().unwrap();
        assert_eq!((r.code, r.stdout.as_str()), (0, TEXTS_OUT), "{}", r.stderr);
    }
    let (objects, lists) = modules(&dir);
    assert_eq!((objects.len(), lists.len()), (1, 1));
}

/// The same for WebAssembly, where the allocator is the runtime's: the rare
/// paths of the texts are the module's.
#[test]
fn for_webassembly() {
    let Some(runner) = wasi_runner() else {
        eprintln!("no WASI engine: skipped");
        return;
    };
    let dir = cache("pre_wasm");
    let p = iscc::frontend("pre_wasm.isc", TEXTS).unwrap();
    let out = tmp("pre_wasm.wasm");
    let ll = out.with_extension("ll");
    let mut codegen = isc_codegen::Options::default();
    codegen.isa = isc_codegen::Isa::Wasm32;
    let opts = iscc::BuildOptions { output: out.clone(), emit_llvm: Some(ll.clone()), emit_asm: None, debug: false, codegen, link: Vec::new(), glibc: false };
    iscc::build_in(&p, &opts, Some(&dir)).unwrap();
    let ir = std::fs::read_to_string(&ll).unwrap();
    assert!(ir.lines().any(|l| l.starts_with("declare hidden ") && l.contains("@isc.pre.fixedRest(")));
    assert!(!ir.contains("@isc.pre.kRefill("));
    let r = run_wasm(&runner, &out, &["2.5", "été"]);
    assert_eq!((r.code, r.stdout.as_str()), (0, TEXTS_OUT), "{}", r.stderr);
}
