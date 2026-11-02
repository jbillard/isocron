//! Semantics shared by the runtime of compiled programs (`isc-runtime`,
//! without the standard library) and by the interpreter (`isc-interp`): the
//! text of every panic message, the printing of numbers and the canonical NaN.
//! Both sides write through `core::fmt::Write`, so that they produce the same
//! bytes. No standard library: `core`, and `alloc` for `bigint` (D156).

#![no_std]

extern crate alloc;

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
    /// `s.slice(a, b)` that cuts a character of UTF-8 (D155).
    NotCharBoundary(i64),
    /// `s.slice(a, b)` with `a < 0`, `a > b` or `b` past the end.
    StrSlice { a: i64, b: i64, len: i64 },
    /// `s.repeat(n)` with `n < 0` (D155).
    RepeatNegative(i64),
    /// `x.toFixed(d)` with `d` outside 0 ..= 100 (D155).
    FixedDigits(i64),
    /// `readStdinText()` of bytes that are not UTF-8 (D155).
    StdinNotUtf8(i64),
    /// `BigInt(s)` of a text that is not an integer (D156).
    ParseBigInt(&'a str),
    /// `a ** b` of a `bigint` with `b` past 2^32 - 1.
    BigExponent,
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
            Panic::NotCharBoundary(i) => write!(f, "slice: the index {} is inside a character of UTF-8", i),
            Panic::StrSlice { a, b, len } => write!(f, "slice out of bounds: [{}..{}] of a string of {} bytes", a, b, len),
            Panic::RepeatNegative(n) => write!(f, "repeat: the count is negative: {}", n),
            Panic::FixedDigits(d) => write!(f, "toFixed: the number of digits must be between 0 and 100, got {}", d),
            Panic::ParseBigInt(t) => write!(f, "BigInt: `{}` is not an integer", t),
            Panic::BigExponent => write!(f, "the exponent of a bigint is too large (above 4294967295)"),
            Panic::StdinNotUtf8(at) => write!(f, "readStdinText: the standard input is not UTF-8 text (invalid from byte {})", at),
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
pub fn tan_f64(x: f64) -> f64 {
    libm::tan(x)
}
pub fn asin_f64(x: f64) -> f64 {
    libm::asin(x)
}
pub fn acos_f64(x: f64) -> f64 {
    libm::acos(x)
}
pub fn atan_f64(x: f64) -> f64 {
    libm::atan(x)
}
pub fn atan2_f64(y: f64, x: f64) -> f64 {
    libm::atan2(y, x)
}
pub fn floor_f64(x: f64) -> f64 {
    libm::floor(x)
}
pub fn ceil_f64(x: f64) -> f64 {
    libm::ceil(x)
}
pub fn trunc_f64(x: f64) -> f64 {
    libm::trunc(x)
}
pub fn abs_f64(x: f64) -> f64 {
    libm::fabs(x)
}
/// `Math.round` (D155): to the nearest integer, a half toward +inf, as
/// JavaScript (`Math.round(-2.5)` is -2, `Math.round(-0.25)` is -0).
pub fn round_f64(x: f64) -> f64 {
    if !x.is_finite() {
        return x;
    }
    let r = libm::floor(x);
    // exact: `x` and `floor(x)` are within 1 of each other
    let y = if x - r >= 0.5 { r + 1.0 } else { r };
    if y == 0.0 && x.is_sign_negative() {
        -0.0
    } else {
        y
    }
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
pub fn tan_f32(x: f32) -> f32 {
    libm::tanf(x)
}
pub fn asin_f32(x: f32) -> f32 {
    libm::asinf(x)
}
pub fn acos_f32(x: f32) -> f32 {
    libm::acosf(x)
}
pub fn atan_f32(x: f32) -> f32 {
    libm::atanf(x)
}
pub fn atan2_f32(y: f32, x: f32) -> f32 {
    libm::atan2f(y, x)
}
pub fn floor_f32(x: f32) -> f32 {
    libm::floorf(x)
}
pub fn ceil_f32(x: f32) -> f32 {
    libm::ceilf(x)
}
pub fn trunc_f32(x: f32) -> f32 {
    libm::truncf(x)
}
pub fn abs_f32(x: f32) -> f32 {
    libm::fabsf(x)
}
pub fn round_f32(x: f32) -> f32 {
    if !x.is_finite() {
        return x;
    }
    let r = libm::floorf(x);
    let y = if x - r >= 0.5 { r + 1.0 } else { r };
    if y == 0.0 && x.is_sign_negative() {
        -0.0
    } else {
        y
    }
}

// ---------------------------------------------------------------------
// the functions of strings of TypeScript (D155), on bytes of UTF-8: the
// same code for the interpreter and the runtime, writing their text
// through `core::fmt::Write`

/// The order of `sort` (D155) on floats: by value, -0 before +0, every NaN
/// after the numbers (equal to each other).
pub fn order_f64(a: f64, b: f64) -> core::cmp::Ordering {
    use core::cmp::Ordering::*;
    match (a.is_nan(), b.is_nan()) {
        (true, true) => Equal,
        (true, false) => Greater,
        (false, true) => Less,
        _ if a == b => a.is_sign_positive().cmp(&b.is_sign_positive()),
        _ if a < b => Less,
        _ => Greater,
    }
}

/// `s.indexOf(t)`: the byte index of the first `t` in `s`, -1 if none.
pub fn index_of(s: &str, t: &str) -> i64 {
    s.find(t).map(|i| i as i64).unwrap_or(-1)
}

/// `s.replaceAll(from, to)`: every `from`, left to right, replaced; an
/// empty `from` puts `to` before each character and at the end (as
/// JavaScript).
pub fn replace_all(w: &mut dyn Write, s: &str, from: &str, to: &str) -> fmt::Result {
    if from.is_empty() {
        for c in s.chars() {
            w.write_str(to)?;
            w.write_char(c)?;
        }
        return w.write_str(to);
    }
    let mut last = 0;
    for (i, _) in s.match_indices(from) {
        w.write_str(&s[last..i])?;
        w.write_str(to)?;
        last = i + from.len();
    }
    w.write_str(&s[last..])
}

/// `s.toUpperCase()`, `s.toLowerCase()`: the mappings of Unicode, without a
/// locale.
pub fn to_case(w: &mut dyn Write, s: &str, upper: bool) -> fmt::Result {
    for c in s.chars() {
        if upper {
            for u in c.to_uppercase() {
                w.write_char(u)?;
            }
        } else {
            for u in c.to_lowercase() {
                w.write_char(u)?;
            }
        }
    }
    Ok(())
}

/// `s.padStart(n, fill)`, `s.padEnd(n, fill)`: `s` padded to `n` bytes with
/// copies of `fill`, the last one cut before a character that would pass
/// `n`; `s` itself when it has `n` bytes or more or when `fill` is empty.
pub fn pad(w: &mut dyn Write, s: &str, n: i64, fill: &str, start: bool) -> fmt::Result {
    let missing = n - s.len() as i64;
    if missing <= 0 || fill.is_empty() {
        return w.write_str(s);
    }
    if !start {
        w.write_str(s)?;
    }
    let mut left = missing as usize;
    'fill: loop {
        for c in fill.chars() {
            if c.len_utf8() > left {
                break 'fill;
            }
            w.write_char(c)?;
            left -= c.len_utf8();
            if left == 0 {
                break 'fill;
            }
        }
    }
    if start {
        w.write_str(s)?;
    }
    Ok(())
}

/// `s.repeat(n)`.
pub fn repeat<'a>(w: &mut dyn Write, s: &str, n: i64) -> Result<(), Panic<'a>> {
    if n < 0 {
        return Err(Panic::RepeatNegative(n));
    }
    for _ in 0..n {
        let _ = w.write_str(s);
    }
    Ok(())
}

/// `s.slice(a, b)`: the bytes `a .. b`, which must not cut a character.
pub fn slice_str<'a>(s: &'a str, a: i64, b: i64) -> Result<&'a str, Panic<'a>> {
    let len = s.len() as i64;
    if a < 0 || a > b || b > len {
        return Err(Panic::StrSlice { a, b, len });
    }
    for i in [a, b] {
        if !s.is_char_boundary(i as usize) {
            return Err(Panic::NotCharBoundary(i));
        }
    }
    Ok(&s[a as usize..b as usize])
}

