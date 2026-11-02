//! Compiler pipeline shared by the `iscc` binary and the tests.

use std::path::{Path, PathBuf};

/// Parses, type-checks, lowers and optimizes a program and the modules it
/// imports. Errors are rendered as `file:line:col: error: message`.
pub fn frontend(path: &str, src: &str) -> Result<isc_ir::Program, String> {
    let mut files = isc_types::source_files(path);
    let mut user = isc_syntax::parse(src, isc_types::USER_FILE).map_err(|d| d.render(&files))?;
    let loaded = load_modules(&mut user, Path::new(path), &mut files);
    let render = |d: isc_syntax::Diagnostic| d.render(&files);
    loaded.map_err(render)?;
    let t = isc_types::check_parsed(&user).map_err(render)?;
    let mut p = isc_ir::lower::lower(&t).map_err(render)?;
    isc_ir::passes::optimize(&mut p).map_err(render)?;
    Ok(p)
}

/// The modules of a program (D139, D150). `import ... from "./m"` (or
/// `"../lib/m"`) reads `m.isc` relative to the importing file; a bare name
/// `"m"` is searched in the directories of `$ISC_PATH` (separated by `:`).
/// A module is identified by its file: read once, whatever the paths that
/// name it; an import cycle is an error. Its declarations are renamed
/// `m.f`, `m.T`, `m.C` (a name with a dot, which no program can write;
/// `m~2` when two files are named `m.isc`) and added to the program;
/// inside `m`, an unqualified name is looked for in `m` first (the type
/// checker). Only what a module exports can be reached from another file:
/// `alias.x` is rewritten here, the names imported by name are given to
/// the type checker, which looks them up after the local variables. A
/// `declare function` keeps its C name and is global.
fn load_modules(root: &mut isc_syntax::ast::Program, path: &Path, files: &mut isc_syntax::SourceFiles) -> Result<(), isc_syntax::Diagnostic> {
    use isc_syntax::ast::{Decl, Program};
    use isc_syntax::{Diagnostic, Span};

    struct Module {
        file: PathBuf,
        key: String,
        declared: Vec<String>,
        exports: Vec<String>,
    }
    struct State {
        modules: Vec<Module>,
        /// The files being loaded, outermost first (for the cycles).
        chain: Vec<(PathBuf, String)>,
        out: Vec<Decl>,
        imported: Vec<(u32, String, String)>,
    }

    fn canonical(p: &Path) -> PathBuf {
        std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
    }

    fn resolve(spec: &str, dir: &Path, span: Span) -> Result<PathBuf, Diagnostic> {
        let file = if spec.ends_with(".isc") { spec.to_string() } else { format!("{}.isc", spec) };
        if spec.starts_with("./") || spec.starts_with("../") || spec.starts_with('/') {
            let p = dir.join(&file);
            if p.is_file() {
                return Ok(p);
            }
            return Err(Diagnostic::new(span, format!("module \"{}\" not found: no file `{}`", spec, p.display())));
        }
        let mut dirs = Vec::new();
        if let Ok(p) = std::env::var("ISC_PATH") {
            dirs.extend(p.split(':').filter(|d| !d.is_empty()).map(PathBuf::from));
        }
        if let Some(p) = dirs.iter().map(|d| d.join(&file)).find(|f| f.is_file()) {
            return Ok(p);
        }
        let hint = if dir.join(&file).is_file() { format!("; the file next to this one is \"./{}\"", spec) } else { String::new() };
        Err(Diagnostic::new(span, format!("module \"{}\" not found in $ISC_PATH{}", spec, hint)))
    }

    fn own_names(p: &Program) -> Vec<String> {
        let mut v = Vec::new();
        for d in &p.decls {
            match d {
                Decl::Fn(f) => v.push(f.name.clone()),
                Decl::Extern(e) => v.push(e.name.clone()),
                Decl::Type(t) => {
                    v.push(t.name.clone());
                    v.extend(t.variants.iter().map(|x| x.name.clone()));
                }
                Decl::Import(_) => {}
            }
        }
        v
    }

    /// Loads the imports of `prog` (file `id`, in `dir`), then resolves its
    /// qualified names and its names imported by name.
    fn process(prog: &mut Program, id: u32, dir: &Path, st: &mut State, files: &mut isc_syntax::SourceFiles) -> Result<(), Diagnostic> {
        let imports: Vec<isc_syntax::ast::Import> =
            prog.decls.iter().filter_map(|d| if let Decl::Import(i) = d { Some(i.clone()) } else { None }).collect();
        let own = own_names(prog);
        let mut aliases: Vec<(String, usize, String)> = Vec::new();
        for imp in &imports {
            let file = canonical(&resolve(&imp.spec, dir, imp.span)?);
            if let Some(k) = st.chain.iter().position(|(f, _)| *f == file) {
                let mut names: Vec<String> = st.chain[k..].iter().map(|(_, n)| n.clone()).collect();
                names.push(st.chain[k].1.clone());
                return Err(Diagnostic::new(imp.span, format!("import cycle: {}", names.join(" -> "))));
            }
            let m = match st.modules.iter().position(|m| m.file == file) {
                Some(m) => m,
                None => {
                    let stem = file.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                    if stem.is_empty() || !stem.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') || stem.starts_with(|c: char| c.is_ascii_digit()) {
                        return Err(Diagnostic::new(imp.span, format!("`{}` cannot be a module: its file name must be a name (letters, digits, `_`)", file.display())));
                    }
                    let mut key = stem.clone();
                    let mut n = 1;
                    while st.modules.iter().any(|m| m.key == key) {
                        n += 1;
                        key = format!("{}~{}", stem, n);
                    }
                    let src = std::fs::read_to_string(&file)
                        .map_err(|e| Diagnostic::new(imp.span, format!("cannot read `{}`: {}", file.display(), e)))?;
                    let fid = files.add(file.display().to_string());
                    let mut mp = isc_syntax::parse(&src, fid)?;
                    for d in &mp.decls {
                        if let Decl::Fn(f) = d {
                            if f.name == "main" {
                                return Err(Diagnostic::new(f.span, format!("the module `{}` declares `main`; only a program does", stem)));
                            }
                        }
                    }
                    st.chain.push((file.clone(), stem.clone()));
                    let mdir = file.parent().map(Path::to_path_buf).unwrap_or_default();
                    process(&mut mp, fid, &mdir, st, files)?;
                    st.chain.pop();
                    let declared = own_names(&mp);
                    for d in mp.decls {
                        match d {
                            Decl::Import(_) => {}
                            Decl::Fn(mut f) => {
                                f.name = format!("{}.{}", key, f.name);
                                st.out.push(Decl::Fn(f));
                            }
                            Decl::Type(mut t) => {
                                t.name = format!("{}.{}", key, t.name);
                                for v in t.variants.iter_mut() {
                                    v.name = format!("{}.{}", key, v.name);
                                }
                                st.out.push(Decl::Type(t));
                            }
                            Decl::Extern(e) => st.out.push(Decl::Extern(e)),
                        }
                    }
                    st.modules.push(Module { file: file.clone(), key, declared, exports: mp.exports });
                    st.modules.len() - 1
                }
            };
            if let Some(a) = &imp.alias {
                aliases.push((a.clone(), m, imp.spec.clone()));
            }
            for (name, local, sp) in &imp.names {
                let md = &st.modules[m];
                if !md.exports.contains(name) {
                    let why = if md.declared.contains(name) { "declares it but does not export it" } else { "does not declare it" };
                    return Err(Diagnostic::new(*sp, format!("`{}` cannot be imported: the module \"{}\" {}", name, imp.spec, why)));
                }
                if own.contains(local) {
                    return Err(Diagnostic::new(*sp, format!("`{}` is both imported and declared in this file", local)));
                }
                if st.imported.iter().any(|(f, l, _)| *f == id && l == local) {
                    return Err(Diagnostic::new(*sp, format!("`{}` is imported twice", local)));
                }
                st.imported.push((id, local.clone(), format!("{}.{}", md.key, name)));
            }
        }
        // `alias.x`: the declaration `x` of the module, if it exports it
        let modules = &st.modules;
        isc_syntax::ast::visit_names(prog, &mut |v: isc_syntax::ast::Visit, sp: Span| {
            let isc_syntax::ast::Visit::Name(n) = v else {
                return Ok(());
            };
            if let Some((a, x)) = n.split_once('.') {
                if let Some((_, m, spec)) = aliases.iter().find(|(al, _, _)| al == a) {
                    let md = &modules[*m];
                    if !md.exports.contains(&x.to_string()) {
                        let why = if md.declared.iter().any(|d| d == x) { "declares it but does not export it" } else { "does not declare it" };
                        return Err(Diagnostic::new(sp, format!("`{}`: the module \"{}\" {}", n, spec, why)));
                    }
                    *n = format!("{}.{}", md.key, x);
                }
            }
            Ok(())
        })?;
        prog.decls.retain(|d| !matches!(d, Decl::Import(..)));
        Ok(())
    }

    if !root.decls.iter().any(|d| matches!(d, Decl::Import(..))) {
        return Ok(());
    }
    let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let root_name = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let mut st = State { modules: Vec::new(), chain: vec![(canonical(path), root_name)], out: Vec::new(), imported: Vec::new() };
    process(root, isc_types::USER_FILE, &dir, &mut st, files)?;
    root.decls.extend(st.out);
    root.imported = st.imported;
    Ok(())
}

