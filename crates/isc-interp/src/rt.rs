//! The interpreter's side of the runtime: file I/O with the standard library,
//! and every message and number printed through `isc-runtime-core`, exactly
//! like compiled programs.

use isc_runtime_core::{self as sem, Panic};
pub use isc_runtime_core::{maximum_f32, maximum_f64, minimum_f32, minimum_f64, CANONICAL_NAN_BITS, PANIC_EXIT_CODE};

pub fn text(p: &Panic) -> String {
    p.to_string()
}

/// Shortest decimal form that reads back to the same value; NaN without sign.
pub fn format_f64(v: f64) -> String {
    let mut s = String::new();
    let _ = sem::write_f64(&mut s, v);
    s
}

pub fn panic_text(line: u32, col: u32, msg: &str) -> String {
    let mut s = String::new();
    let _ = sem::write_panic(&mut s, line, col, &msg);
    s
}

pub fn msg_bounds(index: i128, len: i64) -> String {
    text(&Panic::Bounds { index, len })
}

pub fn msg_slice(a: i64, b: i64, len: i64) -> String {
    text(&Panic::Slice { a, b, len })
}

pub fn msg_window(k: i64, len: i64) -> String {
    text(&Panic::Window { k, len })
}

pub fn msg_size(a: i64, b: i64) -> String {
    text(&Panic::Size { a, b })
}

pub fn msg_div0() -> String {
    text(&Panic::DivByZero)
}

pub fn msg_negsize(k: i64) -> String {
    text(&Panic::NegativeSize(k))
}

pub fn msg_negexp(k: i64) -> String {
    text(&Panic::NegativeExponent(k))
}

pub fn msg_stack(name: &str) -> String {
    text(&Panic::StackOverflow(name))
}

pub fn msg_bench(r: i64) -> String {
    text(&Panic::BenchReps(r))
}

pub fn parse_i64(s: &str) -> Result<i64, String> {
    sem::parse_i64(s).ok_or_else(|| text(&Panic::ParseI64(s)))
}

pub fn parse_f64(s: &str) -> Result<f64, String> {
    sem::parse_f64(s).ok_or_else(|| text(&Panic::ParseF64(s)))
}

pub fn msg_not_utf8(i: i64) -> String {
    text(&Panic::NotUtf8(i))
}

pub fn msg_split_empty() -> String {
    text(&Panic::SplitEmpty)
}

/// `readText` (D135): the file, checked to be UTF-8.
pub fn read_text_file(path: &str) -> Result<String, String> {
    let b = read_bytes_file(path)?;
    String::from_utf8(b).map_err(|e| text(&Panic::TextNotUtf8 { path, at: e.utf8_error().valid_up_to() as i64 }))
}

/// `parseCsvF64` (D135): the rows.
pub fn parse_csv(t: &str, skip: i64) -> Result<Vec<Vec<f64>>, String> {
    let mut vals = Vec::new();
    let (rows, cols) = sem::parse_csv_f64(t, skip, &mut |_, v| vals.push(v)).map_err(|p| text(&p))?;
    Ok((0..rows as usize).map(|r| vals[r * cols as usize..(r + 1) * cols as usize].to_vec()).collect())
}

/// The OS error in the same form as the compiled runtime (`strerror` text and
/// code; it is also the `Display` of `std::io::Error`).
fn io_text(e: &std::io::Error, path: &str, reading: bool) -> String {
    match e.raw_os_error() {
        // under Windows, the POSIX code and the text of glibc, as a program
        // on Windows (D247)
        #[cfg(windows)]
        Some(code) => {
            let code = sem::oserr::from_win32(code as u32);
            let detail = sem::oserr::text(code);
            if reading {
                text(&Panic::Read { path, detail, code })
            } else {
                text(&Panic::Write { path, detail, code })
            }
        }
        #[cfg(not(windows))]
        Some(code) => {
            let full = e.to_string();
            let detail = full.split(" (os error").next().unwrap_or(&full);
            if reading {
                text(&Panic::Read { path, detail, code })
            } else {
                text(&Panic::Write { path, detail, code })
            }
        }
        None => format!("cannot {} `{}`: {}", if reading { "read" } else { "write" }, path, e),
    }
}

