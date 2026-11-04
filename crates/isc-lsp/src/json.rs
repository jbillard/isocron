//! The JSON of the messages of the protocol (D269): a value, read from a
//! text and written back. Small on purpose: the workspace has no
//! dependency for it.

use std::fmt;

#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    /// The members in their order.
    Obj(Vec<(String, Json)>),
}

static NULL: Json = Json::Null;

impl Json {
    /// The member `key` of an object (`Null` when absent or not an object).
    pub fn get(&self, key: &str) -> &Json {
        match self {
            Json::Obj(m) => m.iter().find(|(k, _)| k == key).map(|(_, v)| v).unwrap_or(&NULL),
            _ => &NULL,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_u32(&self) -> Option<u32> {
        match self {
            Json::Num(n) if *n >= 0.0 && *n <= u32::MAX as f64 && n.fract() == 0.0 => Some(*n as u32),
            _ => None,
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Json::Null)
    }
}

/// An object from its members.
pub fn obj(members: Vec<(&str, Json)>) -> Json {
    Json::Obj(members.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

impl From<&str> for Json {
    fn from(s: &str) -> Json {
        Json::Str(s.to_string())
    }
}

impl From<String> for Json {
    fn from(s: String) -> Json {
        Json::Str(s)
    }
}

impl From<u32> for Json {
    fn from(n: u32) -> Json {
        Json::Num(n as f64)
    }
}

impl From<bool> for Json {
    fn from(b: bool) -> Json {
        Json::Bool(b)
    }
}

impl From<Vec<Json>> for Json {
    fn from(v: Vec<Json>) -> Json {
        Json::Arr(v)
    }
}

fn write_str(f: &mut fmt::Formatter<'_>, s: &str) -> fmt::Result {
    f.write_str("\"")?;
    for c in s.chars() {
        match c {
            '"' => f.write_str("\\\"")?,
            '\\' => f.write_str("\\\\")?,
            '\n' => f.write_str("\\n")?,
            '\r' => f.write_str("\\r")?,
            '\t' => f.write_str("\\t")?,
            c if (c as u32) < 0x20 => write!(f, "\\u{:04x}", c as u32)?,
            c => write!(f, "{}", c)?,
        }
    }
    f.write_str("\"")
}

impl fmt::Display for Json {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Json::Null => f.write_str("null"),
            Json::Bool(b) => write!(f, "{}", b),
            Json::Num(n) if n.fract() == 0.0 && n.abs() < 1e15 => write!(f, "{}", *n as i64),
            Json::Num(n) if n.is_finite() => write!(f, "{}", n),
            Json::Num(_) => f.write_str("null"),
            Json::Str(s) => write_str(f, s),
            Json::Arr(v) => {
                f.write_str("[")?;
                for (i, x) in v.iter().enumerate() {
                    if i > 0 {
                        f.write_str(",")?;
                    }
                    write!(f, "{}", x)?;
                }
                f.write_str("]")
            }
            Json::Obj(m) => {
                f.write_str("{")?;
                for (i, (k, v)) in m.iter().enumerate() {
                    if i > 0 {
                        f.write_str(",")?;
                    }
                    write_str(f, k)?;
                    write!(f, ":{}", v)?;
                }
                f.write_str("}")
            }
        }
    }
}

/// The value of a text of JSON, or `None` when it is not one.
pub fn parse(text: &str) -> Option<Json> {
    let mut p = Parser { s: text.as_bytes(), i: 0, depth: 0 };
    let v = p.value()?;
    p.ws();
    (p.i == p.s.len()).then_some(v)
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
    depth: u32,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && matches!(self.s[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }

    fn eat(&mut self, word: &str) -> bool {
        if self.s[self.i..].starts_with(word.as_bytes()) {
            self.i += word.len();
            true
        } else {
            false
        }
    }

    fn value(&mut self) -> Option<Json> {
        self.ws();
        // a message nested this deep is not one of the protocol
        self.depth += 1;
        if self.depth > 256 {
            return None;
        }
        let v = match *self.s.get(self.i)? {
            b'n' if self.eat("null") => Json::Null,
            b't' if self.eat("true") => Json::Bool(true),
            b'f' if self.eat("false") => Json::Bool(false),
            b'"' => Json::Str(self.string()?),
            b'[' => {
                self.i += 1;
                let mut v = Vec::new();
                self.ws();
                if !self.eat("]") {
                    loop {
                        v.push(self.value()?);
                        self.ws();
                        if self.eat("]") {
                            break;
                        }
                        if !self.eat(",") {
                            return None;
                        }
                    }
                }
                Json::Arr(v)
            }
            b'{' => {
                self.i += 1;
                let mut m = Vec::new();
                self.ws();
                if !self.eat("}") {
                    loop {
                        self.ws();
                        if self.s.get(self.i) != Some(&b'"') {
                            return None;
                        }
                        let k = self.string()?;
                        self.ws();
                        if !self.eat(":") {
                            return None;
                        }
                        m.push((k, self.value()?));
                        self.ws();
                        if self.eat("}") {
                            break;
                        }
                        if !self.eat(",") {
                            return None;
                        }
                    }
                }
                Json::Obj(m)
            }
            b'-' | b'0'..=b'9' => {
                let a = self.i;
                while self.i < self.s.len() && matches!(self.s[self.i], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9') {
                    self.i += 1;
                }
                Json::Num(std::str::from_utf8(&self.s[a..self.i]).ok()?.parse().ok()?)
            }
            _ => return None,
        };
        self.depth -= 1;
        Some(v)
    }

    fn hex4(&mut self) -> Option<u32> {
        let h = std::str::from_utf8(self.s.get(self.i..self.i + 4)?).ok()?;
        self.i += 4;
        u32::from_str_radix(h, 16).ok()
    }

    fn string(&mut self) -> Option<String> {
        self.i += 1;
        let mut out = Vec::new();
        loop {
            let c = *self.s.get(self.i)?;
            self.i += 1;
            match c {
                b'"' => return String::from_utf8(out).ok(),
                b'\\' => {
                    let e = *self.s.get(self.i)?;
                    self.i += 1;
                    let ch = match e {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => {
                            let hi = self.hex4()?;
                            let code = if (0xd800..0xdc00).contains(&hi) && self.eat("\\u") {
                                let lo = self.hex4()?;
                                0x10000 + ((hi - 0xd800) << 10) + (lo.checked_sub(0xdc00)? & 0x3ff)
                            } else {
                                hi
                            };
                            char::from_u32(code).unwrap_or('\u{fffd}')
                        }
                        _ => return None,
                    };
                    let mut b = [0; 4];
                    out.extend_from_slice(ch.encode_utf8(&mut b).as_bytes());
                }
                c => out.push(c),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_and_written_back() {
        let t = r#"{"jsonrpc":"2.0","id":7,"params":{"text":"a \"b\"\n\u00e9\ud83d\ude00","n":[1,-2.5,true,null]}}"#;
        let v = parse(t).unwrap();
        assert_eq!(v.get("id").as_u32(), Some(7));
        assert_eq!(v.get("params").get("text").as_str(), Some("a \"b\"\né😀"));
        assert!(v.get("nothing").is_null() && v.get("id").get("x").is_null());
        assert_eq!(parse(&v.to_string()), Some(v.clone()));
        assert_eq!(obj(vec![("a", 3u32.into()), ("b", "x\ty".into())]).to_string(), r#"{"a":3,"b":"x\ty"}"#);
    }

    #[test]
    fn not_json() {
        for t in ["", "{", "[1,]", "{\"a\" 1}", "tru", "\"\\q\"", "1 2", &"[".repeat(1000)] {
            assert_eq!(parse(t), None, "{}", t);
        }
    }
}
