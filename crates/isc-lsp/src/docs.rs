//! The documentation shown on hover (D269): the comments `//` above a
//! declaration, and the functions of the standard library as
//! `docs/language.md` describes them, read from the documentation built
//! into the server (never older than it).

use std::collections::HashMap;
use std::sync::OnceLock;

const LANGUAGE_MD: &str = include_str!("../../../docs/language.md");

/// Where the documentation is read online.
pub const DOCS_URL: &str = "https://github.com/jbillard/isocron/blob/main/docs/language.md";

/// A function as the documentation describes it: the forms of its row (or
/// the title of its section), the other cells of the row under the headers
/// of its table (or the first paragraph of the section), its section.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub forms: String,
    pub fields: Vec<(String, String)>,
    pub section: String,
    pub anchor: String,
}

/// The anchor GitHub gives a heading.
pub fn slug(heading: &str) -> String {
    let s: String = heading.to_lowercase().chars().filter(|&c| c.is_alphanumeric() || c == '_' || c == '-' || c == ' ').collect();
    s.trim().replace(' ', "-")
}

fn is_ident(s: &str) -> bool {
    let mut cs = s.chars();
    cs.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') && cs.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The texts between backquotes.
fn quoted(s: &str) -> Vec<&str> {
    s.split('`').skip(1).step_by(2).collect()
}

/// `` `reduce` ``, `` `min` and `max` ``: a title made only of names of
/// functions; those names.
fn title_names(section: &str) -> Option<Vec<&str>> {
    let mut rest = section;
    let mut names = Vec::new();
    loop {
        let r = rest.strip_prefix('`')?;
        let end = r.find('`')?;
        if !is_ident(&r[..end]) {
            return None;
        }
        names.push(&r[..end]);
        rest = &r[end + 1..];
        if rest.is_empty() {
            return Some(names);
        }
        rest = rest.strip_prefix(", ").or_else(|| rest.strip_prefix(" and "))?;
    }
}

/// The function a form of a row writes: `split(s, sep)`, `x.map(f)`,
/// `s.length`.
fn form_name(form: &str) -> Option<&str> {
    let call_at = |from: usize| -> Option<&str> {
        let r = form[from..].trim_start();
        let n = r.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(r.len());
        (n > 0 && is_ident(&r[..n]) && r[n..].trim_start().starts_with('(')).then(|| &r[..n])
    };
    let starts = std::iter::once(0).chain(form.match_indices('.').map(|(i, _)| i + 1));
    for s in starts {
        if let Some(n) = call_at(s) {
            return Some(n);
        }
    }
    let (_, last) = form.rsplit_once('.')?;
    is_ident(last).then_some(last)
}

/// The functions `md` describes: each row of a table whose first cell
/// writes a call, each section titled by names of functions.
pub fn stdlib_of(md: &str) -> HashMap<String, Vec<Entry>> {
    let mut out: HashMap<String, Vec<Entry>> = HashMap::new();
    let (mut section, mut anchor) = (String::new(), String::new());
    let mut headers: Vec<String> = Vec::new();
    let lines: Vec<&str> = md.lines().collect();
    for (i, l) in lines.iter().enumerate() {
        let hashes = l.chars().take_while(|&c| c == '#').count();
        if (2..=4).contains(&hashes) && l[hashes..].starts_with(' ') {
            section = l[hashes + 1..].to_string();
            anchor = slug(&section);
            if let Some(names) = title_names(&section) {
                let mut j = i + 1;
                while j < lines.len() && lines[j].trim().is_empty() {
                    j += 1;
                }
                let mut para = Vec::new();
                while j < lines.len() && !lines[j].trim().is_empty() && !["```", "|", "#"].iter().any(|p| lines[j].starts_with(p)) {
                    para.push(lines[j].trim());
                    j += 1;
                }
                for n in names {
                    let e = Entry { forms: section.clone(), fields: vec![(String::new(), para.join(" "))], section: section.clone(), anchor: anchor.clone() };
                    out.entry(n.to_string()).or_default().push(e);
                }
            }
            continue;
        }
        if !l.starts_with('|') {
            headers.clear();
            continue;
        }
        let cells: Vec<String> = l.trim().trim_matches('|').split(" | ").map(|c| c.trim().to_string()).collect();
        if headers.is_empty() {
            headers = cells;
            continue;
        }
        // a row of calls starts with one (not `calls `f(x)`, indexing...`)
        if l.chars().all(|c| "|-: ".contains(c)) || !cells[0].starts_with('`') {
            continue;
        }
        for form in quoted(&cells[0]) {
            if let Some(n) = form_name(form) {
                let fields = cells.iter().enumerate().skip(1).map(|(k, c)| (headers.get(k).cloned().unwrap_or_default(), c.clone())).collect();
                let e = Entry { forms: cells[0].clone(), fields, section: section.clone(), anchor: anchor.clone() };
                let v = out.entry(n.to_string()).or_default();
                if !v.contains(&e) {
                    v.push(e);
                }
            }
        }
    }
    out
}

/// The functions of `docs/language.md`.
pub fn stdlib() -> &'static HashMap<String, Vec<Entry>> {
    static TABLE: OnceLock<HashMap<String, Vec<Entry>>> = OnceLock::new();
    TABLE.get_or_init(|| stdlib_of(LANGUAGE_MD))
}

