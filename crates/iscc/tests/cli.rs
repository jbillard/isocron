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

/// D232: `iscc completions bash|zsh|fish` prints the completion of the
/// shell; in bash, the commands, the options of each, their values and the
/// `.isc` files and directories (not the other files).
#[test]
fn completion_of_the_shell() {
    let out = |shell: &str| {
        let o = std::process::Command::new(env!("CARGO_BIN_EXE_iscc")).args(["completions", shell]).output().unwrap();
        assert_eq!(o.status.code(), Some(0), "{}", shell);
        String::from_utf8(o.stdout).unwrap()
    };
    assert!(out("bash").contains("complete -F _iscc iscc"));
    assert!(out("zsh").contains("bashcompinit") && out("zsh").contains("complete -F _iscc iscc"));
    assert!(out("fish").contains("complete -c iscc"));
    assert_eq!(std::process::Command::new(env!("CARGO_BIN_EXE_iscc")).args(["completions", "tcsh"]).output().unwrap().status.code(), Some(2));
    if !std::path::Path::new("/bin/bash").exists() {
        return;
    }
    let dir = tmp("completion_dir");
    let _ = std::fs::create_dir_all(dir.join("sub"));
    for f in ["a.isc", "b.isc", "c.txt"] {
        std::fs::write(dir.join(f), "").unwrap();
    }
    let script = tmp("completion.bash");
    std::fs::write(&script, out("bash")).unwrap();
    let cases: &[(&[&str], &str)] = &[
        (&["iscc", "bu"], "build"),
        (&["iscc", "build", "a.isc", "--i"], "--isa"),
        (&["iscc", "build", "a.isc", "--isa", "avx"], "avx avx2 avx512"),
        (&["iscc", "--color", "a"], "always auto"),
        (&["iscc", "completions", ""], "bash fish zsh"),
        (&["iscc", "run", "--l"], "--lanes --link"),
        (&["iscc", "check", ""], "a.isc b.isc sub"),
    ];
    for (words, want) in cases {
        let quoted: Vec<String> = words.iter().map(|w| format!("'{}'", w)).collect();
        let cmd = format!(
            "source '{}'; COMP_WORDS=({}); COMP_CWORD={}; _iscc; printf '%s\\n' \"${{COMPREPLY[@]}}\" | sort | tr '\\n' ' '",
            script.display(),
            quoted.join(" "),
            words.len() - 1
        );
        let o = std::process::Command::new("/bin/bash").arg("-c").arg(&cmd).current_dir(&dir).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&o.stdout).trim(), *want, "{:?}: {}", words, String::from_utf8_lossy(&o.stderr));
    }
}

/// `iscc --version` (D251): the version of the workspace.
#[test]
fn version_of_the_compiler() {
    let o = std::process::Command::new(env!("CARGO_BIN_EXE_iscc")).arg("--version").output().unwrap();
    assert_eq!(o.status.code(), Some(0));
    let out = String::from_utf8(o.stdout).unwrap();
    assert!(out.starts_with(&format!("iscc {} (", env!("CARGO_PKG_VERSION"))), "{}", out);
}
