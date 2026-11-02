//! Semantics shared by the runtime of compiled programs (`isc-runtime`,
//! without the standard library) and by the interpreter (`isc-interp`): the
//! text of every panic message, the printing of numbers and the canonical NaN.
//! Both sides write through `core::fmt::Write`, so that they produce the same
//! bytes. No allocation, no standard library: only `core`.

#![no_std]

use core::fmt::{self, Write};

/// Exit status of a program that panics.
pub const PANIC_EXIT_CODE: i32 = 101;

/// Alignment of every array (one cache line, SPEC section 8).
pub const ARRAY_ALIGN: usize = 64;

/// Canonical NaN written by `writeF64` (SPEC section 10).
pub const CANONICAL_NAN_BITS: u64 = 0x7FF8_0000_0000_0000;

/// Canonical NaN written by `writeF32`: sign 0, quiet bit 1, payload 0.
pub const CANONICAL_NAN32_BITS: u32 = 0x7FC0_0000;

/// `minimum` of IEEE 754-2019 (SPEC section 10, `min`): a NaN operand gives
/// NaN, and -0 is below +0.
pub fn minimum_f64(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if a == b {
        // equal: the same value, or zeros of possibly different signs
        if a.is_sign_negative() { a } else { b }
    } else if a < b {
        a
    } else {
        b
    }
}

/// `maximum` of IEEE 754-2019 (SPEC section 10, `max`): a NaN operand gives
/// NaN, and +0 is above -0.
pub fn maximum_f64(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if a == b {
        if a.is_sign_positive() { a } else { b }
    } else if a > b {
        a
    } else {
        b
    }
}

/// `minimum_f64` for binary32.
pub fn minimum_f32(a: f32, b: f32) -> f32 {
    minimum_f64(a as f64, b as f64) as f32
}

/// `maximum_f64` for binary32.
pub fn maximum_f32(a: f32, b: f32) -> f32 {
    maximum_f64(a as f64, b as f64) as f32
}

/// The bits written for `x` by `writeF32`: NaNs are canonical.
pub fn canonical_bits32(x: f32) -> u32 {
    if x.is_nan() {
        CANONICAL_NAN32_BITS
    } else {
        x.to_bits()
    }
}

/// `writeF64` for binary32: the shortest decimal form that reads back to
/// the same `f32`.
pub fn write_f32(w: &mut dyn Write, v: f32) -> fmt::Result {
    if v.is_nan() {
        w.write_str("NaN")
    } else if v.is_infinite() {
        w.write_str(if v > 0.0 { "inf" } else { "-inf" })
    } else {
        write!(w, "{}", v)
    }
}

/// The bits written for `x` by `writeF64`: NaNs are canonical.
pub fn canonical_bits(x: f64) -> u64 {
    if x.is_nan() {
        CANONICAL_NAN_BITS
    } else {
        x.to_bits()
    }
}

/// Shortest decimal form that reads back to the same value (the `Display`
/// algorithm of `core`); NaN without sign, `inf`, `-inf`.
pub fn write_f64(w: &mut dyn Write, v: f64) -> fmt::Result {
    if v.is_nan() {
        w.write_str("NaN")
    } else if v.is_infinite() {
        w.write_str(if v > 0.0 { "inf" } else { "-inf" })
    } else {
        write!(w, "{}", v)
    }
}

/// The message of a panic, with its position in the source.
pub fn write_panic(w: &mut dyn Write, line: u32, col: u32, msg: &dyn fmt::Display) -> fmt::Result {
    write!(w, "panic at line {}, column {}: {}", line, col, msg)
}

