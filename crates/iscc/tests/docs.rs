//! Every example of `docs/language.md` is checked here, so that the guide
//! cannot drift from the compiler. Fenced blocks:
//!   ```isc        a program: run by the interpreter and compiled; both must
//!                 exit with 0 and print the same output
//!   ```isc-check  a program that must type-check (it needs input files)
//!   ```isc-error  a program that must be rejected at compile time
//!   ```isc-panic  a program that must panic, compiled and interpreted
//! An ```output block right after a program gives its expected standard
//! output (for `isc-error`: a part of the error message; for `isc-panic`: the
//! panic message). An ```input block between them is the standard input of
//! the program (not run as WebAssembly then). A block ```isc-file NAME is a
//! module, written next to the ```isc program that follows it.

mod common;
use common::*;

struct Example {
    kind: String,
    line: usize,
    src: String,
    expected: Option<String>,
    input: Option<String>,
}

fn examples(doc: &str) -> Vec<Example> {
    let mut blocks: Vec<(String, usize, String, usize)> = Vec::new(); // (lang, start line, text, end line)
    let lines: Vec<&str> = doc.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        if let Some(lang) = lines[i].strip_prefix("```") {
            let start = i + 1;
            let mut j = start;
            while j < lines.len() && !lines[j].starts_with("```") {
                j += 1;
            }
            let text = lines[start..j].iter().map(|l| format!("{}\n", l)).collect::<String>();
            blocks.push((lang.trim().to_string(), start, text, j));
            i = j + 1;
        } else {
            i += 1;
        }
    }
    let mut out = Vec::new();
    for (k, (lang, start, text, end)) in blocks.iter().enumerate() {
        if !lang.starts_with("isc") {
            continue;
        }
        // the block right after `end`, if only blank lines separate them
        let next = |after: usize, k2: usize, want: &str| {
            blocks.get(k2).and_then(|(l2, s2, t2, e2)| {
                let between = &lines[after + 1..s2 - 1];
                (l2 == want && between.iter().all(|l| l.trim().is_empty())).then(|| (t2.clone(), *e2))
            })
        };
        let input = next(*end, k + 1, "input");
        let expected = match &input {
            Some((_, e)) => next(*e, k + 2, "output").map(|x| x.0),
            None => next(*end, k + 1, "output").map(|x| x.0),
        };
        out.push(Example { kind: lang.clone(), line: *start, src: text.clone(), expected, input: input.map(|x| x.0) });
    }
    out
}

fn run_with_input(bin: &std::path::Path, input: &str) -> (i32, String, String) {
    use std::io::Write as _;
    let mut c = std::process::Command::new(bin)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    c.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
    let o = c.wait_with_output().unwrap();
    (o.status.code().unwrap_or(-1), String::from_utf8_lossy(&o.stdout).to_string(), String::from_utf8_lossy(&o.stderr).to_string())
}