#[derive(Default, Clone, Debug)]
pub struct BuildOptions {
    pub output: PathBuf,
    pub emit_llvm: Option<PathBuf>,
    pub emit_asm: Option<PathBuf>,
    /// Keep the symbols in the executable (default: stripped).
    pub debug: bool,
    /// V and target instruction set (experiments; defaults: V = 32, host CPU).
    pub codegen: isc_codegen::Options,
    /// The libraries of the `extern` functions (`--link`, D125): a name
    /// (`-lname`) or the path of a library or an object file.
    pub link: Vec<String>,
}

/// Compiles the IR to a native executable.
pub fn build(p: &isc_ir::Program, opts: &BuildOptions) -> Result<(), String> {
    let ir = isc_codegen::emit_with(p, &opts.codegen)?;
    if let Some(f) = &opts.emit_llvm {
        std::fs::write(f, &ir).map_err(|e| format!("cannot write `{}`: {}", f.display(), e))?;
    }
    let isa = opts.codegen.isa;
    let wasm = matches!(isa, isc_codegen::Isa::Wasm32 | isc_codegen::Isa::Wasm32Threads);
    let threads = isa == isc_codegen::Isa::Wasm32Threads;
    let obj = opts.output.with_extension("o");
    isc_codegen::backend::compile_with(&ir, &obj, opts.emit_asm.as_deref(), opts.codegen.isa)?;
    // the embedded runtime, written next to the object file for the linker
    let (var, bytes, triple) = if threads {
        ("ISC_RUNTIME_WASM_THREADS", RUNTIME_WASM_THREADS, "wasm32-wasip1-threads")
    } else if wasm {
        ("ISC_RUNTIME_WASM", RUNTIME_WASM, "wasm32-wasip1")
    } else {
        ("ISC_RUNTIME", RUNTIME, "")
    };
    let (rt, temporary) = match std::env::var(var) {
        Ok(p) => (PathBuf::from(p), false),
        Err(_) => {
            if bytes.is_empty() {
                let _ = std::fs::remove_file(&obj);
                return Err(format!(
                    "this iscc has no runtime for {}: install the Rust target (`rustup target add {}`) and build iscc again",
                    triple, triple
                ));
            }
            let f = opts.output.with_extension("rt.a");
            std::fs::write(&f, bytes).map_err(|e| format!("cannot write `{}`: {}", f.display(), e))?;
            (f, true)
        }
    };
    let linked = if wasm {
        link_wasm(&obj, &rt, &opts.output, opts.debug, triple, threads, &opts.link)
    } else {
        link(&obj, &rt, &opts.output, opts.debug, &opts.link)
    };
    let _ = std::fs::remove_file(&obj);
    if temporary {
        let _ = std::fs::remove_file(&rt);
    }
    linked
}

