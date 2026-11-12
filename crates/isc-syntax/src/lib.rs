//! Isocron front-end: source positions, diagnostics, lexer, AST and parser.

pub mod ast;
pub mod lexer;
pub mod parser;

use std::fmt;

/// A position in a source file (1-based line and column, in characters).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, PartialOrd, Ord)]
pub struct Span {
    pub file: u32,
    pub line: u32,
    pub col: u32,
}

impl Span {
    pub fn new(file: u32, line: u32, col: u32) -> Span {
        Span { file, line, col }
    }
}

impl fmt::Display for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.line, self.col)
    }
}

/// A compile-time error with its position.
#[derive(Clone, Debug, PartialEq)]
pub struct Diagnostic {
    pub span: Span,
    pub message: String,
    pub notes: Vec<String>,
}

impl Diagnostic {
    pub fn new(span: Span, message: impl Into<String>) -> Diagnostic {
        Diagnostic { span, message: message.into(), notes: Vec::new() }
    }

    pub fn with_note(mut self, note: impl Into<String>) -> Diagnostic {
        self.notes.push(note.into());
        self
    }

    /// Renders the diagnostic as `file:line:col: error: message`.
    pub fn render(&self, files: &SourceFiles) -> String {
        let mut s = format!(
            "{}:{}:{}: error: {}",
            files.name(self.span.file),
            self.span.line,
            self.span.col,
            self.message
        );
        for n in &self.notes {
            s.push_str("\n  ");
            s.push_str(n);
        }
        s
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}: error: {}", self.span.line, self.span.col, self.message)?;
        for n in &self.notes {
            write!(f, "\n  {}", n)?;
        }
        Ok(())
    }
}

pub type Result<T> = std::result::Result<T, Diagnostic>;

/// The file number of the prelude (the standard library written in
/// Isocron).
pub const PRELUDE_FILE: u32 = 1;

/// The first file number of the standard modules (`std:json`, ...): their
/// files are numbered from there, apart from those of the program.
pub const STD_FILE_BASE: u32 = 1 << 30;

/// A file of the program: the one compiled or a module of its own (not the
/// prelude, not a standard module). The memory report places what is
/// written there at its own line; what is written elsewhere, at the line of
/// the program that calls it (D659).
pub fn is_program_file(file: u32) -> bool {
    file != PRELUDE_FILE && file < STD_FILE_BASE
}

/// File names indexed by `Span::file`.
#[derive(Clone, Debug, Default)]
pub struct SourceFiles {
    names: Vec<String>,
    std_names: Vec<String>,
}

impl SourceFiles {
    pub fn new() -> SourceFiles {
        SourceFiles { names: Vec::new(), std_names: Vec::new() }
    }
    pub fn add(&mut self, name: impl Into<String>) -> u32 {
        self.names.push(name.into());
        (self.names.len() - 1) as u32
    }
    /// A standard module: its number from `STD_FILE_BASE`.
    pub fn add_std(&mut self, name: impl Into<String>) -> u32 {
        self.std_names.push(name.into());
        STD_FILE_BASE + (self.std_names.len() - 1) as u32
    }
    pub fn name(&self, file: u32) -> &str {
        let found = if file >= STD_FILE_BASE { self.std_names.get((file - STD_FILE_BASE) as usize) } else { self.names.get(file as usize) };
        found.map(|s| s.as_str()).unwrap_or("<unknown>")
    }
    /// Every file: its number and its name.
    pub fn all(&self) -> Vec<(u32, String)> {
        let own = self.names.iter().enumerate().map(|(i, n)| (i as u32, n.clone()));
        own.chain(self.std_names.iter().enumerate().map(|(i, n)| (STD_FILE_BASE + i as u32, n.clone()))).collect()
    }
}

/// Parses one source file (syntax inspired by TypeScript, D148).
pub fn parse(source: &str, file: u32) -> Result<ast::Program> {
    parser::parse(source, file)
}

/// `parse` with the bodies of the functions parsed when asked, from the
/// places of its declarations (`parser::outline`, D629).
pub fn parse_outlined<'s>(source: &'s str, file: u32, places: &[parser::Place]) -> Result<(ast::Program, parser::Bodies<'s>)> {
    parser::parse_outlined(source, file, places)
}
