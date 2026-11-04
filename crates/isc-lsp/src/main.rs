//! `isc-lsp`: the language server of Isocron (D269), for every editor that
//! speaks the Language Server Protocol (VS Code, Neovim, Helix, Zed, Emacs,
//! Sublime Text, Kate...), on its standard input and output. It reads the
//! programs with the parser and the checker of the compiler, as libraries,
//! and gives:
//! - the errors of the checker, as the text is typed (not only saved);
//! - on hover, the type of a name (a local, a parameter, a size, a field),
//!   the signature of a function and the comments above its declaration,
//!   the documentation of a function of the standard library;
//! - the definition of a name (a local, a function, also in an imported
//!   module);
//! - the outline of a file (its functions and types).
//!
//! `iscc` has none of this: it compiles. Positions are those of the
//! protocol (lines from 0, columns in UTF-16 units); the checker counts
//! lines and characters from 1.

mod docs;
mod hover;
mod json;

use isc_syntax::ast::Decl;
use isc_syntax::{Diagnostic, SourceFiles, Span};
use isc_types::tast::TProgram;
use isc_types::{PRELUDE_FILE, USER_FILE};
use json::{obj, Json};
use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::path::Path;

const USAGE: &str = "isc-lsp: the language server of Isocron (the Language Server Protocol on the
standard input and output; an editor starts it)
  isc-lsp            serve
  isc-lsp --version  the version";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version") {
        println!("isc-lsp {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{}", USAGE);
        return;
    }
    // the checker recurses on the expressions: the stack of a thread of
    // its own, as large as for any program the compiler reads
    let served = std::thread::Builder::new().stack_size(256 << 20).spawn(serve).expect("a thread");
    std::process::exit(served.join().unwrap_or(1));
}

/// One message: headers, an empty line, then `Content-Length` bytes of
/// JSON. `None` at the end of the input.
fn read_message(r: &mut impl BufRead) -> Option<Vec<u8>> {
    let mut len = None;
    loop {
        let mut line = String::new();
        if r.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let line = line.trim_end();
        if line.is_empty() {
            if len.is_some() {
                break;
            }
            continue;
        }
        if let Some((k, v)) = line.split_once(':') {
            if k.eq_ignore_ascii_case("content-length") {
                len = v.trim().parse::<usize>().ok();
            }
        }
    }
    let mut body = vec![0; len?];
    r.read_exact(&mut body).ok()?;
    Some(body)
}

// ---- positions and paths

