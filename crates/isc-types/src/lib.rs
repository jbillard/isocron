//! Isocron type checker (POC subset).

pub mod check;
pub mod codec;
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

/// Checks a parsed program (its modules merged, D139) with the prelude: the
/// bodies of the functions of the prelude that it reaches (D629).
pub fn check_parsed(user: &isc_syntax::ast::Program) -> Result<tast::TProgram, Diagnostic> {
    let p = prelude()?;
    check::check_program_lazy(user, &p.program, &|name: &str| p.body(name), false)
}

/// `check_parsed` with the body of every function of the prelude checked
/// (D629): the tests of the prelude, and the lowering of a program that
/// would need a function of the prelude unchecked (an error of the
/// compiler, which `check_parsed` does not make).
pub fn check_parsed_whole(user: &isc_syntax::ast::Program) -> Result<tast::TProgram, Diagnostic> {
    let p = prelude()?;
    check::check_program_lazy(user, &p.program, &|name: &str| p.body(name), true)
}

/// The prelude: its declarations, the bodies of its functions parsed when
/// they are asked (D629).
struct Prelude {
    program: isc_syntax::ast::Program,
    bodies: isc_syntax::parser::Bodies<'static>,
    /// Its type declarations, its aliases with them, for the bodies.
    types: Vec<isc_syntax::ast::Decl>,
}

impl Prelude {
    /// The body of its function `name` (`std:prelude.` before it: kept for
    /// the modules of the standard library, D337), its type aliases
    /// replaced; none for a constant, parsed whole.
    fn body(&self, name: &str) -> Option<Result<isc_syntax::ast::Expr, Diagnostic>> {
        use isc_syntax::ast::{Decl, Expr, ExprKind, FnDecl, Program, TypeExpr, TypeKind};
        let name = name.strip_prefix(check::PRELUDE_KEPT).unwrap_or(name);
        let body = match self.bodies.body(name)? {
            Ok(b) => b,
            Err(d) => return Some(Err(d)),
        };
        if !self.types.iter().any(|d| matches!(d, Decl::Type(t) if t.alias.is_some())) {
            return Some(Ok(body));
        }
        // (its aliases replaced as `expand_aliases` replaces those of the
        // declarations: in a program of the types and this body)
        let sp = body.span;
        let unit = TypeExpr { kind: TypeKind::Unit, span: sp };
        let f = FnDecl { is_action: false, name: String::new(), span: sp, generics: Vec::new(), params: Vec::new(), ret: unit, modifiers: Vec::new(), body, is_const: false };
        let mut p = Program { decls: self.types.clone(), exports: Vec::new(), imported: Vec::new() };
        p.decls.push(Decl::Fn(f));
        if let Err(d) = check::expand_aliases(&mut p, &Default::default()) {
            return Some(Err(d));
        }
        Some(Ok(match p.decls.pop() {
            Some(Decl::Fn(f)) => f.body,
            _ => Expr { kind: ExprKind::Unit, span: sp },
        }))
    }
}

// (`PRELUDE_PLACES`: the places of its declarations, D629)
include!(concat!(env!("OUT_DIR"), "/prelude_places.rs"));

/// The prelude parsed (the bodies of its functions when asked), `Map`
/// declared in it and its aliases replaced.
fn prelude() -> Result<Prelude, Diagnostic> {
    let (mut prelude, bodies) = isc_syntax::parse_outlined(PRELUDE, PRELUDE_FILE, PRELUDE_PLACES)?;
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
    let types = prelude.decls.iter().filter(|d| matches!(d, Decl::Type(_))).cloned().collect();
    // the type aliases of the prelude replaced, as those of a program (D159)
    check::expand_aliases(&mut prelude, &Default::default())?;
    Ok(Prelude { program: prelude, bodies, types })
}

/// The constructor of the built-in `Map` (D158).
pub const MAP_CTOR: &str = "$Map";

#[cfg(test)]
mod tests {
    /// The prelude parsed from the places of its declarations, each body
    /// asked, is the prelude parsed whole (D629).
    #[test]
    fn the_places_of_the_prelude_parse_it_whole() {
        use isc_syntax::ast::Decl;
        let whole = isc_syntax::parse(super::PRELUDE, super::PRELUDE_FILE).unwrap();
        let (mut parts, bodies) = isc_syntax::parse_outlined(super::PRELUDE, super::PRELUDE_FILE, super::PRELUDE_PLACES).unwrap();
        let mut lazy = 0;
        for d in &mut parts.decls {
            if let Decl::Fn(f) = d {
                if let Some(b) = bodies.body(&f.name) {
                    f.body = b.unwrap();
                    lazy += 1;
                }
            }
        }
        assert!(lazy > 300, "{} bodies parsed when asked", lazy);
        assert_eq!(whole.decls.len(), parts.decls.len());
        for (a, b) in whole.decls.iter().zip(&parts.decls) {
            assert!(a == b, "{:?}", a);
        }
        assert_eq!(whole.exports, parts.exports);
    }
}
