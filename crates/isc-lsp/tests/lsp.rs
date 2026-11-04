//! The language server as an editor drives it (D269): `isc-lsp` started,
//! the messages of the protocol on its standard input and output.

#[path = "../src/json.rs"]
#[allow(dead_code)]
mod json;

use json::{obj, Json};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

struct Client {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    next: u32,
    /// The notifications of the server, in their order.
    notes: Vec<Json>,
}

impl Client {
    fn start() -> Client {
        let mut child = Command::new(env!("CARGO_BIN_EXE_isc-lsp")).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
        let input = child.stdin.take().unwrap();
        let output = BufReader::new(child.stdout.take().unwrap());
        Client { child, input, output, next: 1, notes: Vec::new() }
    }

    fn write(&mut self, msg: Json) {
        let body = msg.to_string();
        write!(self.input, "Content-Length: {}\r\n\r\n{}", body.len(), body).unwrap();
        self.input.flush().unwrap();
    }

    fn read(&mut self) -> Json {
        let mut len = 0;
        loop {
            let mut line = String::new();
            assert!(self.output.read_line(&mut line).unwrap() > 0, "the server closed its output");
            let line = line.trim_end();
            if line.is_empty() {
                break;
            }
            if let Some(v) = line.strip_prefix("Content-Length: ") {
                len = v.parse().unwrap();
            }
        }
        let mut body = vec![0; len];
        self.output.read_exact(&mut body).unwrap();
        json::parse(std::str::from_utf8(&body).unwrap()).expect("JSON")
    }

    fn notify(&mut self, method: &str, params: Json) {
        self.write(obj(vec![("jsonrpc", "2.0".into()), ("method", method.into()), ("params", params)]));
    }

    /// The answer of a request (its result, or its error).
    fn request(&mut self, method: &str, params: Json) -> Json {
        let id = self.next;
        self.next += 1;
        self.write(obj(vec![("jsonrpc", "2.0".into()), ("id", id.into()), ("method", method.into()), ("params", params)]));
        loop {
            let m = self.read();
            if m.get("id").as_u32() == Some(id) {
                return if m.get("error").is_null() { m.get("result").clone() } else { m.get("error").clone() };
            }
            self.notes.push(m);
        }
    }

    /// Waits for the messages sent so far to be handled (a request is
    /// answered after them: the server handles them in order).
    fn sync(&mut self) {
        self.notes.clear();
        self.request("isocron/noSuchRequest", Json::Null);
    }

    /// The diagnostics published last for `uri` since the last `sync`.
    fn last(&self, uri: &str) -> Vec<Json> {
        let mut last = None;
        for n in &self.notes {
            if n.get("method").as_str() == Some("textDocument/publishDiagnostics") && n.get("params").get("uri").as_str() == Some(uri) {
                last = Some(n.get("params").get("diagnostics").clone());
            }
        }
        match last.unwrap_or_else(|| panic!("no diagnostics published for {}", uri)) {
            Json::Arr(v) => v,
            other => panic!("{}", other),
        }
    }

    fn diagnostics(&mut self, uri: &str) -> Vec<Json> {
        self.sync();
        self.last(uri)
    }
}

fn uri(p: &Path) -> String {
    format!("file://{}", p.display())
}

fn pos(line: usize, character: usize) -> Json {
    obj(vec![("line", (line as u32).into()), ("character", (character as u32).into())])
}

fn at(uri: &str, line: usize, character: usize) -> Json {
    obj(vec![("textDocument", obj(vec![("uri", uri.into())])), ("position", pos(line, character))])
}

fn open(c: &mut Client, uri: &str, text: &str) {
    let doc = obj(vec![("uri", uri.into()), ("languageId", "isocron".into()), ("version", 1u32.into()), ("text", text.into())]);
    c.notify("textDocument/didOpen", obj(vec![("textDocument", doc)]));
}

fn change(c: &mut Client, uri: &str, text: &str) {
    let doc = obj(vec![("uri", uri.into()), ("version", 2u32.into())]);
    let change = obj(vec![("text", text.into())]);
    c.notify("textDocument/didChange", obj(vec![("textDocument", doc), ("contentChanges", vec![change].into())]));
}

fn hover_text(r: &Json) -> String {
    r.get("contents").get("value").as_str().unwrap_or("").to_string()
}

/// The place (line from 0, character from 0) of the nth whole `word` on a
/// line (from 1) of `src`.
fn place(src: &str, line: usize, word: &str, nth: usize) -> (usize, usize) {
    let l = src.lines().nth(line - 1).unwrap();
    let id = |c: Option<char>| c.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
    let mut found = 0;
    for (i, _) in l.match_indices(word) {
        if !id(l[..i].chars().last()) && !id(l[i + word.len()..].chars().next()) {
            found += 1;
            if found == nth {
                return (line - 1, i);
            }
        }
    }
    panic!("no {} #{} on line {}", word, nth, line);
}

