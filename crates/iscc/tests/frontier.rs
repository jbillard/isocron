//! The frontier of v1 (D622): the functions of the runtime still written in
//! Rust are those of `crates/isc-runtime/frontier.txt`, no more and no
//! fewer. A function added in Rust fails this test: no new Rust, ever. A
//! function moved to Isocron is removed from the list in the same commit,
//! so the list only shrinks; v2 empties it.

use std::collections::BTreeSet;

/// The names `isc_…` that the runtime exports (`pub extern "C" fn`, unsafe
/// or not), in every file of its sources.
fn exported() -> BTreeSet<String> {
    let dir = format!("{}/../isc-runtime/src", env!("CARGO_MANIFEST_DIR"));
    let mut names = BTreeSet::new();
    for e in std::fs::read_dir(&dir).unwrap().flatten() {
        if e.path().extension().is_none_or(|x| x != "rs") {
            continue;
        }
        for line in std::fs::read_to_string(e.path()).unwrap().lines() {
            let t = line.trim_start();
            let rest = t.strip_prefix("pub unsafe extern \"C\" fn ").or_else(|| t.strip_prefix("pub extern \"C\" fn "));
            if let Some(name) = rest.and_then(|r| r.split('(').next()).filter(|n| n.starts_with("isc_")) {
                names.insert(name.to_string());
            }
        }
    }
    names
}

/// The names of the list (its comments and blank lines left out).
fn listed() -> BTreeSet<String> {
    let path = format!("{}/../isc-runtime/frontier.txt", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(String::from)
        .collect()
}

#[test]
fn the_rust_left_is_the_frontier() {
    let (now, list) = (exported(), listed());
    assert!(now.len() > 10, "{} functions found: the sources moved?", now.len());
    let added: Vec<&String> = now.difference(&list).collect();
    let gone: Vec<&String> = list.difference(&now).collect();
    assert!(added.is_empty(), "functions of the runtime in Rust that are not in crates/isc-runtime/frontier.txt (no new Rust: write them in Isocron): {:?}", added);
    assert!(gone.is_empty(), "functions of crates/isc-runtime/frontier.txt that the runtime no longer has (moved to Isocron: remove them from the list): {:?}", gone);
}