/// The causes of a panic (SPEC section 5).
pub enum Panic<'a> {
    Bounds { index: i128, len: i64 },
    /// A slice `x[a..b]` with `a < 0`, `a > b` or `b > len` (SPEC section 6).
    Slice { a: i64, b: i64, len: i64 },
    /// `window(x, k)` with `k > len` (SPEC section 6).
    Window { k: i64, len: i64 },
    Size { a: i64, b: i64 },
    DivByZero,
    NegativeExponent(i64),
    /// A `size` argument (SPEC section 4: a size is positive or zero).
    NegativeSize(i64),
    BenchReps(i64),
    ParseI64(&'a str),
    /// An operating-system error: `detail` is the text of `strerror(code)`.
    Read { path: &'a str, detail: &'a str, code: i32 },
    Write { path: &'a str, detail: &'a str, code: i32 },
    /// A file of `size`-byte values whose length is not a multiple of `size`.
    NotMultipleOf { path: &'a str, len: u64, size: u32 },
    OutOfMemory(i64),
    /// A block freed twice, or not allocated by the runtime (a defect of the
    /// compiler or of the runtime, never of a program).
    InvalidFree,
    /// The stack of the thread is full: too many nested calls of a recursive
    /// function (SPEC section 11).
    StackOverflow(&'a str),
    /// A `string` given to an `extern` function contains a NUL byte: it
    /// cannot be passed as a C string (D125).
    ExternNul(&'a str),
    /// `parseF64` of a text that is not a number (D134).
    ParseF64(&'a str),
    /// `stringOf` of bytes that are not UTF-8: the index of the first
    /// invalid byte (D134).
    NotUtf8(i64),
    /// `readText` of a file that is not UTF-8 (D135).
    TextNotUtf8 { path: &'a str, at: i64 },
    /// `split(s, "")` (D135).
    SplitEmpty,
    /// A CSV field that is not a number: its line (from 1) and field (from
    /// 1) (D135).
    CsvField { line: i64, field: i64, text: &'a str },
    /// A CSV row whose number of fields differs from the first row's.
    CsvRagged { line: i64, fields: i64, expected: i64 },
}

impl fmt::Display for Panic<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Panic::Bounds { index, len } => {
                write!(f, "index out of bounds: the index is {} but the length is {}", index, len)
            }
            Panic::Slice { a, b, len } => {
                write!(f, "slice out of bounds: [{}..{}] of an array of length {}", a, b, len)
            }
            Panic::Window { k, len } => write!(f, "window of {} elements over an array of length {}", k, len),
            Panic::StackOverflow(name) => write!(f, "stack overflow: too many nested calls of `{}`", name),
            Panic::ExternNul(name) => write!(f, "a string passed to the extern function `{}` contains a NUL byte", name),
            Panic::ParseF64(s) => write!(f, "parseF64: `{}` is not a valid number", s),
            Panic::NotUtf8(i) => write!(f, "stringOf: the bytes are not UTF-8 (invalid from byte {})", i),
            Panic::TextNotUtf8 { path, at } => write!(f, "readText: `{}` is not UTF-8 text (invalid from byte {})", path, at),
            Panic::SplitEmpty => write!(f, "split: the separator is empty"),
            Panic::CsvField { line, field, text } => write!(f, "csv: line {}, field {}: `{}` is not a number", line, field, text),
            Panic::CsvRagged { line, fields, expected } => {
                let word = |n: i64| if n == 1 { "field" } else { "fields" };
                write!(f, "csv: line {} has {} {}, the first row has {}", line, fields, word(*fields), expected)
            }
            Panic::Size { a, b } => write!(f, "size mismatch: {} != {}", a, b),
            Panic::DivByZero => f.write_str("integer division by zero"),
            Panic::NegativeExponent(k) => write!(f, "negative integer exponent: {}", k),
            Panic::NegativeSize(k) => write!(f, "negative size: {}", k),
            Panic::BenchReps(r) => write!(f, "bench: the number of repetitions must be at least 1, got {}", r),
            Panic::ParseI64(s) => write!(f, "parseI64: `{}` is not a valid integer", s),
            // same text as the `Display` of `std::io::Error` for OS errors
            Panic::Read { path, detail, code } => write!(f, "cannot read `{}`: {} (os error {})", path, detail, code),
            Panic::Write { path, detail, code } => write!(f, "cannot write `{}`: {} (os error {})", path, detail, code),
            Panic::NotMultipleOf { path, len, size } => {
                write!(f, "cannot read `{}`: its size ({} bytes) is not a multiple of {}", path, len, size)
            }
            Panic::OutOfMemory(n) => write!(f, "out of memory ({} bytes)", n),
            Panic::InvalidFree => write!(f, "invalid free (a block freed twice or not allocated by the runtime)"),
        }
    }
}

/// `parseI64` (SPEC section 12): decimal, optional sign.
pub fn parse_i64(s: &str) -> Option<i64> {
    s.parse::<i64>().ok()
}

/// `parseF64` (D134): the decimal forms of Rust's `f64::from_str`
/// (`1.5`, `-2e-3`, `inf`, `NaN`), correctly rounded.
pub fn parse_f64(s: &str) -> Option<f64> {
    s.parse::<f64>().ok()
}

/// The lines of a text (D135): separated by `\n`, a `\r` before it removed
/// (CRLF files); no empty last line after a final `\n`; none in `""`.
pub fn text_lines(s: &str) -> impl Iterator<Item = &str> {
    let body = s.strip_suffix('\n').unwrap_or(s);
    let empty = s.is_empty();
    body.split('\n').filter(move |_| !empty).map(|l| l.strip_suffix('\r').unwrap_or(l))
}

/// Reads a CSV text of numbers (D135): one row per line after the first
/// `skip` lines, fields separated by `,` and trimmed, every field a number
/// of `parseF64`; blank lines are ignored; every row has the number of
/// fields of the first. `emit(i, v)` receives the values in row order.
/// Returns the numbers of rows and of columns.
pub fn parse_csv_f64<'a>(text: &'a str, skip: i64, emit: &mut dyn FnMut(usize, f64)) -> Result<(i64, i64), Panic<'a>> {
    let mut rows = 0i64;
    let mut cols = -1i64;
    let mut k = 0usize;
    for (i, l) in text_lines(text).enumerate() {
        let line = i as i64 + 1;
        if line <= skip || l.trim().is_empty() {
            continue;
        }
        let mut fields = 0i64;
        for (j, f) in l.split(',').enumerate() {
            let t = f.trim();
            match parse_f64(t) {
                Some(v) => emit(k, v),
                None => return Err(Panic::CsvField { line, field: j as i64 + 1, text: t }),
            }
            k += 1;
            fields += 1;
        }
        if cols < 0 {
            cols = fields;
        } else if fields != cols {
            return Err(Panic::CsvRagged { line, fields, expected: cols });
        }
        rows += 1;
    }
    Ok((rows, cols.max(0)))
}

/// The text of a scalar for `toString` (D134): that of `print`.
pub fn write_scalar_i64(w: &mut dyn Write, v: i64) -> fmt::Result {
    write!(w, "{}", v)
}

/// Number of size classes of the allocator of the runtime (`isc-runtime`,
/// `alloc.rs`): blocks up to 2^48 bytes.
pub const ALLOC_CLASSES: usize = 16 + 38 * 8;

/// The size class of a block of `s` bytes (s >= 1) and the size of the blocks
/// of that class: multiples of 64 bytes up to 1 KiB, then 8 classes per power
/// of two, so that at most 12.5 % of a block is lost. `None` above 2^48.
pub const fn alloc_class(s: usize) -> Option<(usize, usize)> {
    if s <= 1024 {
        let i = if s == 0 { 0 } else { (s + 63) / 64 - 1 };
        return Some((i, (i + 1) * 64));
    }
    if s as u64 > 1u64 << 48 {
        return None;
    }
    // 2^k < s <= 2^(k+1), k >= 10; steps of 2^(k-3): shifts, no division
    let k = (usize::BITS - 1 - (s - 1).leading_zeros()) as usize;
    let sh = k - 3;
    let m = (s + (1 << sh) - 1) >> sh; // 9..=16
    Some((16 + (k - 10) * 8 + (m - 9), m << sh))
}

/// The size of the blocks of class `i` (`i < ALLOC_CLASSES`).
pub const fn alloc_class_size(i: usize) -> usize {
    if i < 16 {
        return (i + 1) * 64;
    }
    let k = (i - 16) / 8 + 10;
    let m = (i - 16) % 8 + 9;
    m << (k - 3)
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::string::String;

    #[test]
    fn alloc_classes_are_contiguous_and_tight() {
        let mut last = 0;
        for s in 1..(1usize << 22) {
            let (i, size) = alloc_class(s).unwrap();
            assert!(size >= s && size - s < (s / 8).max(64), "{} -> {}", s, size);
            assert_eq!(size % ARRAY_ALIGN, 0);
            assert_eq!(alloc_class_size(i), size);
            assert!(i == last || i == last + 1, "{} jumps from class {} to {}", s, last, i);
            last = i;
        }
        assert_eq!(alloc_class(1usize << 48).map(|c| c.0), Some(ALLOC_CLASSES - 1));
        assert_eq!(alloc_class_size(ALLOC_CLASSES - 1), 1usize << 48);
        assert!(alloc_class((1usize << 48) + 1).is_none());
    }

    fn f(v: f64) -> String {
        let mut s = String::new();
        write_f64(&mut s, v).unwrap();
        s
    }

    #[test]
    fn float_format() {
        assert_eq!(f(0.1), "0.1");
        assert_eq!(f(1.0), "1");
        assert_eq!(f(-0.0), "-0");
        assert_eq!(f(f64::NAN), "NaN");
        assert_eq!(f(-f64::NAN), "NaN");
        assert_eq!(f(f64::NEG_INFINITY), "-inf");
        assert_eq!(f(1.0 / 3.0).parse::<f64>().unwrap(), 1.0 / 3.0);
        assert_eq!(f(1e300), std::format!("{}", 1e300));
        let mut s = String::new();
        write_f32(&mut s, 0.1f32).unwrap();
        assert_eq!(s, "0.1");
    }

    #[test]
    fn nan_is_canonical() {
        assert_eq!(canonical_bits(-f64::NAN), CANONICAL_NAN_BITS);
        assert_eq!(canonical_bits(1.5), 1.5f64.to_bits());
    }

    #[test]
    fn messages() {
        let mut s = String::new();
        write_panic(&mut s, 3, 12, &Panic::Bounds { index: -1, len: 3 }).unwrap();
        assert_eq!(s, "panic at line 3, column 12: index out of bounds: the index is -1 but the length is 3");
        let e = std::io::Error::from_raw_os_error(2);
        let detail = std::format!("{}", e);
        let detail = detail.split(" (os error").next().unwrap();
        let ours = std::format!("{}", Panic::Read { path: "x", detail, code: 2 });
        assert_eq!(ours, std::format!("cannot read `x`: {}", e));
    }
}

// ---------------------------------------------------------------------
// mathematical functions (SPEC section 10, D108): the same code for the
// interpreter and every target, so that their results are the same bits

pub fn exp_f64(x: f64) -> f64 {
    libm::exp(x)
}
pub fn log_f64(x: f64) -> f64 {
    libm::log(x)
}
pub fn sin_f64(x: f64) -> f64 {
    libm::sin(x)
}
pub fn cos_f64(x: f64) -> f64 {
    libm::cos(x)
}
pub fn pow_f64(x: f64, y: f64) -> f64 {
    libm::pow(x, y)
}
pub fn fma_f64(a: f64, b: f64, c: f64) -> f64 {
    libm::fma(a, b, c)
}
pub fn exp_f32(x: f32) -> f32 {
    libm::expf(x)
}
pub fn log_f32(x: f32) -> f32 {
    libm::logf(x)
}
pub fn sin_f32(x: f32) -> f32 {
    libm::sinf(x)
}
pub fn cos_f32(x: f32) -> f32 {
    libm::cosf(x)
}
pub fn pow_f32(x: f32, y: f32) -> f32 {
    libm::powf(x, y)
}
pub fn fma_f32(a: f32, b: f32, c: f32) -> f32 {
    libm::fmaf(a, b, c)
}
