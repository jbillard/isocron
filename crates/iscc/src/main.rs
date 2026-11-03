//! `iscc`, the Isocron compiler driver.
//!
//! ```text
//! iscc check <file.isc> [--report]          type-check (and print the memory report)
//! iscc ir    <file.isc>                     print the optimized IR
//! iscc run   [--lanes V] <file.isc> [args...]   run with the reference interpreter
//! iscc build <file.isc> [-o out] [--emit-llvm f.ll] [--emit-asm f.s] [--debug] [--report]
//!            [--lanes V] [--isa native|sse|avx|avx2|avx512] [--target native|wasm32-wasi|wasm32-wasi-threads]
//! ```
//!
//! `--lanes` changes V, the number of accumulators of the fixed reduction
//! form (SPEC section 10), for experiments only: the language fixes V = 32
//! and results depend on V.
//!
//! `--color auto|always|never` (anywhere, `auto` by default): the messages
//! in colors, with the line of the source under an error, when the error
//! output is a terminal and `NO_COLOR` is not set (D231); the same text as
//! without colors otherwise.

use std::io::IsTerminal;
use std::process::exit;

/// The colors of the messages of `iscc` (D231): ANSI codes, or nothing.
struct Style {
    on: bool,
}

impl Style {
    /// `--color` taken out of `args`; `auto`: the error output is a
    /// terminal and `NO_COLOR` is not set (https://no-color.org).
    fn from_args(args: &mut Vec<String>) -> Style {
        let mut choice = "auto".to_string();
        let mut i = 0;
        while i < args.len() {
            if let Some(v) = args[i].strip_prefix("--color=") {
                choice = v.to_string();
                args.remove(i);
            } else if args[i] == "--color" && i + 1 < args.len() {
                choice = args[i + 1].clone();
                args.drain(i..i + 2);
            } else {
                i += 1;
            }
        }
        let on = match choice.as_str() {
            "always" => true,
            "never" => false,
            "auto" => std::io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty()),
            _ => {
                eprintln!("error: --color must be auto, always or never");
                exit(2)
            }
        };
        Style { on }
    }

    fn paint(&self, code: &str, s: &str) -> String {
        if self.on {
            format!("\x1b[{}m{}\x1b[0m", code, s)
        } else {
            s.to_string()
        }
    }

    /// `error: message` of the driver.
    fn error(&self, msg: &str) -> String {
        format!("{}: {}", self.paint("1;31", "error"), msg)
    }

    /// The messages of the front end (`file:line:col: error: message`,
    /// then notes indented): the place in bold, `error` in red, the notes
    /// in cyan; in colors, the line of the source and a caret under the
    /// column (the file `path`, whose text is `src`, or another file of the
    /// program, read again).
    fn diagnostics(&self, msg: &str, path: &str, src: &str) -> String {
        let mut out = Vec::new();
        for line in msg.lines() {
            let Some((place, text)) = line.split_once(": error: ") else {
                out.push(if line.starts_with("  ") { self.paint("36", line) } else { line.to_string() });
                continue;
            };
            out.push(format!("{}: {}: {}", self.paint("1", place), self.paint("1;31", "error"), self.paint("1", text)));
            if !self.on {
                continue;
            }
            let mut parts = place.rsplitn(3, ':');
            let (col, row, file) = (parts.next(), parts.next(), parts.next());
            let (Some(Ok(col)), Some(Ok(row)), Some(file)) = (col.map(str::parse::<usize>), row.map(str::parse::<usize>), file) else { continue };
            let other;
            let text = if file == path {
                src
            } else {
                match std::fs::read_to_string(file) {
                    Ok(t) => {
                        other = t;
                        &other
                    }
                    Err(_) => continue,
                }
            };
            let Some(code) = text.lines().nth(row.saturating_sub(1)) else { continue };
            let num = row.to_string();
            let pad = " ".repeat(num.len());
            // the caret under the column: the characters before it as spaces,
            // the tabs kept
            let before: String = code.chars().take(col.saturating_sub(1)).map(|c| if c == '\t' { '\t' } else { ' ' }).collect();
            out.push(format!("{} {}", pad, self.paint("1;34", "|")));
            out.push(format!("{} {} {}", self.paint("1;34", &num), self.paint("1;34", "|"), code));
            out.push(format!("{} {} {}{}", pad, self.paint("1;34", "|"), before, self.paint("1;31", "^")));
        }
        out.join("\n")
    }
}

fn usage() -> ! {
    eprintln!(
        "usage (--color auto|always|never anywhere):\n  iscc check <file.isc> [--report]\n  iscc ir <file.isc>\n  iscc run [--lanes V] [--link lib]... <file.isc> [args...]\n  iscc build <file.isc> [-o out] [--emit-llvm file.ll] [--emit-asm file.s] [--debug] [--report] [--lanes V] [--isa native|sse|avx|avx2|avx512] [--target native|wasm32-wasi|wasm32-wasi-threads] [--link lib]..."
    );
    exit(2)
}

