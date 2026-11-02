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

/// File names indexed by `Span::file`.
#[derive(Clone, Debug, Default)]
pub struct SourceFiles {
    names: Vec<String>,
}

impl SourceFiles {
    pub fn new() -> SourceFiles {
        SourceFiles { names: Vec::new() }
    }
    pub fn add(&mut self, name: impl Into<String>) -> u32 {
        self.names.push(name.into());
        (self.names.len() - 1) as u32
    }
    pub fn name(&self, file: u32) -> &str {
        self.names.get(file as usize).map(|s| s.as_str()).unwrap_or("<unknown>")
    }
}

/// Parses one source file (syntax inspired by TypeScript, D148).
pub fn parse(source: &str, file: u32) -> Result<ast::Program> {
    parser::parse(source, file)
}