/// The bytes given in place of the standard input (the tests); none: the
/// standard input of the process.
pub static STDIN: std::sync::Mutex<Option<Vec<u8>>> = std::sync::Mutex::new(None);

/// The whole standard input (D155): read once; a later read finds its end.
pub fn read_stdin() -> Result<Vec<u8>, String> {
    if let Some(b) = STDIN.lock().unwrap().as_mut() {
        return Ok(std::mem::take(b));
    }
    let mut b = Vec::new();
    std::io::Read::read_to_end(&mut std::io::stdin(), &mut b).map_err(|e| io_text(&e, "<standard input>", true))?;
    Ok(b)
}

pub fn read_bytes_file(path: &str) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|e| io_text(&e, path, true))
}

pub fn read_f64_file(path: &str) -> Result<Vec<f64>, String> {
    let b = read_bytes_file(path)?;
    if b.len() % 8 != 0 {
        return Err(text(&Panic::NotMultipleOf { path, len: b.len() as u64, size: 8 }));
    }
    Ok(b.chunks_exact(8).map(|c| f64::from_le_bytes(c.try_into().unwrap())).collect())
}

pub fn read_f32_file(path: &str) -> Result<Vec<f32>, String> {
    let b = read_bytes_file(path)?;
    if b.len() % 4 != 0 {
        return Err(text(&Panic::NotMultipleOf { path, len: b.len() as u64, size: 4 }));
    }
    Ok(b.chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect())
}

pub fn write_f32_file(path: &str, xs: &[f32]) -> Result<(), String> {
    let mut out = Vec::with_capacity(xs.len() * 4);
    for &x in xs {
        out.extend_from_slice(&sem::canonical_bits32(x).to_le_bytes());
    }
    write_bytes_file(path, &out)
}

pub fn format_f32(v: f32) -> String {
    let mut s = String::new();
    let _ = sem::write_f32(&mut s, v);
    s
}

pub fn f64_to_le_canonical(xs: &[f64]) -> Vec<u8> {
    let mut out = Vec::with_capacity(xs.len() * 8);
    for &x in xs {
        out.extend_from_slice(&sem::canonical_bits(x).to_le_bytes());
    }
    out
}

pub fn write_bytes_file(path: &str, data: &[u8]) -> Result<(), String> {
    std::fs::write(path, data).map_err(|e| io_text(&e, path, false))
}

pub fn write_f64_file(path: &str, xs: &[f64]) -> Result<(), String> {
    write_bytes_file(path, &f64_to_le_canonical(xs))
}

/// The code of an error of the system (D322), the same as the runtime's:
/// 1 not found, 2 permission denied, 3 already exists, 4 not empty, 5
/// refused, 6 timed out, otherwise the error of the system negated.
pub fn io_code(e: &std::io::Error) -> i64 {
    use std::io::ErrorKind as K;
    match e.kind() {
        K::NotFound => 1,
        K::PermissionDenied => 2,
        K::AlreadyExists => 3,
        K::DirectoryNotEmpty => 4,
        K::ConnectionRefused => 5,
        K::TimedOut => 6,
        _ => -(e.raw_os_error().unwrap_or(1) as i64),
    }
}

