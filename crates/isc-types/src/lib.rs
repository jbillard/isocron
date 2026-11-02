//! Isocron type checker (POC subset).

pub mod check;
pub mod tast;
pub mod types;

use isc_syntax::{Diagnostic, SourceFiles};

/// Source of the standard library, written in Isocron.
pub const PRELUDE: &str = include_str!("prelude.isc");

/// File id of the user program in `SourceFiles` built by `check_source`.
pub const USER_FILE: u32 = 0;
/// File id of the prelude.
pub const PRELUDE_FILE: u32 = 1;

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
    let prelude = isc_syntax::parse(PRELUDE, PRELUDE_FILE)?;
    check::check_program(user, &prelude)
}