#[test]
fn every_example_of_the_guide_works() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/language.md");
    let doc = std::fs::read_to_string(path).unwrap();
    let exs = examples(&doc);
    assert!(exs.len() >= 10, "only {} examples found", exs.len());
    // the examples also run as WebAssembly when a WASI engine is installed
    let wasm = wasi_runner();
    if wasm.is_none() {
        eprintln!("no WASI engine (wasmtime): the examples are not run as WebAssembly");
    }
    let mut files: Vec<(String, String)> = Vec::new();
    for (n, ex) in exs.iter().enumerate() {
        let at = format!("docs/language.md:{}", ex.line);
        if let Some(name) = ex.kind.strip_prefix("isc-file ") {
            files.push((name.trim().to_string(), ex.src.clone()));
            continue;
        }
        // a program with its modules: from its directory
        if ex.kind == "isc" && !files.is_empty() {
            let d = tmp(&format!("doc_modules_{}", n));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(&d).unwrap();
            for (name, text) in files.drain(..) {
                std::fs::write(d.join(name), text).unwrap();
            }
            let main = d.join("main.isc");
            std::fs::write(&main, &ex.src).unwrap();
            let p = iscc::frontend(&main.to_string_lossy(), &ex.src).unwrap_or_else(|e| panic!("{}: {}", at, e));
            let mut out = Vec::new();
            assert_eq!(isc_interp::run(&p, &[], &mut out).unwrap(), 0, "{}", at);
            let out = String::from_utf8(out).unwrap();
            let bin = tmp(&format!("doc_modules_{}_bin", n));
            iscc::build(&p, &iscc::BuildOptions { output: bin.clone(), ..Default::default() }).unwrap();
            assert_eq!(run_bin(&bin, &[]).stdout, out, "{}: compiled and interpreted outputs differ", at);
            if let Some(e) = &ex.expected {
                assert_eq!(&out, e, "{}: output", at);
            }
            continue;
        }
        match ex.kind.as_str() {
            "isc" => {
                *isc_interp::rt::STDIN.lock().unwrap() = Some(ex.input.clone().unwrap_or_default().into_bytes());
                let out = match interp(&ex.src, &[]) {
                    Outcome::Exit(0, out) => out,
                    Outcome::Exit(c, out) => panic!("{}: exit {} ({})", at, c, out),
                    Outcome::Panic(m, _) => panic!("{}: unexpected panic: {}", at, m),
                };
                let bin = compile(&ex.src, &format!("doc_{}", n));
                let r = run_with_input(&bin, ex.input.as_deref().unwrap_or(""));
                assert_eq!(r.0, 0, "{}: {}", at, r.2);
                assert_eq!(r.1, out, "{}: compiled and interpreted outputs differ", at);
                // a program that reaches what WASI lacks (sockets): refused
                // when built for it (D324)
                let p = iscc::frontend("doc.isc", &ex.src).unwrap_or_else(|e| panic!("{}: {}", at, e));
                if p.sys_paths.iter().any(|(op, _, _)| !op.on_wasi()) {
                    let o = isc_codegen::Options { isa: isc_codegen::Isa::Wasm32, ..isc_codegen::Options::default() };
                    let opts = iscc::BuildOptions { output: tmp(&format!("doc_{}.wasm", n)), emit_llvm: None, emit_asm: None, debug: false, codegen: o, link: Vec::new(), glibc: false };
                    let e = iscc::build(&p, &opts).expect_err("a program that WASI cannot run, built for it");
                    assert!(e.contains("(WASI preview 1) has no"), "{}: {}", at, e);
                } else if let (Some(runner), None) = (&wasm, &ex.input) {
                    let w = compile_wasm(&ex.src, &format!("doc_{}", n));
                    let r = run_wasm(runner, &w, &[]);
                    assert_eq!(r.code, 0, "{}: WebAssembly: {}", at, r.stderr);
                    assert_eq!(r.stdout, out, "{}: WebAssembly and interpreted outputs differ", at);
                }
                if let Some(e) = &ex.expected {
                    assert_eq!(&out, e, "{}: output", at);
                }
            }
            "isc-check" => {
                if let Err(e) = iscc::frontend("doc.isc", &ex.src) {
                    panic!("{}: {}", at, e);
                }
            }
            "isc-error" => match iscc::frontend("doc.isc", &ex.src) {
                Ok(_) => panic!("{}: compiled, but an error is expected", at),
                Err(e) => {
                    let want = ex.expected.as_deref().unwrap_or("").trim();
                    assert!(e.contains(want), "{}: error `{}` does not contain `{}`", at, e, want);
                }
            },
            "isc-panic" => {
                let want = ex.expected.as_deref().expect("an isc-panic block needs an output block").trim();
                match interp(&ex.src, &[]) {
                    Outcome::Panic(m, _) => assert!(m.contains(want), "{}: interpreter: `{}`", at, m),
                    Outcome::Exit(c, _) => panic!("{}: exit {} instead of a panic", at, c),
                }
                let bin = compile(&ex.src, &format!("doc_{}", n));
                let r = run_bin(&bin, &[]);
                assert_eq!(r.code, 101, "{}: exit code", at);
                assert!(r.stderr.contains(want), "{}: compiled: `{}`", at, r.stderr);
            }
            k => panic!("{}: unknown block kind `{}`", at, k),
        }
    }
}