fn is_id(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// The index of the character of `line` at `utf16` units of the protocol.
fn char_index(line: &str, utf16: u32) -> usize {
    let mut units = 0;
    for (i, c) in line.chars().enumerate() {
        if units >= utf16 {
            return i;
        }
        units += c.len_utf16() as u32;
    }
    line.chars().count()
}

/// The units of the protocol before the character `chars` of `line`.
fn utf16_of(line: &str, chars: usize) -> u32 {
    line.chars().take(chars).map(|c| c.len_utf16() as u32).sum()
}

fn line_of(text: &str, n: usize) -> &str {
    text.lines().nth(n).unwrap_or("")
}

/// A range of the protocol on line `n` (from 0), characters `a..b`.
fn range(text: &str, n: usize, a: usize, b: usize) -> Json {
    let l = line_of(text, n);
    let pos = |c: usize| obj(vec![("line", (n as u32).into()), ("character", utf16_of(l, c).into())]);
    obj(vec![("start", pos(a)), ("end", pos(b))])
}

/// The range of the name at `span` (or of the character there).
fn span_range(text: &str, span: Span) -> Json {
    let n = span.line.saturating_sub(1) as usize;
    let chars: Vec<char> = line_of(text, n).chars().collect();
    let a = (span.col.saturating_sub(1) as usize).min(chars.len());
    let mut b = a;
    while b < chars.len() && is_id(chars[b]) {
        b += 1;
    }
    range(text, n, a, if b == a { (a + 1).min(chars.len()) } else { b })
}

/// The range of `name` declared at `span`: the first `name` of its line
/// from there (a declaration starts at `function`, `export`...).
fn name_range(text: &str, span: Span, name: &str) -> Json {
    let n = span.line.saturating_sub(1) as usize;
    let chars: Vec<char> = line_of(text, n).chars().collect();
    let want: Vec<char> = name.chars().collect();
    let from = (span.col.saturating_sub(1) as usize).min(chars.len());
    for a in from..chars.len() {
        let b = a + want.len();
        if b <= chars.len() && chars[a..b] == want[..] && (a == 0 || !is_id(chars[a - 1])) && (b == chars.len() || !is_id(chars[b])) {
            return range(text, n, a, b);
        }
    }
    span_range(text, span)
}

/// The path of a `file:` URI (`file:///c%3A/x` on Windows: `c:/x`).
fn uri_to_path(uri: &str) -> Option<String> {
    let rest = uri.strip_prefix("file://")?;
    let b = rest.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Some(x) = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(x);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    let mut p = String::from_utf8(out).ok()?;
    let d = p.as_bytes();
    if d.len() >= 3 && d[0] == b'/' && d[1].is_ascii_alphabetic() && d[2] == b':' {
        p.remove(0);
    } else if !p.starts_with('/') {
        // `file://server/share/x`: a share of Windows
        p = format!("//{}", p);
    }
    Some(p)
}

/// The `file:` URI of a path.
fn path_to_uri(path: &str) -> String {
    let p = path.strip_prefix(r"\\?\").unwrap_or(path).replace('\\', "/");
    let d = p.as_bytes();
    let p = if d.len() >= 2 && d[0].is_ascii_alphabetic() && d[1] == b':' { format!("/{}", p) } else { p };
    let mut out = String::from("file://");
    for b in p.bytes() {
        if b.is_ascii_alphanumeric() || b"/-._~:".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

/// The byte of `text` at a position of the protocol.
fn offset(text: &str, line: u32, utf16: u32) -> usize {
    let mut start = 0;
    for _ in 0..line {
        match text[start..].find('\n') {
            Some(i) => start += i + 1,
            None => return text.len(),
        }
    }
    let l = &text[start..text[start..].find('\n').map_or(text.len(), |i| start + i)];
    let c = char_index(l, utf16);
    start + l.char_indices().nth(c).map_or(l.len(), |(i, _)| i)
}

/// The word under a position: its text, its line (from 1), its first and
/// last characters (from 0), the column (from 1) of a `(` after it and of
/// a `.` before it.
struct Word {
    text: String,
    line: u32,
    start: usize,
    end: usize,
    paren: Option<u32>,
    dot: Option<u32>,
}

fn word_at(text: &str, line: u32, utf16: u32) -> Option<Word> {
    let l = line_of(text, line as usize);
    let chars: Vec<char> = l.chars().collect();
    let mut a = char_index(l, utf16);
    // the pointer right after a word is on it
    if (a >= chars.len() || !is_id(chars[a])) && a > 0 && is_id(chars[a - 1]) {
        a -= 1;
    }
    if a >= chars.len() || !is_id(chars[a]) {
        return None;
    }
    while a > 0 && is_id(chars[a - 1]) {
        a -= 1;
    }
    if chars[a].is_ascii_digit() {
        return None;
    }
    let mut b = a;
    while b < chars.len() && is_id(chars[b]) {
        b += 1;
    }
    let mut c = b;
    while c < chars.len() && chars[c] == ' ' {
        c += 1;
    }
    let paren = (c < chars.len() && chars[c] == '(').then_some(c as u32 + 1);
    let dot = (a > 0 && chars[a - 1] == '.').then_some(a as u32);
    Some(Word { text: chars[a..b].iter().collect(), line: line + 1, start: a, end: b, paren, dot })
}

// ---- the analysis of a program

enum Checked {
    Ok(TProgram),
    Error(Diagnostic),
    /// The checker stopped on this text (a panic, reported on the standard
    /// error): no answer rather than a wrong one.
    Failed,
}

/// A program read as the compiler reads it (its modules loaded), then
/// checked; not lowered.
struct Analysis {
    files: SourceFiles,
    checked: Checked,
}

fn analyze(path: &str, src: &str) -> Analysis {
    fn check(path: &str, src: &str, files: &mut SourceFiles) -> Result<TProgram, Diagnostic> {
        let mut user = isc_syntax::parse(src, USER_FILE)?;
        isc_types::modules::load_modules(&mut user, Path::new(path), files)?;
        isc_types::check_parsed(&user)
    }
    let mut files = isc_types::source_files(path);
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| check(path, src, &mut files)));
    let checked = match r {
        Ok(Ok(t)) => Checked::Ok(t),
        Ok(Err(d)) => Checked::Error(d),
        Err(_) => Checked::Failed,
    };
    Analysis { files, checked }
}

/// The line (from 0) of the declaration `name` of the file, from its
/// syntax alone (for a program that does not check).
fn declared_line(text: &str, name: &str) -> Option<usize> {
    let p = isc_syntax::parse(text, USER_FILE).ok()?;
    p.decls.iter().find_map(|d| {
        let (n, s) = match d {
            Decl::Fn(f) => (&f.name, f.span),
            Decl::Extern(e) => (&e.name, e.span),
            Decl::Type(t) => (&t.name, t.span),
            Decl::Import(_) => return None,
        };
        (n == name).then(|| s.line.saturating_sub(1) as usize)
    })
}

fn short(name: &str) -> &str {
    name.rsplit('.').next().unwrap_or(name)
}

// ---- the server

struct Server<W: Write> {
    out: W,
    /// The texts of the open files, by URI, as the editor has them.
    texts: HashMap<String, String>,
    /// For an open file, the other file its error was put on.
    elsewhere: HashMap<String, String>,
    shutdown: bool,
}

impl<W: Write> Server<W> {
    fn send(&mut self, msg: Json) {
        let body = msg.to_string();
        let _ = write!(self.out, "Content-Length: {}\r\n\r\n{}", body.len(), body);
        let _ = self.out.flush();
    }

    fn reply(&mut self, id: Json, result: Json) {
        self.send(obj(vec![("jsonrpc", "2.0".into()), ("id", id), ("result", result)]));
    }

    fn fail(&mut self, id: Json, code: i32, message: &str) {
        let error = obj(vec![("code", Json::Num(code as f64)), ("message", message.into())]);
        self.send(obj(vec![("jsonrpc", "2.0".into()), ("id", id), ("error", error)]));
    }

    fn notify(&mut self, method: &str, params: Json) {
        self.send(obj(vec![("jsonrpc", "2.0".into()), ("method", method.into()), ("params", params)]));
    }

    /// The text of a file: as the editor has it, else as saved.
    fn text_of(&self, uri: &str) -> String {
        if let Some(t) = self.texts.get(uri) {
            return t.clone();
        }
        uri_to_path(uri).and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default()
    }

    /// The source of a file of an analysis: the one of `uri` for the
    /// program, a module as it was read; none for the prelude.
    fn source(&self, files: &SourceFiles, file: u32, text: &str) -> Option<String> {
        match file {
            USER_FILE => Some(text.to_string()),
            PRELUDE_FILE => None,
            _ => std::fs::read_to_string(files.name(file)).ok(),
        }
    }

    fn publish(&mut self, uri: &str, diagnostics: Vec<Json>) {
        self.notify("textDocument/publishDiagnostics", obj(vec![("uri", uri.into()), ("diagnostics", diagnostics.into())]));
    }

    /// The error of the checker on an open file (or on the module it
    /// imports where the error is), the previous one cleared.
    fn diagnose(&mut self, uri: &str) {
        let Some(path) = uri_to_path(uri) else { return };
        let text = self.text_of(uri);
        let a = analyze(&path, &text);
        if matches!(a.checked, Checked::Failed) {
            return;
        }
        let mut here = Vec::new();
        let mut other = None;
        if let Checked::Error(d) = &a.checked {
            let mut message = d.message.clone();
            for n in &d.notes {
                message.push('\n');
                message.push_str(n);
            }
            let diag = |src: &str| {
                obj(vec![("range", span_range(src, d.span)), ("severity", 1u32.into()), ("source", "iscc".into()), ("message", message.as_str().into())])
            };
            if d.span.file == USER_FILE {
                here.push(diag(&text));
            } else if let Some(src) = self.source(&a.files, d.span.file, &text) {
                other = Some((path_to_uri(a.files.name(d.span.file)), diag(&src)));
            }
        }
        if let Some(old) = self.elsewhere.remove(uri) {
            if other.as_ref().map(|o| &o.0) != Some(&old) && old != uri {
                self.publish(&old, Vec::new());
            }
        }
        if let Some((o, d)) = other {
            self.publish(&o, vec![d]);
            self.elsewhere.insert(uri.to_string(), o);
        }
        self.publish(uri, here);
    }

    /// The file and the position of a request.
    fn at(&self, params: &Json) -> Option<(String, String, String, Word)> {
        let uri = params.get("textDocument").get("uri").as_str()?.to_string();
        let path = uri_to_path(&uri)?;
        let text = self.text_of(&uri);
        let p = params.get("position");
        let w = word_at(&text, p.get("line").as_u32()?, p.get("character").as_u32()?)?;
        Some((uri, path, text, w))
    }

    fn found(a: &Analysis, w: &Word) -> Option<hover::Hover> {
        let Checked::Ok(t) = &a.checked else { return None };
        let place = hover::Place { file: USER_FILE, line: w.line, start: w.start as u32 + 1, paren: w.paren, dot: w.dot };
        hover::hover(t, &w.text, &place)
    }

    fn hover(&self, params: &Json) -> Json {
        let Some((_, path, text, w)) = self.at(params) else { return Json::Null };
        let a = analyze(&path, &text);
        let mut md = String::new();
        match Self::found(&a, &w) {
            Some(h) => {
                let code = if h.kind == "function" || h.kind == "action" { h.ty.clone() } else { format!("{}: {}", h.name, h.ty) };
                md = format!("```isocron\n{}\n```\n*{}*", code, h.kind);
                if let (Some(id), Checked::Ok(t)) = (h.func, &a.checked) {
                    let f = &t.fns[id];
                    if f.from_prelude {
                        if let Some(s) = docs::stdlib_markdown(short(&f.name)) {
                            md = format!("{}\n\n---\n\n{}", md, s);
                        }
                    } else if let Some(src) = self.source(&a.files, f.span.file, &text) {
                        let c = docs::comment_above(&src, f.span.line.saturating_sub(1) as usize);
                        if !c.is_empty() {
                            md = format!("{}\n\n{}", md, c);
                        }
                    }
                }
            }
            None => {
                // a declaration of the file while it does not check, else a
                // function of the standard library
                let line = if matches!(a.checked, Checked::Ok(_)) { None } else { declared_line(&text, &w.text) };
                if let Some(n) = line {
                    md = format!("```isocron\n{}\n```", docs::declaration_text(&text, n));
                    let c = docs::comment_above(&text, n);
                    if !c.is_empty() {
                        md = format!("{}\n\n{}", md, c);
                    }
                } else if let Some(s) = docs::stdlib_markdown(&w.text) {
                    md = s;
                }
            }
        }
        if md.is_empty() {
            return Json::Null;
        }
        let contents = obj(vec![("kind", "markdown".into()), ("value", md.into())]);
        obj(vec![("contents", contents), ("range", range(&text, w.line as usize - 1, w.start, w.end))])
    }

    fn definition(&self, params: &Json) -> Json {
        let Some((uri, path, text, w)) = self.at(params) else { return Json::Null };
        let a = analyze(&path, &text);
        let location = |u: &str, r: Json| obj(vec![("uri", u.into()), ("range", r)]);
        if let (Some(h), Checked::Ok(t)) = (Self::found(&a, &w), &a.checked) {
            let Some(span) = h.decl else { return Json::Null };
            let name = h.func.map_or(h.name.as_str(), |id| short(&t.fns[id].name));
            return match self.source(&a.files, span.file, &text) {
                Some(src) if span.file == USER_FILE => location(&uri, name_range(&src, span, name)),
                Some(src) => location(&path_to_uri(a.files.name(span.file)), name_range(&src, span, name)),
                None => Json::Null,
            };
        }
        if matches!(a.checked, Checked::Ok(_)) {
            return Json::Null;
        }
        match declared_line(&text, &w.text) {
            Some(n) => location(&uri, name_range(&text, Span::new(USER_FILE, n as u32 + 1, 1), &w.text)),
            None => Json::Null,
        }
    }

    fn symbols(&self, params: &Json) -> Json {
        let Some(uri) = params.get("textDocument").get("uri").as_str() else { return Json::Null };
        let text = self.text_of(uri);
        let Ok(p) = isc_syntax::parse(&text, USER_FILE) else { return Json::Arr(Vec::new()) };
        // the kinds of the protocol: 12 function, 10 enum, 23 struct
        let mut out = Vec::new();
        for d in &p.decls {
            let (name, kind, span) = match d {
                Decl::Fn(f) => (&f.name, 12u32, f.span),
                Decl::Extern(e) => (&e.name, 12, e.span),
                Decl::Type(t) => (&t.name, if t.variants.is_empty() { 23 } else { 10 }, t.span),
                Decl::Import(_) => continue,
            };
            let location = obj(vec![("uri", uri.into()), ("range", name_range(&text, span, name))]);
            out.push(obj(vec![("name", name.as_str().into()), ("kind", kind.into()), ("location", location)]));
        }
        Json::Arr(out)
    }

    fn change(&mut self, params: &Json) {
        let Some(uri) = params.get("textDocument").get("uri").as_str() else { return };
        let mut text = self.text_of(uri);
        if let Json::Arr(changes) = params.get("contentChanges") {
            for c in changes {
                let Some(new) = c.get("text").as_str() else { continue };
                let r = c.get("range");
                if r.is_null() {
                    text = new.to_string();
                } else {
                    let pos = |p: &Json| offset(&text, p.get("line").as_u32().unwrap_or(0), p.get("character").as_u32().unwrap_or(0));
                    let (a, b) = (pos(r.get("start")), pos(r.get("end")));
                    if a <= b {
                        text.replace_range(a..b, new);
                    }
                }
            }
        }
        self.texts.insert(uri.to_string(), text);
        self.diagnose(uri);
    }

    /// One message; `Some(code)` when the server is to exit.
    fn handle(&mut self, msg: &Json) -> Option<i32> {
        let Some(method) = msg.get("method").as_str() else { return None };
        let id = msg.get("id").clone();
        let params = msg.get("params");
        let uri = params.get("textDocument").get("uri").as_str().unwrap_or("").to_string();
        match method {
            "initialize" => {
                let sync = obj(vec![("openClose", true.into()), ("change", 1u32.into()), ("save", obj(vec![("includeText", false.into())]))]);
                let caps = obj(vec![
                    ("textDocumentSync", sync),
                    ("hoverProvider", true.into()),
                    ("definitionProvider", true.into()),
                    ("documentSymbolProvider", true.into()),
                ]);
                let info = obj(vec![("name", "isc-lsp".into()), ("version", env!("CARGO_PKG_VERSION").into())]);
                self.reply(id, obj(vec![("capabilities", caps), ("serverInfo", info)]));
            }
            "shutdown" => {
                self.shutdown = true;
                self.reply(id, Json::Null);
            }
            "exit" => return Some(if self.shutdown { 0 } else { 1 }),
            "textDocument/didOpen" => {
                let text = params.get("textDocument").get("text").as_str().unwrap_or("").to_string();
                self.texts.insert(uri.clone(), text);
                self.diagnose(&uri);
            }
            "textDocument/didChange" => self.change(params),
            "textDocument/didSave" => {
                // a module saved changes the programs that import it
                let open: Vec<String> = self.texts.keys().cloned().collect();
                for u in open {
                    self.diagnose(&u);
                }
            }
            "textDocument/didClose" => {
                self.texts.remove(&uri);
                if let Some(o) = self.elsewhere.remove(&uri) {
                    self.publish(&o, Vec::new());
                }
                self.publish(&uri, Vec::new());
            }
            "textDocument/hover" => {
                let r = self.hover(params);
                self.reply(id, r);
            }
            "textDocument/definition" => {
                let r = self.definition(params);
                self.reply(id, r);
            }
            "textDocument/documentSymbol" => {
                let r = self.symbols(params);
                self.reply(id, r);
            }
            _ if !id.is_null() => self.fail(id, -32601, &format!("isc-lsp does not handle `{}`", method)),
            // the other notifications (`initialized`, `$/cancelRequest`...)
            _ => {}
        }
        None
    }
}

fn serve() -> i32 {
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let mut s = Server { out: std::io::stdout().lock(), texts: HashMap::new(), elsewhere: HashMap::new(), shutdown: false };
    while let Some(body) = read_message(&mut input) {
        let msg = std::str::from_utf8(&body).ok().and_then(json::parse);
        match msg {
            Some(m) => {
                if let Some(code) = s.handle(&m) {
                    return code;
                }
            }
            None => s.fail(Json::Null, -32700, "a message that is not JSON"),
        }
    }
    if s.shutdown {
        0
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uris_and_paths() {
        assert_eq!(uri_to_path("file:///home/a%20b/x.isc").as_deref(), Some("/home/a b/x.isc"));
        assert_eq!(uri_to_path("file:///c%3A/Users/x.isc").as_deref(), Some("c:/Users/x.isc"));
        assert_eq!(uri_to_path("untitled:Untitled-1"), None);
        assert_eq!(path_to_uri("/home/a b/x.isc"), "file:///home/a%20b/x.isc");
        assert_eq!(path_to_uri(r"\\?\C:\Users\x.isc"), "file:///C:/Users/x.isc");
        assert_eq!(uri_to_path(&path_to_uri("/tmp/é.isc")).as_deref(), Some("/tmp/é.isc"));
    }

    #[test]
    fn positions_in_utf16() {
        // `é` is one unit, `😀` two
        let l = "a😀é x";
        assert_eq!(char_index(l, 3), 2);
        assert_eq!(utf16_of(l, 4), 5);
        let t = "ab\nc😀d\n";
        assert_eq!(offset(t, 1, 3), "ab\nc😀".len());
        assert_eq!(offset(t, 5, 0), t.len());
        let w = word_at("    print(stdDev(xs));", 0, 12).unwrap();
        assert_eq!((w.text.as_str(), w.start, w.end, w.paren, w.dot), ("stdDev", 10, 16, Some(17), None));
        assert_eq!(word_at("x = 12;", 0, 5).map(|w| w.text), None);
        assert_eq!(word_at("p.x", 0, 2).map(|w| w.dot), Some(Some(2)));
        // right after a word
        assert_eq!(word_at("abc;", 0, 3).map(|w| w.text), Some("abc".to_string()));
    }
}
