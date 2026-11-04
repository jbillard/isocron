//! Isocron type checker (POC subset).

pub mod check;
pub mod modules;
pub mod tast;
pub mod types;

use isc_syntax::{Diagnostic, SourceFiles};

/// Source of the standard library, written in Isocron.
pub const PRELUDE: &str = include_str!("prelude.isc");

/// File id of the user program in `SourceFiles` built by `check_source`.
pub const USER_FILE: u32 = 0;
/// File id of the prelude.
pub use isc_syntax::PRELUDE_FILE;

pub fn source_files(user_name: &str) -> SourceFiles {
    let mut f = SourceFiles::new();
    f.add(user_name);
    f.add("<prelude>");
    f
}

/// Parses and checks a program together with the prelude.
pub fn check_source(src: &str) -> Result<tast::TProgram, Diagnostic> {
    let user = isc_syntax::parse(src, USER_FILE)?;
    check_parsed(&user)
}

/// Checks a parsed program (its modules merged, D139) with the prelude.
pub fn check_parsed(user: &isc_syntax::ast::Program) -> Result<tast::TProgram, Diagnostic> {
    let mut prelude = isc_syntax::parse(PRELUDE, PRELUDE_FILE)?;
    // `Map<K, V>` (D158): a sum type of the prelude whose only constructor no
    // program can write; the lowering gives it its layout
    use isc_syntax::ast::{Decl, Generic, TypeDecl, Variant};
    let sp = isc_syntax::Span::new(PRELUDE_FILE, 1, 1);
    let g = |n: &str| Generic { name: n.into(), class: None, span: sp };
    prelude.decls.insert(
        0,
        Decl::Type(TypeDecl {
            name: "Map".into(),
            span: sp,
            generics: vec![g("K"), g("V")],
            variants: vec![Variant { name: MAP_CTOR.into(), fields: Vec::new(), span: sp }],
            alias: None,
        }),
    );
    check::check_program(user, &prelude)
}

/// The constructor of the built-in `Map` (D158).
pub const MAP_CTOR: &str = "$Map";