/// A primitive of the system (D322), as the runtime does it: its value and
/// the code of its error in a tuple.
pub fn sys(o: isc_ir::SysOp, vs: &[crate::Value]) -> crate::Value {
    use crate::{Array, Value};
    use isc_ir::SysOp as S;
    use std::rc::Rc;
    let text = |v: &Value| match v {
        Value::Str(s) => s.to_string(),
        v => panic!("a string expected, got {:?}", v),
    };
    let tuple = |xs: Vec<Value>| Value::Tuple(Rc::new(xs));
    let code = |r: std::io::Result<()>| Value::Int(match r {
        Ok(()) => 0,
        Err(e) => io_code(&e),
    });
    let strs = |v: Vec<String>| Value::Array(Rc::new(Array::Str(v.into_iter().map(|x| Rc::from(x.as_str())).collect())));
    let lossy = |b: &[u8]| String::from_utf8_lossy(b).into_owned();
    match o {
        S::Getenv => match Some(text(&vs[0])).filter(|n| !n.is_empty() && !n.contains(['=', '\0'])).and_then(std::env::var_os) {
            Some(v) => tuple(vec![Value::Str(Rc::from(lossy(v.as_encoded_bytes()).as_str())), Value::Int(1)]),
            None => tuple(vec![Value::Str(Rc::from("")), Value::Int(0)]),
        },
        S::Environ => {
            let mut all: Vec<Vec<u8>> = std::env::vars_os().map(|(k, v)| [k.as_encoded_bytes(), b"=", v.as_encoded_bytes()].concat()).collect();
            all.sort();
            strs(all.iter().map(|b| lossy(b)).collect())
        }
        S::Stat => match std::fs::metadata(text(&vs[0])) {
            Ok(m) => {
                let kind = if m.is_file() { 1 } else if m.is_dir() { 2 } else { 3 };
                let at = m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos() as i64);
                tuple(vec![Value::Int(kind), Value::Int(m.len() as i64), Value::Int(at), Value::Int(0)])
            }
            Err(e) => tuple(vec![Value::Int(0), Value::Int(0), Value::Int(0), Value::Int(io_code(&e))]),
        },
        S::ReadDir => match std::fs::read_dir(text(&vs[0])) {
            Ok(d) => {
                let mut names: Vec<Vec<u8>> = d.filter_map(|x| x.ok()).map(|x| x.file_name().as_encoded_bytes().to_vec()).collect();
                names.sort();
                tuple(vec![strs(names.iter().map(|b| lossy(b)).collect()), Value::Int(0)])
            }
            Err(e) => tuple(vec![strs(Vec::new()), Value::Int(io_code(&e))]),
        },
        S::Mkdir => code(std::fs::create_dir(text(&vs[0]))),
        S::RemoveFile => code(std::fs::remove_file(text(&vs[0]))),
        S::RemoveDir => code(std::fs::remove_dir(text(&vs[0]))),
        S::Rename => code(std::fs::rename(text(&vs[0]), text(&vs[1]))),
        S::Now => Value::Int(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as i64)),
        S::Cwd => match std::env::current_dir() {
            Ok(p) => tuple(vec![Value::Str(Rc::from(lossy(p.as_os_str().as_encoded_bytes()).as_str())), Value::Int(0)]),
            Err(e) => tuple(vec![Value::Str(Rc::from("")), Value::Int(io_code(&e))]),
        },
        S::ReadFile => match std::fs::read(text(&vs[0])) {
            Ok(b) => tuple(vec![Value::Array(Rc::new(Array::Int(b.into_iter().map(|x| x as i64).collect()))), Value::Int(0)]),
            Err(e) => tuple(vec![Value::Array(Rc::new(Array::Int(Vec::new()))), Value::Int(io_code(&e))]),
        },
        S::WriteFile => {
            let bytes: Vec<u8> = match &vs[1] {
                Value::Array(a) => match &**a {
                    Array::Int(v) => v.iter().map(|&x| x as u8).collect(),
                    a => panic!("bytes expected, got {:?}", a),
                },
                v => panic!("bytes expected, got {:?}", v),
            };
            let append = matches!(vs[2], Value::Bool(true));
            let r = std::fs::OpenOptions::new().create(true).write(true).append(append).truncate(!append).open(text(&vs[0])).and_then(|mut f| std::io::Write::write_all(&mut f, &bytes));
            code(r)
        }
    }
}
