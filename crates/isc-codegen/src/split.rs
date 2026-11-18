//! A large module of LLVM IR cut into parts compiled at once (D711): each
//! part a module of its own, optimized and made into machine code by a
//! thread of its own, the parts linked together.
//!
//! The text of the module, as `emit` writes it, is cut where its functions
//! start and end. The functions are put together as LLVM would inline them:
//!
//! - a function called by one function alone goes with it (LLVM inlines a
//!   function whose call is the last one almost always), unless it is
//!   larger than LLVM would inline even then (`ATTACH_LINES`);
//! - a small function called by several (`DUP_LINES` lines at most) is
//!   copied into each part that calls it, internal there, as it was in the
//!   whole module (so that each part inlines it as the whole module did);
//! - the others (large, or whose address is taken, or called from outside)
//!   start a group of their own, seen by the other parts (hidden: never
//!   exported from the program).
//!
//! The groups are shared between the parts by their lines, the largest
//! first, each to the part that has the fewest. The variables of the module
//! are in every part, defined once in the program (`linkonce_odr` in a
//! comdat of their name: the linker keeps one), so that a part folds what
//! it reads of a constant as the whole module did. The declarations, the
//! comdats and the metadata are in every part, and each function defined
//! elsewhere is declared.

use std::collections::{HashMap, HashSet};

/// The lines of the bodies of the functions of a part, at least: a module
/// with fewer than two parts of these is compiled whole (the programs of
/// the measures have 58 000 lines at most).
pub const PART_LINES: usize = 100_000;

/// The largest function copied into each part that calls it: LLVM inlines
/// a function of several calls up to a cost of 250 (about 50 instructions,
/// more with its bonuses); 300 or 2 000 lines, the largest program of
/// isocron-ui took the same time (61 and 64 s), 10 000 and 90 000 lines
/// copied into each part.
const DUP_LINES: usize = 500;

/// The largest function that goes with the one function that calls it: LLVM
/// inlines a function at its last call up to a cost of 250 + 15 000 (its
/// threshold at `O3` and its bonus for the last call to an internal
/// function), about 3 000 instructions at 5 each.
const ATTACH_LINES: usize = 4_000;

/// A function of the module.
struct Func {
    name: String,
    /// The line `define ... {`.
    header: usize,
    /// The line after its `}`.
    end: usize,
    /// Internal to the module (`define internal` or `private`).
    internal: bool,
    /// The functions of the module it calls, once each.
    callees: Vec<usize>,
    /// The functions that call it.
    callers: Vec<usize>,
    /// Its address taken (read other than by a call).
    taken: bool,
}

/// Where a function goes.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Place {
    /// In the group of this function.
    Group(usize),
    /// Copied into each part that calls it.
    Copied,
}

/// The names `@name` on a line, each with whether it is the function a
/// `call` calls (the name right after `call ...` and before its `(`).
fn names_on(line: &str) -> Vec<(&str, bool)> {
    let b = line.as_bytes();
    let call_at = line.find("call ");
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'@' {
            let s = i + 1;
            let mut j = s;
            while j < b.len() && (b[j].is_ascii_alphanumeric() || matches!(b[j], b'_' | b'.' | b'$' | b'-')) {
                j += 1;
            }
            if j > s {
                let called = call_at.is_some_and(|c| c < i) && b.get(j) == Some(&b'(') && !line[call_at.unwrap_or(0)..i].contains('(');
                out.push((&line[s..j], called));
            }
            i = j.max(i + 1);
        } else {
            i += 1;
        }
    }
    out
}

/// The name of the function of a line `define ... @name(...`.
fn defined_name(line: &str) -> Option<&str> {
    let at = line.find('@')?;
    let rest = &line[at + 1..];
    Some(&rest[..rest.find('(')?])
}