fn dir(name: &str) -> PathBuf {
    // `ISC_TEST_TMP`: a directory of its own for a run of the suites at the
    // same time as another (both allocators at once), as in the tests of iscc
    let root = match std::env::var_os("ISC_TEST_TMP") {
        Some(t) => PathBuf::from(t),
        None => PathBuf::from(env!("CARGO_TARGET_TMPDIR")),
    };
    let d = root.join("isc-lsp-tests").join(name);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::canonicalize(&d).unwrap()
}

const GEOM: &str = "// the area of a disc
// of radius r
export function area(r: f64): f64 {
    return 3.0 * r * r;
}
";

const PROGRAM: &str = "import { area } from \"./geom\";
type Point = { x: f64, y: f64 };

function stdDev(x: f64[n]): f64 {
    const m = mean(x);
    return sqrt(sum((x - m) ** 2) / f64(n));
}

action main(args: string[k]): i32 {
    const p: Point = { x: 1.0, y: 2.0 };
    const xs = map(iota(10), (i) => f64(i) * p.x);
    print(stdDev(xs) + area(2.0));
    return 0;
}
";

#[test]
fn hover_definitions_errors_and_outline() {
    let d = dir("program");
    std::fs::write(d.join("geom.isc"), GEOM).unwrap();
    let file = d.join("h.isc");
    std::fs::write(&file, PROGRAM).unwrap();
    let u = uri(&file);
    let mut c = Client::start();
    let init = c.request("initialize", obj(vec![("processId", Json::Null), ("rootUri", uri(&d).into()), ("capabilities", obj(vec![]))]));
    let caps = init.get("capabilities");
    assert!(caps.get("hoverProvider") == &Json::Bool(true) && caps.get("definitionProvider") == &Json::Bool(true), "{}", init);
    c.notify("initialized", obj(vec![]));
    open(&mut c, &u, PROGRAM);
    assert_eq!(c.diagnostics(&u), Vec::<Json>::new());

    // the hovers: the code shown, and what it is
    let cases: &[((usize, &str, usize), &str)] = &[
        ((4, "stdDev", 1), "```isocron\nfunction stdDev(x: f64[n]): f64\n```\n*function*"),
        ((4, "x", 1), "```isocron\nx: f64[n]\n```\n*parameter*"),
        ((5, "m", 1), "```isocron\nm: f64\n```\n*const*"),
        ((5, "mean", 1), "```isocron\nfunction mean(x: f64[n]): f64\n```\n*function*"),
        ((6, "x", 1), "```isocron\nx: f64[n]\n```\n*parameter*"),
        ((6, "m", 1), "```isocron\nm: f64\n```\n*const*"),
        ((6, "n", 1), "```isocron\nn: i64\n```\n*size*"),
        ((9, "main", 1), "```isocron\naction main(args: string[k]): i32\n```\n*action*"),
        ((10, "p", 1), "```isocron\np: {x: f64, y: f64}\n```\n*const*"),
        ((11, "xs", 1), "```isocron\nxs: f64[10]\n```\n*const*"),
        ((11, "i", 1), "```isocron\ni: i64\n```\n*parameter*"),
        ((11, "i", 2), "```isocron\ni: i64\n```\n*parameter*"),
        ((11, "x", 1), "```isocron\nx: f64\n```\n*field*"),
        ((12, "stdDev", 1), "```isocron\nfunction stdDev(x: f64[n]): f64\n```\n*function*"),
        // a function of a module: its signature, then its comment
        ((12, "area", 1), "```isocron\nfunction area(r: f64): f64\n```\n*function*\n\nthe area of a disc\nof radius r"),
    ];
    for ((line, word, nth), want) in cases {
        let (l, ch) = place(PROGRAM, *line, word, *nth);
        let r = c.request("textDocument/hover", at(&u, l, ch));
        assert_eq!(hover_text(&r), *want, "{} #{} at {}:{}", word, nth, l, ch);
        // the range is the word
        assert_eq!(r.get("range").get("start").get("character").as_u32(), Some(ch as u32));
        assert_eq!(r.get("range").get("end").get("character").as_u32(), Some((ch + word.len()) as u32));
    }
    // the pointer in the middle of a word
    let (l, ch) = place(PROGRAM, 4, "stdDev", 1);
    assert_eq!(hover_text(&c.request("textDocument/hover", at(&u, l, ch + 3))), cases[0].1);
    // a function of the standard library: its documentation, a link
    let (l, ch) = place(PROGRAM, 12, "print", 1);
    let t = hover_text(&c.request("textDocument/hover", at(&u, l, ch)));
    assert!(t.starts_with("`print(x)`") && t.contains("*Type*: `(T) => void`") && t.contains("docs/language.md#"), "{}", t);
    // nothing for a keyword, a number
    for (line, word) in [(5, "const"), (11, "10")] {
        let (l, ch) = place(PROGRAM, line, word, 1);
        assert_eq!(c.request("textDocument/hover", at(&u, l, ch)), Json::Null, "{}", word);
    }

    // the definitions: of a function of a module, of a local read
    let (l, ch) = place(PROGRAM, 12, "area", 1);
    let r = c.request("textDocument/definition", at(&u, l, ch));
    assert!(r.get("uri").as_str().unwrap().ends_with("/geom.isc"), "{}", r);
    assert_eq!(r.get("range").get("start"), &pos(2, 16));
    let (l, ch) = place(PROGRAM, 6, "m", 1);
    let r = c.request("textDocument/definition", at(&u, l, ch));
    assert_eq!(r.get("uri").as_str(), Some(u.as_str()));
    assert_eq!(r.get("range").get("start"), &pos(4, 10));
    let (l, ch) = place(PROGRAM, 12, "stdDev", 1);
    assert_eq!(c.request("textDocument/definition", at(&u, l, ch)).get("range").get("start"), &pos(3, 9));

    // the outline
    let r = c.request("textDocument/documentSymbol", obj(vec![("textDocument", obj(vec![("uri", u.as_str().into())]))]));
    let Json::Arr(syms) = &r else { panic!("{}", r) };
    let names: Vec<&str> = syms.iter().map(|s| s.get("name").as_str().unwrap()).collect();
    assert_eq!(names, ["Point", "stdDev", "main"]);

    // the text as typed, not saved: an error at its place...
    let wrong = PROGRAM.replace("const m = mean(x);", "const m = mean(y);");
    change(&mut c, &u, &wrong);
    let ds = c.diagnostics(&u);
    assert_eq!(ds.len(), 1, "{:?}", ds);
    assert_eq!(ds[0].get("message").as_str(), Some("unknown variable `y`"));
    assert_eq!(ds[0].get("range").get("start"), &pos(4, 19));
    assert_eq!(ds[0].get("range").get("end"), &pos(4, 20));
    // ...where a function of the file is still told by its declaration
    let (l, ch) = place(&wrong, 12, "stdDev", 1);
    let t = hover_text(&c.request("textDocument/hover", at(&u, l, ch)));
    assert_eq!(t, "```isocron\nfunction stdDev(x: f64[n]): f64\n```");
    // ...and a new constant, known once the text checks
    let more = PROGRAM.replace("    return sqrt(", "    const q = m * 2.0;\n    return sqrt(");
    change(&mut c, &u, &more);
    assert_eq!(c.diagnostics(&u), Vec::<Json>::new());
    let (l, ch) = place(&more, 6, "q", 1);
    assert_eq!(hover_text(&c.request("textDocument/hover", at(&u, l, ch))), "```isocron\nq: f64\n```\n*const*");

    // a request it does not handle: an error, not a silence
    let r = c.request("textDocument/formatting", at(&u, 0, 0));
    assert_eq!(r.get("code"), &Json::Num(-32601.0));
    assert_eq!(c.request("shutdown", Json::Null), Json::Null);
    c.notify("exit", Json::Null);
    assert_eq!(c.child.wait().unwrap().code(), Some(0));
}