/// The runtime of compiled programs (`crates/isc-runtime`), built by
/// `build.rs` and embedded in `iscc`. `$ISC_RUNTIME` may name another
/// `libisc_runtime.a`.
const RUNTIME: &[u8] = include_bytes!(env!("ISC_RUNTIME_LIB"));

/// The same runtime for WebAssembly (`wasm32-wasip1`); empty when the Rust
/// target was not installed when `iscc` was built. `$ISC_RUNTIME_WASM` may
/// name another one.
const RUNTIME_WASM: &[u8] = include_bytes!(env!("ISC_RUNTIME_WASM_LIB"));

/// The same runtime with threads (`wasm32-wasip1-threads`).
const RUNTIME_WASM_THREADS: &[u8] = include_bytes!(env!("ISC_RUNTIME_WASM_THREADS_LIB"));

/// The Rust sysroot, which holds the WebAssembly linker (`rust-lld`) and the
/// WASI C library of each target (`lib/rustlib/<target>/lib/self-contained`):
/// `$ISC_RUST_SYSROOT`, else `rustc --print sysroot`.
fn rust_sysroot() -> Result<PathBuf, String> {
    if let Ok(s) = std::env::var("ISC_RUST_SYSROOT") {
        return Ok(s.into());
    }
    let rustc = std::env::var("HOME").map(|h| PathBuf::from(h).join(".cargo/bin/rustc")).ok().filter(|p| p.is_file());
    let rustc = rustc.unwrap_or_else(|| "rustc".into());
    let o = std::process::Command::new(&rustc)
        .args(["--print", "sysroot"])
        .output()
        .map_err(|e| format!("cannot run `{}` to find the WebAssembly linker: {}", rustc.display(), e))?;
    Ok(String::from_utf8_lossy(&o.stdout).trim().into())
}

