//! The lexer of Isocron (syntax inspired by TypeScript, DECISIONS D148):
//! tokens with their positions, numbers with their type suffix, strings
//! with their escapes, and the scalar types.

use crate::{Diagnostic, Result, Span};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Scalar {
    I8,
    I16,
    I32,
    I64,
    U8,
    U16,
    U32,
    U64,
    F32,
    F64,
    Bool,
}

impl Scalar {
    pub fn from_name(s: &str) -> Option<Scalar> {
        Some(match s {
            "i8" => Scalar::I8,
            "i16" => Scalar::I16,
            "i32" => Scalar::I32,
            "i64" => Scalar::I64,
            "u8" => Scalar::U8,
            "u16" => Scalar::U16,
            "u32" => Scalar::U32,
            "u64" => Scalar::U64,
            "f32" => Scalar::F32,
            "f64" => Scalar::F64,
            "boolean" => Scalar::Bool,
            _ => return None,
        })
    }
    pub fn name(self) -> &'static str {
        match self {
            Scalar::I8 => "i8",
            Scalar::I16 => "i16",
            Scalar::I32 => "i32",
            Scalar::I64 => "i64",
            Scalar::U8 => "u8",
            Scalar::U16 => "u16",
            Scalar::U32 => "u32",
            Scalar::U64 => "u64",
            Scalar::F32 => "f32",
            Scalar::F64 => "f64",
            Scalar::Bool => "boolean",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum S {
    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,
    Comma,
    Semi,
    Colon,
    Dot,
    Ellipsis,
    Question,
    /// `??` (D158)
    QuestionQuestion,
    FatArrow,
    Assign,
    EqEq,
    EqEqEq,
    NotEq,
    NotEqEq,
    Lt,
    Le,
    Gt,
    Ge,
    Plus,
    Minus,
    Star,
    StarStar,
    Slash,
    Percent,
    Amp,
    AmpAmp,
    Bar,
    BarBar,
    Caret,
    Bang,
    At,
    /// The delimiters of a template literal (D150): `` ` ``, `${` and the
    /// `}` that closes an insertion.
    Backtick,
    TplOpen,
    TplClose,
}

impl S {
    pub(crate) fn text(self) -> &'static str {
        match self {
            S::LParen => "(",
            S::RParen => ")",
            S::LBracket => "[",
            S::RBracket => "]",
            S::LBrace => "{",
            S::RBrace => "}",
            S::Comma => ",",
            S::Semi => ";",
            S::Colon => ":",
            S::Dot => ".",
            S::Ellipsis => "...",
            S::Question => "?",
            S::QuestionQuestion => "??",
            S::FatArrow => "=>",
            S::Assign => "=",
            S::EqEq => "==",
            S::EqEqEq => "===",
            S::NotEq => "!=",
            S::NotEqEq => "!==",
            S::Lt => "<",
            S::Le => "<=",
            S::Gt => ">",
            S::Ge => ">=",
            S::Plus => "+",
            S::Minus => "-",
            S::Star => "*",
            S::StarStar => "**",
            S::Slash => "/",
            S::Percent => "%",
            S::Amp => "&",
            S::AmpAmp => "&&",
            S::Bar => "|",
            S::BarBar => "||",
            S::Caret => "^",
            S::Bang => "!",
            S::At => "@",
            S::Backtick => "`",
            S::TplOpen => "${",
            S::TplClose => "}",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum T {
    /// A `bigint` literal `123n`, `0xffn` (D156): its digits and radix.
    BigInt(String, u32),
    /// A regular expression `/source/flags` (D157).
    Regex(String, String),
    Int { value: u128, suffix: Option<Scalar> },
    Float { value: f64, value32: f32, suffix: Option<Scalar> },
    Str(String),
    Ident(String),
    Sym(S),
    Eof,
}

#[derive(Clone, Debug)]
pub(crate) struct Tok {
    pub(crate) t: T,
    pub(crate) span: Span,
}

pub(crate) fn describe(t: &T) -> String {
    match t {
        T::Int { value, .. } => format!("integer `{}`", value),
        T::Float { value, .. } => format!("float `{}`", value),
        T::BigInt(d, _) => format!("bigint `{}n`", d),
        T::Regex(s, f) => format!("regular expression `/{}/{}`", s, f),
        T::Str(s) => format!("string {:?}", s),
        T::Ident(s) => format!("`{}`", s),
        T::Sym(s) => format!("`{}`", s.text()),
        T::Eof => "the end of the file".into(),
    }
}

/// The tokens of a file and their positions (1-based line and column, in
/// characters); comments `//` and `/* */` are skipped.
pub(crate) fn lex(src: &str, file: u32) -> Result<Vec<Tok>> {
    lex_at(src, file, 1, 1)
}

/// `lex` of a part of a file that starts at `line`:`col` (D629).
pub(crate) fn lex_at(src: &str, file: u32, line: u32, col: u32) -> Result<Vec<Tok>> {
    // (a file of ASCII, as the prelude, which every program reads: each
    // byte a character, without decoding)
    let chars: Vec<char> = if src.is_ascii() { src.bytes().map(char::from).collect() } else { src.chars().collect() };
    let mut toks = Vec::new();
    let (mut i, mut line, mut col) = (0usize, line, col);
    // the open insertions `${` of template literals, each with its depth of
    // braces
    let mut tpl: Vec<u32> = Vec::new();
    while i < chars.len() {
        let c = chars[i];
        if c == '`' {
            let span = Span::new(file, line, col);
            toks.push(Tok { t: T::Sym(S::Backtick), span });
            i += 1;
            col += 1;
            lex_template_text(&chars, &mut i, &mut line, &mut col, file, span, &mut toks, &mut tpl)?;
            continue;
        }
        if c == '}' && tpl.last() == Some(&0) {
            tpl.pop();
            let span = Span::new(file, line, col);
            toks.push(Tok { t: T::Sym(S::TplClose), span });
            i += 1;
            col += 1;
            lex_template_text(&chars, &mut i, &mut line, &mut col, file, span, &mut toks, &mut tpl)?;
            continue;
        }
        if let Some(d) = tpl.last_mut() {
            if c == '{' {
                *d += 1;
            } else if c == '}' {
                *d -= 1;
            }
        }
        if c == '\n' {
            i += 1;
            line += 1;
            col = 1;
            continue;
        }
        if c.is_whitespace() {
            i += 1;
            col += 1;
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'*') {
            let span = Span::new(file, line, col);
            i += 2;
            col += 2;
            loop {
                if i >= chars.len() {
                    return Err(Diagnostic::new(span, "unterminated comment `/* ... */`"));
                }
                if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                    i += 2;
                    col += 2;
                    break;
                }
                if chars[i] == '\n' {
                    line += 1;
                    col = 1;
                } else {
                    col += 1;
                }
                i += 1;
            }
            continue;
        }
        let span = Span::new(file, line, col);
        let start = i;
        // `/source/flags` where an expression starts (D157): after an
        // operator, an opening bracket, a comma, `return`; else a division
        let regex_here = c == '/'
            && match toks.last().map(|t: &Tok| &t.t) {
                None => true,
                Some(T::Sym(S::RParen | S::RBracket | S::RBrace | S::TplClose)) => false,
                Some(T::Sym(_)) => true,
                Some(T::Ident(w)) => w == "return",
                _ => false,
            };
        if regex_here {
            let mut j = i + 1;
            let mut class = false;
            let mut src = String::new();
            loop {
                match chars.get(j) {
                    None | Some('\n') => return Err(Diagnostic::new(span, "unterminated regular expression")),
                    Some('\\') => {
                        src.push('\\');
                        if let Some(&d) = chars.get(j + 1) {
                            src.push(d);
                        }
                        j += 2;
                        continue;
                    }
                    Some('[') => class = true,
                    Some(']') => class = false,
                    Some('/') if !class => break,
                    _ => {}
                }
                src.push(chars[j]);
                j += 1;
            }
            j += 1;
            let mut flags = String::new();
            while j < chars.len() && chars[j].is_ascii_alphabetic() {
                flags.push(chars[j]);
                j += 1;
            }
            i = j;
            col += (i - start) as u32;
            toks.push(Tok { t: T::Regex(src, flags), span });
            continue;
        }
        let t = if c.is_ascii_digit() {
            lex_number(&chars, &mut i, span)?
        } else if c == '"' || c == '\'' {
            lex_quoted(&chars, &mut i, span, c)?
        } else if c.is_ascii_alphabetic() || c == '_' || (c == '$' && file == crate::PRELUDE_FILE) {
            // a name of the prelude may start with `$`: no program can
            // write it (D158)
            i += 1;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            T::Ident(chars[start..i].iter().collect())
        } else {
            let at = |k: usize| chars.get(i + k).copied();
            let (s, len) = match (c, at(1), at(2)) {
                ('.', Some('.'), Some('.')) => (S::Ellipsis, 3),
                ('=', Some('='), Some('=')) => (S::EqEqEq, 3),
                ('!', Some('='), Some('=')) => (S::NotEqEq, 3),
                ('=', Some('>'), _) => (S::FatArrow, 2),
                ('=', Some('='), _) => (S::EqEq, 2),
                ('!', Some('='), _) => (S::NotEq, 2),
                ('<', Some('='), _) => (S::Le, 2),
                ('>', Some('='), _) => (S::Ge, 2),
                ('*', Some('*'), _) => (S::StarStar, 2),
                ('&', Some('&'), _) => (S::AmpAmp, 2),
                ('|', Some('|'), _) => (S::BarBar, 2),
                ('(', ..) => (S::LParen, 1),
                (')', ..) => (S::RParen, 1),
                ('[', ..) => (S::LBracket, 1),
                (']', ..) => (S::RBracket, 1),
                ('{', ..) => (S::LBrace, 1),
                ('}', ..) => (S::RBrace, 1),
                (',', ..) => (S::Comma, 1),
                (';', ..) => (S::Semi, 1),
                (':', ..) => (S::Colon, 1),
                ('.', ..) => (S::Dot, 1),
                ('?', Some('?'), _) => (S::QuestionQuestion, 2),
                ('?', ..) => (S::Question, 1),
                ('=', ..) => (S::Assign, 1),
                ('<', ..) => (S::Lt, 1),
                ('>', ..) => (S::Gt, 1),
                ('+', ..) => (S::Plus, 1),
                ('-', ..) => (S::Minus, 1),
                ('*', ..) => (S::Star, 1),
                ('/', ..) => (S::Slash, 1),
                ('%', ..) => (S::Percent, 1),
                ('&', ..) => (S::Amp, 1),
                ('|', ..) => (S::Bar, 1),
                ('^', ..) => (S::Caret, 1),
                ('!', ..) => (S::Bang, 1),
                ('@', ..) => (S::At, 1),
                _ => return Err(Diagnostic::new(span, format!("unexpected character `{}`", c))),
            };
            i += len;
            T::Sym(s)
        };
        col += (i - start) as u32;
        toks.push(Tok { t, span });
    }
    if !tpl.is_empty() {
        return Err(Diagnostic::new(Span::new(file, line, col), "unterminated template literal: an insertion `${` is not closed"));
    }
    toks.push(Tok { t: T::Eof, span: Span::new(file, line, col) });
    Ok(toks)
}

/// A string literal between two `quote` characters (`"` or `'`), its
/// escapes replaced (D134).
fn lex_quoted(chars: &[char], i: &mut usize, span: Span, quote: char) -> Result<T> {
    *i += 1;
    let mut s = String::new();
    loop {
        let Some(&c) = chars.get(*i) else {
            return Err(Diagnostic::new(span, "unterminated string literal".to_string()));
        };
        match c {
            q if q == quote => {
                *i += 1;
                return Ok(T::Str(s));
            }
            '\n' => return Err(Diagnostic::new(span, "unterminated string literal (a string is on one line; `\\n` for a new line)".to_string())),
            '\\' => s.push(lex_escape(chars, i, span)?),
            _ => {
                s.push(c);
                *i += 1;
            }
        }
    }
}

/// The escape at `chars[*i]` (a `\\`), in a string or a template literal.
fn lex_escape(chars: &[char], i: &mut usize, span: Span) -> Result<char> {
    let e = chars.get(*i + 1).copied();
    *i += 2;
    Ok(match e {
        Some('n') => '\n',
        Some('t') => '\t',
        Some('r') => '\r',
        Some('0') => '\0',
        Some('\\') => '\\',
        Some('"') => '"',
        Some('\'') => '\'',
        Some('`') => '`',
        Some('$') => '$',
        Some('u') if chars.get(*i) == Some(&'{') => {
            let start = *i + 1;
            let mut j = start;
            while j < chars.len() && chars[j].is_ascii_hexdigit() {
                j += 1;
            }
            let hex: String = chars[start..j].iter().collect();
            let code = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32);
            match (chars.get(j), code) {
                (Some('}'), Some(ch)) if !hex.is_empty() => {
                    *i = j + 1;
                    ch
                }
                _ => return Err(Diagnostic::new(span, "invalid escape `\\u{...}` in a string literal".to_string())),
            }
        }
        Some(x) => return Err(Diagnostic::new(span, format!("unknown escape `\\{}` in a string literal", x))),
        None => return Err(Diagnostic::new(span, "unterminated string literal".to_string())),
    })
}

/// The text of a template literal from `chars[*i]` up to its end `` ` `` or
/// an insertion `${` (D150): a string token, then the delimiter; an
/// insertion pushes its depth of braces on `tpl`.
#[allow(clippy::too_many_arguments)]
fn lex_template_text(
    chars: &[char],
    i: &mut usize,
    line: &mut u32,
    col: &mut u32,
    file: u32,
    start: Span,
    toks: &mut Vec<Tok>,
    tpl: &mut Vec<u32>,
) -> Result<()> {
    let span = Span::new(file, *line, *col);
    let mut s = String::new();
    loop {
        match chars.get(*i) {
            None => return Err(Diagnostic::new(start, "unterminated template literal")),
            Some('`') => {
                toks.push(Tok { t: T::Str(s), span });
                toks.push(Tok { t: T::Sym(S::Backtick), span: Span::new(file, *line, *col) });
                *i += 1;
                *col += 1;
                return Ok(());
            }
            Some('$') if chars.get(*i + 1) == Some(&'{') => {
                toks.push(Tok { t: T::Str(s), span });
                toks.push(Tok { t: T::Sym(S::TplOpen), span: Span::new(file, *line, *col) });
                *i += 2;
                *col += 2;
                tpl.push(0);
                return Ok(());
            }
            Some('\\') => {
                let k = *i;
                s.push(lex_escape(chars, i, Span::new(file, *line, *col))?);
                *col += (*i - k) as u32;
            }
            Some('\n') => {
                s.push('\n');
                *i += 1;
                *line += 1;
                *col = 1;
            }
            Some(&c) => {
                s.push(c);
                *i += 1;
                *col += 1;
            }
        }
    }
}

/// A number and its optional type suffix (`42u8`, `1.0f32`).
fn lex_number(chars: &[char], i: &mut usize, span: Span) -> Result<T> {
    let radix_hex = chars[*i] == '0' && matches!(chars.get(*i + 1), Some('x') | Some('X'));
    let mut is_float = false;
    let mut text = String::new();
    if radix_hex {
        *i += 2;
        while *i < chars.len() && (chars[*i].is_ascii_hexdigit() || chars[*i] == '_') {
            if chars[*i] != '_' {
                text.push(chars[*i]);
            }
            *i += 1;
        }
        if text.is_empty() {
            return Err(Diagnostic::new(span, "hexadecimal literal has no digits"));
        }
    } else {
        let digits = |i: &mut usize, text: &mut String| {
            while *i < chars.len() && (chars[*i].is_ascii_digit() || chars[*i] == '_') {
                if chars[*i] != '_' {
                    text.push(chars[*i]);
                }
                *i += 1;
            }
        };
        digits(i, &mut text);
        if *i + 1 < chars.len() && chars[*i] == '.' && chars[*i + 1].is_ascii_digit() {
            is_float = true;
            text.push('.');
            *i += 1;
            digits(i, &mut text);
        }
        if *i < chars.len() && (chars[*i] == 'e' || chars[*i] == 'E') {
            let mut j = *i + 1;
            if j < chars.len() && (chars[j] == '+' || chars[j] == '-') {
                j += 1;
            }
            if j < chars.len() && chars[j].is_ascii_digit() {
                is_float = true;
                text.push('e');
                if chars[*i + 1] == '-' {
                    text.push('-');
                }
                *i = j;
                digits(i, &mut text);
            }
        }
    }
    // `123n`: a `bigint` (D156)
    if !is_float && *i < chars.len() && chars[*i] == 'n' && !chars.get(*i + 1).is_some_and(|c| c.is_ascii_alphanumeric() || *c == '_') {
        *i += 1;
        return Ok(T::BigInt(text, if radix_hex { 16 } else { 10 }));
    }
    // Optional type suffix: `42u8`, `1.0f32`.
    let mut suffix = None;
    if *i < chars.len() && chars[*i].is_ascii_alphabetic() {
        let s0 = *i;
        while *i < chars.len() && (chars[*i].is_ascii_alphanumeric() || chars[*i] == '_') {
            *i += 1;
        }
        let word: String = chars[s0..*i].iter().collect();
        match Scalar::from_name(&word) {
            Some(Scalar::Bool) | None => {
                return Err(Diagnostic::new(span, format!("invalid literal suffix `{}`", word)));
            }
            Some(s) => suffix = Some(s),
        }
    }
    if is_float {
        let value: f64 = text
            .parse()
            .map_err(|_| Diagnostic::new(span, format!("invalid float literal `{}`", text)))?;
        let value32: f32 = text
            .parse()
            .map_err(|_| Diagnostic::new(span, format!("invalid float literal `{}`", text)))?;
        Ok(T::Float { value, value32, suffix })
    } else {
        let value = u128::from_str_radix(&text, if radix_hex { 16 } else { 10 })
            .map_err(|_| Diagnostic::new(span, "integer literal is too large"))?;
        Ok(T::Int { value, suffix })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(s: &str) -> Vec<T> {
        lex(s, 0).unwrap().into_iter().map(|t| t.t).collect()
    }

    #[test]
    fn numbers() {
        assert_eq!(
            toks("42 0xFF 1_000_000 42u8"),
            vec![
                T::Int { value: 42, suffix: None },
                T::Int { value: 255, suffix: None },
                T::Int { value: 1_000_000, suffix: None },
                T::Int { value: 42, suffix: Some(Scalar::U8) },
                T::Eof
            ]
        );
        assert_eq!(
            toks("3.14 1e-9 1.0f32"),
            vec![
                T::Float { value: 3.14, value32: 3.14, suffix: None },
                T::Float { value: 1e-9, value32: 1e-9, suffix: None },
                T::Float { value: 1.0, value32: 1.0, suffix: Some(Scalar::F32) },
                T::Eof
            ]
        );
        // a method of a literal: the point is not a decimal point
        assert_eq!(toks("1.x")[..2], [T::Int { value: 1, suffix: None }, T::Sym(S::Dot)]);
    }

    #[test]
    fn comments_and_symbols() {
        assert_eq!(
            toks("a // comment\n/* two\nlines */ === !== => ** ... && || ^ @"),
            vec![
                T::Ident("a".into()),
                T::Sym(S::EqEqEq),
                T::Sym(S::NotEqEq),
                T::Sym(S::FatArrow),
                T::Sym(S::StarStar),
                T::Sym(S::Ellipsis),
                T::Sym(S::AmpAmp),
                T::Sym(S::BarBar),
                T::Sym(S::Caret),
                T::Sym(S::At),
                T::Eof
            ]
        );
        assert_eq!(toks("'a\\'b' \"\\u{263A}\"")[..2], [T::Str("a'b".into()), T::Str("\u{263A}".into())]);
    }

    #[test]
    fn templates() {
        assert_eq!(
            toks("`a${x}b${ {c: 1}.c }`"),
            vec![
                T::Sym(S::Backtick),
                T::Str("a".into()),
                T::Sym(S::TplOpen),
                T::Ident("x".into()),
                T::Sym(S::TplClose),
                T::Str("b".into()),
                T::Sym(S::TplOpen),
                T::Sym(S::LBrace),
                T::Ident("c".into()),
                T::Sym(S::Colon),
                T::Int { value: 1, suffix: None },
                T::Sym(S::RBrace),
                T::Sym(S::Dot),
                T::Ident("c".into()),
                T::Sym(S::TplClose),
                T::Str("".into()),
                T::Sym(S::Backtick),
                T::Eof
            ]
        );
        // a template on two lines: the positions after it follow
        let t = lex("`a\nb` c", 0).unwrap();
        assert_eq!((t[3].span.line, t[3].span.col), (2, 4));
        assert!(lex("`a${x", 0).is_err());
        assert!(lex("`a", 0).is_err());
    }

    #[test]
    fn positions() {
        let t = lex("a b\n  /* x */ c", 0).unwrap();
        assert_eq!((t[0].span.line, t[0].span.col), (1, 1));
        assert_eq!((t[1].span.line, t[1].span.col), (1, 3));
        assert_eq!((t[2].span.line, t[2].span.col), (2, 11));
    }

    #[test]
    fn errors() {
        assert!(lex("a # b", 0).is_err());
        assert!(lex("12abc", 0).is_err());
        assert!(lex("1bool", 0).is_err());
        assert!(lex("340282366920938463463374607431768211456", 0).is_err());
        assert!(lex("/* open", 0).is_err());
        assert!(lex("\"open", 0).is_err());
    }
}
