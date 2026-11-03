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
    /// A regular expression that cannot be compiled (D157): its source and
    /// the first line of the reason.
    RegExp { source: &'a str, reason: alloc::string::String },
    /// `s.replaceAll(re, to)` with a `RegExp` without the flag `g`.
    ReplaceAllNotGlobal,
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
            Panic::RegExp { source, reason } => write!(f, "RegExp: invalid regular expression `{}`: {}", source, reason),
            Panic::ReplaceAllNotGlobal => write!(f, "replaceAll: the RegExp must have the flag g"),
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
mod replacements {
    extern crate std;
    use std::string::String;

    /// The replacement by the bounds of the matches (D174) gives what the
    /// path with captures gives, empty matches included (ECMAScript's
    /// rule: an empty match right after a match is replaced too).
    #[test]
    fn the_fast_replacement_as_the_complete_one() {
        let texts = ["aaa", "abc", "", "a\nb\n\nc", "xaxbxx", "héllo wörld", ">h\nab\ncd\n"];
        let patterns = ["g/a*", "g/x*", "g/", "g/\\b", "gm/^", "gm/$", "g/a|", "gm/^>.*\n|\n", "g/[^x]*", "/a*", "g/ö|é", "gi/A"];
        let tos = ["-", "[$&]", "$$", "<$`|$'>", ""];
        for v in patterns {
            for t in texts {
                for to in tos {
                    let mut fast = String::new();
                    super::rx::SLOW_PATH.store(false, core::sync::atomic::Ordering::Relaxed);
                    super::rx::replace(&mut fast, v, t, to, false).ok().unwrap();
                    let mut slow = String::new();
                    super::rx::SLOW_PATH.store(true, core::sync::atomic::Ordering::Relaxed);
                    super::rx::replace(&mut slow, v, t, to, false).ok().unwrap();
                    super::rx::SLOW_PATH.store(false, core::sync::atomic::Ordering::Relaxed);
                    assert_eq!(fast, slow, "{:?}.replace({:?}, {:?})", t, v, to);
                }
            }
        }
        // `"aaa".match(/a*/g)` is `["aaa", ""]` in ECMAScript
        assert_eq!(super::rx::matches("g/a*", "aaa"), std::vec!["aaa", ""]);
        assert_eq!(super::rx::count("g/a*", "aaa"), 2);
        assert_eq!(super::rx::count("g/x*", "abc"), 4);
    }
}

#[cfg(test)]
mod split_searches {
    extern crate std;
    use std::string::String;
    use std::vec::Vec;

    /// The global search split into parts searched apart, then merged
    /// (D189), finds the matches of one search: empty matches, matches
    /// across the cuts, look-arounds at the cuts, characters of several
    /// bytes, cuts anywhere.
    #[test]
    fn merged_parts_find_the_matches_of_one_search() {
        let mut seed: u64 = 7;
        let mut rnd = |n: u64| {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 33) % n
        };
        let alphabet: Vec<char> = "aNtBY<>|\n>éx".chars().collect();
        let mut texts: Vec<String> = std::vec!["".into(), "aaa".into(), ">h\nab\ncd\n".into()];
        for len in [7u64, 40, 300, 2000] {
            for _ in 0..3 {
                texts.push((0..len).map(|_| alphabet[rnd(alphabet.len() as u64) as usize]).collect());
            }
        }
        let patterns = [
            "g/a[NSt]|BY", "g/<[^>]*>", "g/\\|[^|][^|]*\\|", "gm/^>.*\n|\n", "g/a*", "g/", "g/\\b", "gm/^", "gm/$", "g/x|é", "gi/t",
        ];
        for v in patterns {
            let re = super::rx::of(v);
            for t in &texts {
                let whole = super::rx::bounds_from(&re, t, 0, usize::MAX);
                for parts in 1..6 {
                    for _ in 0..8 {
                        let mut starts: Vec<usize> = (1..parts).map(|_| rnd(t.len() as u64 + 1) as usize).filter(|&c| t.is_char_boundary(c) && c > 0).collect();
                        starts.push(0);
                        starts.sort();
                        starts.dedup();
                        let found: Vec<Vec<(usize, usize)>> = (0..starts.len())
                            .map(|k| super::rx::bounds_from(&re, t, starts[k], starts.get(k + 1).copied().unwrap_or(usize::MAX)))
                            .collect();
                        let merged = super::rx::merge_bounds(&re, t, &starts, &found);
                        assert_eq!(merged, whole, "{:?} in {:?} cut at {:?}", v, t, starts);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod quotients_of_one_word {
    extern crate std;
    use num_bigint::BigInt;
    use std::vec::Vec;

    /// The division whose quotient is one word (D196) as num-bigint's:
    /// `b * q + r` for random `q` of 64 bits and `r < b`, divisors whose top
    /// word is small, quotients at 2^64 and around, signs mixed.
    #[test]
    fn small_quotients_as_num_bigint() {
        let mut seed: u64 = 3;
        let mut rnd = || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            seed
        };
        let word = |w: u64| BigInt::from(w);
        let mut cases: Vec<(BigInt, BigInt)> = Vec::new();
        for words in 2..9 {
            for top_bits in [1u32, 7, 33, 64] {
                for _ in 0..40 {
                    let mut b = BigInt::from(0);
                    for _ in 0..words - 1 {
                        b = (b << 64) + word(rnd());
                    }
                    b = (b << top_bits) + word(rnd() >> (64 - top_bits));
                    if b == BigInt::from(0) {
                        continue;
                    }
                    let q = match rnd() % 4 {
                        0 => word(rnd() >> (rnd() % 64)),
                        1 => word(u64::MAX),
                        2 => BigInt::from(1u128 << 64),
                        _ => word(rnd() % 10),
                    };
                    let r = (word(rnd()) * word(rnd())) % &b;
                    let a = &b * &q + r;
                    cases.push((a.clone(), b.clone()));
                    cases.push((-a.clone(), b.clone()));
                    cases.push((a - 1, -b));
                }
            }
        }
        for (a, b) in &cases {
            let got = super::big256::op(3, &super::big256::encode(a), &super::big256::encode(b)).ok().unwrap();
            let want = super::big256::encode(&(a / b));
            assert_eq!(got, want, "{} / {}", a, b);
        }
    }
}

#[cfg(test)]
mod big_divisions {
    extern crate std;
    use num_bigint::BigUint;

    fn big(w: &[u64]) -> BigUint {
        let mut v = std::vec::Vec::new();
        for &x in w {
            v.push(x as u32);
            v.push((x >> 32) as u32);
        }
        BigUint::new(v)
    }

    /// Knuth's algorithm D against num-bigint (D171): random words, words
    /// near their limits (the corrections of qhat, the step that adds back),
    /// divisors of one word and longer than the dividend.
    #[test]
    fn divisions_as_num_bigint() {
        let mut s = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            s
        };
        let pick = |r: u64| match r % 6 {
            0 => 0,
            1 => u64::MAX,
            2 => 1 << 63,
            3 => (1 << 63) - 1,
            4 => 1,
            _ => r.rotate_left(17),
        };
        for case in 0..20000 {
            let la = 1 + (next() % 9) as usize;
            let lb = 1 + (next() % 6) as usize;
            let a: std::vec::Vec<u64> = (0..la).map(|_| if case % 2 == 0 { next() } else { pick(next()) }).collect();
            let mut b: std::vec::Vec<u64> = (0..lb).map(|_| if case % 3 == 0 { next() } else { pick(next()) }).collect();
            if b.iter().all(|&x| x == 0) {
                b[0] = 1;
            }
            let (q, r) = super::limbs::divrem_for_test(&a, &b);
            let (ba, bb) = (big(&a), big(&b));
            assert_eq!(big(&q), &ba / &bb, "{:?} / {:?}", a, b);
            assert_eq!(big(&r), &ba % &bb, "{:?} % {:?}", a, b);
        }
    }
}

#[cfg(test)]
mod big_forms {
    extern crate std;
    use num_bigint::BigInt;

    /// The two stored forms of `bigint` (D168): the same values, canonical,
    /// zero empty, the conversion of the literals exact.
    #[test]
    fn the_two_forms_hold_the_same_values() {
        let mut s = 0x1234_5678_9abc_def0u64;
        let mut next = || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            s
        };
        let mut values: std::vec::Vec<BigInt> = std::vec![BigInt::from(0), BigInt::from(1), BigInt::from(-1), BigInt::from(255), BigInt::from(256), BigInt::from(u64::MAX), -BigInt::from(u64::MAX)];
        for len in (0..40).chain([60, 120]) {
            let mut v = BigInt::from(next() % 1000);
            for _ in 0..len {
                v = v * BigInt::from(next()) + BigInt::from(next() >> (next() % 64));
            }
            values.push(if next() % 2 == 0 { v.clone() } else { -v });
        }
        for v in &values {
            let e128 = super::big::encode(v);
            let e256 = super::big256::encode(v);
            assert_eq!(&super::big::decode(&e128), v);
            assert_eq!(&super::big256::decode(&e256), v);
            assert_eq!(super::big_to_256(&e128), e256);
            assert_eq!(e256.last() == Some(&0), false, "{} has a zero at the top", v);
            assert_eq!(e256.is_empty(), *v == BigInt::from(0));
        }
        for a in &values {
            for b in values.iter().take(12).chain(values.iter().rev().take(3)) {
                for op in [0u8, 1, 2, 3, 4, 6] {
                    let r128 = super::big::op(op, &super::big::encode(a), &super::big::encode(b));
                    let r256 = super::big256::op(op, &super::big256::encode(a), &super::big256::encode(b));
                    match (r128, r256) {
                        (Ok(x), Ok(y)) => assert_eq!(super::big_to_256(&x), y, "op {} of {} and {}", op, a, b),
                        (Err(_), Err(_)) => {}
                        _ => panic!("op {} of {} and {}: a panic on one side only", op, a, b),
                    }
                }
                assert_eq!(super::big::cmp(&super::big::encode(a), &super::big::encode(b)), super::big256::cmp(&super::big256::encode(a), &super::big256::encode(b)), "{} and {}", a, b);
            }
        }
    }
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

/// `s.replace(from, to)` (the first `from`) and `s.replaceAll(from, to)`
/// of a text (D157): through the engine of regular expressions, `from`
/// escaped, so that `to` has ECMAScript's patterns `$&`, `$$`, ...
pub fn replace_text(w: &mut dyn Write, s: &str, from: &str, to: &str, all: bool) -> fmt::Result {
    let mut src = alloc::string::String::new();
    for c in from.chars() {
        if "\\.+*?()|[]{}^$#&-~".contains(c) {
            src.push('\\');
        }
        src.push(c);
    }
    let v = if all { alloc::format!("g/{}", src) } else { alloc::format!("/{}", src) };
    let _ = rx::replace(w, &v, s, to, false);
    Ok(())
}

/// The replacement of every `from` without patterns (unused: kept for the
/// tests of the engine).
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

/// The functions of `bigint` (D156) for one stored form: a sign (`+` or
/// `-`) then the digits of the magnitude, least significant first, without
/// a zero at the top; zero is empty. `big` stores digits of 7 bits, valid
/// UTF-8, for the interpreter (whose `bigint` is a string) and the IR;
/// `big256` stores bytes, for the compiled programs, so that a value goes
/// to and from num-bigint by whole bytes (D168). Both are canonical: equal
/// values have equal bytes.
macro_rules! big_module {
    ($name:ident, $radix:expr) => {
        pub mod $name {
    use super::Panic;
    use alloc::vec::Vec;
    use core::cmp::Ordering;
    use core::fmt::{self, Write};
    use num_bigint::{BigInt, BigUint, Sign};
    use num_traits::{Signed, ToPrimitive, Zero};

    pub fn decode(e: &[u8]) -> BigInt {
        let neg = e.first() == Some(&b'-');
        let digits = if e.is_empty() { e } else { &e[1..] };
        let mag = if digits.is_empty() {
            BigUint::zero()
        } else if $radix == 256 {
            super::mag_from_bytes(digits)
        } else {
            BigUint::from_radix_le(digits, $radix).unwrap_or_default()
        };
        BigInt::from_biguint(if neg { Sign::Minus } else { Sign::Plus }, mag)
    }

    pub fn encode(b: &BigInt) -> Vec<u8> {
        let mut out = Vec::new();
        if !b.is_zero() {
            out.push(if b.is_negative() { b'-' } else { b'+' });
            let mut d = if $radix == 256 { super::mag_to_bytes(b.magnitude()) } else { b.magnitude().to_radix_le($radix) };
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
        if $radix == 256 {
            if let Some(r) = super::limbs::op(op, a, b) {
                return Ok(r);
            }
        }
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
        if $radix == 256 {
            return super::limbs::cmp(a, b);
        }
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
    };
}

/// The magnitude of bytes, least significant first, by words of 32 bits
/// (num-bigint's `from_bytes_le` takes them one by one, D168).
fn mag_from_bytes(d: &[u8]) -> num_bigint::BigUint {
    let mut v: alloc::vec::Vec<u32> = alloc::vec::Vec::with_capacity(d.len().div_ceil(4));
    let mut c = d.chunks_exact(4);
    for w in &mut c {
        v.push(u32::from_le_bytes([w[0], w[1], w[2], w[3]]));
    }
    let r = c.remainder();
    if !r.is_empty() {
        let mut w = [0u8; 4];
        w[..r.len()].copy_from_slice(r);
        v.push(u32::from_le_bytes(w));
    }
    num_bigint::BigUint::new(v)
}

/// The bytes of a magnitude, least significant first, without a zero at
/// the top: its words of 64 bits copied.
fn mag_to_bytes(m: &num_bigint::BigUint) -> alloc::vec::Vec<u8> {
    let mut out = alloc::vec::Vec::with_capacity(m.bits().div_ceil(8) as usize + 8);
    for w in m.iter_u64_digits() {
        out.extend_from_slice(&w.to_le_bytes());
    }
    while out.last() == Some(&0) {
        out.pop();
    }
    out
}

/// The common operations of `big256` on its bytes (D171): addition,
/// subtraction, product (schoolbook, for operands up to `MUL_LIMBS` words;
/// num-bigint's Karatsuba beyond), comparison. A magnitude is its bytes,
/// least significant first, without a zero at the top; it is read by words
/// of 64 bits.
mod limbs {
    use alloc::vec::Vec;
    use core::cmp::Ordering;

    const MUL_LIMBS: usize = 48;

    /// The words of a magnitude (its bytes read by 8, the last one padded).
    fn words(m: &[u8]) -> Vec<u64> {
        let mut v = Vec::with_capacity(m.len().div_ceil(8));
        let mut c = m.chunks_exact(8);
        for w in &mut c {
            v.push(u64::from_le_bytes([w[0], w[1], w[2], w[3], w[4], w[5], w[6], w[7]]));
        }
        let r = c.remainder();
        if !r.is_empty() {
            let mut w = [0u8; 8];
            w[..r.len()].copy_from_slice(r);
            v.push(u64::from_le_bytes(w));
        }
        v
    }

    /// The word i of a magnitude (zero beyond it).
    #[inline(always)]
    fn word(m: &[u8], i: usize) -> u64 {
        let s = i * 8;
        if s + 8 <= m.len() {
            u64::from_le_bytes([m[s], m[s + 1], m[s + 2], m[s + 3], m[s + 4], m[s + 5], m[s + 6], m[s + 7]])
        } else if s < m.len() {
            let mut w = [0u8; 8];
            w[..m.len() - s].copy_from_slice(&m[s..]);
            u64::from_le_bytes(w)
        } else {
            0
        }
    }

    /// A value under construction: its sign byte, then its words as bytes.
    struct Out(Vec<u8>);

    impl Out {
        fn new(neg: bool, words: usize) -> Out {
            let mut v = Vec::with_capacity(words * 8 + 1);
            v.push(if neg { b'-' } else { b'+' });
            Out(v)
        }
        #[inline(always)]
        fn push(&mut self, w: u64) {
            self.0.extend_from_slice(&w.to_le_bytes());
        }
        /// Without the zeros at the top; zero is empty.
        fn done(mut self) -> Vec<u8> {
            while self.0.last() == Some(&0) {
                self.0.pop();
            }
            if self.0.len() == 1 {
                self.0.clear();
            }
            self.0
        }
    }

    fn signed(neg: bool, w: &[u64]) -> Vec<u8> {
        let mut out = Out::new(neg, w.len());
        for &x in w {
            out.push(x);
        }
        out.done()
    }

    fn split(e: &[u8]) -> (bool, &[u8]) {
        if e.is_empty() {
            (false, e)
        } else {
            (e[0] == b'-', &e[1..])
        }
    }

    fn cmp_mag(a: &[u8], b: &[u8]) -> Ordering {
        a.len().cmp(&b.len()).then_with(|| a.iter().rev().cmp(b.iter().rev()))
    }

    fn add(neg: bool, a: &[u8], b: &[u8]) -> Vec<u8> {
        let (a, b) = if a.len() >= b.len() { (a, b) } else { (b, a) };
        let n = a.len().div_ceil(8);
        // every byte written below: not zeroed first (D196)
        let mut v: Vec<u8> = unwritten(1 + 8 * (n + 1));
        v[0] = if neg { b'-' } else { b'+' };
        let mut carry = false;
        // the whole words of both, without a test of bounds (D171)
        let both = b.len() / 8;
        {
            let dst = &mut v[1..1 + both * 8];
            for ((x, y), z) in a.chunks_exact(8).zip(b.chunks_exact(8)).zip(dst.chunks_exact_mut(8)) {
                let xa = u64::from_le_bytes([x[0], x[1], x[2], x[3], x[4], x[5], x[6], x[7]]);
                let yb = u64::from_le_bytes([y[0], y[1], y[2], y[3], y[4], y[5], y[6], y[7]]);
                let (s, c1) = xa.overflowing_add(yb);
                let (s, c2) = s.overflowing_add(carry as u64);
                z.copy_from_slice(&s.to_le_bytes());
                carry = c1 | c2;
            }
        }
        for i in both..n {
            let (s, c1) = word(a, i).overflowing_add(word(b, i));
            let (s, c2) = s.overflowing_add(carry as u64);
            v[1 + 8 * i..9 + 8 * i].copy_from_slice(&s.to_le_bytes());
            carry = c1 | c2;
        }
        v[1 + 8 * n..9 + 8 * n].copy_from_slice(&(carry as u64).to_le_bytes());
        Out(v).done()
    }

    /// A buffer of `n` bytes for a result that writes each of them (not
    /// zeroed: `memset` was 7 % of pidigits, D196).
    fn unwritten(n: usize) -> Vec<u8> {
        let mut v = Vec::with_capacity(n);
        // SAFETY: u8 has no invalid value; the callers write every byte
        // before reading any
        unsafe { v.set_len(n) };
        v
    }

    /// a - b, |a| >= |b|
    fn sub(neg: bool, a: &[u8], b: &[u8]) -> Vec<u8> {
        let n = a.len().div_ceil(8);
        // every byte written below: not zeroed first (D196)
        let mut v: Vec<u8> = unwritten(1 + 8 * n);
        v[0] = if neg { b'-' } else { b'+' };
        let mut borrow = false;
        let both = b.len() / 8;
        {
            let dst = &mut v[1..1 + both * 8];
            for ((x, y), z) in a.chunks_exact(8).zip(b.chunks_exact(8)).zip(dst.chunks_exact_mut(8)) {
                let xa = u64::from_le_bytes([x[0], x[1], x[2], x[3], x[4], x[5], x[6], x[7]]);
                let yb = u64::from_le_bytes([y[0], y[1], y[2], y[3], y[4], y[5], y[6], y[7]]);
                let (d, b1) = xa.overflowing_sub(yb);
                let (d, b2) = d.overflowing_sub(borrow as u64);
                z.copy_from_slice(&d.to_le_bytes());
                borrow = b1 | b2;
            }
        }
        for i in both..n {
            let (d, b1) = word(a, i).overflowing_sub(word(b, i));
            let (d, b2) = d.overflowing_sub(borrow as u64);
            v[1 + 8 * i..9 + 8 * i].copy_from_slice(&d.to_le_bytes());
            borrow = b1 | b2;
        }
        Out(v).done()
    }

    /// The product by one word: the whole words of `a` without a test of
    /// bounds, written in place in the result (D171).
    fn mul_word(neg: bool, a: &[u8], w: u64) -> Vec<u8> {
        let n = a.len().div_ceil(8);
        // every byte written below: not zeroed first (D196)
        let mut v: Vec<u8> = unwritten(1 + 8 * (n + 1));
        v[0] = if neg { b'-' } else { b'+' };
        let mut c: u128 = 0;
        let full = a.len() / 8;
        {
            let (src, dst) = (&a[..full * 8], &mut v[1..1 + full * 8]);
            for (x, y) in src.chunks_exact(8).zip(dst.chunks_exact_mut(8)) {
                let t = (u64::from_le_bytes([x[0], x[1], x[2], x[3], x[4], x[5], x[6], x[7]]) as u128) * (w as u128) + c;
                y.copy_from_slice(&(t as u64).to_le_bytes());
                c = t >> 64;
            }
        }
        for i in full..n {
            let t = (word(a, i) as u128) * (w as u128) + c;
            v[1 + 8 * i..9 + 8 * i].copy_from_slice(&(t as u64).to_le_bytes());
            c = t >> 64;
        }
        v[1 + 8 * n..9 + 8 * n].copy_from_slice(&(c as u64).to_le_bytes());
        Out(v).done()
    }

    fn mul(a: &[u64], b: &[u64]) -> Vec<u64> {
        let (a, b) = if a.len() >= b.len() { (a, b) } else { (b, a) };
        let mut r = alloc::vec![0u64; a.len() + b.len()];
        for (j, &bj) in b.iter().enumerate() {
            if bj == 0 {
                continue;
            }
            let mut c: u128 = 0;
            for (i, &ai) in a.iter().enumerate() {
                let t = (ai as u128) * (bj as u128) + r[i + j] as u128 + c;
                r[i + j] = t as u64;
                c = t >> 64;
            }
            r[j + a.len()] = c as u64;
        }
        r
    }

    /// The quotient and the remainder of magnitudes, b not zero: by one
    /// word in 128 bits, else Knuth's algorithm D (The Art of Computer
    /// Programming, 4.3.1), in words of 64 bits.
    fn divrem(a: &[u64], b: &[u64]) -> (Vec<u64>, Vec<u64>) {
        let mut b = b.to_vec();
        while b.last() == Some(&0) {
            b.pop();
        }
        let mut a = a.to_vec();
        while a.last() == Some(&0) {
            a.pop();
        }
        if a.len() < b.len() {
            return (Vec::new(), a);
        }
        if b.len() == 1 {
            let d = b[0] as u128;
            let mut q = alloc::vec![0u64; a.len()];
            let mut r: u128 = 0;
            for i in (0..a.len()).rev() {
                let t = (r << 64) | a[i] as u128;
                q[i] = (t / d) as u64;
                r = t % d;
            }
            return (q, alloc::vec![r as u64]);
        }
        let n = b.len();
        let m = a.len() - n;
        let s = b[n - 1].leading_zeros();
        // normalized: the top bit of v set
        let shl = |x: &[u64], extra: bool| -> Vec<u64> {
            let mut out = Vec::with_capacity(x.len() + 1);
            let mut carry = 0u64;
            for &w in x {
                out.push(if s == 0 { w } else { (w << s) | carry });
                carry = if s == 0 { 0 } else { w >> (64 - s) };
            }
            if extra {
                out.push(carry);
            }
            out
        };
        let v = shl(&b, false);
        let mut u = shl(&a, true);
        let mut q = alloc::vec![0u64; m + 1];
        let base: u128 = 1 << 64;
        for j in (0..=m).rev() {
            let num = ((u[j + n] as u128) << 64) | u[j + n - 1] as u128;
            let mut qhat = num / v[n - 1] as u128;
            let mut rhat = num % v[n - 1] as u128;
            while qhat >= base || qhat * v[n - 2] as u128 > ((rhat << 64) | u[j + n - 2] as u128) {
                qhat -= 1;
                rhat += v[n - 1] as u128;
                if rhat >= base {
                    break;
                }
            }
            // u[j..j+n+1] -= qhat * v
            let mut borrow: i128 = 0;
            let mut carry: u128 = 0;
            for i in 0..n {
                let p = qhat * v[i] as u128 + carry;
                carry = p >> 64;
                let t = u[i + j] as i128 - (p as u64) as i128 + borrow;
                u[i + j] = t as u64;
                borrow = t >> 64;
            }
            let t = u[j + n] as i128 - carry as i128 + borrow;
            u[j + n] = t as u64;
            if t < 0 {
                // one too many: add v back
                qhat -= 1;
                let mut c: u128 = 0;
                for i in 0..n {
                    let t = u[i + j] as u128 + v[i] as u128 + c;
                    u[i + j] = t as u64;
                    c = t >> 64;
                }
                u[j + n] = u[j + n].wrapping_add(c as u64);
            }
            q[j] = qhat as u64;
        }
        // the remainder: u >> s
        let mut r = Vec::with_capacity(n);
        for i in 0..n {
            r.push(if s == 0 { u[i] } else { (u[i] >> s) | (u[i + 1] << (64 - s)) });
        }
        (q, r)
    }

    /// The quotient of magnitudes `a / b` when it is less than 2^64 (`a <
    /// b * 2^64`: `a` of at most as many words as `b`, or of one more and
    /// `a / 2^64 < b`; `b` of at least two words), None otherwise (D196).
    /// Then `a << s` fits in the words of `b` and one more, the places read
    /// below. Knuth's estimate from the top two words of `a`
    /// shifted as `b` is normalized (its top bit set), refined by the second
    /// word of `b`: at most 2 above the quotient; then the sign of
    /// `a - q * b` in one pass over the words, nothing stored, `q` lowered
    /// while it is negative. No copy, no shift of the operands.
    fn quotient_word(a: &[u8], b: &[u8]) -> Option<u64> {
        let (na, nb) = (a.len().div_ceil(8), b.len().div_ceil(8));
        if nb < 2 || na > nb + 1 {
            return None;
        }
        if cmp_mag(a, b) == Ordering::Less {
            return Some(0);
        }
        // one word more: the quotient has one word only if a / 2^64 < b
        if na == nb + 1 && cmp_mag(&a[8..], b) != Ordering::Less {
            return None;
        }
        let s = word(b, nb - 1).leading_zeros();
        let sh = |hi: u64, lo: u64| if s == 0 { hi } else { (hi << s) | (lo >> (64 - s)) };
        let v1 = sh(word(b, nb - 1), word(b, nb - 2));
        let v2 = sh(word(b, nb - 2), if nb >= 3 { word(b, nb - 3) } else { 0 });
        // the words of `a << s` at the places nb, nb - 1, nb - 2
        let aw = |i: usize| if i < na { word(a, i) } else { 0 };
        let u2 = if s == 0 { aw(nb) } else { (aw(nb) << s) | (aw(nb - 1) >> (64 - s)) };
        let u1 = sh(aw(nb - 1), aw(nb - 2));
        let u0 = sh(aw(nb - 2), if nb >= 3 { aw(nb - 3) } else { 0 });
        let num = ((u2 as u128) << 64) | u1 as u128;
        let mut qhat = num / v1 as u128;
        let mut rhat = num % v1 as u128;
        while qhat >> 64 != 0 || qhat * v2 as u128 > ((rhat << 64) | u0 as u128) {
            qhat -= 1;
            rhat += v1 as u128;
            if rhat >> 64 != 0 {
                break;
            }
        }
        let mut q = qhat as u64;
        // a - q * b >= 0 ? the whole words of both without a test of
        // bounds, then the others
        let below = |q: u64| -> bool {
            let mut carry: u128 = 0;
            let mut borrow = false;
            let full = a.len().min(b.len()) / 8;
            for (x, y) in a[..full * 8].chunks_exact(8).zip(b[..full * 8].chunks_exact(8)) {
                let xa = u64::from_le_bytes([x[0], x[1], x[2], x[3], x[4], x[5], x[6], x[7]]);
                let yb = u64::from_le_bytes([y[0], y[1], y[2], y[3], y[4], y[5], y[6], y[7]]);
                let p = (q as u128) * yb as u128 + carry;
                carry = p >> 64;
                let (d, b1) = xa.overflowing_sub(p as u64);
                let (_, b2) = d.overflowing_sub(borrow as u64);
                borrow = b1 | b2;
            }
            for i in full..na.max(nb) {
                let p = (q as u128) * (if i < nb { word(b, i) } else { 0 }) as u128 + carry;
                carry = p >> 64;
                let (d, b1) = aw(i).overflowing_sub(p as u64);
                let (_, b2) = d.overflowing_sub(borrow as u64);
                borrow = b1 | b2;
            }
            carry != 0 || borrow
        };
        while below(q) {
            q -= 1;
        }
        Some(q)
    }

    /// `+ - * / %` and the negation (`op` 0 to 4, 6) of two values of
    /// `big256`; None for the others (num-bigint does them). `/` truncates
    /// toward zero and `%` has the sign of the dividend, as ECMAScript; a
    /// division by zero is left to num-bigint (its panic).
    pub fn op(op: u8, a: &[u8], b: &[u8]) -> Option<Vec<u8>> {
        let (na, ma) = split(a);
        let (nb, mb) = split(b);
        match op {
            0 | 1 => {
                let nb = if op == 1 && !mb.is_empty() { !nb } else { nb };
                Some(if na == nb {
                    add(na, ma, mb)
                } else {
                    match cmp_mag(ma, mb) {
                        Ordering::Greater => sub(na, ma, mb),
                        Ordering::Less => sub(nb, mb, ma),
                        Ordering::Equal => Vec::new(),
                    }
                })
            }
            2 => {
                if ma.is_empty() || mb.is_empty() {
                    return Some(Vec::new());
                }
                if mb.len() <= 8 {
                    return Some(mul_word(na != nb, ma, word(mb, 0)));
                }
                if ma.len() <= 8 {
                    return Some(mul_word(na != nb, mb, word(ma, 0)));
                }
                let (wa, wb) = (words(ma), words(mb));
                if wa.len().min(wb.len()) > MUL_LIMBS {
                    return None;
                }
                Some(signed(na != nb, &mul(&wa, &wb)))
            }
            3 | 4 => {
                if mb.is_empty() {
                    return None;
                }
                // a quotient of one word: estimated from the top words, one
                // pass to check it (D196)
                if op == 3 {
                    if let Some(q) = quotient_word(ma, mb) {
                        return Some(if q == 0 { Vec::new() } else { signed(na != nb, &[q]) });
                    }
                }
                let (q, r) = divrem(&words(ma), &words(mb));
                Some(if op == 3 { signed(na != nb, &q) } else { signed(na, &r) })
            }
            6 => Some(if ma.is_empty() { Vec::new() } else { signed(!na, &words(ma)) }),
            _ => None,
        }
    }

    /// The order of two values of `big256`.
    pub fn cmp(a: &[u8], b: &[u8]) -> Ordering {
        let (na, ma) = split(a);
        let (nb, mb) = split(b);
        let sign = |neg: bool, m: &[u8]| if m.is_empty() { 0 } else if neg { -1 } else { 1 };
        let (sa, sb) = (sign(na, ma), sign(nb, mb));
        if sa != sb {
            return sa.cmp(&sb);
        }
        let o = cmp_mag(ma, mb);
        if sa < 0 {
            o.reverse()
        } else {
            o
        }
    }

    #[cfg(test)]
    pub fn divrem_for_test(a: &[u64], b: &[u64]) -> (Vec<u64>, Vec<u64>) {
        divrem(a, b)
    }
}

big_module!(big, 128);
big_module!(big256, 256);

/// A value of `big` in the form of `big256` (the literals of the compiled
/// code, D168).
pub fn big_to_256(e: &[u8]) -> alloc::vec::Vec<u8> {
    big256::encode(&big::decode(e))
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

// ---------------------------------------------------------------------
// regular expressions (D157): a `RegExp` is the text `flags/source`; the
// engine of the crate `regex` (leftmost first, as ECMAScript; linear time:
// no backreference, no look-around)

pub mod rx {
    use super::Panic;
    use alloc::string::ToString;
    use alloc::vec::Vec;
    use core::fmt::{self, Write};
    pub use regex_automata::meta::Regex;
    use regex_automata::util::syntax;

    /// The value of `RegExp(source, flags)`, its flags checked.
    pub fn value<'a>(source: &'a str, flags: &'a str, w: &mut dyn Write) -> Result<(), Panic<'a>> {
        for c in flags.chars() {
            if !"gimsu".contains(c) {
                return Err(Panic::RegExp { source, reason: alloc::format!("the flag `{}` is not supported (g, i, m, s, u)", c) });
            }
        }
        compile(source, flags)?;
        let _ = write!(w, "{}/{}", flags, source);
        Ok(())
    }

    pub fn parts(v: &str) -> (&str, &str) {
        v.split_once('/').unwrap_or(("", v))
    }

    pub fn compile<'a>(source: &'a str, flags: &str) -> Result<Regex, Panic<'a>> {
        let cfg = syntax::Config::new()
            .case_insensitive(flags.contains('i'))
            .multi_line(flags.contains('m'))
            .dot_matches_new_line(flags.contains('s'));
        Regex::builder().syntax(cfg).build(source).map_err(|e| {
            // the reason: the last line of the error of syntax (`error:
            // look-around ... is not supported`)
            let text = match e.syntax_error() {
                Some(se) => se.to_string(),
                None => e.to_string(),
            };
            let last = text.lines().last().unwrap_or("").trim();
            let reason = last.strip_prefix("error: ").unwrap_or(last).to_string();
            Panic::RegExp { source, reason }
        })
    }

    pub fn of(v: &str) -> Regex {
        let (flags, source) = parts(v);
        // the value was checked when it was made
        compile(source, flags).unwrap_or_else(|_| Regex::new("[^\\s\\S]").unwrap())
    }

    /// `re.test(s)`.
    pub fn test(v: &str, s: &str) -> bool {
        of(v).is_match(s)
    }

    /// `s.search(re)`: the byte index of the first match, -1 if none.
    pub fn search(v: &str, s: &str) -> i64 {
        of(v).find(s).map(|m| m.start() as i64).unwrap_or(-1)
    }

    /// The matches of `re` in `s`, left to right, as ECMAScript's global
    /// search finds them: after an empty match, the next search starts one
    /// character further; an empty match right after a match counts (where
    /// the iterator of the crate skips it). By the fast engines, without
    /// captures (D174).
    fn each_match(re: &Regex, s: &str, f: impl FnMut(usize, usize) -> bool) {
        each_match_from(re, s, 0, f)
    }

    /// `each_match` from the byte `at` (a boundary of characters), with a
    /// cache of its own (not the pool of the regex, D189).
    pub fn each_match_from(re: &Regex, s: &str, mut at: usize, mut f: impl FnMut(usize, usize) -> bool) {
        let mut cache = re.create_cache();
        while at <= s.len() {
            let Some(m) = re.search_with(&mut cache, &regex_automata::Input::new(s).range(at..)) else { break };
            if !f(m.start(), m.end()) {
                break;
            }
            at = next_at(s, m.start(), m.end());
        }
    }

    /// Where the global search goes on after the match `s[start..end]`: its
    /// end, one character further after an empty match.
    pub fn next_at(s: &str, start: usize, end: usize) -> usize {
        if end > start {
            end
        } else {
            s[end..].chars().next().map(|c| end + c.len_utf8()).unwrap_or(s.len() + 1)
        }
    }

    /// The matches of the global search from `from` (a boundary of
    /// characters) up to the first one that starts at `until` or after, that
    /// one included (D189).
    pub fn bounds_from(re: &Regex, s: &str, from: usize, until: usize) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        each_match_from(re, s, from, |a, b| {
            out.push((a, b));
            a < until
        });
        out
    }

    /// The matches of the global search over all of `s`, from the searches
    /// started at `starts[k]` (`starts[0] = 0`, increasing boundaries of
    /// characters), each one up to its first match at `starts[k + 1]` or
    /// after (`bounds_from`). The search after a match depends only on where
    /// it goes on (`next_at`): while that place is one where the search of
    /// a part went on (its start, or after one of its matches), the next
    /// match is the next one of that part; otherwise one true search, until
    /// such a place is met again. The same matches as one search (D189).
    pub fn merge_bounds(re: &Regex, s: &str, starts: &[usize], parts: &[Vec<(usize, usize)>]) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        let mut cache = re.create_cache();
        // the part whose search went on from `at` (the part `c` with
        // `starts[c] <= at`: its start, or after one of its matches), and
        // the index of its next match
        let follow = |at: usize| -> Option<(usize, usize)> {
            let c = match starts.binary_search(&at) {
                Ok(c) => return Some((c, 0)),
                Err(0) => return None,
                Err(c) => c - 1,
            };
            parts[c].iter().position(|&(a, b)| next_at(s, a, b) == at).map(|j| (c, j + 1))
        };
        let mut at = 0;
        let mut cur = follow(0);
        while at <= s.len() {
            match cur {
                Some((k, j)) if j < parts[k].len() => {
                    let (a, b) = parts[k][j];
                    out.push((a, b));
                    at = next_at(s, a, b);
                    // the last match of a part starts in a following one:
                    // its search stopped there
                    cur = if k + 1 < starts.len() && a >= starts[k + 1] { follow(at) } else { Some((k, j + 1)) };
                }
                // a part stops only at a match in a following part: this
                // one met no more match up to the end of `s`
                Some(_) => break,
                None => {
                    let Some(m) = re.search_with(&mut cache, &regex_automata::Input::new(s).range(at..)) else { break };
                    out.push((m.start(), m.end()));
                    at = next_at(s, m.start(), m.end());
                    cur = follow(at);
                }
            }
        }
        out
    }

    /// `s` with each match of `bounds` replaced by `to` (without groups).
    pub fn replace_bounds(s: &str, to: &str, bounds: &[(usize, usize)]) -> alloc::string::String {
        let mut out = alloc::string::String::with_capacity(s.len());
        let mut last = 0;
        for &(a, b) in bounds {
            out.push_str(&s[last..a]);
            substitute_bounds(&mut out, to, s, a, b);
            last = b;
        }
        out.push_str(&s[last..]);
        out
    }

    /// `to` has no group (`$1` to `$99`): the bounds of the matches are
    /// enough (D174).
    pub fn without_groups(to: &str) -> bool {
        let b = to.as_bytes();
        !(0..b.len().saturating_sub(1)).any(|i| b[i] == b'$' && b[i + 1].is_ascii_digit()) && !SLOW_PATH.load(core::sync::atomic::Ordering::Relaxed)
    }

    /// `countMatches(s, re)`: the matches, left to right, without overlap.
    pub fn count(v: &str, s: &str) -> i64 {
        let mut n = 0;
        each_match(&of(v), s, |_, _| {
            n += 1;
            true
        });
        n
    }

    /// `matches(s, re)`: their texts.
    pub fn matches<'s>(v: &str, s: &'s str) -> Vec<&'s str> {
        let mut out = Vec::new();
        each_match(&of(v), s, |a, b| {
            out.push(&s[a..b]);
            true
        });
        out
    }

    /// `s.replace(re, to)` (every match with the flag `g`, else the first)
    /// and `s.replaceAll(re, to)`: `to` with ECMAScript's patterns `$$`,
    /// `$&`, `` $` ``, `$'`, `$1` to `$99`.
    pub fn replace<'a>(w: &mut dyn Write, v: &str, s: &str, to: &str, all: bool) -> Result<(), Panic<'a>> {
        let (flags, _) = parts(v);
        if all && !flags.contains('g') {
            return Err(Panic::ReplaceAllNotGlobal);
        }
        let re = of(v);
        let global = flags.contains('g');
        // without `$1` to `$99`, the bounds of the matches are enough: found
        // by the fast engines, without captures (D174)
        let b = to.as_bytes();
        let groups = (0..b.len().saturating_sub(1)).any(|i| b[i] == b'$' && b[i + 1].is_ascii_digit());
        if !groups && !SLOW_PATH.load(core::sync::atomic::Ordering::Relaxed) {
            let mut out = alloc::string::String::with_capacity(s.len());
            let mut last = 0;
            each_match(&re, s, |a, b| {
                out.push_str(&s[last..a]);
                substitute_bounds(&mut out, to, s, a, b);
                last = b;
                global
            });
            out.push_str(&s[last..]);
            let _ = w.write_str(&out);
            return Ok(());
        }
        let mut last = 0;
        let mut caps = re.create_captures();
        let mut at = 0;
        while at <= s.len() {
            re.search_captures(&regex_automata::Input::new(s).range(at..), &mut caps);
            let Some(m) = caps.get_match() else { break };
            let _ = w.write_str(&s[last..m.start()]);
            let _ = substitute(w, to, s, &caps);
            last = m.end();
            if !global {
                break;
            }
            // an empty match: the next search starts after a character
            at = if m.end() > m.start() { m.end() } else { s[m.end()..].chars().next().map(|c| m.end() + c.len_utf8()).unwrap_or(s.len() + 1) };
        }
        let _ = w.write_str(&s[last..]);
        Ok(())
    }

    /// For the tests: the path with captures, to compare with (D174).
    pub static SLOW_PATH: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

    /// `to` for the match `s[start..end]`, without groups: `$$`, `$&`,
    /// `` $` ``, `$'`, any other `$` as it is.
    fn substitute_bounds(out: &mut alloc::string::String, to: &str, s: &str, start: usize, end: usize) {
        if !to.contains('$') {
            out.push_str(to);
            return;
        }
        let b = to.as_bytes();
        let mut i = 0;
        let mut plain = 0;
        while i < b.len() {
            if b[i] == b'$' && i + 1 < b.len() {
                let piece = match b[i + 1] {
                    b'$' => Some("$"),
                    b'&' => Some(&s[start..end]),
                    b'`' => Some(&s[..start]),
                    b'\'' => Some(&s[end..]),
                    _ => None,
                };
                if let Some(p) = piece {
                    out.push_str(&to[plain..i]);
                    out.push_str(p);
                    i += 2;
                    plain = i;
                    continue;
                }
            }
            i += 1;
        }
        out.push_str(&to[plain..]);
    }

    fn substitute(w: &mut dyn Write, to: &str, s: &str, caps: &regex_automata::util::captures::Captures) -> fmt::Result {
        let m = caps.get_match().unwrap();
        let b = to.as_bytes();
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'$' && i + 1 < b.len() {
                match b[i + 1] {
                    b'$' => {
                        w.write_char('$')?;
                        i += 2;
                        continue;
                    }
                    b'&' => {
                        w.write_str(&s[m.range()])?;
                        i += 2;
                        continue;
                    }
                    b'`' => {
                        w.write_str(&s[..m.start()])?;
                        i += 2;
                        continue;
                    }
                    b'\'' => {
                        w.write_str(&s[m.end()..])?;
                        i += 2;
                        continue;
                    }
                    d if d.is_ascii_digit() => {
                        // two digits when that group exists, else one
                        let groups = caps.group_len();
                        let one = (d - b'0') as usize;
                        let two = if i + 2 < b.len() && b[i + 2].is_ascii_digit() { Some(one * 10 + (b[i + 2] - b'0') as usize) } else { None };
                        let (n, len) = match two {
                            Some(t) if t >= 1 && t < groups => (t, 3),
                            _ => (one, 2),
                        };
                        if n >= 1 && n < groups {
                            if let Some(g) = caps.get_group(n) {
                                w.write_str(&s[g.range()])?;
                            }
                            i += len;
                            continue;
                        }
                    }
                    _ => {}
                }
            }
            let c = to[i..].chars().next().unwrap();
            w.write_char(c)?;
            i += c.len_utf8();
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------
// `Map<K, V>` (D158): its keys in the order of their first insertion, and
// the permutation of these keys sorted, to find one by bisection. The keys
// are compared by a total order in which equal keys are those of
// ECMAScript's SameValueZero (NaN is NaN, -0 is +0).

pub mod map {
    use alloc::vec::Vec;
    use core::cmp::Ordering;

    /// The distinct keys of `n` keys (compared by `cmp(i, j)`): for each, in
    /// the order of its first occurrence, that occurrence, its last one,
    /// its number of occurrences; and the permutation of the distinct keys
    /// sorted.
    pub struct Groups {
        pub first: Vec<i64>,
        pub last: Vec<i64>,
        pub count: Vec<i64>,
        pub order: Vec<i64>,
    }

    pub fn group(n: usize, cmp: &dyn Fn(usize, usize) -> Ordering) -> Groups {
        let mut idx: Vec<usize> = (0..n).collect();
        idx.sort_by(|&i, &j| cmp(i, j).then(i.cmp(&j)));
        // the runs of equal keys, in the order of the keys
        let mut runs: Vec<(usize, usize, usize)> = Vec::new();
        let mut k = 0;
        while k < n {
            let mut e = k + 1;
            while e < n && cmp(idx[k], idx[e]) == Ordering::Equal {
                e += 1;
            }
            // the run is sorted by index: its first and last occurrences
            runs.push((idx[k], idx[e - 1], e - k));
            k = e;
        }
        // positions by first occurrence
        let mut by_first: Vec<usize> = (0..runs.len()).collect();
        by_first.sort_by_key(|&r| runs[r].0);
        let mut pos = alloc::vec![0i64; runs.len()];
        for (p, &r) in by_first.iter().enumerate() {
            pos[r] = p as i64;
        }
        Groups {
            first: by_first.iter().map(|&r| runs[r].0 as i64).collect(),
            last: by_first.iter().map(|&r| runs[r].1 as i64).collect(),
            count: by_first.iter().map(|&r| runs[r].2 as i64).collect(),
            // the runs are in the order of the keys
            order: (0..runs.len()).map(|r| pos[r]).collect(),
        }
    }

    /// The position of a key among the keys of a map, -1 if absent:
    /// `probe(i)` compares the key at position `i` with the one looked for.
    pub fn find(order: &[i64], probe: &dyn Fn(usize) -> Ordering) -> i64 {
        let (mut lo, mut hi) = (0usize, order.len());
        while lo < hi {
            let mid = (lo + hi) / 2;
            match probe(order[mid] as usize) {
                Ordering::Less => lo = mid + 1,
                Ordering::Greater => hi = mid,
                Ordering::Equal => return order[mid],
            }
        }
        -1
    }

    /// The order of the keys of floats: SameValueZero for equality.
    pub fn order_f64(a: f64, b: f64) -> Ordering {
        let z = |x: f64| if x == 0.0 { 0.0 } else { x };
        super::order_f64(z(a), z(b))
    }
}

// ---------------------------------------------------------------------
// `$group(keys)` (D167): the distinct keys of an array in the order of
// their first occurrence, by a table of hashes (linear probing). The result
// does not depend on the hash: only on the equality of the keys.

/// For the m distinct keys of the `n` keys, in the order of their first
/// occurrence, three columns of m numbers: the index of the first
/// occurrence of each, of its last one, and its count.
pub fn group<H: Fn(usize) -> u64, E: Fn(usize, usize) -> bool>(n: usize, hash: H, eq: E) -> alloc::vec::Vec<i64> {
    use alloc::vec::Vec;
    let mut first: Vec<i64> = Vec::new();
    let mut last: Vec<i64> = Vec::new();
    let mut count: Vec<i64> = Vec::new();
    let mut hashes: Vec<u64> = Vec::new();
    let mut cap = 16usize;
    let mut table: Vec<usize> = alloc::vec![usize::MAX; cap];
    for i in 0..n {
        let h = hash(i);
        let mut s = (h as usize) & (cap - 1);
        loop {
            let g = table[s];
            if g == usize::MAX {
                table[s] = first.len();
                first.push(i as i64);
                last.push(i as i64);
                count.push(1);
                hashes.push(h);
                if first.len() * 2 > cap {
                    cap *= 2;
                    table = alloc::vec![usize::MAX; cap];
                    for (k, &hk) in hashes.iter().enumerate() {
                        let mut t = (hk as usize) & (cap - 1);
                        while table[t] != usize::MAX {
                            t = (t + 1) & (cap - 1);
                        }
                        table[t] = k;
                    }
                }
                break;
            }
            if hashes[g] == h && eq(first[g] as usize, i) {
                last[g] = i as i64;
                count[g] += 1;
                break;
            }
            s = (s + 1) & (cap - 1);
        }
    }
    first.extend_from_slice(&last);
    first.extend_from_slice(&count);
    first
}

/// `group` of integer keys within a small range (D194): the keys minus the
/// smallest index a table of the groups, no hash and no probing; the same
/// three columns. `key(i)` is the key `i` as an `i64` in the order of the
/// keys (a `u64` with its top bit flipped); `None` when the range of the
/// keys is too large for a table (more than `DENSE_MAX` and more than `n`
/// slots).
pub fn group_dense(n: usize, key: impl Fn(usize) -> i64) -> Option<alloc::vec::Vec<i64>> {
    use alloc::vec::Vec;
    const DENSE_MAX: u64 = 1 << 16;
    if n == 0 {
        return Some(Vec::new());
    }
    let (mut lo, mut hi) = (key(0), key(0));
    for i in 1..n {
        let k = key(i);
        lo = lo.min(k);
        hi = hi.max(k);
    }
    let span = (hi.wrapping_sub(lo) as u64).saturating_add(1);
    if span > DENSE_MAX && span > n as u64 {
        return None;
    }
    let mut slot: Vec<u32> = alloc::vec![u32::MAX; span as usize];
    let mut first: Vec<i64> = Vec::new();
    let mut last: Vec<i64> = Vec::new();
    let mut count: Vec<i64> = Vec::new();
    for i in 0..n {
        let s = key(i).wrapping_sub(lo) as u64 as usize;
        let g = slot[s];
        if g == u32::MAX {
            slot[s] = first.len() as u32;
            first.push(i as i64);
            last.push(i as i64);
            count.push(1);
        } else {
            last[g as usize] = i as i64;
            count[g as usize] += 1;
        }
    }
    first.extend_from_slice(&last);
    first.extend_from_slice(&count);
    Some(first)
}

/// The hash of an integer key (its bits).
#[inline(always)]
pub fn hash_word(x: u64) -> u64 {
    let x = x.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x ^ (x >> 29)
}

/// The hash of the bytes of a string or a `bigint` key.
#[inline(always)]
pub fn hash_bytes(b: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64 ^ b.len() as u64;
    let mut chunks = b.chunks_exact(8);
    for c in &mut chunks {
        h = (h ^ u64::from_le_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]])).wrapping_mul(0x0000_0100_0000_01B3).rotate_left(23);
    }
    for &x in chunks.remainder() {
        h = (h ^ x as u64).wrapping_mul(0x0000_0100_0000_01B3);
    }
    hash_word(h)
}

#[cfg(test)]
mod dense_groups {
    extern crate std;
    use std::vec::Vec;

    /// The groups of integer keys by a table (D194) are those of the table
    /// of hashes: small ranges, negative keys, a range too large (`None`),
    /// one key, none.
    #[test]
    fn dense_groups_as_hashed_ones() {
        let mut seed: u64 = 11;
        let mut rnd = |n: u64| {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 33) % n
        };
        let mut cases: Vec<Vec<i64>> = std::vec![std::vec![], std::vec![7], std::vec![-3, -3, 5, -3]];
        for (n, range, base) in [(100u64, 4u64, 0i64), (5000, 300, -150), (2000, 70000, 10), (300, 1 << 40, -(1 << 39))] {
            cases.push((0..n).map(|_| base + rnd(range) as i64).collect());
        }
        for keys in &cases {
            let hashed = super::group(keys.len(), |i| super::hash_word(keys[i] as u64), |i, j| keys[i] == keys[j]);
            match super::group_dense(keys.len(), |i| keys[i]) {
                Some(d) => assert_eq!(d, hashed, "{:?}", &keys[..keys.len().min(10)]),
                None => assert!(keys.iter().max().unwrap() - keys.iter().min().unwrap() >= 1 << 16),
            }
        }
    }
}