/// Links a WebAssembly program for WASI preview 1 with `rust-lld` (wasm-ld),
/// the runtime and the wasi-libc of the Rust target (`crt1-command.o`,
/// `libc.a`): the result is a WASI command module, run by any WASI engine.
/// With threads (wasi-threads), the memory is shared and imported, and
/// `wasi_thread_start` is exported, as the proposal requires.
fn link_wasm(obj: &Path, rt: &Path, out: &Path, debug: bool, triple: &str, threads: bool, libs: &[String]) -> Result<(), String> {
    let sysroot = rust_sysroot()?;
    let rustlib = sysroot.join("lib/rustlib");
    let lld = std::fs::read_dir(&rustlib)
        .map_err(|e| format!("cannot read `{}`: {}", rustlib.display(), e))?
        .filter_map(|d| d.ok())
        .map(|d| d.path().join("bin/rust-lld"))
        .find(|p| p.is_file())
        .ok_or_else(|| format!("no `rust-lld` under `{}`", rustlib.display()))?;
    let libc_dir = rustlib.join(triple).join("lib/self-contained");
    if !libc_dir.join("libc.a").is_file() {
        return Err(format!("no WASI C library in `{}`: rustup target add {}", libc_dir.display(), triple));
    }
    let mut cmd = std::process::Command::new(&lld);
    cmd.args(["-flavor", "wasm", "-o"]).arg(out).arg(libc_dir.join("crt1-command.o")).arg(obj);
    // the libraries of the `extern` functions and the C library before the
    // runtime, as natively (D125)
    for l in libs {
        if l.contains('/') || l.ends_with(".a") || l.ends_with(".o") {
            cmd.arg(l);
        } else {
            cmd.arg(format!("-l{}", l));
        }
    }
    cmd.arg(libc_dir.join("libc.a")).arg(rt).arg(libc_dir.join("libc.a"));
    // the stack of the main thread: 8 MiB, as on Linux (wasm-ld gives 64 KiB)
    cmd.args(["--gc-sections", "-z", "stack-size=8388608"]);
    if threads {
        cmd.args(["--shared-memory", "--import-memory", "--export-memory", "--max-memory=4294967296"]);
        cmd.args(["--export=wasi_thread_start"]);
    }
    if !debug {
        cmd.arg("--strip-all");
    }
    let status = cmd.status().map_err(|e| format!("cannot run the WebAssembly linker `{}`: {}", lld.display(), e))?;
    if !status.success() {
        return Err(format!("the WebAssembly linker `{}` failed ({})", lld.display(), status));
    }
    Ok(())
}

/// Links the program with the runtime (which needs only the C library, and
/// `libm` for `fmod`). Unused sections are dropped and, unless `debug`, the
/// symbols are stripped: the code that runs is the same, the file is smaller.
fn link(obj: &Path, rt: &Path, out: &Path, debug: bool, libs: &[String]) -> Result<(), String> {
    let cc = std::env::var("ISC_CC").unwrap_or_else(|_| "cc".into());
    let mut cmd = std::process::Command::new(&cc);
    cmd.arg("-o").arg(out).arg(obj).arg("-Wl,--gc-sections");
    // the libraries of the `extern` functions, then the C library, before
    // the runtime: the runtime's archive holds weak copies of C functions
    // (Rust's `compiler_builtins`, `cbrt` among them) that would otherwise
    // replace those of the C library for an `extern` (D125)
    for l in libs {
        if l.contains('/') || l.ends_with(".so") || l.ends_with(".a") || l.ends_with(".o") {
            cmd.arg(l);
        } else {
            cmd.arg(format!("-l{}", l));
        }
    }
    cmd.args(["-lm", "-lc"]).arg(rt);
    if !debug {
        cmd.arg("-s");
    }
    let status = cmd
        .args(["-lm", "-lc"])
        .status()
        .map_err(|e| format!("cannot run the linker `{}`: {}", cc, e))?;
    if !status.success() {
        return Err(format!("the linker `{}` failed ({})", cc, status));
    }
    Ok(())
}