/// `x.toFixed(d)` (D155), as JavaScript: `d` decimals, the exact value of
/// `x` rounded to the nearest, a half away from zero (JavaScript takes the
/// larger of two equally near numbers, after the sign); `NaN`,
/// `Infinity`, `-Infinity`; from 1e21 on, the text of `print`.
pub fn write_fixed<'a>(w: &mut dyn Write, x: f64, d: i64) -> Result<(), Panic<'a>> {
    if !(0..=100).contains(&d) {
        return Err(Panic::FixedDigits(d));
    }
    let _ = fixed(w, x, d as usize);
    Ok(())
}

fn fixed(w: &mut dyn Write, x: f64, d: usize) -> fmt::Result {
    if x.is_nan() {
        return w.write_str("NaN");
    }
    if x.is_infinite() {
        return w.write_str(if x > 0.0 { "Infinity" } else { "-Infinity" });
    }
    if x.abs() >= 1e21 {
        return write_f64(w, x);
    }
    // the exact decimal value of |x| (a double has at most 1074 decimals):
    // its integer digits, then its first d + 1 decimals
    struct Digits {
        int: [u8; 24],
        nint: usize,
        frac: [u8; 102],
        nfrac: usize,
        point: bool,
        want: usize,
    }
    impl Write for Digits {
        fn write_str(&mut self, s: &str) -> fmt::Result {
            for b in s.bytes() {
                if b == b'.' {
                    self.point = true;
                } else if !self.point {
                    self.int[self.nint] = b - b'0';
                    self.nint += 1;
                } else if self.nfrac < self.want {
                    self.frac[self.nfrac] = b - b'0';
                    self.nfrac += 1;
                }
            }
            Ok(())
        }
    }
    let mut g = Digits { int: [0; 24], nint: 0, frac: [0; 102], nfrac: 0, point: false, want: d + 1 };
    write!(g, "{:.1100}", x.abs())?;
    let up = g.frac[d] >= 5;
    // the kept digits, integer then decimals, rounded up from the right
    let mut all = [0u8; 126];
    let n = g.nint + d;
    all[..g.nint].copy_from_slice(&g.int[..g.nint]);
    all[g.nint..n].copy_from_slice(&g.frac[..d]);
    let mut carry = up;
    let mut i = n;
    while carry && i > 0 {
        i -= 1;
        if all[i] == 9 {
            all[i] = 0;
        } else {
            all[i] += 1;
            carry = false;
        }
    }
    if x < 0.0 {
        w.write_char('-')?;
    }
    if carry {
        w.write_char('1')?;
    }
    for (k, digit) in all[..n].iter().enumerate() {
        if k == g.nint {
            w.write_char('.')?;
        }
        w.write_char((b'0' + digit) as char)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------
// `bigint` (D156): a value is the byte `+` or `-`, then the digits of its
// magnitude in base 128, least significant first, without a last zero; 0
// is no byte (the zero of every value of the IR): canonical (equal values,
// equal bytes), and ASCII (the interpreter holds it as a string)

pub mod big {
    use super::Panic;
    use alloc::vec::Vec;
    use core::cmp::Ordering;
    use core::fmt::{self, Write};
    use num_bigint::{BigInt, BigUint, Sign};
    use num_traits::{Signed, ToPrimitive, Zero};

    pub fn decode(e: &[u8]) -> BigInt {
        let neg = e.first() == Some(&b'-');
        let digits = if e.is_empty() { e } else { &e[1..] };
        let mag = if digits.is_empty() { BigUint::zero() } else { BigUint::from_radix_le(digits, 128).unwrap_or_default() };
        BigInt::from_biguint(if neg { Sign::Minus } else { Sign::Plus }, mag)
    }

    pub fn encode(b: &BigInt) -> Vec<u8> {
        let mut out = Vec::new();
        if !b.is_zero() {
            out.push(if b.is_negative() { b'-' } else { b'+' });
            let mut d = b.magnitude().to_radix_le(128);
            while d.last() == Some(&0) {
                d.pop();
            }
            out.extend_from_slice(&d);
        }
        out
    }

    pub fn from_i64(x: i64) -> Vec<u8> {
        encode(&BigInt::from(x))
    }

    pub fn from_u64(x: u64) -> Vec<u8> {
        encode(&BigInt::from(x))
    }

    /// A literal: its decimal digits (from the compiler).
    pub fn literal(decimal: &str, neg: bool) -> Vec<u8> {
        literal_radix(decimal, 10, neg)
    }

    pub fn literal_radix(digits: &str, radix: u32, neg: bool) -> Vec<u8> {
        let m = BigUint::parse_bytes(digits.as_bytes(), radix).unwrap_or_default();
        encode(&BigInt::from_biguint(if neg { Sign::Minus } else { Sign::Plus }, m))
    }

    /// `+ - * / % **` and the negation (`op` 0 to 6): `/` truncates toward
    /// zero and `%` has the sign of the dividend, as ECMAScript.
    pub fn op(op: u8, a: &[u8], b: &[u8]) -> Result<Vec<u8>, Panic<'static>> {
        let (x, y) = (decode(a), decode(b));
        let r = match op {
            0 => x + y,
            1 => x - y,
            2 => x * y,
            3 | 4 => {
                if y.is_zero() {
                    return Err(Panic::DivByZero);
                }
                if op == 3 {
                    x / y
                } else {
                    x % y
                }
            }
            5 => {
                if y.is_negative() {
                    return Err(Panic::NegativeExponent(y.to_i64().unwrap_or(i64::MIN)));
                }
                let e = y.to_u32().ok_or(Panic::BigExponent)?;
                num_traits::Pow::pow(x, e)
            }
            _ => -x,
        };
        Ok(encode(&r))
    }

    pub fn cmp(a: &[u8], b: &[u8]) -> Ordering {
        decode(a).cmp(&decode(b))
    }

    /// The value modulo 2^64, as `BigInt.asIntN(64, x)`.
    pub fn to_i64(a: &[u8]) -> i64 {
        let x = decode(a);
        let low = x.magnitude().iter_u64_digits().next().unwrap_or(0);
        if x.is_negative() {
            (low as i64).wrapping_neg()
        } else {
            low as i64
        }
    }

    /// The nearest `f64` (a half to even), as `Number(x)`.
    pub fn to_f64(a: &[u8]) -> f64 {
        let x = decode(a);
        let m = x.magnitude();
        let bits = m.bits();
        let v = if bits <= 64 {
            m.to_u64().unwrap_or(0) as f64
        } else {
            // the 64 first bits, and a last bit set when any of the others
            // is: rounded once, to nearest even
            let shift = bits - 64;
            let top: BigUint = m >> shift;
            let mut t = top.to_u64().unwrap_or(0);
            if m.trailing_zeros().unwrap_or(0) < shift {
                t |= 1;
            }
            let f = t as f64;
            if shift > 1100 {
                f64::INFINITY
            } else {
                libm::scalbn(f, shift as i32)
            }
        };
        if x.is_negative() {
            -v
        } else {
            v
        }
    }

    pub fn write_decimal(w: &mut dyn Write, a: &[u8]) -> fmt::Result {
        let x = decode(a);
        w.write_str(&x.to_str_radix(10))
    }

    /// `BigInt(s)`: an optional sign, then decimal digits (white space
    /// around allowed, as ECMAScript).
    pub fn parse(s: &str) -> Option<Vec<u8>> {
        let t = s.trim();
        let (neg, digits) = match t.as_bytes().first() {
            Some(b'-') => (true, &t[1..]),
            Some(b'+') => (false, &t[1..]),
            _ => (false, t),
        };
        if digits.is_empty() || !digits.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        Some(literal(digits, neg))
    }
}

// ---------------------------------------------------------------------
// `Number(s)`, `parseFloat(s)`, `parseInt(s, radix)` of ECMAScript
// (D156): a `f64`, NaN when the text is not a number, never a panic

/// The white space of ECMAScript (`StrWhiteSpaceChar`): its spaces (the
/// category Zs, the tabs, U+FEFF) and its line terminators.
fn js_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{B}' | '\u{C}' | '\r' | ' ' | '\u{A0}' | '\u{1680}' | '\u{2000}'..='\u{200A}' | '\u{2028}' | '\u{2029}' | '\u{202F}' | '\u{205F}' | '\u{3000}' | '\u{FEFF}'
    )
}

