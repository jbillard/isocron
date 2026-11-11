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
    // one run of the examples at a time on the machine: some write in the
    // same folder of `/tmp` (the example of `sys`), and the suite runs the
    // test in two processes at once (its variants isc and glibc)
    let lock = std::fs::File::create(std::env::temp_dir().join("isocron-docs-examples.lock")).unwrap();
    lock.lock().unwrap();
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
                if p.sys_paths[2].iter().any(|(op, _, _)| !op.on_wasi()) {
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

/// The fenced blocks of `text`: their language, the line of their fence,
/// their text.
fn fences(text: &str) -> Vec<(String, usize, String)> {
    let mut out = Vec::new();
    let mut open: Option<(String, usize, String)> = None;
    for (i, l) in text.lines().enumerate() {
        match (&mut open, l.strip_prefix("```")) {
            (None, Some(lang)) => open = Some((lang.trim().to_string(), i + 1, String::new())),
            (Some(_), Some(_)) => out.push(open.take().unwrap()),
            (Some((_, _, t)), None) => {
                t.push_str(l);
                t.push('\n');
            }
            (None, None) => {}
        }
    }
    out
}

/// A command of a block of the README, run by `sh` in `dir` with `iscc` on
/// the `PATH`: its exit code and its output (both streams).
fn readme_command(dir: &std::path::Path, cmd: &str) -> (i32, String) {
    let bin = std::path::Path::new(env!("CARGO_BIN_EXE_iscc")).parent().unwrap().to_path_buf();
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default());
    let o = std::process::Command::new("sh").arg("-c").arg(format!("{} 2>&1", cmd)).current_dir(dir).env("PATH", path).output().unwrap();
    (o.status.code().unwrap_or(-1), String::from_utf8_lossy(&o.stdout).into_owned())
}

/// The server of the README (`http.serve`), interpreted then compiled, on a
/// free port (8080 may be taken): its first line, `hello world` for
/// `/?world`, and a second copy on the same port that says why it cannot
/// serve and exits with 1.
fn readme_server(dir: &std::path::Path, name: &str, src: &str, first: &str) {
    use std::io::{BufRead, Read, Write};
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let file = format!("{}_{}.isc", name, port);
    std::fs::write(dir.join(&file), src.replace("8080", &port.to_string())).unwrap();
    let (code, out) = readme_command(dir, &format!("iscc build {} -o {}_bin", file, name));
    assert_eq!(code, 0, "{}", out);
    let bin = std::path::Path::new(env!("CARGO_BIN_EXE_iscc"));
    let ways: [(&std::path::Path, Vec<String>); 2] = [(bin, vec!["run".into(), file.clone()]), (&dir.join(format!("{}_bin", name)), vec![])];
    for (prog, args) in ways.iter() {
        let mut server = std::process::Command::new(prog).args(args).current_dir(dir).stdout(std::process::Stdio::piped()).spawn().unwrap();
        let mut lines = std::io::BufReader::new(server.stdout.take().unwrap());
        let mut line = String::new();
        lines.read_line(&mut line).unwrap();
        assert_eq!(line.trim_end(), first.replace("8080", &port.to_string()), "{:?} {:?}", prog, args);
        let mut stream = loop {
            if let Ok(s) = std::net::TcpStream::connect(("127.0.0.1", port)) {
                break s;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        };
        stream.write_all(b"GET /?world HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n").unwrap();
        let mut reply = String::new();
        stream.read_to_string(&mut reply).unwrap();
        assert!(reply.starts_with("HTTP/1.1 200") && reply.ends_with("\r\n\r\nhello world"), "{:?}: {}", prog, reply);
        let second = std::process::Command::new(prog).args(args).current_dir(dir).output().unwrap();
        assert_eq!(second.status.code(), Some(1));
        assert_eq!(
            String::from_utf8_lossy(&second.stdout),
            format!("{}\ncannot serve: already exists: 0.0.0.0:{}\n", first.replace("8080", &port.to_string()), port)
        );
        server.kill().unwrap();
        server.wait().unwrap();
    }
}

/// D348: every example of the README is run, not only checked. A program
/// is written under the name its first comment or the command after it
/// gives; the commands of the ```console block after it are run and their
/// output compared with the lines under them; a ```sh block after it, each
/// command run; a function alone (no `main`) is type-checked; the server is
/// started, asked, and stopped (`readme_server`).
#[test]
fn every_example_of_the_readme_works() {
    let readme = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../README.md")).unwrap();
    let blocks = fences(&readme);
    let dir = tmp("readme_examples");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut ran = 0;
    for (k, (lang, line, src)) in blocks.iter().enumerate() {
        if lang != "ts" {
            continue;
        }
        let next = blocks.get(k + 1).filter(|b| (b.0 == "console" || b.0 == "sh") && b.1 <= line + src.lines().count() + 4);
        let named = src.lines().next().and_then(|l| l.strip_prefix("// ")).and_then(|l| l.split(".isc").next().filter(|_| l.contains(".isc")));
        let from_cmd = next.and_then(|b| b.2.split_whitespace().find(|w| w.ends_with(".isc")).map(|w| w.trim_end_matches(".isc")));
        let name = named.or(from_cmd).map(str::to_string).unwrap_or_else(|| format!("readme_{}", line));
        let mut text = src.clone();
        if !text.contains("action main") {
            text.push_str("\naction main(args: string[k]): i32 {\n    return 0;\n}\n");
        }
        std::fs::write(dir.join(format!("{}.isc", name)), &text).unwrap();
        ran += 1;
        if text.contains("http.serve(") {
            let first = next.and_then(|b| b.2.lines().nth(1)).expect("the first line of the server");
            readme_server(&dir, &name, &text, first);
            continue;
        }
        match next {
            Some((kind, _, cmds)) if kind == "console" => {
                let mut lines = cmds.lines().peekable();
                while let Some(l) = lines.next() {
                    let cmd = l.strip_prefix("$ ").unwrap_or_else(|| panic!("README line {}: {}", line, l));
                    let cmd = cmd.split(" #").next().unwrap().trim();
                    let mut want = String::new();
                    while let Some(o) = lines.peek().filter(|o| !o.starts_with("$ ")) {
                        want.push_str(o);
                        want.push('\n');
                        lines.next();
                    }
                    let (code, out) = readme_command(&dir, cmd);
                    if cmd.starts_with("iscc check") && !want.is_empty() {
                        assert_ne!(code, 0, "README line {}: `{}` accepted", line, cmd);
                    } else {
                        assert_eq!(code, 0, "README line {}: `{}`: {}", line, cmd, out);
                    }
                    assert_eq!(out, want, "README line {}: `{}`", line, cmd);
                }
            }
            Some((_, _, cmds)) => {
                for l in cmds.lines() {
                    let cmd = l.split(" #").next().unwrap().trim();
                    let (code, out) = readme_command(&dir, cmd);
                    assert_eq!(code, 0, "README line {}: `{}`: {}", line, cmd, out);
                    if cmd.ends_with(&format!("./{}", name)) || cmd.starts_with("iscc run") {
                        assert!(!out.is_empty(), "README line {}: `{}` printed nothing", line, cmd);
                    }
                }
            }
            None => {
                let (code, out) = readme_command(&dir, &format!("iscc check {}.isc", name));
                assert_eq!(code, 0, "README line {}: {}", line, out);
            }
        }
    }
    assert!(ran >= 5, "only {} examples in the README", ran);
}
