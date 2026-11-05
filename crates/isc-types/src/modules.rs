//! The modules of a program (D139, D150): the files it imports, loaded,
//! checked for cycles and merged into it, their names qualified. In the
//! front end (moved from `iscc`, D269) so that every tool that checks a
//! program (the compiler, the language server) loads them the same way.

use std::path::{Path, PathBuf};

/// The modules of the standard library (SPEC section 18, D322): files of
/// Isocron included in the compiler, named by a bare spec (`"sys"`) before
/// `$ISC_PATH`; their key is `std:<name>`, which no file can have, and only
/// their functions call the primitives of the system (`sys*`).
pub const STD_MODULES: &[(&str, &str)] = &[("sys", include_str!("std/sys.isc"))];

/// The prefix of the keys of the standard modules (D322).
pub const STD_KEY: &str = "std:";

/// The source of the standard module `name`.
pub fn std_module(name: &str) -> Option<&'static str> {
    STD_MODULES.iter().find(|(n, _)| *n == name).map(|(_, src)| *src)
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
pub fn load_modules(root: &mut isc_syntax::ast::Program, path: &Path, files: &mut isc_syntax::SourceFiles) -> Result<(), isc_syntax::Diagnostic> {
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
        // a bare name of the standard library: that module, whatever the
        // files of `$ISC_PATH` (D322)
        if std_module(spec).is_some() {
            return Ok(PathBuf::from(format!("{}{}", STD_KEY, spec)));
        }
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
            let found = resolve(&imp.spec, dir, imp.span)?;
            let standard = found.to_str().and_then(|f| f.strip_prefix(STD_KEY)).map(str::to_string);
            let file = if standard.is_some() { found } else { canonical(&found) };
            if let Some(k) = st.chain.iter().position(|(f, _)| *f == file) {
                let mut names: Vec<String> = st.chain[k..].iter().map(|(_, n)| n.clone()).collect();
                names.push(st.chain[k].1.clone());
                return Err(Diagnostic::new(imp.span, format!("import cycle: {}", names.join(" -> "))));
            }
            let m = match st.modules.iter().position(|m| m.file == file) {
                Some(m) => m,
                None => {
                    let stem = match &standard {
                        Some(name) => name.clone(),
                        None => file.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(),
                    };
                    if stem.is_empty() || !stem.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') || stem.starts_with(|c: char| c.is_ascii_digit()) {
                        return Err(Diagnostic::new(imp.span, format!("`{}` cannot be a module: its file name must be a name (letters, digits, `_`)", file.display())));
                    }
                    let mut key = match &standard {
                        Some(name) => format!("{}{}", STD_KEY, name),
                        None => stem.clone(),
                    };
                    let mut n = 1;
                    while standard.is_none() && st.modules.iter().any(|m| m.key == key) {
                        n += 1;
                        key = format!("{}~{}", stem, n);
                    }
                    let src = match &standard {
                        Some(name) => std_module(name).unwrap_or_default().to_string(),
                        None => std::fs::read_to_string(&file).map_err(|e| Diagnostic::new(imp.span, format!("cannot read `{}`: {}", file.display(), e)))?,
                    };
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
                    // a standard module imports others by bare names only
                    let mdir = if standard.is_some() { PathBuf::new() } else { file.parent().map(Path::to_path_buf).unwrap_or_default() };
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
    process(root, crate::USER_FILE, &dir, &mut st, files)?;
    root.decls.extend(st.out);
    root.imported = st.imported;
    Ok(())
}
