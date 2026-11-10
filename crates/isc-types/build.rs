//! The places of the declarations of the prelude (D629), found once, when
//! the checker is built: a program parses the headers of its functions and
//! the bodies of those it reaches, not the whole of it.

fn main() {
    println!("cargo:rerun-if-changed=src/prelude.isc");
    let src = std::fs::read_to_string("src/prelude.isc").expect("src/prelude.isc");
    let places = isc_syntax::parser::outline(&src, isc_syntax::PRELUDE_FILE).unwrap_or_else(|d| panic!("the prelude, line {} column {}: {}", d.span.line, d.span.col, d.message));
    let mut out = String::from("// The places of the declarations of the prelude: made by build.rs (D629).\npub(crate) const PRELUDE_PLACES: &[isc_syntax::parser::Place] = &[\n");
    for p in places {
        out.push_str(&format!("    isc_syntax::parser::Place {{ start: {}, line: {}, col: {}, body: {:?}, end: {} }},\n", p.start, p.line, p.col, p.body, p.end));
    }
    out.push_str("];\n");
    let dir = std::env::var("OUT_DIR").expect("OUT_DIR");
    std::fs::write(std::path::Path::new(&dir).join("prelude_places.rs"), out).expect("prelude_places.rs");
}