/// `module` cut into at most `threads` parts, or nothing when it is
/// compiled whole: too small, one thread, debugging information (one unit
/// of it each part would make), aliases, or a variable or a function it
/// defines for other objects (but the start of the program: `main`, its
/// body and the start of the code, in the first part).
pub fn split_module(module: &str, threads: usize) -> Option<Vec<String>> {
    let lines: Vec<&str> = module.lines().collect();
    let mut funcs: Vec<Func> = Vec::new();
    let mut index: HashMap<&str, usize> = HashMap::new();
    let mut i = 0;
    let mut body_lines = 0usize;
    while i < lines.len() {
        let l = lines[i];
        if l.starts_with("define ") {
            let name = defined_name(l)?;
            let start = i;
            while i < lines.len() && lines[i] != "}" {
                i += 1;
            }
            if i == lines.len() {
                return None;
            }
            i += 1;
            body_lines += i - start;
            let internal = l.starts_with("define internal ") || l.starts_with("define private ");
            if !internal && !matches!(name, "main" | "isc_main_body" | "isc_code_start") {
                return None;
            }
            index.insert(name, funcs.len());
            funcs.push(Func { name: name.to_string(), header: start, end: i, internal, callees: Vec::new(), callers: Vec::new(), taken: false });
            continue;
        }
        if l.starts_with("!llvm.dbg.cu") || l.contains(" = alias ") || l.contains(" = ifunc ") {
            return None;
        }
        if let Some(rest) = l.strip_prefix('@') {
            // (a variable defined for other objects)
            let def = rest.split_once(" = ").map(|(_, d)| d).unwrap_or("");
            if !(def.starts_with("private ") || def.starts_with("internal ") || def.starts_with("external ") || def.starts_with("linkonce_odr ")) {
                return None;
            }
        }
        i += 1;
    }
    // (`$ISC_SPLIT_LINES`: fewer lines a part, for the tests)
    let per = std::env::var("ISC_SPLIT_LINES").ok().and_then(|v| v.trim().parse::<usize>().ok()).filter(|&n| n > 0).unwrap_or(PART_LINES);
    let parts = threads.min(body_lines / per);
    if parts < 2 || funcs.is_empty() {
        return None;
    }
    let mut inside = vec![false; lines.len()];
    for f in &funcs {
        for k in f.header..f.end {
            inside[k] = true;
        }
    }
    // what the program reaches, as LLVM finds it before it drops the rest
    // (`globaldce`): from the functions seen outside the module (`main`,
    // its body, the start of the code) and what lines other than functions
    // and variables name (metadata), through every name read by a function
    // or by the value of a variable. A function not reached is in no part,
    // nor a variable: whole, the module had them dropped (the functions of
    // the windows of Windows in a program for Linux, which name functions of
    // Windows); in a part, seen by the others, they would be kept
    let mut vindex: HashMap<&str, usize> = HashMap::new();
    let mut vlines: Vec<usize> = Vec::new();
    for (k, l) in lines.iter().enumerate() {
        if let (false, Some((name, def))) = (inside[k], l.strip_prefix('@').and_then(|r| r.split_once(" = "))) {
            if !def.starts_with("external ") {
                vindex.insert(name, vlines.len());
                vlines.push(k);
            }
        }
    }
    let (mut alive, mut alive_v) = (vec![false; funcs.len()], vec![false; vlines.len()]);
    let mut todo: Vec<(bool, usize)> = Vec::new();
    for (fi, f) in funcs.iter().enumerate() {
        if !f.internal {
            alive[fi] = true;
            todo.push((true, fi));
        }
    }
    for (k, l) in lines.iter().enumerate() {
        if !inside[k] && !l.starts_with('@') {
            for (name, _) in names_on(l) {
                if let Some(&g) = index.get(name) {
                    funcs[g].taken = true;
                    if !alive[g] {
                        alive[g] = true;
                        todo.push((true, g));
                    }
                }
            }
        }
    }
    while let Some((is_fn, x)) = todo.pop() {
        let range = if is_fn { funcs[x].header + 1..funcs[x].end } else { vlines[x]..vlines[x] + 1 };
        for k in range {
            for (name, _) in names_on(lines[k]) {
                if let Some(&g) = index.get(name) {
                    if !alive[g] {
                        alive[g] = true;
                        todo.push((true, g));
                    }
                } else if let Some(&v) = vindex.get(name) {
                    if !alive_v[v] {
                        alive_v[v] = true;
                        todo.push((false, v));
                    }
                }
            }
        }
    }
    let dead_vars: HashSet<usize> = (0..vlines.len()).filter(|&v| !alive_v[v]).map(|v| vlines[v]).collect();
    // the calls between the functions reached, and the functions whose
    // address is taken (in a function or a variable reached)
    for fi in 0..funcs.len() {
        if !alive[fi] {
            continue;
        }
        let mut callees = HashSet::new();
        for k in funcs[fi].header + 1..funcs[fi].end {
            for (name, called) in names_on(lines[k]) {
                if let Some(&g) = index.get(name) {
                    if called {
                        callees.insert(g);
                    } else {
                        funcs[g].taken = true;
                    }
                }
            }
        }
        let mut callees: Vec<usize> = callees.into_iter().collect();
        callees.sort_unstable();
        for &g in &callees {
            funcs[g].callers.push(fi);
        }
        funcs[fi].callees = callees;
    }
    for (v, &k) in vlines.iter().enumerate() {
        if alive_v[v] {
            for (name, _) in names_on(lines[k]) {
                if let Some(&g) = index.get(name) {
                    funcs[g].taken = true;
                }
            }
        }
    }
    let size = |f: &Func| f.end - f.header;
    // where each function goes
    let late: HashSet<&str> = crate::backend::LATE_INLINE.iter().copied().collect();
    let mut place: Vec<Option<Place>> = vec![None; funcs.len()];
    fn resolve(fi: usize, funcs: &[Func], late: &HashSet<&str>, place: &mut Vec<Option<Place>>, visiting: &mut Vec<bool>) -> Place {
        if let Some(p) = place[fi] {
            return p;
        }
        let f = &funcs[fi];
        let small = f.end - f.header <= DUP_LINES || late.contains(f.name.as_str());
        let p = if !f.internal || f.taken || f.callers.is_empty() || visiting[fi] {
            Place::Group(fi)
        } else if f.callers.len() == 1 && f.end - f.header <= ATTACH_LINES {
            visiting[fi] = true;
            let p = match resolve(f.callers[0], funcs, late, place, visiting) {
                Place::Group(r) => Place::Group(r),
                Place::Copied if small => Place::Copied,
                Place::Copied => Place::Group(fi),
            };
            visiting[fi] = false;
            p
        } else if small {
            Place::Copied
        } else {
            Place::Group(fi)
        };
        place[fi] = Some(p);
        p
    }
    let mut visiting = vec![false; funcs.len()];
    for fi in (0..funcs.len()).filter(|&fi| alive[fi]) {
        resolve(fi, &funcs, &late, &mut place, &mut visiting);
    }
    // (a function not reached: `None`, in no part)
    let place: Vec<Option<Place>> = place;
    // the groups shared between the parts: the largest first, each to the
    // part with the fewest lines; the start of the program in the first
    let mut group_lines: HashMap<usize, usize> = HashMap::new();
    for (fi, p) in place.iter().enumerate() {
        if let Some(Place::Group(r)) = p {
            *group_lines.entry(*r).or_insert(0) += size(&funcs[fi]);
        }
    }
    let mut groups: Vec<usize> = group_lines.keys().copied().collect();
    groups.sort_by_key(|&r| (std::cmp::Reverse(group_lines[&r]), r));
    let mut load = vec![0usize; parts];
    let mut part_of: HashMap<usize, usize> = HashMap::new();
    for r in groups {
        let k = if matches!(funcs[r].name.as_str(), "isc_code_start" | "main") { 0 } else { (0..parts).min_by_key(|&k| (load[k], k)).unwrap_or(0) };
        part_of.insert(r, k);
        load[k] += group_lines[&r];
    }
    // the functions each part defines: its groups, and the copies its
    // functions call
    let mut defines: Vec<Vec<bool>> = vec![vec![false; funcs.len()]; parts];
    for (fi, p) in place.iter().enumerate() {
        if let Some(Place::Group(r)) = p {
            defines[part_of[r]][fi] = true;
        }
    }
    for d in defines.iter_mut() {
        let mut todo: Vec<usize> = (0..funcs.len()).filter(|&fi| d[fi]).collect();
        while let Some(fi) = todo.pop() {
            for &g in &funcs[fi].callees {
                if place[g] == Some(Place::Copied) && !d[g] {
                    d[g] = true;
                    todo.push(g);
                }
            }
        }
    }
    if std::env::var_os("ISC_TIME").is_some() {
        let mut top: Vec<(usize, &str)> = group_lines.iter().map(|(r, n)| (*n, funcs[*r].name.as_str())).collect();
        top.sort_unstable_by(|a, b| b.cmp(a));
        let copied: Vec<usize> = defines.iter().map(|d| (0..funcs.len()).filter(|&fi| d[fi] && place[fi] == Some(Place::Copied)).map(|fi| size(&funcs[fi])).sum()).collect();
        eprintln!("iscc split: {} groups, largest {:?}; parts {:?}; copied lines {:?}", top.len(), &top[..top.len().min(6)], load, copied);
    }
    // the text of each part
    let mut common = String::new();
    for (k, l) in lines.iter().enumerate() {
        if inside[k] || dead_vars.contains(&k) {
            continue;
        }
        if let Some(rest) = l.strip_prefix('@') {
            if let Some((name, def)) = rest.split_once(" = ") {
                let def = def.strip_prefix("private ").or_else(|| def.strip_prefix("internal "));
                if let Some(def) = def {
                    // (defined once in the program: a comdat of its name)
                    let (head, align) = match def.rfind(", align ") {
                        Some(a) if def[a + 8..].bytes().all(|c| c.is_ascii_digit()) => (&def[..a], &def[a..]),
                        _ => (def, ""),
                    };
                    common.push_str(&format!("${} = comdat any\n@{} = linkonce_odr hidden {}, comdat{}\n", name, name, head, align));
                    continue;
                }
            }
        }
        common.push_str(l);
        common.push('\n');
    }
    let mut texts = Vec::with_capacity(parts);
    for d in &defines {
        let mut t = String::with_capacity(common.len() + module.len() / parts);
        t.push_str(&common);
        for (fi, f) in funcs.iter().enumerate() {
            let header = lines[f.header];
            let shown = place[fi] == Some(Place::Group(fi)) && f.internal;
            if d[fi] {
                // (a group's first function seen by the other parts)
                if shown {
                    t.push_str(&header.replacen("define internal ", "define hidden ", 1).replacen("define private ", "define hidden ", 1));
                } else {
                    t.push_str(header);
                }
                t.push('\n');
                for l in &lines[f.header + 1..f.end] {
                    t.push_str(l);
                    t.push('\n');
                }
            } else if shown || !f.internal {
                // declared where it is not defined
                let decl = header.trim_end_matches('{').trim_end();
                let decl = decl.replacen("define internal ", "declare hidden ", 1).replacen("define private ", "declare hidden ", 1).replacen("define ", "declare ", 1);
                t.push_str(&decl);
                t.push('\n');
            }
        }
        texts.push(t);
    }
    Some(texts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_calls() {
        let l = "  %t3 = tail call tailcc i64 @isc_fn_3(ptr @.str.1, i64 %t2)";
        assert_eq!(names_on(l), vec![("isc_fn_3", true), (".str.1", false)]);
        assert_eq!(names_on("  store ptr @isc_task_3.1, ptr %p"), vec![("isc_task_3.1", false)]);
        assert_eq!(defined_name("define internal tailcc void @isc_fn_12(ptr %a) align 64 {"), Some("isc_fn_12"));
    }
}
