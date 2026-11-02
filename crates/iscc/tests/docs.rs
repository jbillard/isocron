//! Every example of `docs/language.md` is checked here, so that the guide
//! cannot drift from the compiler. Fenced blocks:
//!   ```isc        a program: run by the interpreter and compiled; both must
//!                 exit with 0 and print the same output
//!   ```isc-check  a program that must type-check (it needs input files)
//!   ```isc-error  a program that must be rejected at compile time
//!   ```isc-panic  a program that must panic, compiled and interpreted
//! An ```output block right after a program gives its expected standard
//! output (for `isc-error`: a part of the error message; for `isc-panic`: the
//! panic message).

mod common;
use common::*;

struct Example {
    kind: String,
    line: usize,
    src: String,
    expected: Option<String>,
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
        let expected = blocks.get(k + 1).and_then(|(l2, s2, t2, _)| {
            let between = &lines[end + 1..s2 - 1];
            (l2 == "output" && between.iter().all(|l| l.trim().is_empty())).then(|| t2.clone())
        });
        out.push(Example { kind: lang.clone(), line: *start, src: text.clone(), expected });
    }
    out
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
    for (n, ex) in exs.iter().enumerate() {
        let at = format!("docs/language.md:{}", ex.line);
        match ex.kind.as_str() {
            "isc" => {
                let out = match interp(&ex.src, &[]) {
                    Outcome::Exit(0, out) => out,
                    Outcome::Exit(c, out) => panic!("{}: exit {} ({})", at, c, out),
                    Outcome::Panic(m, _) => panic!("{}: unexpected panic: {}", at, m),
                };
                let bin = compile(&ex.src, &format!("doc_{}", n));
                let r = run_bin(&bin, &[]);
                assert_eq!(r.code, 0, "{}: {}", at, r.stderr);
                assert_eq!(r.stdout, out, "{}: compiled and interpreted outputs differ", at);
                if let Some(runner) = &wasm {
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