/// An error in a module the file imports is put on the module, and cleared
/// there when it is gone.
#[test]
fn an_error_in_an_imported_module() {
    let d = dir("module");
    let bad = "export function twice(x: f64): f64 {\n    return x + y;\n}\n";
    std::fs::write(d.join("util.isc"), bad).unwrap();
    let main = "import { twice } from \"./util\";\n\naction main(args: string[k]): i32 {\n    print(twice(1.0));\n    return 0;\n}\n";
    let file = d.join("main.isc");
    std::fs::write(&file, main).unwrap();
    let (u, mu) = (uri(&file), uri(&d.join("util.isc")));
    let mut c = Client::start();
    c.request("initialize", obj(vec![("processId", Json::Null), ("capabilities", obj(vec![]))]));
    open(&mut c, &u, main);
    let ds = c.diagnostics(&mu);
    assert_eq!(ds.len(), 1);
    assert_eq!(ds[0].get("message").as_str(), Some("unknown variable `y`"));
    assert_eq!(ds[0].get("range").get("start"), &pos(1, 15));
    // the module fixed and saved
    std::fs::write(d.join("util.isc"), bad.replace("x + y", "x + x")).unwrap();
    c.notify("textDocument/didSave", obj(vec![("textDocument", obj(vec![("uri", mu.as_str().into())]))]));
    c.sync();
    assert_eq!(c.last(&mu), Vec::<Json>::new());
    assert_eq!(c.last(&u), Vec::<Json>::new());
    c.request("shutdown", Json::Null);
    c.notify("exit", Json::Null);
    assert_eq!(c.child.wait().unwrap().code(), Some(0));
}

/// `isc-lsp --version`, and the end of its input without `shutdown`: the
/// code 1 the protocol asks for.
#[test]
fn version_and_end_of_input() {
    let o = Command::new(env!("CARGO_BIN_EXE_isc-lsp")).arg("--version").output().unwrap();
    assert_eq!(String::from_utf8(o.stdout).unwrap().trim(), format!("isc-lsp {}", env!("CARGO_PKG_VERSION")));
    let mut c = Client::start();
    drop(c.input);
    assert_eq!(c.child.wait().unwrap().code(), Some(1));
}