fn lanes_arg(s: &str) -> u32 {
    match s.parse::<u32>() {
        Ok(v) if v >= 2 && v.is_power_of_two() && 4096 % v == 0 => v,
        _ => {
            eprintln!("error: --lanes must be a power of two between 2 and 4096");
            exit(2)
        }
    }
}

fn main() {
    let started = std::time::Instant::now();
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let style = Style::from_args(&mut args);
    if args.len() < 2 {
        usage();
    }
    let cmd = args.remove(0);
    let mut run_lanes = isc_interp::LANES as u32;
    // the options of `run`, before the file: V, the libraries of the
    // `extern` functions (D125)
    while cmd == "run" && matches!(args.first().map(|a| a.as_str()), Some("--lanes" | "--link")) {
        if args.len() < 3 {
            usage();
        }
        if args[0] == "--lanes" {
            run_lanes = lanes_arg(&args[1]);
        } else if let Err(m) = isc_interp::ffi::load_library(&args[1]) {
            eprintln!("{}", style.error(&m.to_string()));
            exit(2)
        }
        args.drain(0..2);
    }
    let path = args[0].clone();
    let src = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{}", style.error(&format!("cannot read `{}`: {}", path, e)));
            exit(2)
        }
    };
    let prog = match iscc::frontend(&path, &src) {
        Ok(p) => p,
        Err(msg) => {
            eprintln!("{}", style.diagnostics(&msg, &path, &src));
            exit(1)
        }
    };
    // the memory report (SPEC section 8, D98): text on the standard output,
    // JSON next to the output of `build`
    let report = args.iter().any(|a| a == "--report") && matches!(cmd.as_str(), "check" | "build");
    if report {
        args.retain(|a| a != "--report");
        let es = isc_ir::report::entries(&prog);
        print!("{}", isc_ir::report::text(&path, &es));
    }
    match cmd.as_str() {
        "check" => {}
        "ir" => print!("{}", isc_ir::print::program(&prog)),
        "run" => {
            let mut out = std::io::stdout();
            match isc_interp::run_with(&prog, &args[1..], &mut out, run_lanes as usize) {
                Ok(code) => exit(code),
                Err(p) => {
                    eprintln!("{}", p);
                    exit(isc_interp::rt::PANIC_EXIT_CODE)
                }
            }
        }
        "build" => {
            let mut opts = iscc::BuildOptions::default();
            let stem = std::path::Path::new(&path).file_stem().map(|s| s.to_string_lossy().to_string());
            opts.output = stem.unwrap_or_else(|| "a.out".into()).into();
            let mut wasm_target = false;
            let mut explicit_output = false;
            let mut i = 1;
            while i < args.len() {
                let next = || args.get(i + 1).cloned().unwrap_or_else(|| usage());
                if args[i] == "--debug" {
                    opts.debug = true;
                    i += 1;
                    continue;
                }
                match args[i].as_str() {
                    "-o" => {
                        opts.output = next().into();
                        explicit_output = true;
                    }
                    "--emit-llvm" => opts.emit_llvm = Some(next().into()),
                    "--link" => opts.link.push(next()),
                    "--emit-asm" => opts.emit_asm = Some(next().into()),
                    "--lanes" => opts.codegen.lanes = lanes_arg(&next()),
                    "--target" => match next().as_str() {
                        "native" => {}
                        "wasm32-wasi" => {
                            opts.codegen.isa = isc_codegen::Isa::Wasm32;
                            wasm_target = true;
                        }
                        "wasm32-wasi-threads" => {
                            opts.codegen.isa = isc_codegen::Isa::Wasm32Threads;
                            wasm_target = true;
                        }
                        _ => usage(),
                    },
                    "--isa" => {
                        opts.codegen.isa = match next().as_str() {
                            "native" => isc_codegen::Isa::Native,
                            "avx2" => isc_codegen::Isa::Avx2,
                            "avx512" => isc_codegen::Isa::Avx512,
                            "sse" => isc_codegen::Isa::Sse,
                            "avx" | "ivybridge" => isc_codegen::Isa::Avx,
                            "wasm" => isc_codegen::Isa::Wasm32,
                            _ => usage(),
                        }
                    }
                    _ => usage(),
                }
                i += 2;
            }
            if wasm_target && !explicit_output {
                opts.output.set_extension("wasm");
            }
            if report {
                let es = isc_ir::report::entries(&prog);
                let mut j = opts.output.clone().into_os_string();
                j.push(".report.json");
                if let Err(e) = std::fs::write(&j, isc_ir::report::json(&path, &es)) {
                    eprintln!("{}", style.error(&format!("cannot write `{}`: {}", std::path::Path::new(&j).display(), e)));
                    exit(1)
                }
            }
            if let Err(e) = iscc::build(&prog, &opts) {
                eprintln!("{}", style.error(&e.to_string()));
                exit(1)
            }
            // in colors (a terminal): what was made, and in how long
            if style.on {
                eprintln!(
                    "{} {} -> {} in {} ms",
                    style.paint("1;32", "compiled"),
                    path,
                    opts.output.display(),
                    started.elapsed().as_millis()
                );
            }
        }
        _ => usage(),
    }
}
