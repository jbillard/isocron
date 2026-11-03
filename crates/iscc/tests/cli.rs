//! The messages of `iscc` (D231): the same text as without colors when the
//! error output is not a terminal (a pipe, here) or `NO_COLOR` is set; in
//! colors with `--color always`: the place in bold, `error` in red, the line
//! of the source and a caret under the column; after a build, what was made.

mod common;
use common::*;

fn iscc(args: &[&str], env: &[(&str, &str)]) -> (i32, String) {
    let mut c = std::process::Command::new(env!("CARGO_BIN_EXE_iscc"));
    c.args(args);
    for (k, v) in env {
        c.env(k, v);
    }
    let o = c.output().unwrap();
    (o.status.code().unwrap_or(-1), String::from_utf8_lossy(&o.stderr).to_string())
}

#[test]
fn messages_in_colors_on_demand() {
    let bad = tmp("cli_bad.isc");
    std::fs::write(&bad, "action main(args: string[k]): i32 {\n    const x = 1 + true;\n    return 0;\n}\n").unwrap();
    let b = bad.to_str().unwrap();
    let plain = format!("{}:2:19: error: expected an integer literal and a numeric type (class Num), found `boolean`\n", b);
    // a pipe: the text of before, whatever NO_COLOR
    assert_eq!(iscc(&["check", b], &[]), (1, plain.clone()));
    assert_eq!(iscc(&["check", b, "--color", "auto"], &[("NO_COLOR", "1")]), (1, plain.clone()));
    assert_eq!(iscc(&["--color=never", "check", b], &[]), (1, plain.clone()));
    // in colors
    let strip = |s: &str| {
        let mut out = String::new();
        let mut esc = false;
        for c in s.chars() {
            match (esc, c) {
                (_, '\u{1b}') => esc = true,
                (true, 'm') => esc = false,
                (true, _) => {}
                (false, c) => out.push(c),
            }
        }
        out
    };
    let (code, err) = iscc(&["check", "--color", "always", b], &[]);
    assert_eq!(code, 1);
    assert!(err.contains("\u{1b}[1;31merror\u{1b}[0m"), "{}", err);
    assert!(strip(&err).contains("2 |     const x = 1 + true;"), "{}", err);
    let caret = err.lines().find(|l| l.ends_with("^\u{1b}[0m")).expect("a caret");
    assert_eq!(strip(caret), format!("  | {}^", " ".repeat(18)));
    // a build: what was made
    let good = tmp("cli_good.isc");
    std::fs::write(&good, "action main(args: string[k]): i32 {\n    print(1);\n    return 0;\n}\n").unwrap();
    let out = tmp("cli_good");
    let (code, err) = iscc(&["build", good.to_str().unwrap(), "-o", out.to_str().unwrap(), "--color", "always"], &[]);
    assert_eq!(code, 0, "{}", err);
    assert!(strip(&err).starts_with("compiled "), "{}", err);
    let (code, err) = iscc(&["build", good.to_str().unwrap(), "-o", out.to_str().unwrap()], &[]);
    assert_eq!((code, err.as_str()), (0, ""));
    // a bad choice
    assert_eq!(iscc(&["check", b, "--color", "sometimes"], &[]).0, 2);
}
