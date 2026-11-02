//! The interpreter's side of the runtime: file I/O with the standard library,
//! and every message and number printed through `isc-runtime-core`, exactly
//! like compiled programs.

use isc_runtime_core::{self as sem, Panic};
pub use isc_runtime_core::{maximum_f32, maximum_f64, minimum_f32, minimum_f64, CANONICAL_NAN_BITS, PANIC_EXIT_CODE};

fn text(p: &Panic) -> String {
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

/// `read_text` (D135): the file, checked to be UTF-8.
pub fn read_text_file(path: &str) -> Result<String, String> {
    let b = read_bytes_file(path)?;
    String::from_utf8(b).map_err(|e| text(&Panic::TextNotUtf8 { path, at: e.utf8_error().valid_up_to() as i64 }))
}

/// `parse_csv_f64` (D135): the rows.
pub fn parse_csv(t: &str, skip: i64) -> Result<Vec<Vec<f64>>, String> {
    let mut vals = Vec::new();
    let (rows, cols) = sem::parse_csv_f64(t, skip, &mut |_, v| vals.push(v)).map_err(|p| text(&p))?;
    Ok((0..rows as usize).map(|r| vals[r * cols as usize..(r + 1) * cols as usize].to_vec()).collect())
}

/// The OS error in the same form as the compiled runtime (`strerror` text and
/// code; it is also the `Display` of `std::io::Error`).
fn io_text(e: &std::io::Error, path: &str, reading: bool) -> String {
    match e.raw_os_error() {
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
