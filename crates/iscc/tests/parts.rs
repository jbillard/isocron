//! D711: a large module compiled in parts at once, each part a module of
//! its own (`isc_codegen::split`), linked together: the same program. The
//! parts made smaller here (`ISC_SPLIT_LINES`) so that small programs are
//! cut too: recursion and nodes (binary-trees), tasks whose functions are
//! given by their address (spectral-norm), texts and tables (fasta).

use std::path::{Path, PathBuf};

fn build(src: &Path, out: &Path, env: &[(&str, &str)]) -> String {
    let mut c = std::process::Command::new(env!("CARGO_BIN_EXE_iscc"));
    c.arg("build").arg(src).arg("-o").arg(out).env("ISC_TIME", "1").env_remove("ISC_SPLIT_LINES").env_remove("ISC_CODEGEN_THREADS");
    for (k, v) in env {
        c.env(k, v);
    }
    let o = c.output().unwrap();
    let err = String::from_utf8_lossy(&o.stderr).to_string();
    assert!(o.status.success(), "{}: {}", src.display(), err);
    err
}

fn run(bin: &Path, args: &[&str]) -> String {
    let o = std::process::Command::new(bin).args(args).output().unwrap();
    assert!(o.status.success(), "{}: {}", bin.display(), String::from_utf8_lossy(&o.stderr));
    String::from_utf8_lossy(&o.stdout).to_string()
}

fn dir(name: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn a_module_in_parts_is_the_same_program() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let d = dir("parts");
    for (name, args) in [("binarytrees", vec!["10"]), ("spectralnorm", vec!["120"]), ("fasta", vec!["2500"])] {
        let src = root.join(format!("examples/benchmarks_game/{}.isc", name));
        let whole = d.join(format!("{}_whole", name));
        let cut = d.join(format!("{}_parts", name));
        let w = build(&src, &whole, &[("ISC_CODEGEN_THREADS", "1")]);
        assert!(!w.contains("parts"), "{}: cut with one thread: {}", name, w);
        let c = build(&src, &cut, &[("ISC_SPLIT_LINES", "400"), ("ISC_CODEGEN_THREADS", "3")]);
        assert!(c.contains("module cut into 3 parts"), "{}: not cut: {}", name, c);
        assert_eq!(run(&cut, &args), run(&whole, &args), "{}", name);
        // no part left next to the program
        assert!(std::fs::read_dir(&d).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().contains(".p1.")), "a part left");
    }
}

#[test]
fn the_parts_kept_by_the_object_cache() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let d = dir("parts_cache");
    let cache = d.join("cache");
    let _ = std::fs::remove_dir_all(&cache);
    let src = root.join("examples/benchmarks_game/binarytrees.isc");
    let env = [("ISC_SPLIT_LINES", "400"), ("ISC_CODEGEN_THREADS", "2"), ("ISC_OBJECT_CACHE", cache.to_str().unwrap())];
    let a = d.join("first");
    let b = d.join("second");
    let made = build(&src, &a, &env);
    assert!(made.contains("module cut into 2 parts"), "{}", made);
    // the second from the cache: no part compiled, the same program
    let again = build(&src, &b, &env);
    assert!(!again.contains("module cut"), "{}", again);
    assert_eq!(run(&b, &["8"]), run(&a, &["8"]));
    let names: Vec<String> = std::fs::read_dir(&cache).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    assert!(names.iter().any(|n| n.ends_with(".p1.o")) && names.iter().any(|n| n.ends_with(".parts")), "{:?}", names);
}
