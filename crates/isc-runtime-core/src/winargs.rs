//! The command line and the environment of a program under Windows, which
//! the system gives in UTF-16 (D541): read by the runtime itself, without
//! the C runtime of MinGW (whose `argv` is in the code page of the system).

/// The code units of the arguments of the command line `w`, by the rules
/// of Windows (those of its C runtime since 2008, as Rust's standard library
/// reads them): the name of the program first, up to a space or a tab out
/// of quotes, its quotes removed (no escapes in it); then arguments
/// separated by spaces and tabs out of quotes, where `2n` backslashes
/// before a quote give `n` backslashes and the quote opens or closes, `2n +
/// 1` give `n` and the quote itself, other backslashes are themselves, and
/// `""` in quotes is a quote. `each` receives each argument, the name of the
/// program first; `cur` is the place of the argument being read (cleared
/// before each).
pub fn command_line_args(w: &[u16], cur: &mut alloc::vec::Vec<u16>, each: &mut dyn FnMut(&[u16])) {
    const QUOTE: u16 = b'"' as u16;
    const BACKSLASH: u16 = b'\\' as u16;
    const SPACE: u16 = b' ' as u16;
    const TAB: u16 = b'\t' as u16;
    let blank = |c: u16| c == SPACE || c == TAB;
    cur.clear();
    let mut i = 0;
    let mut quoted = false;
    while i < w.len() {
        let c = w[i];
        i += 1;
        if c == QUOTE {
            quoted = !quoted;
        } else if blank(c) && !quoted {
            break;
        } else {
            cur.push(c);
        }
    }
    each(cur);
    while i < w.len() && blank(w[i]) {
        i += 1;
    }
    cur.clear();
    let mut quoted = false;
    while i < w.len() {
        let c = w[i];
        i += 1;
        if blank(c) && !quoted {
            each(cur);
            cur.clear();
            while i < w.len() && blank(w[i]) {
                i += 1;
            }
        } else if c == BACKSLASH {
            let mut k = 1;
            while i < w.len() && w[i] == BACKSLASH {
                k += 1;
                i += 1;
            }
            if i < w.len() && w[i] == QUOTE {
                cur.extend(core::iter::repeat_n(BACKSLASH, k / 2));
                if k % 2 == 1 {
                    cur.push(QUOTE);
                    i += 1;
                }
            } else {
                cur.extend(core::iter::repeat_n(BACKSLASH, k));
            }
        } else if c == QUOTE && quoted {
            if i == w.len() {
                // `quoted` kept: the argument ends here, even empty
                break;
            }
            if w[i] == QUOTE {
                cur.push(QUOTE);
                i += 1;
            } else {
                quoted = false;
            }
        } else if c == QUOTE {
            quoted = true;
        } else {
            cur.push(c);
        }
    }
    if !cur.is_empty() || quoted {
        each(cur);
    }
}

/// The UTF-8 of the UTF-16 `w` into `out`, which holds at least `3 *
/// w.len()` bytes (a unit gives at most 3 bytes, a pair of two units 4); a
/// half of a pair alone gives U+FFFD, as Windows itself converts it. The
/// count of bytes written.
pub fn utf8_of_utf16(w: &[u16], out: &mut [u8]) -> usize {
    let mut n = 0;
    for c in core::char::decode_utf16(w.iter().copied()) {
        let c = c.unwrap_or(core::char::REPLACEMENT_CHARACTER);
        n += c.encode_utf8(&mut out[n..]).len();
    }
    n
}