/// The length of the longest prefix of `s` that is a `StrDecimalLiteral`
/// without its sign (`Infinity`, digits, a point, an exponent).
fn decimal_prefix(s: &str) -> usize {
    if s.starts_with("Infinity") {
        return 8;
    }
    let b = s.as_bytes();
    let mut i = 0;
    let digits = |i: &mut usize| {
        let s = *i;
        while *i < b.len() && b[*i].is_ascii_digit() {
            *i += 1;
        }
        *i - s
    };
    let int = digits(&mut i);
    let mut frac = 0;
    if i < b.len() && b[i] == b'.' {
        let mut j = i + 1;
        frac = digits(&mut j);
        if int > 0 || frac > 0 {
            i = j;
        }
    }
    if int == 0 && frac == 0 {
        return 0;
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        let mut j = i + 1;
        if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
            j += 1;
        }
        if digits(&mut j) > 0 {
            i = j;
        }
    }
    i
}

/// The value of a decimal literal (validated by `decimal_prefix`).
fn decimal_value(t: &str) -> f64 {
    if t == "Infinity" {
        return f64::INFINITY;
    }
    t.parse::<f64>().unwrap_or(f64::NAN)
}

/// `Number(s)`: the whole text (white space around it ignored) is a number:
/// decimal with a sign, `Infinity`, or `0x`, `0o`, `0b` without a sign;
/// empty: 0; anything else: NaN.
pub fn js_number(s: &str) -> f64 {
    let t = s.trim_matches(js_space);
    if t.is_empty() {
        return 0.0;
    }
    let b = t.as_bytes();
    if b.len() > 2 && b[0] == b'0' {
        let radix = match b[1] {
            b'x' | b'X' => 16,
            b'o' | b'O' => 8,
            b'b' | b'B' => 2,
            _ => 0,
        };
        if radix > 0 {
            let digits = &t[2..];
            return if digits.chars().all(|c| c.is_digit(radix)) { radix_value(digits, radix) } else { f64::NAN };
        }
    }
    let (neg, rest) = match b[0] {
        b'-' => (true, &t[1..]),
        b'+' => (false, &t[1..]),
        _ => (false, t),
    };
    let n = decimal_prefix(rest);
    if n == 0 || n != rest.len() {
        return f64::NAN;
    }
    let v = decimal_value(rest);
    if neg {
        -v
    } else {
        v
    }
}