/// The documentation of `word` in the standard library, as Markdown: its
/// forms, the cells of its row under their headers, a link to its section.
pub fn stdlib_markdown(word: &str) -> Option<String> {
    let entries = stdlib().get(word)?;
    let parts: Vec<String> = entries
        .iter()
        .map(|e| {
            let fields: Vec<String> = e.fields.iter().map(|(h, v)| if h.is_empty() { v.clone() } else { format!("*{}*: {}", h, v) }).collect();
            format!("{}\n\n{}\n\n[{}]({}#{})", e.forms, fields.join("  \n"), e.section.replace('`', ""), DOCS_URL, e.anchor)
        })
        .collect();
    Some(parts.join("\n\n---\n\n"))
}

/// The comment lines (`// ...`) right above line `n` (from 0) of `text`,
/// without their `//`, one per line; empty when there are none.
pub fn comment_above(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    for l in lines[..n.min(lines.len())].iter().rev() {
        let Some(c) = l.trim_start().strip_prefix("//") else { break };
        out.push(c.strip_prefix(' ').unwrap_or(c).trim_end());
    }
    out.reverse();
    out.join("\n")
}

/// Line `n` (from 0) of `text` as a declaration: up to its `{` for a
/// function or an action.
pub fn declaration_text(text: &str, n: usize) -> String {
    let l = text.lines().nth(n).unwrap_or("");
    let first = l.split_whitespace().find(|w| *w != "export" && *w != "declare");
    let is_fn = first.is_some_and(|w| w == "function" || w == "action");
    match l.find('{') {
        Some(b) if is_fn && b > 0 => l[..b].trim().to_string(),
        _ => l.trim().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_functions_of_the_documentation() {
        let t = stdlib();
        assert!(t.len() >= 60, "{} names", t.len());
        for n in ["map", "iota", "split", "reduce", "fold", "print", "trim", "tryParseI64", "length", "min", "max"] {
            assert!(t.contains_key(n), "no {}", n);
        }
        // a name of the table of the operators (`calls `f(x)``) is not one
        assert!(!t.contains_key("f"));
        let s = stdlib_markdown("split").unwrap();
        assert!(s.contains("`split(s, sep)`") && s.contains("*Type*: `(string, string) => string[k]`"), "{}", s);
        assert!(s.contains(&format!("({}#strings)", DOCS_URL)), "{}", s);
        assert!(stdlib_markdown("reduce").unwrap().contains("combines the elements"));
        assert_eq!(stdlib_markdown("noSuchFunction"), None);
    }

    #[test]
    fn forms_and_titles() {
        assert_eq!(form_name("split(s, sep)"), Some("split"));
        assert_eq!(form_name("x.map(f)"), Some("map"));
        assert_eq!(form_name("s.length"), Some("length"));
        assert_eq!(form_name("[e1, e2, ...]"), None);
        assert_eq!(title_names("`min` and `max`"), Some(vec!["min", "max"]));
        assert_eq!(title_names("Errors as values: `Result`"), None);
        assert_eq!(slug("Errors as values: `Result`"), "errors-as-values-result");
    }

    #[test]
    fn comments_and_declarations() {
        let text = "// the area of a disc\n// of radius r\nexport function area(r: f64): f64 {\n    return r;\n}\n\nconst N = 3;\n";
        assert_eq!(comment_above(text, 2), "the area of a disc\nof radius r");
        assert_eq!(comment_above(text, 6), "");
        assert_eq!(declaration_text(text, 2), "export function area(r: f64): f64");
        assert_eq!(declaration_text(text, 6), "const N = 3;");
    }
}