/// The value of the variable `name` in `block`, the environment as
/// `NAME=value` strings each ended by a zero byte, the block by an empty
/// one: the place of its value in `block` and its length. Names are the
/// same whatever the case of their ASCII letters, as Windows compares
/// them; a name starts at its first byte, which may be `=` (the current
/// directories of the drives, `=C:=C:\x`).
pub fn env_find(block: &[u8], name: &[u8]) -> Option<(usize, usize)> {
    let mut at = 0;
    while at < block.len() && block[at] != 0 {
        let len = block[at..].iter().position(|&b| b == 0).unwrap_or(block.len() - at);
        let entry = &block[at..at + len];
        if let Some(eq) = entry.iter().skip(1).position(|&b| b == b'=').map(|p| p + 1) {
            if entry[..eq].eq_ignore_ascii_case(name) {
                return Some((at + eq + 1, len - eq - 1));
            }
        }
        at += len + 1;
    }
    None
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use alloc::string::String;
    use alloc::vec::Vec;

    fn args(line: &str) -> Vec<String> {
        let w: Vec<u16> = line.encode_utf16().collect();
        let mut out = Vec::new();
        let mut cur = Vec::new();
        command_line_args(&w, &mut cur, &mut |a| out.push(String::from_utf16(a).unwrap()));
        out
    }

    /// The examples of Microsoft's documentation of the parsing of the
    /// command line (`main` function and command-line arguments), and the
    /// cases around them.
    #[test]
    fn command_lines_as_windows_reads_them() {
        let cases: &[(&str, &[&str])] = &[
            (r#"p "abc" d e"#, &["p", "abc", "d", "e"]),
            (r#"p a\\\b d"e f"g h"#, &["p", r"a\\\b", "de fg", "h"]),
            (r#"p a\\\"b c d"#, &["p", r#"a\"b"#, "c", "d"]),
            (r#"p a\\\\"b c" d e"#, &["p", r"a\\b c", "d", "e"]),
            (r#"p a"b"" c d"#, &["p", r#"ab" c d"#]),
            ("p", &["p"]),
            ("p ", &["p"]),
            ("p\t a \t b\t", &["p", "a", "b"]),
            (r#"p "" a """#, &["p", "", "a", ""]),
            (r#"p """#, &["p", ""]),
            (r#"p ""x"#, &["p", "x"]),
            (r#""C:\Program Files\p.exe" one "two words" """#, &[r"C:\Program Files\p.exe", "one", "two words", ""]),
            // the name goes on after its closing quote, up to a space
            (r#""C:\dir\p.exe"x y"#, &[r"C:\dir\p.exex", "y"]),
            (r#"p "say \"hi\"" C:\dir\file"#, &["p", r#"say "hi""#, r"C:\dir\file"]),
            (r#"p a\ b\\"#, &["p", r"a\", r"b\\"]),
            (r#"p "a\\" b"#, &["p", r"a\", "b"]),
            ("p été 東京 🦀", &["p", "été", "東京", "🦀"]),
            ("", &[""]),
        ];
        for (line, want) in cases {
            assert_eq!(args(line), want.iter().map(|s| String::from(*s)).collect::<Vec<_>>(), "{}", line);
        }
    }

    #[test]
    fn utf16_to_utf8_with_replacements() {
        for s in ["", "a", "été", "東京", "🦀 et 𝄞", "\u{FFFD}"] {
            let w: Vec<u16> = s.encode_utf16().collect();
            let mut out = alloc::vec![0u8; 3 * w.len()];
            let n = utf8_of_utf16(&w, &mut out);
            assert_eq!(&out[..n], s.as_bytes());
        }
        // halves of pairs alone: U+FFFD each
        let w = [0xD83Eu16, b'a' as u16, 0xDD80, 0xD83E];
        let mut out = [0u8; 12];
        let n = utf8_of_utf16(&w, &mut out);
        assert_eq!(core::str::from_utf8(&out[..n]).unwrap(), "\u{FFFD}a\u{FFFD}\u{FFFD}");
    }

    #[test]
    fn variables_of_the_environment_whatever_their_case() {
        let block = b"=C:=C:\\x\0Path=C:\\Windows\0ISC_THREADS=4\0EMPTY=\0A=b=c\0\0";
        let value = |name: &str| env_find(block, name.as_bytes()).map(|(at, n)| &block[at..at + n]);
        assert_eq!(value("PATH"), Some(&b"C:\\Windows"[..]));
        assert_eq!(value("path"), Some(&b"C:\\Windows"[..]));
        assert_eq!(value("isc_threads"), Some(&b"4"[..]));
        assert_eq!(value("EMPTY"), Some(&b""[..]));
        assert_eq!(value("A"), Some(&b"b=c"[..]));
        assert_eq!(value("=C:"), Some(&b"C:\\x"[..]));
        assert_eq!(value("C:"), None);
        assert_eq!(value("ISC"), None);
        assert_eq!(value(""), None);
        assert_eq!(env_find(b"\0", b"A"), None);
    }
}