/// The integer of these digits in this radix, rounded once to the nearest
/// `f64`.
fn radix_value(digits: &str, radix: u32) -> f64 {
    let d: alloc::vec::Vec<u8> = digits.chars().map(|c| c.to_digit(radix).unwrap_or(0) as u8).collect();
    match num_bigint::BigUint::from_radix_be(&d, radix) {
        Some(m) => big::to_f64(&big::encode(&num_bigint::BigInt::from(m))),
        None => f64::NAN,
    }
}

/// `parseFloat(s)`: the longest decimal prefix after the white space and a
/// sign; none: NaN.
pub fn js_parse_float(s: &str) -> f64 {
    let t = s.trim_start_matches(js_space);
    let (neg, rest) = match t.as_bytes().first() {
        Some(b'-') => (true, &t[1..]),
        Some(b'+') => (false, &t[1..]),
        _ => (false, t),
    };
    let n = decimal_prefix(rest);
    if n == 0 {
        return f64::NAN;
    }
    let v = decimal_value(&rest[..n]);
    if neg {
        -v
    } else {
        v
    }
}

/// `parseInt(s, radix)`: the longest prefix of digits of the radix (2 to
/// 36; 0: none given, 10, or 16 after `0x`) after the white space and a
/// sign; none, or a radix outside: NaN.
pub fn js_parse_int(s: &str, radix: i64) -> f64 {
    let t = s.trim_start_matches(js_space);
    let (neg, mut rest) = match t.as_bytes().first() {
        Some(b'-') => (true, &t[1..]),
        Some(b'+') => (false, &t[1..]),
        _ => (false, t),
    };
    let mut r = radix;
    if r != 0 && !(2..=36).contains(&r) {
        return f64::NAN;
    }
    if (r == 0 || r == 16) && (rest.starts_with("0x") || rest.starts_with("0X")) {
        rest = &rest[2..];
        r = 16;
    }
    if r == 0 {
        r = 10;
    }
    let n = rest.chars().take_while(|c| c.is_digit(r as u32)).count();
    if n == 0 {
        return f64::NAN;
    }
    let v = radix_value(&rest[..n], r as u32);
    if neg {
        -v
    } else {
        v
    }
}
