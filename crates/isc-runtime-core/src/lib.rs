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

/// The tasks of thread `k` of `threads` in a parallel loop of `n` tasks of
/// the same cost (D254): a contiguous range, the ranges of the threads in
/// their order, their lengths within one of each other. Contiguous rather
/// than every `threads`-th task: a thread reads and writes one stream (the
/// prefetchers of the cache follow it), and the threads do not fault the
/// same huge page of a new array at the same time.
pub fn static_share(n: i64, threads: i64, k: i64) -> core::ops::Range<i64> {
    let at = |j: i64| (n as i128 * j as i128 / threads.max(1) as i128) as i64;
    at(k)..at(k + 1)
}

#[cfg(test)]
mod shares {
    /// Each task once, in order, by contiguous ranges of lengths within one
    /// of each other; more threads than tasks: empty ranges.
    #[test]
    fn contiguous_ranges_of_the_tasks() {
        for n in [0i64, 1, 2, 3, 7, 64, 1000, 62_053, 1 << 40] {
            for t in [1i64, 2, 3, 4, 7, 8, 64] {
                let mut next = 0;
                let (mut lo, mut hi) = (i64::MAX, 0);
                for k in 0..t {
                    let r = super::static_share(n, t, k);
                    assert_eq!(r.start, next, "n={} t={} k={}", n, t, k);
                    assert!(r.end >= r.start);
                    lo = lo.min(r.end - r.start);
                    hi = hi.max(r.end - r.start);
                    next = r.end;
                }
                assert_eq!(next, n, "n={} t={}", n, t);
                assert!(hi - lo <= 1, "n={} t={}: {}..{}", n, t, lo, hi);
            }
        }
    }
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

    /// D240: the quotient certain from the top words, and not when it is
    /// not: dividends just below `(q + 1) * b` (the estimate from the top
    /// words is then `q + 1`), just above `q * b`, at `q * b` and `(q + 1) *
    /// b`, with divisors whose low words are all ones or zero.
    #[test]
    fn quotients_at_the_edge_of_the_estimate() {
        let mut seed: u64 = 11;
        let mut rnd = || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            seed
        };
        let one = BigInt::from(1);
        let mut n = 0;
        for words in 2..7 {
            for low in 0..3 {
                for _ in 0..60 {
                    let mut b = BigInt::from(rnd() | 1);
                    for k in 1..words {
                        let w = match low {
                            0 => u64::MAX,
                            1 => 0,
                            _ => rnd(),
                        };
                        b = if k + 1 == words { (BigInt::from(rnd() >> (rnd() % 63)) << (64 * k)) + b } else { (BigInt::from(w) << (64 * k)) + b };
                    }
                    if b <= one {
                        continue;
                    }
                    let q = BigInt::from(rnd() >> (rnd() % 64));
                    let small = BigInt::from(rnd() % 1000 + 1);
                    let cases = [&b * &q, &b * &q + &one, &b * (&q + &one) - &small, &b * (&q + &one) - &one, &b * (&q + &one)];
                    for a in cases.iter() {
                        if a.sign() == num_bigint::Sign::Minus {
                            continue;
                        }
                        let got = super::big256::op(3, &super::big256::encode(a), &super::big256::encode(&b)).ok().unwrap();
                        assert_eq!(got, super::big256::encode(&(a / &b)), "{} / {}", a, b);
                        n += 1;
                    }
                }
            }
        }
        assert!(n > 3000);
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
// the errors of the system under Windows (D246, D247): the POSIX code of a
// Win32 error and the text of glibc for it, so that a program and the
// interpreter on Windows give the messages of Linux

/// TLS by the library of the system (D343): OpenSSL loaded at run time.
#[cfg(not(target_family = "wasm"))]
pub mod tls;

pub mod oserr {
    /// The POSIX code of a Win32 error (0 if none; EIO for the others).
    pub fn from_win32(code: u32) -> i32 {
        match code {
            0 => 0,
            1 => 21,                           // EISDIR (the read of a directory)
            2 | 3 | 15 | 123 | 161 | 206 => 2, // ENOENT (a bad name too)
            4 => 24,                           // EMFILE
            5 | 32 | 33 => 13,                 // EACCES (sharing, lock)
            6 => 9,                            // EBADF
            8 | 14 => 12,                      // ENOMEM
            19 => 30,                          // EROFS
            80 | 183 => 17,                    // EEXIST
            87 => 22,                          // EINVAL
            112 | 39 => 28,                    // ENOSPC
            267 => 20,                         // ENOTDIR
            _ => 5,                            // EIO
        }
    }

    /// The text of glibc for a POSIX code, NUL-terminated.
    pub fn text_c(code: i32) -> &'static [u8] {
        match code {
            0 => b"Success\0",
            2 => b"No such file or directory\0",
            5 => b"Input/output error\0",
            9 => b"Bad file descriptor\0",
            12 => b"Cannot allocate memory\0",
            13 => b"Permission denied\0",
            17 => b"File exists\0",
            20 => b"Not a directory\0",
            21 => b"Is a directory\0",
            22 => b"Invalid argument\0",
            24 => b"Too many open files\0",
            28 => b"No space left on device\0",
            29 => b"Illegal seek\0",
            30 => b"Read-only file system\0",
            _ => b"Unknown error\0",
        }
    }

    /// The text of glibc for a POSIX code.
    pub fn text(code: i32) -> &'static str {
        let t = text_c(code);
        core::str::from_utf8(&t[..t.len() - 1]).unwrap_or("Unknown error")
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
    find_bytes(s.as_bytes(), t.as_bytes()).map(|i| i as i64).unwrap_or(-1)
}

/// The index of the first `t` in `s` (D301): a byte by `memchr`; up to 16
/// bytes, their first byte by `memchr`, then the others compared; longer,
/// `memmem`. In UTF-8 the bytes of a text match only at the start of its
/// characters: the index of `str::find`, whose general search prepares
/// itself at each call (57 ns for a byte in 60, 3.8 in C).
pub fn find_bytes(s: &[u8], t: &[u8]) -> Option<usize> {
    match t.len() {
        0 => Some(0),
        1 => memchr::memchr(t[0], s),
        m if m <= 16 => {
            let mut at = 0;
            while at + m <= s.len() {
                let i = at + memchr::memchr(t[0], &s[at..=s.len() - m])?;
                if s[i + 1..i + m] == t[1..] {
                    return Some(i);
                }
                at = i + 1;
            }
            None
        }
        _ => memchr::memmem::find(s, t),
    }
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

    /// `(a * b + c) / d` (D389): from the leading words when they decide
    /// it, else the product, the sum and the quotient made whole.
    pub fn muladd_div(a: &[u8], b: &[u8], c: &[u8], d: &[u8]) -> Result<Vec<u8>, Panic<'static>> {
        if $radix == 256 {
            if let Some(q) = super::limbs::muladd_quot(a, b, c, d) {
                return Ok(q);
            }
        }
        let p = op(2, a, b)?;
        let s = op(0, &p, c)?;
        op(3, &s, d)
    }

    /// `z + x * y`, or `z - x * y` (`sub`), D406: in one pass when one of
    /// `x` and `y` is of one word (`limbs::muladd`), else the product then
    /// the sum; the same value either way.
    pub fn muladd(z: &[u8], x: &[u8], y: &[u8], sub: bool) -> Result<Vec<u8>, Panic<'static>> {
        if $radix == 256 {
            if let Some(r) = super::limbs::muladd(z, x, y, sub) {
                return Ok(r);
            }
        }
        let p = op(2, x, y)?;
        op(if sub { 1 } else { 0 }, z, &p)
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

    /// `dst[i] = x[i] + y[i]` (or `-`, `SUB`) on `4 * quads` words with one
    /// chain of `adc` (`sbb`) through the flags, from the carry `c`: the
    /// carry out (D241). In assembly: with AVX-512 LLVM turned the loop of
    /// Rust into vector code that carries through masks, slower than the
    /// chain (pidigits 0.506 s -> 0.746 s).
    #[cfg(target_arch = "x86_64")]
    #[inline(always)]
    unsafe fn chain<const SUB: bool>(x: *const u8, y: *const u8, dst: *mut u8, quads: usize, c: bool) -> bool {
        let out: u8;
        let (mut x, mut y, mut d, mut n) = (x, y, dst, quads);
        macro_rules! body {
            ($op:literal) => {
                core::arch::asm!(
                    "bt {c:e}, 0",
                    "2:",
                    "mov {t0}, [{x}]",
                    "mov {t1}, [{x} + 8]",
                    concat!($op, " {t0}, [{y}]"),
                    concat!($op, " {t1}, [{y} + 8]"),
                    "mov [{d}], {t0}",
                    "mov [{d} + 8], {t1}",
                    "mov {t0}, [{x} + 16]",
                    "mov {t1}, [{x} + 24]",
                    concat!($op, " {t0}, [{y} + 16]"),
                    concat!($op, " {t1}, [{y} + 24]"),
                    "mov [{d} + 16], {t0}",
                    "mov [{d} + 24], {t1}",
                    "lea {x}, [{x} + 32]",
                    "lea {y}, [{y} + 32]",
                    "lea {d}, [{d} + 32]",
                    "dec {n}",
                    "jnz 2b",
                    "setc {o}",
                    x = inout(reg) x,
                    y = inout(reg) y,
                    d = inout(reg) d,
                    n = inout(reg) n,
                    c = in(reg) c as u32,
                    t0 = out(reg) _,
                    t1 = out(reg) _,
                    o = lateout(reg_byte) out,
                    options(nostack),
                )
            };
        }
        if SUB {
            body!("sbb");
        } else {
            body!("adc");
        }
        let _ = (x, y, d, n);
        out != 0
    }

    fn add(neg: bool, a: &[u8], b: &[u8]) -> Vec<u8> {
        let (a, b) = if a.len() >= b.len() { (a, b) } else { (b, a) };
        let n = a.len().div_ceil(8);
        // every byte written below: not zeroed first (D196)
        let mut v: Vec<u8> = unwritten(1 + 8 * (n + 1));
        v[0] = if neg { b'-' } else { b'+' };
        let mut carry = false;
        // the whole words of both, without a test of bounds (D171); by 4 in
        // assembly on x86-64 (D241)
        let both = b.len() / 8;
        #[cfg(target_arch = "x86_64")]
        let done = {
            let quads = both / 4;
            if quads > 0 {
                // SAFETY: `a` and `b` have `both * 8` bytes, `v` as many from 1
                carry = unsafe { chain::<false>(a.as_ptr(), b.as_ptr(), v[1..].as_mut_ptr(), quads, false) };
            }
            quads * 4
        };
        #[cfg(not(target_arch = "x86_64"))]
        let done = 0;
        {
            let dst = &mut v[1 + done * 8..1 + both * 8];
            for ((x, y), z) in a[done * 8..].chunks_exact(8).zip(b[done * 8..both * 8].chunks_exact(8)).zip(dst.chunks_exact_mut(8)) {
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
        #[cfg(target_arch = "x86_64")]
        let done = {
            let quads = both / 4;
            if quads > 0 {
                // SAFETY: `a` and `b` have `both * 8` bytes, `v` as many from 1
                borrow = unsafe { chain::<true>(a.as_ptr(), b.as_ptr(), v[1..].as_mut_ptr(), quads, false) };
            }
            quads * 4
        };
        #[cfg(not(target_arch = "x86_64"))]
        let done = 0;
        {
            let dst = &mut v[1 + done * 8..1 + both * 8];
            for ((x, y), z) in a[done * 8..].chunks_exact(8).zip(b[done * 8..both * 8].chunks_exact(8)).zip(dst.chunks_exact_mut(8)) {
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
        // the words by 4 in assembly where `mulx` is (D239): one chain of
        // `adc` through the flags, the high word of each product added to the
        // low word of the next (about a cycle a word, where the loop below
        // chains an addition and an `adc` per word)
        #[cfg(all(target_arch = "x86_64", target_feature = "bmi2"))]
        let done = {
            let quads = full / 4;
            if quads > 0 {
                let mut src = a.as_ptr();
                let mut dst = v[1..].as_mut_ptr();
                let mut n = quads;
                let carry: u64;
                // SAFETY: `a` has `full * 8` bytes from its start and `v` as
                // many from its byte 1: `quads * 32` bytes read and written
                unsafe {
                    core::arch::asm!(
                        "xor {c:e}, {c:e}",
                        "2:",
                        "mulx {h0}, {l0}, [{s}]",
                        "mulx {h1}, {l1}, [{s} + 8]",
                        "adc {l0}, {c}",
                        "mov [{d}], {l0}",
                        "adc {l1}, {h0}",
                        "mov [{d} + 8], {l1}",
                        "mulx {h0}, {l0}, [{s} + 16]",
                        "mulx {c}, {l1}, [{s} + 24]",
                        "adc {l0}, {h1}",
                        "mov [{d} + 16], {l0}",
                        "adc {l1}, {h0}",
                        "mov [{d} + 24], {l1}",
                        "lea {s}, [{s} + 32]",
                        "lea {d}, [{d} + 32]",
                        "dec {n}",
                        "jnz 2b",
                        "adc {c}, 0",
                        s = inout(reg) src,
                        d = inout(reg) dst,
                        n = inout(reg) n,
                        c = out(reg) carry,
                        l0 = out(reg) _,
                        l1 = out(reg) _,
                        h0 = out(reg) _,
                        h1 = out(reg) _,
                        in("rdx") w,
                        options(nostack),
                    );
                }
                let _ = (src, dst, n);
                c = carry as u128;
            }
            quads * 4
        };
        #[cfg(not(all(target_arch = "x86_64", target_feature = "bmi2")))]
        let done = 0;
        {
            let (src, dst) = (&a[done * 8..full * 8], &mut v[1 + done * 8..1 + full * 8]);
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
        // certain from the top words alone (D240): the lower words of `b`
        // add less than one unit of its third word, so `qhat * b <= a` when
        // `a`'s three top words exceed `qhat * (v1, v2 + 1)`, that is
        // `rhat * 2^64 + u0 >= qhat * (v2 + 1)`; `qhat` is never below the
        // quotient, so it is the quotient, without a pass over the words
        if qhat >> 64 == 0 && rhat >> 64 == 0 && ((rhat << 64) | u0 as u128) >= qhat * (v2 as u128 + 1) {
            return Some(q);
        }
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

    /// The bits of a magnitude (zero for zero).
    fn bits(m: &[u8]) -> u64 {
        match m.last() {
            Some(&t) => 8 * (m.len() as u64 - 1) + 8 - t.leading_zeros() as u64,
            None => 0,
        }
    }

    /// `m / 2^e`, its bits at most `e + 120` (16 bytes read from the byte
    /// of `e`).
    fn window(m: &[u8], e: u64) -> u128 {
        let s = (e / 8) as usize;
        let mut b = [0u8; 16];
        if s < m.len() {
            let n = (m.len() - s).min(16);
            b[..n].copy_from_slice(&m[s..s + n]);
        }
        u128::from_le_bytes(b) >> (e % 8)
    }

    /// The quotient `(a * k + b) / c` from the leading words alone (D389),
    /// all of them non-negative, `k` (or `a`) of one word, `c` of more than
    /// 128 bits, `a` and `b` of at most 56 bits more than `c`. Each cut at
    /// the bit `e = bits(c) - 64`: `a = A 2^e + a'`,
    /// the same for `b` and `c` (C from 2^63 to 2^64), so that
    /// `(A k + B) 2^e <= a k + b < (A k + B + k + 1) 2^e` and
    /// `C 2^e <= c < (C + 1) 2^e`: the quotient lies from `(A k + B) / (C +
    /// 1)` to `(A k + B + k) / C`. When both are the same it is the
    /// quotient; None otherwise, and outside these conditions (the caller
    /// computes it whole).
    pub fn muladd_quot(x: &[u8], y: &[u8], z: &[u8], w: &[u8]) -> Option<Vec<u8>> {
        let ((nx, mx), (ny, my), (nz, mz), (nw, mw)) = (split(x), split(y), split(z), split(w));
        if nx || ny || nz || nw || mw.is_empty() {
            return None;
        }
        let (a, k) = if my.len() <= 8 {
            (mx, word(my, 0))
        } else if mx.len() <= 8 {
            (my, word(mx, 0))
        } else {
            return None;
        };
        let cb = bits(mw);
        if cb <= 128 {
            return None;
        }
        let e = cb - 64;
        // `a` and `b` may be longer than `c` (a quotient above one): their
        // windows of 120 bits at most, the products checked
        if bits(a) > e + 120 || bits(mz) > e + 120 {
            return None;
        }
        let c = window(mw, e);
        let lo = window(a, e).checked_mul(k as u128)?.checked_add(window(mz, e))?;
        let q = lo / (c + 1);
        if q != lo.checked_add(k as u128)? / c {
            return None;
        }
        let mut out = Out::new(false, 2);
        out.push(q as u64);
        out.push((q >> 64) as u64);
        Some(out.done())
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

    /// Whether the processor has ADX (`adcx`, `adox`: two chains of carries,
    /// one through CF, one through OF), read once by `cpuid` (leaf 7, bit
    /// 19 of EBX): not part of the levels x86-64-v3 and v4 of the runtimes.
    #[cfg(all(target_arch = "x86_64", target_feature = "bmi2"))]
    fn adx() -> bool {
        use core::sync::atomic::{AtomicU8, Ordering as O};
        static ADX: AtomicU8 = AtomicU8::new(0);
        match ADX.load(O::Relaxed) {
            1 => true,
            2 => false,
            _ => {
                #[allow(unused_unsafe)]
                // SAFETY: `cpuid` exists on every processor of x86-64
                let r = unsafe { core::arch::x86_64::__cpuid_count(7, 0) };
                let yes = r.ebx & (1 << 19) != 0;
                ADX.store(if yes { 1 } else { 2 }, O::Relaxed);
                yes
            }
        }
    }

    /// `z + x * y`, or `z - x * y` (`sub`), of values of `big256`, when
    /// one of `x` and `y` is of one word and the processor has ADX (D406):
    /// one pass over the words, where the product then the sum took two
    /// (pidigits: `(acc + num * 2) * k2`, `(acc - den * d) * 10`). None
    /// otherwise (the two passes). The words by 4 in assembly: the product
    /// of each word by `mulx`, its high word added to the next low one by
    /// the chain of `adcx` (CF), the word of `z` added by the chain of
    /// `adox` (OF; for `-`, the complement of the product plus 1); the rest
    /// in 128 bits. A difference below zero is negated (its sign turned).
    pub fn muladd(z: &[u8], x: &[u8], y: &[u8], sub: bool) -> Option<Vec<u8>> {
        #[cfg(not(all(target_arch = "x86_64", target_feature = "bmi2")))]
        {
            let _ = (z, x, y, sub);
            None
        }
        #[cfg(all(target_arch = "x86_64", target_feature = "bmi2"))]
        {
            let (nz, mz) = split(z);
            let (nx, mx) = split(x);
            let (ny, my) = split(y);
            let (mx, w) = if my.len() <= 8 {
                (mx, word(my, 0))
            } else if mx.len() <= 8 {
                (my, word(mx, 0))
            } else {
                return None;
            };
            if mx.is_empty() || w == 0 || mz.is_empty() || !adx() {
                return None;
            }
            // the sign of the product added: `z + p` or `z - p`
            let pneg = (nx != ny) != sub;
            // the magnitudes taken one from the other when the signs differ
            let minus = pneg != nz;
            let (zw, xw) = (mz.len().div_ceil(8), mx.len().div_ceil(8));
            let len = zw.max(xw + 1);
            // which magnitude is the larger, from the lengths and the top
            // words (`p` is `x * w`: from `x[xw - 1] * w` to that plus `w`,
            // in words from `xw - 1` on), so that the larger is taken from
            // and no pass negates; undecided (rare): `z - p`, negated after
            let p_larger = minus && {
                if zw > xw + 1 {
                    false
                } else if zw + 1 < xw + 1 && zw < xw {
                    true
                } else {
                    let q0 = (word(mx, xw - 1) as u128) * (w as u128);
                    let zq = (word(mz, xw - 1) as u128) | ((word(mz, xw) as u128) << 64);
                    zq < q0
                }
            };
            // every byte written below: not zeroed first (D196)
            let mut v: Vec<u8> = unwritten(1 + 8 * (len + 1));
            let quads = (mz.len() / 8).min(mx.len() / 8) / 4;
            let (mut c1, mut c2) = (0u64, minus as u64);
            if quads > 0 {
                let (mut xs, mut zs, mut d, mut n) = (mx.as_ptr(), mz.as_ptr(), v[1..].as_mut_ptr(), quads);
                let (h, o): (u64, u8);
                // SAFETY (each block): `quads * 32` bytes read from `mx` and
                // `mz` (both hold as many whole words) and written to `v`
                // from its byte 1 (it holds `len + 1` words); only `lea`,
                // `mov`, `mulx` and `not` between the `adcx` and the `adox`
                // of a chain, which leave CF and OF; the loop counted by
                // `jrcxz`, which reads no flag
                if !minus {
                    unsafe {
                        core::arch::asm!(
                            "xor {h1:e}, {h1:e}",
                            "mov {zr}, 0",
                            "2:",
                            "mulx {h0}, {l}, [{x}]",
                            "adcx {l}, {h1}",
                            "adox {l}, [{z}]",
                            "mov [{d}], {l}",
                            "mulx {h1}, {l}, [{x} + 8]",
                            "adcx {l}, {h0}",
                            "adox {l}, [{z} + 8]",
                            "mov [{d} + 8], {l}",
                            "mulx {h0}, {l}, [{x} + 16]",
                            "adcx {l}, {h1}",
                            "adox {l}, [{z} + 16]",
                            "mov [{d} + 16], {l}",
                            "mulx {h1}, {l}, [{x} + 24]",
                            "adcx {l}, {h0}",
                            "adox {l}, [{z} + 24]",
                            "mov [{d} + 24], {l}",
                            "lea {x}, [{x} + 32]",
                            "lea {z}, [{z} + 32]",
                            "lea {d}, [{d} + 32]",
                            "lea rcx, [rcx - 1]",
                            "jrcxz 3f",
                            "jmp 2b",
                            "3:",
                            "adcx {h1}, {zr}",
                            "seto {o}",
                            x = inout(reg) xs, z = inout(reg) zs, d = inout(reg) d, inout("rcx") n, in("rdx") w,
                            h1 = out(reg) h, h0 = out(reg) _, l = out(reg) _, zr = out(reg) _, o = lateout(reg_byte) o,
                            options(nostack),
                        )
                    }
                } else if !p_larger {
                    // z + not(p) + 1: OF set first (the 1)
                    unsafe {
                        core::arch::asm!(
                            "xor {h1:e}, {h1:e}",
                            "mov {zr}, 0",
                            "mov {l}, -1",
                            "adox {l}, {l}",
                            "2:",
                            "mulx {h0}, {l}, [{x}]",
                            "adcx {l}, {h1}",
                            "not {l}",
                            "adox {l}, [{z}]",
                            "mov [{d}], {l}",
                            "mulx {h1}, {l}, [{x} + 8]",
                            "adcx {l}, {h0}",
                            "not {l}",
                            "adox {l}, [{z} + 8]",
                            "mov [{d} + 8], {l}",
                            "mulx {h0}, {l}, [{x} + 16]",
                            "adcx {l}, {h1}",
                            "not {l}",
                            "adox {l}, [{z} + 16]",
                            "mov [{d} + 16], {l}",
                            "mulx {h1}, {l}, [{x} + 24]",
                            "adcx {l}, {h0}",
                            "not {l}",
                            "adox {l}, [{z} + 24]",
                            "mov [{d} + 24], {l}",
                            "lea {x}, [{x} + 32]",
                            "lea {z}, [{z} + 32]",
                            "lea {d}, [{d} + 32]",
                            "lea rcx, [rcx - 1]",
                            "jrcxz 3f",
                            "jmp 2b",
                            "3:",
                            "adcx {h1}, {zr}",
                            "seto {o}",
                            x = inout(reg) xs, z = inout(reg) zs, d = inout(reg) d, inout("rcx") n, in("rdx") w,
                            h1 = out(reg) h, h0 = out(reg) _, l = out(reg) _, zr = out(reg) _, o = lateout(reg_byte) o,
                            options(nostack),
                        )
                    }
                } else {
                    // p + not(z) + 1: OF set first (the 1)
                    unsafe {
                        core::arch::asm!(
                            "xor {h1:e}, {h1:e}",
                            "mov {zr}, 0",
                            "mov {l}, -1",
                            "adox {l}, {l}",
                            "2:",
                            "mulx {h0}, {l}, [{x}]",
                            "mov {t}, [{z}]",
                            "adcx {l}, {h1}",
                            "not {t}",
                            "adox {l}, {t}",
                            "mov [{d}], {l}",
                            "mulx {h1}, {l}, [{x} + 8]",
                            "mov {t}, [{z} + 8]",
                            "adcx {l}, {h0}",
                            "not {t}",
                            "adox {l}, {t}",
                            "mov [{d} + 8], {l}",
                            "mulx {h0}, {l}, [{x} + 16]",
                            "mov {t}, [{z} + 16]",
                            "adcx {l}, {h1}",
                            "not {t}",
                            "adox {l}, {t}",
                            "mov [{d} + 16], {l}",
                            "mulx {h1}, {l}, [{x} + 24]",
                            "mov {t}, [{z} + 24]",
                            "adcx {l}, {h0}",
                            "not {t}",
                            "adox {l}, {t}",
                            "mov [{d} + 24], {l}",
                            "lea {x}, [{x} + 32]",
                            "lea {z}, [{z} + 32]",
                            "lea {d}, [{d} + 32]",
                            "lea rcx, [rcx - 1]",
                            "jrcxz 3f",
                            "jmp 2b",
                            "3:",
                            "adcx {h1}, {zr}",
                            "seto {o}",
                            x = inout(reg) xs, z = inout(reg) zs, d = inout(reg) d, inout("rcx") n, in("rdx") w,
                            h1 = out(reg) h, h0 = out(reg) _, l = out(reg) _, t = out(reg) _, zr = out(reg) _, o = lateout(reg_byte) o,
                            options(nostack),
                        )
                    }
                }
                let _ = (xs, zs, d, n);
                c1 = h;
                c2 = o as u64;
            }
            for i in quads * 4..len {
                let p = (word(mx, i) as u128) * (w as u128) + c1 as u128;
                c1 = (p >> 64) as u64;
                let (a, b) = match (minus, p_larger) {
                    (false, _) => (word(mz, i), p as u64),
                    (true, false) => (word(mz, i), !(p as u64)),
                    (true, true) => (!word(mz, i), p as u64),
                };
                let t = a as u128 + b as u128 + c2 as u128;
                c2 = (t >> 64) as u64;
                v[1 + 8 * i..9 + 8 * i].copy_from_slice(&(t as u64).to_le_bytes());
            }
            let mut neg = if p_larger { pneg } else { nz };
            if minus && c2 == 0 {
                // below zero (undecided by the top words): the difference is
                // 2^(64 len) less what was written, negated (two's
                // complement), its sign turned
                let mut c = 1u64;
                for i in 0..len {
                    let x = u64::from_le_bytes(v[1 + 8 * i..9 + 8 * i].try_into().unwrap());
                    let (r, o) = (!x).overflowing_add(c);
                    c = o as u64;
                    v[1 + 8 * i..9 + 8 * i].copy_from_slice(&r.to_le_bytes());
                }
                neg = !neg;
            }
            // the last carry of a sum (none for a difference)
            let top = if minus { 0 } else { c2 };
            v[1 + 8 * len..9 + 8 * len].copy_from_slice(&top.to_le_bytes());
            v[0] = if neg { b'-' } else { b'+' };
            Some(Out(v).done())
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

    /// D406: `muladd` gives the bytes of the product then the sum (`op`):
    /// magnitudes of 0 to 330 bytes (partial last words, the 4 words of
    /// the assembly and the rest), words of all ones (every carry), every
    /// sign, `z` the product itself, one more and one less (a difference
    /// of zero, of one, below zero), a multiplier of one word in `x` or
    /// `y`.
    /// D406: the time of each way (not a check: run by hand).
    #[test]
    #[ignore]
    fn muladd_timed() {
        extern crate std;
        let make = |n: usize, b: u8| -> Vec<u8> {
            let mut v = alloc::vec![b'+'];
            for i in 0..n {
                v.push((i as u8).wrapping_mul(37).wrapping_add(b) | 1);
            }
            v
        };
        for n in [520 * 8, 64 * 8] {
            let (z, x, y) = (make(n, 3), make(n - 3, 7), alloc::vec![b'+', 0x35, 0x12, 0x07]);
            let rounds = 4_000_000 / n;
            for sub in [false, true] {
                let t0 = std::time::Instant::now();
                let mut acc = 0usize;
                for _ in 0..rounds {
                    acc += muladd(&z, &x, &y, sub).unwrap().len();
                }
                let t1 = std::time::Instant::now();
                for _ in 0..rounds {
                    let p = op(2, &x, &y).unwrap();
                    acc += op(if sub { 1 } else { 0 }, &z, &p).unwrap().len();
                }
                let t2 = std::time::Instant::now();
                let words = (rounds * n / 8) as f64;
                std::println!("{} words sub {}: one pass {:.3} ns a word, two passes {:.3} ns a word ({})", n / 8, sub, (t1 - t0).as_nanos() as f64 / words, (t2 - t1).as_nanos() as f64 / words, acc);
            }
        }
    }

    #[test]
    fn muladd_as_the_two_passes() {
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let mut make = |bytes: usize, neg: bool, ones: bool| -> Vec<u8> {
            let mut v = alloc::vec![if neg { b'-' } else { b'+' }];
            for _ in 0..bytes {
                v.push(if ones { 0xff } else { next() as u8 });
            }
            while v.last() == Some(&0) {
                v.pop();
            }
            if v.len() == 1 {
                v.clear();
            }
            v
        };
        let mut fused = 0;
        let mut k = 0u64;
        for case in 0..30000u64 {
            k = k.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407 + case);
            let zb = (k >> 8) as usize % 331;
            let xb = (k >> 20) as usize % 331;
            let yb = (k >> 32) as usize % 9;
            let ones = k >> 60 == 0;
            let x = make(xb, k & 1 != 0, ones);
            let y = make(yb, k & 2 != 0, ones && k & 16 != 0);
            let sub = k & 4 != 0;
            let p = op(2, &x, &y).unwrap();
            let z = match (k >> 40) % 5 {
                0 => p.clone(),
                1 => op(0, &p, b"+\x01").unwrap(),
                2 => op(1, &p, b"+\x01").unwrap(),
                _ => make(zb, k & 8 != 0, ones),
            };
            let want = op(if sub { 1 } else { 0 }, &z, &p).unwrap();
            let (x, y) = if k & 32 != 0 { (y, x) } else { (x, y) };
            if let Some(r) = muladd(&z, &x, &y, sub) {
                assert_eq!(r, want, "case {}: z {:?} x {:?} y {:?} sub {}", case, z, x, y, sub);
                fused += 1;
            }
        }
        #[cfg(all(target_arch = "x86_64", target_feature = "bmi2"))]
        if adx() {
            assert!(fused > 10000, "{} cases in one pass", fused);
        }
        let _ = fused;
    }
}

big_module!(big, 128);
big_module!(big256, 256);

/// The end of the last whole character of the UTF-8 bytes `b` (D408): before
/// the first byte of a character cut at the end (a lead byte followed by
/// fewer bytes than it announces), else the length (bytes that are not
/// UTF-8 left as they are).
pub fn utf8_whole(b: &[u8]) -> usize {
    for back in 1..=b.len().min(3) {
        let c = b[b.len() - back];
        if c & 0xc0 == 0x80 {
            continue;
        }
        let want = match c {
            0xc0..=0xdf => 2,
            0xe0..=0xef => 3,
            0xf0..=0xf7 => 4,
            _ => return b.len(),
        };
        return if back < want { b.len() - back } else { b.len() };
    }
    b.len()
}

#[cfg(test)]
mod utf8_cuts {
    /// D408: every prefix of texts of 1 to 4 bytes a character, cut at each
    /// byte: the end of its last whole character; ASCII, bytes that are not
    /// UTF-8 (a lone continuation, 0xff) left whole.
    #[test]
    fn the_last_whole_character() {
        extern crate std;
        let s = "aé日😀b€";
        let b = s.as_bytes();
        let ends: std::vec::Vec<usize> = s.char_indices().map(|(i, c)| i + c.len_utf8()).collect();
        for n in 0..=b.len() {
            let want = ends.iter().copied().filter(|&e| e <= n).max().unwrap_or(0);
            assert_eq!(super::utf8_whole(&b[..n]), want, "{} bytes", n);
        }
        assert_eq!(super::utf8_whole(b"ab\x80"), 3);
        assert_eq!(super::utf8_whole(b"ab\xff"), 3);
        assert_eq!(super::utf8_whole(b""), 0);
    }
}

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
        build(source, flags, true)
    }

    /// `compile`, with or without the prefilter of its literals (D219).
    fn build<'a>(source: &'a str, flags: &str, prefilter: bool) -> Result<Regex, Panic<'a>> {
        let cfg = syntax::Config::new()
            .case_insensitive(flags.contains('i'))
            .multi_line(flags.contains('m'))
            .dot_matches_new_line(flags.contains('s'));
        Regex::builder().configure(Regex::config().auto_prefilter(prefilter)).syntax(cfg).build(source).map_err(|e| {
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

    /// `of` without the prefilter: the automaton alone over every byte
    /// (D219).
    pub fn of_plain(v: &str) -> Regex {
        let (flags, source) = parts(v);
        build(source, flags, false).unwrap_or_else(|_| Regex::new("[^\\s\\S]").unwrap())
    }

    /// The number of matches of the global search of `re` in `s`, with the
    /// cache `cache` (kept by the caller from one call to the next).
    pub fn count_with(re: &Regex, cache: &mut regex_automata::meta::Cache, s: &str) -> usize {
        let (mut at, mut n) = (0usize, 0usize);
        while at <= s.len() {
            let Some(m) = re.search_with(cache, &regex_automata::Input::new(s).range(at..)) else { break };
            n += 1;
            at = next_at(s, m.start(), m.end());
        }
        n
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
        let mut cache = re.create_cache();
        merge_bounds_with(s, starts, parts, |at| re.search_with(&mut cache, &regex_automata::Input::new(s).range(at..)).map(|m| (m.start(), m.end())))
    }

    /// `merge_bounds` as runs of the matches of the parts (D281).
    pub fn merge_runs(re: &Regex, s: &str, starts: &[usize], parts: &[Vec<(usize, usize)>]) -> Vec<Run> {
        let mut cache = re.create_cache();
        merge_runs_with(s, starts, parts, |at| re.search_with(&mut cache, &regex_automata::Input::new(s).range(at..)).map(|m| (m.start(), m.end())))
    }

    /// A run of the matches of a search merged from its parts (D281): the
    /// matches `lo .. hi` of part `k`, or one match found alone at a cut.
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub enum Run {
        Part(usize, usize, usize),
        One(usize, usize),
    }

    /// The matches of `runs` over `parts`, in order.
    pub fn runs_iter<'a>(parts: &'a [Vec<(usize, usize)>], runs: &'a [Run]) -> impl Iterator<Item = (usize, usize)> + 'a {
        runs.iter().flat_map(move |r| {
            let (slice, one): (&[(usize, usize)], Option<(usize, usize)>) = match *r {
                Run::Part(k, lo, hi) => (&parts[k][lo..hi], None),
                Run::One(a, b) => (&[], Some((a, b))),
            };
            slice.iter().copied().chain(one)
        })
    }

    /// The number of matches of `runs`.
    pub fn runs_len(runs: &[Run]) -> usize {
        runs.iter().map(|r| match *r {
            Run::Part(_, lo, hi) => hi - lo,
            Run::One(..) => 1,
        }).sum()
    }

    /// `merge_bounds` with `find(at)`, the first match of a search from
    /// `at`, for any engine (D224).
    pub fn merge_bounds_with(s: &str, starts: &[usize], parts: &[Vec<(usize, usize)>], find: impl FnMut(usize) -> Option<(usize, usize)>) -> Vec<(usize, usize)> {
        let runs = merge_runs_with(s, starts, parts, find);
        let mut out = Vec::with_capacity(runs_len(&runs));
        for r in &runs {
            match *r {
                Run::Part(k, lo, hi) => out.extend_from_slice(&parts[k][lo..hi]),
                Run::One(a, b) => out.push((a, b)),
            }
        }
        out
    }

    /// The matches of the search split into the parts that start at
    /// `starts` (each searched up to the first match at or after the next
    /// start, that one included, or only to the next start when no match
    /// crosses it: D309), as runs of the matches of the parts (D281:
    /// no copy of them): where a part goes on after a match that crosses
    /// into it, found by halving (D276), or the matches found by `find`
    /// until it does.
    pub fn merge_runs_with(s: &str, starts: &[usize], parts: &[Vec<(usize, usize)>], mut find: impl FnMut(usize) -> Option<(usize, usize)>) -> Vec<Run> {
        let mut out = Vec::new();
        // the part whose search went on from `at` (the part `c` with
        // `starts[c] <= at`: its start, or after one of its matches), and
        // the index of its next match
        let follow = |at: usize| -> Option<(usize, usize)> {
            let c = match starts.binary_search(&at) {
                Ok(c) => return Some((c, 0)),
                Err(0) => return None,
                Err(c) => c - 1,
            };
            let p = &parts[c];
            let j = p.partition_point(|&(a, b)| next_at(s, a, b) < at);
            (j < p.len() && next_at(s, p[j].0, p[j].1) == at).then_some((c, j + 1))
        };
        let mut at = 0;
        let mut cur = follow(0);
        while at <= s.len() {
            match cur {
                Some((k, j)) if j < parts[k].len() => {
                    // the matches of the part before the next part starts,
                    // then the one that crosses into it (if any): they follow
                    // each other in it
                    let p = &parts[k];
                    let e = if k + 1 < starts.len() { p[j..].partition_point(|&(a, _)| a < starts[k + 1]) + j } else { p.len() };
                    let last = if e < p.len() { e + 1 } else { e };
                    out.push(Run::Part(k, j, last));
                    let (a, b) = p[last - 1];
                    at = next_at(s, a, b);
                    cur = if k + 1 < starts.len() && a >= starts[k + 1] { follow(at) } else { Some((k, last)) };
                }
                // the part met no more match before the start of the next
                // one, without a match across it (where a search by masks
                // stops, D309), or up to the end of `s`: the next part goes on
                // from its start, its search the same from there
                Some((k, _)) if k + 1 < starts.len() && at <= starts[k + 1] => {
                    at = starts[k + 1];
                    cur = Some((k + 1, 0));
                }
                // it met no more match up to the end of `s`
                Some(_) => break,
                None => {
                    let Some((a, b)) = find(at) else { break };
                    out.push(Run::One(a, b));
                    at = next_at(s, a, b);
                    cur = follow(at);
                }
            }
        }
        out
    }

    /// Simple shapes of patterns, searched by masks of 64 bytes instead of
    /// an automaton walked byte by byte (D224, D237). A pattern is an
    /// alternation of alternatives, each one:
    ///
    /// - fixed: a sequence of classes of ASCII bytes, of a length L from 1
    ///   to 64 (`agggtaaa|tttaccct` with `i`, `a[NSt]|BY`). In 64 bytes, the
    ///   mask of each class (a comparison per distinct byte), then the
    ///   starts of the sequence: the and of the masks of its classes, each
    ///   shifted by its place;
    /// - delimited: such a head, then `[^c]*` (or `[^c]+`), then the ASCII
    ///   byte c (`<[^>]*>`, `\|[^|][^|]*\|`, `.*\n`): after a head at p, the
    ///   match ends at the first c from p + L (none: no match of it from p
    ///   on);
    ///
    /// either at the start of a line (`^` with `m`) or anywhere. At a place
    /// the first alternative that matches there is the match (the order of
    /// a backtracking engine: `a|ab` matches `a`), as `^>.*\n|\n`.
    ///
    /// The classes must have no member outside ASCII (a member dropped
    /// would change the matches): ASCII bytes are whole characters in UTF-8,
    /// so the matches of bytes are the matches of characters. The global
    /// search: left to right, without overlap, as `each_match_from`.
    pub mod simple {
        use alloc::vec::Vec;
        use regex_syntax::hir::{Class, Hir, HirKind};

        /// At most this many alternatives, distinct classes, distinct bytes.
        const ALTS: usize = 32;
        const CLASSES: usize = 16;
        const BYTES: usize = 32;

        /// One alternative: its classes, at the start of a line or not, and
        /// for a delimited one its stop byte and the least number of bytes
        /// between its head and it.
        struct Alt {
            seq: Vec<u8>,
            line: bool,
            stop: Option<(u8, usize)>,
            /// the class of its stop byte alone: found in the masks of the
            /// blocks (D282)
            stop_class: u8,
        }

        pub struct Simple {
            /// each distinct byte of the classes, its classes (a bit each),
            /// and whether it is a lower case letter that stands for its
            /// upper case too (the two in the same classes: compared with
            /// the bytes `| 0x20`, which only these two give, D238)
            bytes: Vec<(u8, u16, bool)>,
            /// the alternatives, in their order
            alts: Vec<Alt>,
            /// the class `\n` (for the starts of lines), when an alternative
            /// is at the start of a line
            nl: Option<usize>,
            /// the plan of the fixed alternatives over few bytes (D275)
            fixed: Option<Fixed>,
        }

        /// Fixed alternatives only (no stop byte, none at the start of a
        /// line), over at most 8 distinct bytes (D275): per 64 bytes, the
        /// mask of each byte (a comparison), each class the `or` of the masks
        /// of its bytes, each alternative the `and` of its classes shifted by
        /// their places across two blocks (a shift of 128 bits), over its
        /// whole length: no branch that depends on the text before its
        /// candidates, where the general search leaves each alternative at
        /// its first empty mask (a branch the text decides).
        struct Fixed {
            /// the distinct bytes and whether each is compared `| 0x20`
            bytes: [(u8, bool); FIXED_BYTES],
            nb: usize,
            /// for each class used, the bytes in it (a bit each)
            classes: [u8; CLASSES],
            ncl: usize,
            /// for at most 8 classes on a processor with AVX-512 VBMI: the
            /// classes of each ASCII byte (a bit each), looked up for the 64
            /// bytes of a block by one permutation (D278)
            table: Option<[u8; 128]>,
            /// with the table, on a processor with GFNI, alternatives of at
            /// most 16 places: the search by lanes (D306), by groups of 8
            /// blocks (`bounds_groups`, D311)
            lanes: bool,
        }

        /// At most this many distinct bytes for `Fixed`.
        const FIXED_BYTES: usize = 8;

        /// A match added to `out`, which a search fills from `from` and has
        /// read up to `at` of a text of `n` bytes (D283): when full, room for
        /// the matches of the rest at the density seen so far, once, rather
        /// than by doublings each a copy of all the matches (3.5 million in
        /// a replacement of regex-redux: 56 MB copied about 20 times).
        #[inline(always)]
        fn push_match(out: &mut Vec<(usize, usize)>, m: (usize, usize), from: usize, at: usize, n: usize) {
            if out.len() == out.capacity() && out.len() >= 1024 {
                let seen = (at - from).max(1);
                let rest = (n.saturating_sub(at) as u128 * out.len() as u128 / seen as u128) as usize;
                out.reserve(rest + rest / 8 + 1024);
            }
            out.push(m);
        }

        /// The ASCII bytes of a class or literal of one character (a bit
        /// each); None if it has a member outside ASCII.
        fn ascii_set(h: &Hir) -> Option<u128> {
            match h.kind() {
                HirKind::Literal(l) if l.0.len() == 1 && l.0[0] < 0x80 => Some(1u128 << l.0[0]),
                HirKind::Class(Class::Unicode(c)) => {
                    let mut m = 0u128;
                    for r in c.ranges() {
                        if r.end() as u32 >= 0x80 {
                            return None;
                        }
                        for b in r.start() as u32..=r.end() as u32 {
                            m |= 1u128 << b;
                        }
                    }
                    Some(m)
                }
                HirKind::Class(Class::Bytes(c)) => {
                    let mut m = 0u128;
                    for r in c.ranges() {
                        if r.end() >= 0x80 {
                            return None;
                        }
                        for b in r.start()..=r.end() {
                            m |= 1u128 << b;
                        }
                    }
                    Some(m)
                }
                HirKind::Capture(c) => ascii_set(&c.sub),
                _ => None,
            }
        }

        /// The class "every character but the ASCII byte c": its c.
        fn all_but(h: &Hir) -> Option<u8> {
            match h.kind() {
                HirKind::Class(Class::Unicode(c)) => {
                    let r: Vec<_> = c.ranges().iter().map(|r| (r.start() as u32, r.end() as u32)).collect();
                    match r.as_slice() {
                        [(0, a), (b, 0x10FFFF)] if *a + 2 == *b && *b - 1 < 0x80 => Some((*b - 1) as u8),
                        [(1, 0x10FFFF)] => Some(0),
                        _ => None,
                    }
                }
                HirKind::Class(Class::Bytes(c)) => {
                    let r: Vec<_> = c.ranges().iter().map(|r| (r.start() as u32, r.end() as u32)).collect();
                    match r.as_slice() {
                        [(0, a), (b, 0xFF)] if *a + 2 == *b && *b - 1 < 0x80 => Some((*b - 1) as u8),
                        [(1, 0xFF)] => Some(0),
                        _ => None,
                    }
                }
                _ => None,
            }
        }

        /// The sequences of classes a pattern of fixed length matches: a
        /// literal, a class, a concatenation, an alternation, a group, a
        /// repetition of fixed count; None otherwise or beyond ALTS.
        fn expand(h: &Hir) -> Option<Vec<Vec<u128>>> {
            match h.kind() {
                HirKind::Empty => Some(alloc::vec![Vec::new()]),
                HirKind::Literal(l) => {
                    if l.0.iter().any(|&b| b >= 0x80) {
                        return None;
                    }
                    Some(alloc::vec![l.0.iter().map(|&b| 1u128 << b).collect()])
                }
                HirKind::Class(_) => Some(alloc::vec![alloc::vec![ascii_set(h)?]]),
                HirKind::Capture(c) => expand(&c.sub),
                HirKind::Concat(hs) => {
                    let mut acc: Vec<Vec<u128>> = alloc::vec![Vec::new()];
                    for x in hs {
                        let e = expand(x)?;
                        if acc.len() * e.len() > ALTS {
                            return None;
                        }
                        let mut next = Vec::with_capacity(acc.len() * e.len());
                        for a in &acc {
                            for b in &e {
                                let mut v = a.clone();
                                v.extend_from_slice(b);
                                next.push(v);
                            }
                        }
                        acc = next;
                    }
                    Some(acc)
                }
                HirKind::Alternation(hs) => {
                    let mut out = Vec::new();
                    for x in hs {
                        out.extend(expand(x)?);
                        if out.len() > ALTS {
                            return None;
                        }
                    }
                    Some(out)
                }
                HirKind::Repetition(r) if r.max == Some(r.min) && r.min <= 64 => {
                    let e = expand(&r.sub)?;
                    let mut acc: Vec<Vec<u128>> = alloc::vec![Vec::new()];
                    for _ in 0..r.min {
                        if acc.len() * e.len() > ALTS {
                            return None;
                        }
                        let mut next = Vec::new();
                        for a in &acc {
                            for b in &e {
                                let mut v = a.clone();
                                v.extend_from_slice(b);
                                next.push(v);
                            }
                        }
                        acc = next;
                    }
                    Some(acc)
                }
                _ => None,
            }
        }

        /// The shape of the pattern of the value `v` (`flags/source`), if it
        /// is one of the two.
        /// The alternatives of one branch of the pattern: `^` with `m` (the
        /// start of a line), then a delimited or a fixed shape.
        fn branch(h: &Hir, out: &mut Vec<(Vec<u128>, bool, Option<(u8, usize)>)>) -> Option<()> {
            let mut h = h;
            while let HirKind::Capture(c) = h.kind() {
                h = &c.sub;
            }
            let parts: &[Hir] = match h.kind() {
                HirKind::Concat(hs) => hs,
                _ => core::slice::from_ref(h),
            };
            let (line, parts) = match parts.first().map(|x| x.kind()) {
                Some(HirKind::Look(regex_syntax::hir::Look::StartLF)) => (true, &parts[1..]),
                Some(HirKind::Look(_)) => return None,
                _ => (false, parts),
            };
            // delimited: head, [^c] (optional), [^c]* or [^c]+, c
            let n = parts.len();
            if n >= 3 {
                let last = ascii_set(&parts[n - 1]).filter(|m| m.count_ones() == 1).map(|m| m.trailing_zeros() as u8);
                if let (Some(c), HirKind::Repetition(r)) = (last, parts[n - 2].kind()) {
                    if r.max.is_none() && r.min <= 1 && all_but(&r.sub) == Some(c) {
                        let mut k = n - 2;
                        let mut mid = r.min as usize;
                        if mid == 0 && k >= 2 && all_but(&parts[k - 1]) == Some(c) {
                            k -= 1;
                            mid = 1;
                        }
                        let head = seqs_of(&parts[..k])?;
                        for q in head {
                            out.push((q, line, Some((c, mid))));
                        }
                        return Some(());
                    }
                }
            }
            for q in seqs_of(parts)? {
                out.push((q, line, None));
            }
            Some(())
        }

        /// The sequences of classes of a concatenation, in the order of a
        /// backtracking engine.
        fn seqs_of(parts: &[Hir]) -> Option<Vec<Vec<u128>>> {
            let mut acc: Vec<Vec<u128>> = alloc::vec![Vec::new()];
            for x in parts {
                let e = expand(x)?;
                if acc.len() * e.len() > ALTS {
                    return None;
                }
                let mut next = Vec::with_capacity(acc.len() * e.len());
                for a in &acc {
                    for b in &e {
                        let mut v = a.clone();
                        v.extend_from_slice(b);
                        next.push(v);
                    }
                }
                acc = next;
            }
            Some(acc)
        }

        /// The shape of the pattern of the value `v` (`flags/source`), if it
        /// is an alternation of the shapes above.
        pub fn of(v: &str) -> Option<Simple> {
            // the masks need SIMD (SSE2 at least)
            if !cfg!(target_arch = "x86_64") {
                return None;
            }
            let (flags, source) = super::parts(v);
            let cfg = regex_automata::util::syntax::Config::new()
                .case_insensitive(flags.contains('i'))
                .multi_line(flags.contains('m'))
                .dot_matches_new_line(flags.contains('s'));
            let hir = regex_automata::util::syntax::parse_with(source, &cfg).ok()?;
            let mut h = &hir;
            while let HirKind::Capture(c) = h.kind() {
                h = &c.sub;
            }
            let mut raw = Vec::new();
            match h.kind() {
                HirKind::Alternation(hs) => {
                    for x in hs {
                        branch(x, &mut raw)?;
                    }
                }
                _ => branch(h, &mut raw)?,
            }
            if raw.is_empty() || raw.len() > ALTS || raw.iter().any(|(q, _, _)| q.is_empty() || q.len() > 64) {
                return None;
            }
            // distinct classes (the newline last, for the starts of lines),
            // distinct bytes
            let mut classes: Vec<u128> = Vec::new();
            let mut alts = Vec::new();
            let index = |classes: &mut Vec<u128>, m: u128| -> Option<u8> {
                let k = match classes.iter().position(|&c| c == m) {
                    Some(k) => k,
                    None => {
                        classes.push(m);
                        classes.len() - 1
                    }
                };
                (k < CLASSES).then_some(k as u8)
            };
            for (q, line, stop) in &raw {
                let mut seq = Vec::with_capacity(q.len());
                for &m in q {
                    if m == 0 {
                        return None;
                    }
                    seq.push(index(&mut classes, m)?);
                }
                alts.push(Alt { seq, line: *line, stop: *stop, stop_class: 0 });
            }
            for a in alts.iter_mut() {
                if let Some((b, _)) = a.stop {
                    a.stop_class = index(&mut classes, 1u128 << b)?;
                }
            }
            let nl = if alts.iter().any(|a| a.line) { Some(index(&mut classes, 1u128 << b'\n')? as usize) } else { None };
            let class_of = |b: u8| -> u16 {
                let mut cl = 0u16;
                for (k, &m) in classes.iter().enumerate() {
                    if m >> b & 1 == 1 {
                        cl |= 1 << k;
                    }
                }
                cl
            };
            let mut bytes = Vec::new();
            for b in 0..128u8 {
                let cl = class_of(b);
                if cl == 0 {
                    continue;
                }
                let letter = b.is_ascii_alphabetic();
                let pair = class_of(b ^ 0x20);
                if letter && b.is_ascii_uppercase() && pair == cl {
                    // its lower case compares for it
                    continue;
                }
                bytes.push((b, cl, letter && b.is_ascii_lowercase() && pair == cl));
            }
            if bytes.len() > BYTES {
                return None;
            }
            let fixed = Fixed::of(&bytes, &alts);
            Some(Simple { bytes, alts, nl, fixed })
        }

        impl Fixed {
            fn of(bytes: &[(u8, u16, bool)], alts: &[Alt]) -> Option<Fixed> {
                if bytes.len() > FIXED_BYTES || alts.iter().any(|a| a.stop.is_some() || a.line || a.seq.is_empty() || a.seq.len() > 64) {
                    return None;
                }
                let ncl = alts.iter().flat_map(|a| a.seq.iter()).map(|&k| k as usize + 1).max().unwrap_or(0);
                if ncl > CLASSES {
                    return None;
                }
                let mut fx = Fixed { bytes: [(0, false); FIXED_BYTES], nb: bytes.len(), classes: [0; CLASSES], ncl, table: None, lanes: false };
                let mut table = [0u8; 128];
                for (i, &(v, cl, fold)) in bytes.iter().enumerate() {
                    fx.bytes[i] = (v, fold);
                    for c in 0..CLASSES {
                        if cl >> c & 1 == 1 {
                            fx.classes[c] |= 1 << i;
                        }
                    }
                    // the byte, and its upper case when it stands for it
                    for b in [v, if fold { v & !0x20 } else { v }] {
                        if (b as usize) < 128 {
                            table[b as usize] |= cl as u8;
                        }
                    }
                }
                if ncl <= 8 && bytes.iter().all(|&(v, _, _)| v < 128) && vbmi() {
                    fx.table = Some(table);
                    fx.lanes = gfni() && alts.iter().all(|a| a.seq.len() <= 16);
                }
                Some(fx)
            }

            /// The masks of the classes over the 64 bytes from `at` (bytes
            /// past the end of `s` in no class).
            #[inline(always)]
            fn masks<const N: usize>(&self, s: &[u8], at: usize, cm: &mut [u64; CLASSES]) {
                let mut tail = [0u8; 64];
                let (p, valid) = if at >= s.len() {
                    *cm = [0; CLASSES];
                    return;
                } else if at + 64 <= s.len() {
                    (s[at..].as_ptr(), !0u64)
                } else {
                    let n = s.len() - at;
                    tail[..n].copy_from_slice(&s[at..]);
                    (tail.as_ptr(), (1u64 << n) - 1)
                };
                #[cfg(all(target_arch = "x86_64", target_feature = "avx512bw"))]
                if let Some(t) = &self.table {
                    // SAFETY: `table` is set only where `vbmi()` saw the
                    // instructions; `p` has 64 readable bytes
                    unsafe { class_masks_vbmi(t, self.ncl, p, valid, cm) };
                    return;
                }
                let bm: [u64; N] = byte_masks::<N>(&self.bytes, p);
                for c in 0..self.ncl {
                    let set = self.classes[c];
                    let mut m = 0u64;
                    for (i, &b) in bm.iter().enumerate() {
                        // the same bytes every block: predicted
                        if set >> i & 1 == 1 {
                            m |= b;
                        }
                    }
                    cm[c] = m & valid;
                }
            }

            fn bounds<const N: usize>(&self, sm: &Simple, s: &str, from: usize, until: usize) -> Vec<(usize, usize)> {
                let b = s.as_bytes();
                let n = b.len();
                let mut out = Vec::new();
                let mut base = from;
                let mut blocks = [[0u64; CLASSES]; 2];
                let mut cur = 0usize;
                self.masks::<N>(b, base, &mut blocks[cur]);
                let mut allowed = from;
                // the start of the next part, where the search may stop (D309);
                // none for `find_at` (`until` at `from`: the first match from there)
                let stop_at = if until > from { until } else { usize::MAX };
                let mut st = [0u64; ALTS];
                while base < n {
                    let nx = 1 - cur;
                    self.masks::<N>(b, base + 64, &mut blocks[nx]);
                    let (c, x) = (&blocks[cur], &blocks[nx]);
                    let mut any = 0u64;
                    for (a, alt) in sm.alts.iter().enumerate() {
                        let m = starts(&alt.seq, c, x);
                        st[a & (ALTS - 1)] = m;
                        any |= m;
                    }
                    while any != 0 {
                        let j = any.trailing_zeros();
                        any &= any - 1;
                        let p = base + j as usize;
                        if p < allowed {
                            continue;
                        }
                        // at the next part, no match across its start: it finds the
                        // same matches from there (D309)
                        if p >= stop_at && allowed <= stop_at {
                            return out;
                        }
                        // the first alternative that matches at p
                        let Some(a) = (0..sm.alts.len()).find(|&a| st[a & (ALTS - 1)] >> j & 1 == 1) else { continue };
                        let e = p + sm.alts[a].seq.len();
                        push_match(&mut out, (p, e), from, base + 64, until.min(n));
                        allowed = e;
                        if p >= until {
                            return out;
                        }
                    }
                    base += 64;
                    // past the start of the next part, no match across it: the
                    // next part finds the same matches from there (D309: the
                    // search went on to the first match after it, to the end of
                    // the text when there was none)
                    if base >= stop_at && allowed <= stop_at {
                        return out;
                    }
                    cur = nx;
                    // inside a long match: on from its end
                    if allowed > base + 64 {
                        base = allowed;
                        self.masks::<N>(b, base, &mut blocks[cur]);
                    }
                }
                out
            }
        }

        /// The processor has AVX-512 VBMI (with the state of AVX-512 saved
        /// by the system): `vpermi2b`, a lookup of 64 bytes in a table of 128.
        #[cfg(all(target_arch = "x86_64", target_feature = "avx512bw"))]
        fn vbmi() -> bool {
            use core::arch::x86_64::{__cpuid_count, _xgetbv};
            // SAFETY: cpuid exists on every x86-64; xgetbv when OSXSAVE says so
            unsafe {
                let l1 = __cpuid_count(1, 0);
                if l1.ecx >> 27 & 1 == 0 {
                    return false;
                }
                // the opmask and the upper zmm registers saved (bits 5 to 7)
                if _xgetbv(0) & 0xe6 != 0xe6 {
                    return false;
                }
                __cpuid_count(7, 0).ecx >> 1 & 1 == 1
            }
        }

        #[cfg(not(all(target_arch = "x86_64", target_feature = "avx512bw")))]
        fn vbmi() -> bool {
            false
        }

        /// The processor has GFNI (`gf2p8affineqb`, with AVX-512: on 64
        /// bytes), where it has AVX-512 VBMI.
        #[cfg(all(target_arch = "x86_64", target_feature = "avx512bw"))]
        fn gfni() -> bool {
            vbmi() && core::arch::x86_64::__cpuid_count(7, 0).ecx >> 8 & 1 == 1
        }

        #[cfg(not(all(target_arch = "x86_64", target_feature = "avx512bw")))]
        fn gfni() -> bool {
            false
        }

        /// The 64 bytes of `s` from `at` (those past its end zero, not read)
        /// and the mask of those of the text.
        #[cfg(all(target_arch = "x86_64", target_feature = "avx512bw"))]
        #[target_feature(enable = "avx512f,avx512bw")]
        fn block_at(s: &[u8], at: usize) -> (core::arch::x86_64::__m512i, u64) {
            use core::arch::x86_64::*;
            if at + 64 <= s.len() {
                let b: &[u8; 64] = s[at..at + 64].try_into().expect("64 bytes");
                // SAFETY: 64 readable bytes
                (unsafe { _mm512_loadu_si512(b.as_ptr().cast()) }, !0)
            } else {
                let rest = &s[at.min(s.len())..];
                let valid = (1u64 << rest.len()) - 1;
                // SAFETY: the bytes of `rest` only (`valid`): a masked load
                // reads no byte outside its mask
                (unsafe { _mm512_maskz_loadu_epi8(valid, rest.as_ptr().cast()) }, valid)
            }
        }

        /// The masks of the classes of the 64 bytes `x` (`valid` those of the
        /// text) as lanes, lane c the mask of class c (D306): the classes of
        /// each byte looked up in `t` (a bit each), the bytes of each lane of
        /// 8 reversed, their bits transposed (`gf2p8affineqb`: byte i of a
        /// lane gets bit i of its 8 bytes), then byte c of each lane into
        /// lane c. Five instructions for every class, where one test each
        /// and their stores took 54 instructions a block.
        #[cfg(all(target_arch = "x86_64", target_feature = "avx512bw"))]
        #[target_feature(enable = "avx512f,avx512bw,avx512vbmi,gfni")]
        fn class_lanes(x: core::arch::x86_64::__m512i, valid: u64, lo: core::arch::x86_64::__m512i, hi: core::arch::x86_64::__m512i) -> core::arch::x86_64::__m512i {
            use core::arch::x86_64::*;
            let ascii = !_mm512_movepi8_mask(x) & valid;
            let cls = _mm512_maskz_permutex2var_epi8(ascii, lo, x, hi);
            let rev = _mm512_set_epi64(
                0x3839_3a3b_3c3d_3e3f,
                0x3031_3233_3435_3637,
                0x2829_2a2b_2c2d_2e2f,
                0x2021_2223_2425_2627,
                0x1819_1a1b_1c1d_1e1f,
                0x1011_1213_1415_1617,
                0x0809_0a0b_0c0d_0e0f,
                0x0001_0203_0405_0607,
            );
            let r = _mm512_permutexvar_epi8(rev, cls);
            let t = _mm512_gf2p8affine_epi64_epi8(_mm512_set1_epi64(0x8040_2010_0804_0201u64 as i64), r, 0);
            // lane c: byte c of each lane, in order
            let gather = _mm512_set_epi64(
                0x3f37_2f27_1f17_0f07,
                0x3e36_2e26_1e16_0e06,
                0x3d35_2d25_1d15_0d05,
                0x3c34_2c24_1c14_0c04,
                0x3b33_2b23_1b13_0b03,
                0x3a32_2a22_1a12_0a02,
                0x3931_2921_1911_0901,
                0x3830_2820_1810_0800,
            );
            _mm512_permutexvar_epi8(gather, t)
        }

        /// The masks of the classes of the 8 blocks of 64 bytes from `at`
        /// (bytes past the end of `b` in no class), transposed (D311): lane i
        /// of vector c the mask of class c in block i. Each block's classes as
        /// lanes (`class_lanes`), then an 8 by 8 transposition of their lanes.
        #[cfg(all(target_arch = "x86_64", target_feature = "avx512bw"))]
        #[target_feature(enable = "avx512f,avx512bw,avx512vbmi,gfni")]
        #[inline]
        fn group_lanes(b: &[u8], at: usize, lo: core::arch::x86_64::__m512i, hi: core::arch::x86_64::__m512i) -> [core::arch::x86_64::__m512i; 8] {
            use core::arch::x86_64::*;
            let mut l = [_mm512_setzero_si512(); 8];
            for (i, li) in l.iter_mut().enumerate() {
                let p = at + 64 * i;
                if p < b.len() {
                    let (x, valid) = block_at(b, p);
                    *li = class_lanes(x, valid, lo, hi);
                }
            }
            let t = [
                _mm512_unpacklo_epi64(l[0], l[1]),
                _mm512_unpackhi_epi64(l[0], l[1]),
                _mm512_unpacklo_epi64(l[2], l[3]),
                _mm512_unpackhi_epi64(l[2], l[3]),
                _mm512_unpacklo_epi64(l[4], l[5]),
                _mm512_unpackhi_epi64(l[4], l[5]),
                _mm512_unpacklo_epi64(l[6], l[7]),
                _mm512_unpackhi_epi64(l[6], l[7]),
            ];
            let (i0, i1) = (_mm512_set_epi64(13, 12, 5, 4, 9, 8, 1, 0), _mm512_set_epi64(15, 14, 7, 6, 11, 10, 3, 2));
            let u = [
                _mm512_permutex2var_epi64(t[0], i0, t[2]),
                _mm512_permutex2var_epi64(t[1], i0, t[3]),
                _mm512_permutex2var_epi64(t[0], i1, t[2]),
                _mm512_permutex2var_epi64(t[1], i1, t[3]),
                _mm512_permutex2var_epi64(t[4], i0, t[6]),
                _mm512_permutex2var_epi64(t[5], i0, t[7]),
                _mm512_permutex2var_epi64(t[4], i1, t[6]),
                _mm512_permutex2var_epi64(t[5], i1, t[7]),
            ];
            let (j0, j1) = (_mm512_set_epi64(11, 10, 9, 8, 3, 2, 1, 0), _mm512_set_epi64(15, 14, 13, 12, 7, 6, 5, 4));
            [
                _mm512_permutex2var_epi64(u[0], j0, u[4]),
                _mm512_permutex2var_epi64(u[1], j0, u[5]),
                _mm512_permutex2var_epi64(u[2], j0, u[6]),
                _mm512_permutex2var_epi64(u[3], j0, u[7]),
                _mm512_permutex2var_epi64(u[0], j1, u[4]),
                _mm512_permutex2var_epi64(u[1], j1, u[5]),
                _mm512_permutex2var_epi64(u[2], j1, u[6]),
                _mm512_permutex2var_epi64(u[3], j1, u[7]),
            ]
        }

        /// `Fixed::bounds` by groups of 8 blocks (D311): the masks of the
        /// classes of a group transposed, a lane for each block
        /// (`group_lanes`), each alternative the `and` of its places lane by
        /// lane (the mask of its class shifted by its place, with the next
        /// block's), no reduction of lanes; a group without a start of match
        /// left at once. The starts of each block of the others, their choice
        /// and their order as `Fixed::bounds`, and the stop at the next part
        /// (D309). About 20 instructions a block for two alternatives of 8
        /// places, where the search by lanes of D306 took 111: a count 0.20 -> 0.063 ns a
        /// byte in its measure alone.
        #[cfg(all(target_arch = "x86_64", target_feature = "avx512bw"))]
        #[target_feature(enable = "avx512f,avx512bw,avx512vbmi,gfni")]
        fn bounds_groups(t: &[u8; 128], sm: &Simple, s: &str, from: usize, until: usize) -> Vec<(usize, usize)> {
            use core::arch::x86_64::*;
            let b = s.as_bytes();
            let n = b.len();
            let (lo, _) = block_at(t, 0);
            let (hi, _) = block_at(t, 64);
            let mut out = Vec::new();
            let mut allowed = from;
            // the start of the next part, where the search may stop (D309);
            // none for `find_at` (`until` at `from`: the first match from there)
            let stop_at = if until > from { until } else { usize::MAX };
            let na = sm.alts.len().min(ALTS);
            let mut st = [[0u64; 8]; ALTS];
            // each place of each alternative after its first: its class and the
            // shifts of its place, prepared once
            let places: Vec<Vec<(usize, __m128i, __m128i)>> = sm.alts[..na]
                .iter()
                .map(|alt| alt.seq.iter().enumerate().skip(1).map(|(j, &k)| ((k & 7) as usize, _mm_set_epi64x(0, j as i64), _mm_set_epi64x(0, 64 - j as i64))).collect())
                .collect();
            let firsts: Vec<usize> = sm.alts[..na].iter().map(|alt| (alt.seq[0] & 7) as usize).collect();
            let mut base = from;
            let mut cur = group_lanes(b, base, lo, hi);
            while base < n {
                let next = group_lanes(b, base + 512, lo, hi);
                // lane i: the masks of block i + 1, the last from the next group
                let mut nb = [_mm512_setzero_si512(); 8];
                for (c, v) in nb.iter_mut().enumerate() {
                    *v = _mm512_alignr_epi64(next[c], cur[c], 1);
                }
                let mut any = _mm512_setzero_si512();
                for (a, ps) in places.iter().enumerate() {
                    let mut m = cur[firsts[a]];
                    for &(k, sh, back) in ps {
                        let v = _mm512_or_si512(_mm512_srl_epi64(cur[k], sh), _mm512_sll_epi64(nb[k], back));
                        m = _mm512_and_si512(m, v);
                    }
                    any = _mm512_or_si512(any, m);
                    // SAFETY: 8 lanes of 64 bits into 8 integers of 64 bits
                    unsafe { _mm512_storeu_si512(st[a].as_mut_ptr().cast(), m) };
                }
                if _mm512_test_epi64_mask(any, any) != 0 {
                    for blk in 0..8 {
                        let bb = base + 64 * blk;
                        let mut any = 0u64;
                        for row in st.iter().take(na) {
                            any |= row[blk];
                        }
                        while any != 0 {
                            let j = any.trailing_zeros();
                            any &= any - 1;
                            let p = bb + j as usize;
                            if p < allowed {
                                continue;
                            }
                            // at the next part, no match across its start: it
                            // finds the same matches from there (D309)
                            if p >= stop_at && allowed <= stop_at {
                                return out;
                            }
                            // the first alternative that matches at p
                            let Some(a) = (0..na).find(|&a| st[a][blk] >> j & 1 == 1) else { continue };
                            let e = p + sm.alts[a].seq.len();
                            push_match(&mut out, (p, e), from, bb + 64, until.min(n));
                            allowed = e;
                            if p >= until {
                                return out;
                            }
                        }
                    }
                }
                base += 512;
                // past the start of the next part, no match across it (D309)
                if base >= stop_at && allowed <= stop_at {
                    return out;
                }
                cur = next;
            }
            out
        }


        /// The masks of the `ncl` classes over the 64 bytes at `p` (`valid`
        /// the bytes of the text): each byte's classes looked up in `t`,
        /// bytes of 128 and more in none, then one test per class (D278).
        #[cfg(all(target_arch = "x86_64", target_feature = "avx512bw"))]
        #[target_feature(enable = "avx512f,avx512bw,avx512vbmi")]
        unsafe fn class_masks_vbmi(t: &[u8; 128], ncl: usize, p: *const u8, valid: u64, cm: &mut [u64; CLASSES]) {
            use core::arch::x86_64::*;
            let x = _mm512_loadu_si512(p as *const __m512i);
            let lo = _mm512_loadu_si512(t.as_ptr() as *const __m512i);
            let hi = _mm512_loadu_si512(t.as_ptr().add(64) as *const __m512i);
            let ascii = !_mm512_movepi8_mask(x) & valid;
            let cls = _mm512_maskz_permutex2var_epi8(ascii, lo, x, hi);
            for (c, m) in cm.iter_mut().enumerate().take(ncl) {
                *m = _mm512_test_epi8_mask(cls, _mm512_set1_epi8(1 << c));
            }
        }

        /// The mask of each of the first `N` bytes of `bytes` over the 64
        /// bytes at `p` (`| 0x20` first where its flag says).
        #[cfg(all(target_arch = "x86_64", target_feature = "avx512bw"))]
        #[inline(always)]
        fn byte_masks<const N: usize>(bytes: &[(u8, bool); FIXED_BYTES], p: *const u8) -> [u64; N] {
            use core::arch::x86_64::*;
            let mut out = [0u64; N];
            // SAFETY: `p` has 64 readable bytes (the text or `tail`)
            unsafe {
                let a = _mm512_loadu_si512(p as *const __m512i);
                let af = _mm512_or_si512(a, _mm512_set1_epi8(0x20));
                for (i, o) in out.iter_mut().enumerate() {
                    let (v, fold) = bytes[i];
                    *o = _mm512_cmpeq_epi8_mask(if fold { af } else { a }, _mm512_set1_epi8(v as i8));
                }
            }
            out
        }

        #[cfg(all(target_arch = "x86_64", target_feature = "avx2", not(target_feature = "avx512bw")))]
        #[inline(always)]
        fn byte_masks<const N: usize>(bytes: &[(u8, bool); FIXED_BYTES], p: *const u8) -> [u64; N] {
            use core::arch::x86_64::*;
            let mut out = [0u64; N];
            // SAFETY: `p` has 64 readable bytes (the text or `tail`)
            unsafe {
                let v0 = _mm256_loadu_si256(p as *const __m256i);
                let v1 = _mm256_loadu_si256(p.add(32) as *const __m256i);
                let low = _mm256_set1_epi8(0x20);
                let (f0, f1) = (_mm256_or_si256(v0, low), _mm256_or_si256(v1, low));
                for (i, o) in out.iter_mut().enumerate() {
                    let (v, fold) = bytes[i];
                    let x = _mm256_set1_epi8(v as i8);
                    let (y0, y1) = if fold { (f0, f1) } else { (v0, v1) };
                    *o = (_mm256_movemask_epi8(_mm256_cmpeq_epi8(y0, x)) as u32 as u64) | ((_mm256_movemask_epi8(_mm256_cmpeq_epi8(y1, x)) as u32 as u64) << 32);
                }
            }
            out
        }

        #[cfg(all(target_arch = "x86_64", not(target_feature = "avx2")))]
        #[inline(always)]
        fn byte_masks<const N: usize>(bytes: &[(u8, bool); FIXED_BYTES], p: *const u8) -> [u64; N] {
            use core::arch::x86_64::*;
            let mut out = [0u64; N];
            // SAFETY: `p` has 64 readable bytes (the text or `tail`)
            unsafe {
                let r = [
                    _mm_loadu_si128(p as *const __m128i),
                    _mm_loadu_si128(p.add(16) as *const __m128i),
                    _mm_loadu_si128(p.add(32) as *const __m128i),
                    _mm_loadu_si128(p.add(48) as *const __m128i),
                ];
                let low = _mm_set1_epi8(0x20);
                let f = [_mm_or_si128(r[0], low), _mm_or_si128(r[1], low), _mm_or_si128(r[2], low), _mm_or_si128(r[3], low)];
                for (i, o) in out.iter_mut().enumerate() {
                    let (v, fold) = bytes[i];
                    let x = _mm_set1_epi8(v as i8);
                    let y = if fold { &f } else { &r };
                    *o = (_mm_movemask_epi8(_mm_cmpeq_epi8(y[0], x)) as u16 as u64)
                        | ((_mm_movemask_epi8(_mm_cmpeq_epi8(y[1], x)) as u16 as u64) << 16)
                        | ((_mm_movemask_epi8(_mm_cmpeq_epi8(y[2], x)) as u16 as u64) << 32)
                        | ((_mm_movemask_epi8(_mm_cmpeq_epi8(y[3], x)) as u16 as u64) << 48);
                }
            }
            out
        }

        /// Without SIMD (not used: `of` finds no shape there).
        #[cfg(not(target_arch = "x86_64"))]
        #[inline(always)]
        fn byte_masks<const N: usize>(bytes: &[(u8, bool); FIXED_BYTES], p: *const u8) -> [u64; N] {
            let mut out = [0u64; N];
            // SAFETY: `p` has 64 readable bytes (the text or `tail`)
            let blk = unsafe { core::slice::from_raw_parts(p, 64) };
            for (i, o) in out.iter_mut().enumerate() {
                let (v, fold) = bytes[i];
                for (j, &x) in blk.iter().enumerate() {
                    let y = if fold { x | 0x20 } else { x };
                    *o |= ((y == v) as u64) << j;
                }
            }
            out
        }

        /// The masks of the classes over the 64 bytes from `at` (bytes past
        /// the end of `s` in no class).
        #[inline(always)]
        fn masks(sm: &Simple, s: &[u8], at: usize, cm: &mut [u64; CLASSES]) {
            *cm = [0; CLASSES];
            if at >= s.len() {
                return;
            }
            let mut tail = [0u8; 64];
            let (p, valid) = if at + 64 <= s.len() {
                (s[at..].as_ptr(), !0u64)
            } else {
                let n = s.len() - at;
                tail[..n].copy_from_slice(&s[at..]);
                (tail.as_ptr(), (1u64 << n) - 1)
            };
            eqs(sm, p, cm);
            if valid != !0u64 {
                for m in cm.iter_mut() {
                    *m &= valid;
                }
            }
        }

        /// AVX-512 (the runtime for x86-64-v4, D241): one comparison gives
        /// the mask of the 64 bytes.
        #[cfg(all(target_arch = "x86_64", target_feature = "avx512bw"))]
        #[inline(always)]
        fn eqs(sm: &Simple, p: *const u8, cm: &mut [u64; CLASSES]) {
            use core::arch::x86_64::*;
            // SAFETY: `p` has 64 readable bytes (the text or `tail`)
            unsafe {
                let a = _mm512_loadu_si512(p as *const __m512i);
                let af = _mm512_or_si512(a, _mm512_set1_epi8(0x20));
                for &(v, cl, fold) in &sm.bytes {
                    let m = _mm512_cmpeq_epi8_mask(if fold { af } else { a }, _mm512_set1_epi8(v as i8));
                    let mut c = cl;
                    while c != 0 {
                        cm[(c.trailing_zeros() & 15) as usize] |= m;
                        c &= c - 1;
                    }
                }
            }
        }

        #[cfg(all(target_arch = "x86_64", target_feature = "avx2", not(target_feature = "avx512bw")))]
        #[inline(always)]
        fn eqs(sm: &Simple, p: *const u8, cm: &mut [u64; CLASSES]) {
            use core::arch::x86_64::*;
            // SAFETY: `p` has 64 readable bytes (the text or `tail`)
            unsafe {
                let a = _mm256_loadu_si256(p as *const __m256i);
                let b = _mm256_loadu_si256(p.add(32) as *const __m256i);
                let low = _mm256_set1_epi8(0x20);
                let (af, bf) = (_mm256_or_si256(a, low), _mm256_or_si256(b, low));
                for &(v, cl, fold) in &sm.bytes {
                    let x = _mm256_set1_epi8(v as i8);
                    let (ya, yb) = if fold { (af, bf) } else { (a, b) };
                    let m = (_mm256_movemask_epi8(_mm256_cmpeq_epi8(ya, x)) as u32 as u64) | ((_mm256_movemask_epi8(_mm256_cmpeq_epi8(yb, x)) as u32 as u64) << 32);
                    let mut c = cl;
                    while c != 0 {
                        cm[(c.trailing_zeros() & 15) as usize] |= m;
                        c &= c - 1;
                    }
                }
            }
        }

        /// SSE2, in every x86-64: 4 comparisons of 16 bytes per byte.
        #[cfg(all(target_arch = "x86_64", not(target_feature = "avx2")))]
        #[inline(always)]
        fn eqs(sm: &Simple, p: *const u8, cm: &mut [u64; CLASSES]) {
            use core::arch::x86_64::*;
            // SAFETY: `p` has 64 readable bytes (the text or `tail`)
            unsafe {
                let v0 = _mm_loadu_si128(p as *const __m128i);
                let v1 = _mm_loadu_si128(p.add(16) as *const __m128i);
                let v2 = _mm_loadu_si128(p.add(32) as *const __m128i);
                let v3 = _mm_loadu_si128(p.add(48) as *const __m128i);
                let low = _mm_set1_epi8(0x20);
                let f = [_mm_or_si128(v0, low), _mm_or_si128(v1, low), _mm_or_si128(v2, low), _mm_or_si128(v3, low)];
                let r = [v0, v1, v2, v3];
                for &(v, cl, fold) in &sm.bytes {
                    let x = _mm_set1_epi8(v as i8);
                    let y = if fold { &f } else { &r };
                    let m = (_mm_movemask_epi8(_mm_cmpeq_epi8(y[0], x)) as u16 as u64)
                        | ((_mm_movemask_epi8(_mm_cmpeq_epi8(y[1], x)) as u16 as u64) << 16)
                        | ((_mm_movemask_epi8(_mm_cmpeq_epi8(y[2], x)) as u16 as u64) << 32)
                        | ((_mm_movemask_epi8(_mm_cmpeq_epi8(y[3], x)) as u16 as u64) << 48);
                    let mut c = cl;
                    while c != 0 {
                        cm[(c.trailing_zeros() & 15) as usize] |= m;
                        c &= c - 1;
                    }
                }
            }
        }

        /// Without SIMD (not used: `of` finds no shape there, the automaton
        /// is faster than this).
        #[cfg(not(target_arch = "x86_64"))]
        #[inline(always)]
        fn eqs(sm: &Simple, p: *const u8, cm: &mut [u64; CLASSES]) {
            // SAFETY: `p` has 64 readable bytes (the text or `tail`)
            let blk = unsafe { core::slice::from_raw_parts(p, 64) };
            for &(v, cl, fold) in &sm.bytes {
                let mut m = 0u64;
                for (i, &x) in blk.iter().enumerate() {
                    let y = if fold { x | 0x20 } else { x };
                    m |= ((y == v) as u64) << i;
                }
                let mut c = cl;
                while c != 0 {
                    cm[c.trailing_zeros() as usize] |= m;
                    c &= c - 1;
                }
            }
        }

        /// The places in the 64 bytes of `cur` where the sequence `seq`
        /// starts (its bytes past them in `next`).
        #[inline(always)]
        fn starts(seq: &[u8], cur: &[u64; CLASSES], next: &[u64; CLASSES]) -> u64 {
            // (the indices of classes are below CLASSES by construction: the
            // masks drop the checks of bounds, D238)
            // the first 6 places without a branch, each shift a constant
            // (D275): leaving at the first empty mask is a branch the text
            // decides, mispredicted about once a block; after 6 places of
            // classes of a few bytes nearly no candidate is left, and the
            // test that goes on is predicted
            let at = |j: usize| -> u64 {
                let k = (seq[j] & 15) as usize;
                (cur[k] >> j) | (next[k] << (64 - j))
            };
            let l = seq.len();
            let mut m = cur[(seq[0] & 15) as usize];
            if l > 1 {
                m &= at(1);
            }
            if l > 2 {
                m &= at(2);
            }
            if l > 3 {
                m &= at(3);
            }
            if l > 4 {
                m &= at(4);
            }
            if l > 5 {
                m &= at(5);
            }
            for (j, &k) in seq.iter().enumerate().skip(6) {
                if m == 0 {
                    break;
                }
                let k = (k & 15) as usize;
                m &= (cur[k] >> (j & 63)) | (next[k] << ((64 - j) & 63));
            }
            m
        }

        impl Simple {
            /// The matches of the global search from `from` (a boundary of
            /// characters) up to the first one that starts at `until` or
            /// after, that one included (as `bounds_from`); only up to
            /// `until` when no match crosses it (D309: from there the search
            /// of the next part finds the same ones).
            pub fn bounds_from(&self, s: &str, from: usize, until: usize) -> Vec<(usize, usize)> {
                if let Some(fx) = &self.fixed {
                    #[cfg(all(target_arch = "x86_64", target_feature = "avx512bw"))]
                    if let (true, Some(t)) = (fx.lanes, &fx.table) {
                        // SAFETY: `lanes` is set only where `vbmi()` and
                        // `gfni()` saw the instructions
                        return unsafe { bounds_groups(t, self, s, from, until) };
                    }
                    return match fx.nb {
                        1 => fx.bounds::<1>(self, s, from, until),
                        2 => fx.bounds::<2>(self, s, from, until),
                        3 => fx.bounds::<3>(self, s, from, until),
                        4 => fx.bounds::<4>(self, s, from, until),
                        5 => fx.bounds::<5>(self, s, from, until),
                        6 => fx.bounds::<6>(self, s, from, until),
                        7 => fx.bounds::<7>(self, s, from, until),
                        _ => fx.bounds::<8>(self, s, from, until),
                    };
                }
                let b = s.as_bytes();
                let n = b.len();
                let na = self.alts.len();
                let mut out = Vec::new();
                let mut base = from;
                let mut blocks = [[0u64; CLASSES]; 2];
                let mut cur = 0usize;
                masks(self, b, base, &mut blocks[cur]);
                // the start of a line before the block: the place 0, or a
                // newline just before
                let line_before = |at: usize| at == 0 || b[at - 1] == b'\n';
                let mut carry = line_before(base) as u64;
                // no match starts before `allowed` (the end of the last one);
                // a delimited alternative without its stop byte left is dead
                let mut allowed = from;
                // the start of the next part, where the search may stop (D309);
                // none for `find_at` (`until` at `from`: the first match from there)
                let stop_at = if until > from { until } else { usize::MAX };
                let mut dead = 0u64;
                let mut st = [0u64; ALTS];
                while base < n {
                    let nx = 1 - cur;
                    masks(self, b, base + 64, &mut blocks[nx]);
                    let (c, x) = (&blocks[cur], &blocks[nx]);
                    let lines = match self.nl {
                        Some(k) => (c[k] << 1) | carry,
                        None => 0,
                    };
                    let mut any = 0u64;
                    for (a, alt) in self.alts.iter().enumerate() {
                        let mut m = if dead >> a & 1 == 1 { 0 } else { starts(&alt.seq, c, x) };
                        if alt.line {
                            m &= lines;
                        }
                        st[a & (ALTS - 1)] = m;
                        any |= m;
                    }
                    while any != 0 {
                        let j = any.trailing_zeros();
                        any &= any - 1;
                        let p = base + j as usize;
                        if p < allowed {
                            continue;
                        }
                        // at the next part, no match across its start: it finds the
                        // same matches from there (D309)
                        if p >= stop_at && allowed <= stop_at {
                            return out;
                        }
                        // the first alternative that matches at p
                        let mut found = None;
                        for (a, alt) in self.alts.iter().enumerate().take(na) {
                            if st[a & (ALTS - 1)] >> j & 1 == 0 || dead >> a & 1 == 1 {
                                continue;
                            }
                            let h = p + alt.seq.len();
                            match alt.stop {
                                None => {
                                    found = Some(h);
                                    break;
                                }
                                // the stop in the masks of this block and the
                                // next (`h` is before the end of the next), a
                                // search of the text after them only when it is
                                // not there (D282)
                                Some((stop, mid)) => match {
                                    let k = alt.stop_class as usize & (CLASSES - 1);
                                    let off = h - base;
                                    let w = (((x[k] as u128) << 64) | c[k] as u128) >> off;
                                    if w != 0 {
                                        Some(w.trailing_zeros() as usize)
                                    } else if base + 128 < n {
                                        memchr::memchr(stop, &b[base + 128..]).map(|q| base + 128 + q - h)
                                    } else {
                                        None
                                    }
                                } {
                                    None => dead |= 1 << a,
                                    Some(q) if q < mid => {}
                                    Some(q) => {
                                        found = Some(h + q + 1);
                                        break;
                                    }
                                },
                            }
                        }
                        let Some(e) = found else { continue };
                        push_match(&mut out, (p, e), from, base + 64, until.min(n));
                        allowed = e;
                        if p >= until {
                            return out;
                        }
                    }
                    if dead.count_ones() as usize == na {
                        return out;
                    }
                    carry = self.nl.map(|k| blocks[cur][k] >> 63).unwrap_or(0);
                    base += 64;
                    // past the start of the next part, no match across it: the
                    // next part finds the same matches from there (D309: the
                    // search went on to the first match after it, to the end of
                    // the text when there was none)
                    if base >= stop_at && allowed <= stop_at {
                        return out;
                    }
                    cur = nx;
                    // inside a long match: on from its end
                    if allowed > base + 64 {
                        base = allowed;
                        masks(self, b, base, &mut blocks[cur]);
                        carry = line_before(base) as u64;
                    }
                }
                out
            }

            /// The first match of a search from `at`.
            pub fn find_at(&self, s: &str, at: usize) -> Option<(usize, usize)> {
                self.bounds_from(s, at, at).first().copied()
            }

            /// The stop of each alternative (for the tests).
            #[cfg(test)]
            fn stops(&self) -> Vec<Option<(u8, usize)>> {
                self.alts.iter().map(|a| a.stop).collect()
            }
        }

        #[cfg(test)]
        mod tests {
            use super::*;

            /// Which patterns have a shape, and which do not (the automaton).
            #[test]
            fn shapes_of_patterns() {
                for v in ["ig/agggtaaa|tttaccct", "ig/[cgt]gggtaaa|tttaccc[acg]", "g/tHa[Nt]", "g/aND|caN|Ha[DS]|WaS", "g/a[NSt]|BY", "g/a{3}", "g/(ab|ba)x"] {
                    let sm = of(v).unwrap_or_else(|| panic!("{} has no shape", v));
                    assert!(sm.stops().iter().all(|s| s.is_none()), "{}", v);
                }
                for (v, c, mid) in [("g/<[^>]*>", b'>', 0), ("g/\\|[^|][^|]*\\|", b'|', 1), ("g/<[^>]+>", b'>', 1), ("g/x[^y]*y", b'y', 0)] {
                    let sm = of(v).unwrap_or_else(|| panic!("{} has no shape", v));
                    assert_eq!(sm.stops(), alloc::vec![Some((c, mid))], "{}", v);
                }
                // classes beyond ASCII ignoring case (the Kelvin sign, the
                // long s), lengths that differ, repetitions, anchors, empty
                // alternatives of other lengths, kinds, at the start of a line (D237)
                for v in ["g/ab|c", "g/a|ab", "mg/^>.*\\n|\\n", "g/x[^y]*y|q", "mg/^ab"] {
                    assert!(of(v).is_some(), "{} has no shape", v);
                }
                for v in ["ig/k", "ig/ks", "g/a*", "g/^ab", "g/ab$", "g/é", "g/", "g/[^a]", "g/.b"] {
                    assert!(of(v).is_none(), "{} has a shape", v);
                }
            }

            /// D282: the stop of a delimited alternative found in the masks of
            /// the block and the next one, else searched after them: stops at
            /// every distance from 0 to 300 bytes of their heads (in the
            /// block, in the next, beyond), too close for `mid`, missing at
            /// the end; the matches of the automaton from any start.
            #[test]
            fn stops_found_in_the_masks_of_the_blocks() {
                let mut text = alloc::string::String::new();
                for d in 0..300usize {
                    text.push('<');
                    text.push_str(&"a|".repeat(d / 2));
                    text.push_str(if d % 2 == 0 { "" } else { "|" });
                    text.push('>');
                    text.push_str(&"x".repeat(d % 7));
                }
                text.push_str("<open|never closed");
                for v in ["g/<[^>]*>", "g/\\|[^|][^|]*\\|", "g/<[^>]+>", "g/<[^>]*>|a"] {
                    let sm = of(v).unwrap();
                    let re = super::super::of(v);
                    for from in [0usize, 1, 63, 64, 100, 5000] {
                        assert_eq!(sm.bounds_from(&text, from, usize::MAX), super::super::bounds_from(&re, &text, from, usize::MAX), "{} from {}", v, from);
                    }
                }
            }

            /// D276: the matches of a search split into parts, merged, are
            /// those of one search: parts cut anywhere (inside matches, at
            /// their ends), patterns of many matches, of overlapping
            /// candidates (`aa` in `aaa`), of empty ones, delimited.
            #[test]
            fn merged_parts_are_one_search() {
                let mut h = 0x2545f4914f6cdd1du64;
                let text: alloc::string::String = (0..30000)
                    .map(|_| {
                        h ^= h << 13;
                        h ^= h >> 7;
                        h ^= h << 17;
                        b"aaacgt<x>|tHaNaND"[(h % 17) as usize] as char
                    })
                    .collect();
                for v in ["g/aa", "g/a[NSt]|BY|aa", "g/<[^>]*>", "g/\\|[^|][^|]*\\|", "g/tHa[Nt]", "g/x*"] {
                    let re = super::super::of(v);
                    let whole = super::super::bounds_from(&re, &text, 0, usize::MAX);
                    for k in [2usize, 3, 7, 64] {
                        let mut starts: Vec<usize> = (0..k).map(|i| text.len() * i / k).collect();
                        starts.dedup();
                        // a cut inside a match, at its end
                        if let Some(&(a, b)) = whole.get(whole.len() / 2) {
                            starts.push(a + (b - a) / 2);
                            starts.push(b);
                        }
                        starts.sort();
                        starts.dedup();
                        let parts: Vec<Vec<(usize, usize)>> = (0..starts.len())
                            .map(|i| super::super::bounds_from(&re, &text, starts[i], starts.get(i + 1).copied().unwrap_or(usize::MAX)))
                            .collect();
                        assert_eq!(super::super::merge_bounds(&re, &text, &starts, &parts), whole, "{} in {} parts", v, starts.len());
                    }
                }
            }

            /// The matches a search by masks from 0 up to `until` gives
            /// (D309), from those of a whole search: those that start before
            /// `until`, and the next one when the last of them crosses
            /// `until`; up to 0, the first one.
            fn up_to(whole: Vec<(usize, usize)>, until: usize) -> Vec<(usize, usize)> {
                // from 0 up to 0: the first match (`find_at`)
                if until == 0 {
                    return whole[..whole.len().min(1)].to_vec();
                }
                let k = whole.partition_point(|&(a, _)| a < until);
                let k = if k > 0 && k < whole.len() && whole[k - 1].1 > until { k + 1 } else { k };
                whole[..k].to_vec()
            }

            /// D309: a search by masks split into parts, each stopped at the
            /// next part when no match crosses it, merged, is one search:
            /// parts cut anywhere (inside matches, at their ends), parts
            /// without matches after the first ones (their searches stop at
            /// the next part), fixed and delimited patterns, by lanes and by
            /// masks.
            #[test]
            fn parts_stopped_at_the_next_are_one_search() {
                let mut h = 0x9e37_79b9_7f4a_7c15u64;
                let mut rnd = |k: u64| {
                    h ^= h << 13;
                    h ^= h >> 7;
                    h ^= h << 17;
                    h % k
                };
                // matches in the first half only, then none
                let head: alloc::string::String = (0..20000).map(|_| b"aaacgt<x>|tHaNaND"[rnd(17) as usize] as char).collect();
                let tail: alloc::string::String = (0..20000).map(|_| b"cgcgtc"[rnd(6) as usize] as char).collect();
                for text in [head.clone() + &tail, tail.clone() + &head, head.clone()] {
                    for v in ["g/aa", "g/a[NSt]|BY|aa", "g/<[^>]*>", "g/\\|[^|][^|]*\\|", "g/tHa[Nt]", "ig/acg|tHa"] {
                        let sm = of(v).unwrap();
                        let whole = sm.bounds_from(&text, 0, usize::MAX);
                        assert_eq!(whole, super::super::bounds_from(&super::super::of(v), &text, 0, usize::MAX), "{}", v);
                        for k in [2usize, 3, 4, 7, 64] {
                            let mut starts: alloc::vec::Vec<usize> = (0..k).map(|i| text.len() * i / k).collect();
                            if let Some(&(a, b)) = whole.get(whole.len() / 3) {
                                starts.push(a + (b - a) / 2);
                                starts.push(b);
                            }
                            starts.sort();
                            starts.dedup();
                            let parts: alloc::vec::Vec<alloc::vec::Vec<(usize, usize)>> = (0..starts.len())
                                .map(|i| sm.bounds_from(&text, starts[i], starts.get(i + 1).copied().unwrap_or(usize::MAX)))
                                .collect();
                            let merged = super::super::merge_bounds_with(&text, &starts, &parts, |at| sm.find_at(&text, at));
                            assert_eq!(merged, whole, "{} in {} parts", v, starts.len());
                        }
                    }
                }
            }

            /// D275: the fixed alternatives over few bytes (the plan without
            /// a branch per place) find the matches of the automaton: the
            /// patterns of regex-redux and others, over a text of random
            /// bases with upper cases and other bytes, matches across the
            /// blocks, from any start, up to any `until`.
            #[test]
            fn fixed_alternatives_find_the_matches_of_the_automaton() {
                let mut h = 0x9e3779b97f4a7c15u64;
                let text: alloc::string::String = (0..20000)
                    .map(|_| {
                        h ^= h << 13;
                        h ^= h >> 7;
                        h ^= h << 17;
                        b"acgtacgtacgtACGTNBDHKMRSVWYx\n<>|"[(h % 32) as usize] as char
                    })
                    .collect();
                let text = text + "agggtaaatttaccctagggtaaa";
                let pats = [
                    "ig/agggtaaa|tttaccct", "ig/[cgt]gggtaaa|tttaccc[acg]", "ig/a[act]ggtaaa|tttacc[agt]t", "ig/ag[act]gtaaa|tttac[agt]ct",
                    "ig/agg[act]taaa|ttta[agt]cct", "ig/aggg[acg]aaa|ttt[cgt]ccct", "ig/agggt[cgt]aa|tt[acg]accct", "ig/agggta[cgt]a|t[acg]taccct",
                    "ig/agggtaa[cgt]|[acg]ttaccct", "g/tHa[Nt]", "g/aND|caN|Ha[DS]|WaS", "g/a[NSt]|BY", "g/ac|a", "g/a|ac", "ig/acgtacgtacgtacgtacgtacgtacgtacgtacgtacgtacgtacgtacgtacgtacgtacgt",
                ];
                for v in pats {
                    let sm = of(v).unwrap();
                    assert!(sm.fixed.is_some(), "{} has no fixed plan", v);
                    // the lookup of the classes where the processor has it (D278)
                    let fx = sm.fixed.as_ref().unwrap();
                    assert_eq!(fx.table.is_some(), vbmi() && fx.ncl <= 8, "{}", v);
                    // the search by lanes where it has GFNI (D306)
                    assert_eq!(fx.lanes, fx.table.is_some() && gfni() && sm.alts.iter().all(|a| a.seq.len() <= 16), "{}", v);
                    let re = super::super::of(v);
                    for from in [0usize, 1, 62, 63, 64, 65, 127, 4097] {
                        assert_eq!(sm.bounds_from(&text, from, usize::MAX), super::super::bounds_from(&re, &text, from, usize::MAX), "{} from {}", v, from);
                    }
                    for until in [0usize, 64, 1000, 19999] {
                        assert_eq!(sm.bounds_from(&text, 0, until), up_to(super::super::bounds_from(&re, &text, 0, usize::MAX), until), "{} until {}", v, until);
                    }
                }
                // a stop byte or a start of line: the general search
                for v in ["g/<[^>]*>", "mg/^>.*\\n|\\n", "mg/^a|t"] {
                    assert!(of(v).unwrap().fixed.is_none(), "{}", v);
                }
            }

            /// D306: the search by lanes finds the matches of the search by
            /// masks: alternatives of 1 to 16 places, of classes, with upper
            /// cases, texts of every length around the blocks with bytes
            /// outside ASCII, from any start, up to any `until`.
            #[test]
            fn lanes_find_the_matches_of_the_masks() {
                let mut h = 0x2545_f491_4f6c_dd1du64;
                let mut rnd = |k: u64| {
                    h ^= h << 13;
                    h ^= h >> 7;
                    h ^= h << 17;
                    h % k
                };
                let pats = [
                    "g/a", "g/ac|ca", "ig/a[cg]t|tt", "ig/agggtaaa|tttaccct", "g/[ac][gt][ac][gt][ac]|gggggggg", "ig/acgtacgtacgtacgt|tgcatgca",
                    "ig/acgtacgtacgtacg|c[at]g", "g/aaaaaaaaa|aaaa", "g/a|aa|aaa", "ig/tHa[Nt]", "g/aND|caN|Ha[DS]|WaS", "g/a[NSt]|BY",
                ];
                for len in [0usize, 1, 15, 63, 64, 65, 127, 128, 129, 200, 1000, 4099] {
                    let text: alloc::string::String =
                        (0..len).map(|_| ['a', 'c', 'g', 't', 'A', 'C', 'G', 'T', 'é', 'N', 'H', 'D', 'S', 'W', 'B', 'Y'][rnd(16) as usize]).collect();
                    for v in pats {
                        let sm = of(v).unwrap();
                        let fx = sm.fixed.as_ref().unwrap();
                        let general = |from: usize, until: usize| match fx.nb {
                            1 => fx.bounds::<1>(&sm, &text, from, until),
                            2 => fx.bounds::<2>(&sm, &text, from, until),
                            3 => fx.bounds::<3>(&sm, &text, from, until),
                            4 => fx.bounds::<4>(&sm, &text, from, until),
                            5 => fx.bounds::<5>(&sm, &text, from, until),
                            6 => fx.bounds::<6>(&sm, &text, from, until),
                            7 => fx.bounds::<7>(&sm, &text, from, until),
                            _ => fx.bounds::<8>(&sm, &text, from, until),
                        };
                        let starts: alloc::vec::Vec<usize> = [0usize, 1, 63, 64, 65, 130].into_iter().filter(|&f| f <= text.len() && text.is_char_boundary(f)).collect();
                        for &from in &starts {
                            for until in [usize::MAX, from + 1, from + 64, from + 500] {
                                assert_eq!(sm.bounds_from(&text, from, until), general(from, until), "{} over {} bytes from {} until {}", v, len, from, until);
                            }
                        }
                    }
                }
            }

            /// The matches of the masks are those of the automaton.
            #[test]
            fn masks_find_the_matches_of_the_automaton() {
                let text: alloc::string::String = ">head one\naaXbbaaaa-AAxagggtaaaTTTACCCT<2>at||a|bc|tHaNaND<x>caNHaSWaSBY|q<<k>tail <open|\nab\n>x\n".repeat(37);
                for v in ["ig/agggtaaa|tttaccct", "g/aa", "g/a{3}", "g/a[NSt]|BY", "g/<[^>]*>", "g/\\|[^|][^|]*\\|", "g/<[^>]+>", "ig/tHa[Nt]", "g/ab|c", "g/a|ab", "g/aB|a", "mg/^>.*\\n|\\n", "mg/^a|t", "g/<[^>]*>|a", "g/x[^y]*y|q|<[^>]+>"] {
                    let sm = of(v).unwrap();
                    let re = super::super::of(v);
                    for from in [0usize, 1, 63, 64, 65, 500] {
                        assert_eq!(sm.bounds_from(&text, from, usize::MAX), super::super::bounds_from(&re, &text, from, usize::MAX), "{} from {}", v, from);
                    }
                    assert_eq!(sm.bounds_from(&text, 0, 700), up_to(super::super::bounds_from(&re, &text, 0, usize::MAX), 700), "{} until 700", v);
                }
            }
        }
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

/// The groups of integer keys (as their 64 bits), each key kept in its slot
/// of the table with its last index and its count (D243): a probe and its
/// update touch one line, where `group` reads the key of the first
/// occurrence in the array of keys (a random read of memory per key when
/// the array is large) and then two more arrays. With `direct`, the keys
/// below DIRECT_MAX are counted in a table indexed by the key, which grows
/// to the largest of them (D244: no hash, no probe). Indices below 2^32.
/// The keys of a range of indices are added in increasing order; the tables
/// of consecutive ranges merged in their order give the groups of the whole
/// range in the order of their first occurrence: the columns of `group`.
pub struct WordGroups {
    /// (key, last index, count) per slot, count 0 when empty
    slots: alloc::vec::Vec<(u64, u32, u32)>,
    /// (last index, count) of the key i below DIRECT_MAX (`direct`)
    small: alloc::vec::Vec<(u32, u32)>,
    direct: bool,
    /// per group in the order of its first occurrence: its first index and
    /// its key
    first: alloc::vec::Vec<u32>,
    keys: alloc::vec::Vec<u64>,
    /// the groups in `slots`
    hashed: usize,
    /// the bits of an index (`new_for`), 0: never scattered
    index_bits: u32,
    /// after more than SCATTER_AFTER groups in `slots` (D253): the keys
    /// that fit with their index in 64 bits scattered (`Scatter`)
    scatter: Option<alloc::boxed::Box<Scatter>>,
    /// the keys of a chunk to hash, with their hashes and indices
    /// the entries of a round of scatter (`new_for`)
    round: usize,
    work: alloc::vec::Vec<(u64, u64, u32)>,
}

/// The keys a `WordGroups` scatters (D253): lists by the high bits of their
/// hash, of `key << bits | index` in the order of the indices, in pieces
/// reused from a round to the next; at the end of each round of entries
/// (`SCATTER_TOTAL`), each list counted at once in the small table of its
/// list (a few thousand entries into a table that stays in the L1 cache),
/// the pieces then free for the next round.
pub struct Scatter {
    pieces: alloc::vec::Vec<alloc::vec::Vec<alloc::vec::Vec<u64>>>,
    /// per list: the next entry of its last piece and its end (null: none)
    cursor: [(*mut u64, *mut u64); SCATTER_BUCKETS],
    spare: alloc::vec::Vec<alloc::vec::Vec<u64>>,
    entries: usize,
    /// the entries of a round (`SCATTER_TOTAL`)
    round: usize,
    bits: u32,
    /// the groups of each list
    tables: alloc::vec::Vec<ListTable>,
}

#[derive(Clone, Copy, Default)]
struct ListSlot {
    key: u64,
    first: u32,
    last: u32,
    count: u32,
}

/// The groups of one list of scatter of a part (D253): a small table of
/// slots holding the key, its first and last indices and its count; the
/// keys of a list share the high bits of their hash, its slots take the low
/// ones. Indices added in increasing order.
pub struct ListTable {
    slots: alloc::vec::Vec<ListSlot>,
    used: usize,
}

impl Default for ListTable {
    fn default() -> Self {
        Self::new()
    }
}

impl ListTable {
    pub fn new() -> ListTable {
        ListTable { slots: alloc::vec![ListSlot::default(); 64], used: 0 }
    }

    /// The key `k` (hash `h`): `count` more, first at `first` if new, last
    /// at `last`.
    #[inline(always)]
    pub fn add(&mut self, k: u64, h: u64, first: u32, last: u32, count: u32) {
        let mask = self.slots.len() - 1;
        let mut s = (h as usize) & mask;
        loop {
            // SAFETY: `s` is masked by the length less one (a power of two)
            let e = unsafe { self.slots.get_unchecked_mut(s) };
            if e.count == 0 {
                *e = ListSlot { key: k, first, last, count };
                self.used += 1;
                if self.used * 2 > self.slots.len() {
                    self.grow();
                }
                return;
            }
            if e.key == k {
                e.last = last;
                e.count += count;
                return;
            }
            s = (s + 1) & mask;
        }
    }

    #[cold]
    fn grow(&mut self) {
        let old = core::mem::take(&mut self.slots);
        self.slots = alloc::vec![ListSlot::default(); old.len() * 2];
        self.used = 0;
        for e in old.into_iter().filter(|e| e.count > 0) {
            self.add(e.key, hash_word(e.key), e.first, e.last, e.count);
        }
    }

    /// Its groups: (first, key, last, count), in no order.
    pub fn groups(&self) -> impl Iterator<Item = (u32, u64, u32, u32)> + '_ {
        self.slots.iter().filter(|e| e.count > 0).map(|e| (e.first, e.key, e.last, e.count))
    }
}

/// The entries of the rounds of scatter of all the parts of a grouping
/// together (128 MiB): each part's round `SCATTER_TOTAL / parts` entries,
/// between SCATTER_ROUND_MIN and SCATTER_ROUND_MAX (D295: k-nucleotide on 4
/// parts, the pass of the 12-mers 523.0 ms by rounds of 2^20 entries, 495.2
/// ms by 2^22, 521.0 ms by 2^23; each round reads again the table of every
/// list of the part, and the new pages of larger rounds cost more than that).
pub const SCATTER_TOTAL: usize = 1 << 24;
const SCATTER_ROUND_MIN: usize = 1 << 16;
const SCATTER_ROUND_MAX: usize = 1 << 22;

impl Scatter {
    fn new(bits: u32, round: usize) -> Scatter {
        Scatter {
            pieces: (0..SCATTER_BUCKETS).map(|_| alloc::vec::Vec::new()).collect(),
            cursor: [(core::ptr::null_mut(), core::ptr::null_mut()); SCATTER_BUCKETS],
            spare: alloc::vec::Vec::new(),
            entries: 0,
            round,
            bits,
            tables: (0..SCATTER_BUCKETS).map(|_| ListTable::new()).collect(),
        }
    }

    #[inline(always)]
    fn push(&mut self, b: usize, e: u64) {
        let (cur, end) = self.cursor[b];
        if cur == end {
            self.new_piece(b);
        }
        // SAFETY: the cursor is inside the last piece of the list `b`
        unsafe {
            let c = self.cursor[b].0;
            c.write(e);
            self.cursor[b].0 = c.add(1);
        }
        self.entries += 1;
        if self.entries == self.round {
            self.count_round();
        }
    }

    #[cold]
    fn new_piece(&mut self, b: usize) {
        if let Some(last) = self.pieces[b].last_mut() {
            // SAFETY: written to its capacity (its cursor reached its end)
            unsafe { last.set_len(last.capacity()) };
        }
        let mut piece = self.spare.pop().unwrap_or_else(|| alloc::vec::Vec::with_capacity(SCATTER_PIECE));
        piece.clear();
        let p = piece.as_mut_ptr();
        // SAFETY: the piece has room for its capacity
        self.cursor[b] = (p, unsafe { p.add(piece.capacity()) });
        self.pieces[b].push(piece);
    }

    /// The entries of the round counted, list by list; the pieces spare.
    fn count_round(&mut self) {
        let bits = self.bits;
        let mask = (1u64 << bits) - 1;
        for b in 0..SCATTER_BUCKETS {
            let n = self.pieces[b].len();
            if n == 0 {
                continue;
            }
            let (cur, _) = self.cursor[b];
            let t = &mut self.tables[b];
            for (k, mut piece) in core::mem::take(&mut self.pieces[b]).into_iter().enumerate() {
                // SAFETY: the full pieces are written to their capacity, the
                // last one up to the cursor
                let used = if k + 1 < n { piece.capacity() } else { unsafe { cur.offset_from(piece.as_ptr()) as usize } };
                unsafe { piece.set_len(used) };
                for &e in piece.iter() {
                    let k = e >> bits;
                    let i = (e & mask) as u32;
                    t.add(k, hash_word(k), i, i, 1);
                }
                self.spare.push(piece);
            }
            self.cursor[b] = (core::ptr::null_mut(), core::ptr::null_mut());
        }
        self.entries = 0;
    }
}

/// More groups in its table than this: a `WordGroups` scatters its keys
/// (D253; a table of 2^15 keys and more is beyond the L2 cache).
pub const SCATTER_AFTER: usize = 1 << 15;
/// The lists of scattered keys, by the high bits of their hash.
pub const SCATTER_BUCKETS: usize = 256;
/// The entries of a piece of a list of scattered keys.
const SCATTER_PIECE: usize = 1024;

/// The list of the scattered key `k` (its hash `h`): the high bits of the
/// hash, the slots of a table taking the low ones.
#[inline(always)]
pub fn scatter_bucket(h: u64) -> usize {
    (h >> 56) as usize
}

impl Default for WordGroups {
    fn default() -> Self {
        Self::new(false)
    }
}

impl WordGroups {
    /// The indices below this bound only (and counts).
    pub const MAX_INDEX: usize = u32::MAX as usize;
    /// The keys counted by their value (`direct`).
    pub const DIRECT_MAX: u64 = 1 << 16;

    /// `direct`: the small keys in a table of their values (for many keys:
    /// that table may take 512 KiB).
    pub fn new(direct: bool) -> WordGroups {
        WordGroups {
            slots: alloc::vec![(0, 0, 0); 1024],
            small: alloc::vec::Vec::new(),
            direct,
            first: alloc::vec::Vec::new(),
            keys: alloc::vec::Vec::new(),
            hashed: 0,
            index_bits: 0,
            scatter: None,
            work: alloc::vec::Vec::new(),
            round: 1 << 20,
        }
    }

    /// `new` for the indices below `n`, one of the tables of `parts` parts;
    /// with `scatter`, its keys may be scattered (D253) once its table is
    /// large (for several tables at once: one table alone stays in the L3
    /// cache), by rounds of `SCATTER_TOTAL / parts` entries.
    pub fn new_for(direct: bool, n: usize, scatter: bool, parts: usize) -> WordGroups {
        let mut g = Self::new(direct);
        let bits = usize::BITS - n.max(1).leading_zeros();
        g.index_bits = if bits <= 32 && scatter { bits } else { 0 };
        g.round = (SCATTER_TOTAL / parts.max(1)).clamp(SCATTER_ROUND_MIN, SCATTER_ROUND_MAX);
        g
    }

    /// Its keys are scattered (some of them).
    pub fn scattered(&self) -> bool {
        self.scatter.is_some()
    }

    /// The groups of its scattered keys, by list; empty when not scattered.
    /// The last round counted.
    pub fn scattered_tables(&mut self) -> &[ListTable] {
        match &mut self.scatter {
            Some(sc) => {
                sc.count_round();
                &sc.tables
            }
            None => &[],
        }
    }

    /// The key `k` counted by its value (not hashed).
    pub fn is_small_key(&self, k: u64) -> bool {
        self.is_small(k)
    }

    /// The group of the key `k`, first at `first`, last at `last`, `count`
    /// times, after every index added before.
    pub fn add_aggregate(&mut self, k: u64, first: u32, last: u32, count: u32) {
        self.add_count(k, first, last, count)
    }

    /// The number of groups.
    pub fn len(&self) -> usize {
        self.first.len()
    }

    pub fn is_empty(&self) -> bool {
        self.first.is_empty()
    }

    /// The group `g` (in the order of its first occurrence): its first
    /// index, its key, its last index and its count.
    pub fn group(&self, g: usize) -> (u32, u64, u32, u32) {
        let k = self.keys[g];
        let (l, c) = self.entry(k);
        (self.first[g], k, l, c)
    }

    #[inline(always)]
    fn slot(slots: &[(u64, u32, u32)], k: u64) -> usize {
        let mask = slots.len() - 1;
        let mut s = (hash_word(k) as usize) & mask;
        loop {
            let (key, _, c) = slots[s];
            if c == 0 || key == k {
                return s;
            }
            s = (s + 1) & mask;
        }
    }

    #[inline(always)]
    fn is_small(&self, k: u64) -> bool {
        self.direct && k < Self::DIRECT_MAX
    }

    /// The key `k` at the index `i` (after every index added before), `c`
    /// times, its last index `l`.
    #[inline(always)]
    fn add_count(&mut self, k: u64, i: u32, l: u32, c: u32) {
        if self.is_small(k) {
            let j = k as usize;
            if j >= self.small.len() {
                self.grow_small(j);
            }
            let e = &mut self.small[j];
            if e.1 == 0 {
                self.first.push(i);
                self.keys.push(k);
            }
            *e = (l, e.1 + c);
            return;
        }
        let s = Self::slot(&self.slots, k);
        let e = &mut self.slots[s];
        if e.2 != 0 {
            e.1 = l;
            e.2 += c;
            return;
        }
        *e = (k, l, c);
        self.first.push(i);
        self.keys.push(k);
        self.hashed += 1;
        if self.hashed * 2 > self.slots.len() {
            self.grow_slots();
        }
    }

    #[cold]
    fn grow_small(&mut self, j: usize) {
        let n = (j + 1).next_power_of_two().max(256);
        self.small.resize(n, (0, 0));
    }

    #[cold]
    fn grow_slots(&mut self) {
        let mut slots = alloc::vec![(0u64, 0u32, 0u32); self.slots.len() * 2];
        for &k in &self.keys {
            if !self.is_small(k) {
                let old = self.slots[Self::slot(&self.slots, k)];
                let t = Self::slot(&slots, k);
                slots[t] = old;
            }
        }
        self.slots = slots;
    }

    /// The key `k` at the index `i` (below MAX_INDEX).
    #[inline(always)]
    pub fn add(&mut self, k: u64, i: usize) {
        self.add_count(k, i as u32, i as u32, 1)
    }

    /// The keys `keys` at the indices `start ..` (after every index added
    /// before), in order (D245): by chunks of 1024, the hashes of a chunk
    /// computed first (once per key), then each key added while the slot of
    /// the key AHEAD places later is prefetched; the table, its mask and the
    /// table of small keys held in registers between two growths.
    pub fn add_block(&mut self, keys: &[u64], start: usize) {
        const CHUNK: usize = 1024;
        const AHEAD: usize = 16;
        // the keys of a chunk to hash, their hashes and indices: allocated
        // once (zeroing them at each call cost 24 bytes a key)
        let mut work = core::mem::take(&mut self.work);
        if work.is_empty() {
            work = alloc::vec![(0u64, 0u64, 0u32); CHUNK];
        }
        let direct = self.direct;
        for (c, chunk) in keys.chunks(CHUNK).enumerate() {
            let base = (start + c * CHUNK) as u32;
            // every key of the chunk small (the keys of fewer than 9 bases):
            // counted by their value in one loop, the table grown first to
            // the largest of them (D296: the test of each key and of the
            // length of the table were half of the instructions of a key)
            let top = chunk.iter().copied().max().unwrap_or(0);
            if direct && top < Self::DIRECT_MAX {
                if top as usize >= self.small.len() {
                    self.grow_small(top as usize);
                }
                let small = &mut self.small[..];
                for (j, &k) in chunk.iter().enumerate() {
                    let i = base + j as u32;
                    let e = &mut small[k as usize];
                    if e.1 == 0 {
                        self.first.push(i);
                        self.keys.push(k);
                    }
                    *e = (i, e.1 + 1);
                }
                continue;
            }
            // the small keys counted here; the scattered ones put in their
            // list (one pass, D253); the others kept to be hashed
            let mut m = 0usize;
            if let Some(sc) = &mut self.scatter {
                let bits = self.index_bits;
                for (j, &k) in chunk.iter().enumerate() {
                    let i = base + j as u32;
                    if direct && k < Self::DIRECT_MAX {
                        Self::small_add(&mut self.small, &mut self.first, &mut self.keys, k, i);
                    } else {
                        let h = hash_word(k);
                        if k >> (64 - bits) == 0 {
                            sc.push(scatter_bucket(h), k << bits | i as u64);
                        } else {
                            work[m] = (k, h, i);
                            m += 1;
                        }
                    }
                }
            } else {
                for (j, &k) in chunk.iter().enumerate() {
                    let i = base + j as u32;
                    if direct && k < Self::DIRECT_MAX {
                        Self::small_add(&mut self.small, &mut self.first, &mut self.keys, k, i);
                    } else {
                        work[m] = (k, hash_word(k), i);
                        m += 1;
                    }
                }
            }
            if m == 0 {
                continue;
            }
            let mut sp = self.slots.as_mut_ptr();
            let mut mask = self.slots.len() - 1;
            for u in 0..m {
                if u + AHEAD < m {
                    let a = (work[u + AHEAD].1 as usize) & mask;
                    #[cfg(target_arch = "x86_64")]
                    // SAFETY: a prefetch reads nothing; `a` is a slot
                    unsafe {
                        core::arch::x86_64::_mm_prefetch(sp.add(a) as *const i8, core::arch::x86_64::_MM_HINT_T0)
                    };
                    #[cfg(not(target_arch = "x86_64"))]
                    let _ = a;
                }
                let (k, h, i) = work[u];
                let mut s = (h as usize) & mask;
                loop {
                    // SAFETY: `s` is masked by the length of the table less
                    // one (a power of two), `sp` its start
                    let e = unsafe { &mut *sp.add(s) };
                    if e.2 == 0 {
                        *e = (k, i, 1);
                        self.first.push(i);
                        self.keys.push(k);
                        self.hashed += 1;
                        if self.hashed * 2 > self.slots.len() {
                            self.grow_slots();
                            sp = self.slots.as_mut_ptr();
                            mask = self.slots.len() - 1;
                        }
                        // a large table: the next keys scattered (D253)
                        if self.hashed > SCATTER_AFTER && self.scatter.is_none() && self.index_bits > 0 {
                            self.scatter = Some(alloc::boxed::Box::new(Scatter::new(self.index_bits, self.round)));
                        }
                        break;
                    }
                    if e.0 == k {
                        e.1 = i;
                        e.2 += 1;
                        break;
                    }
                    s = (s + 1) & mask;
                }
            }
        }
        self.work = work;
    }

    /// The small key `k` at the index `i` (`direct`, below DIRECT_MAX), on
    /// the fields (the others borrowed apart).
    #[inline(always)]
    fn small_add(small: &mut alloc::vec::Vec<(u32, u32)>, first: &mut alloc::vec::Vec<u32>, keys: &mut alloc::vec::Vec<u64>, k: u64, i: u32) {
        let j = k as usize;
        if j >= small.len() {
            let n = (j + 1).next_power_of_two().max(256);
            small.resize(n, (0, 0));
        }
        // SAFETY: `j` is below the length of `small` (just grown if not)
        let e = unsafe { small.get_unchecked_mut(j) };
        if e.1 == 0 {
            first.push(i);
            keys.push(k);
        }
        *e = (i, e.1 + 1);
    }

    /// The line of the slot of `k` brought into the caches, for a key added
    /// a few keys later (the loads of several keys then overlap, where each
    /// probe waits for its line, D243).
    #[inline(always)]
    pub fn prefetch(&self, k: u64) {
        if self.is_small(k) {
            return;
        }
        let s = (hash_word(k) as usize) & (self.slots.len() - 1);
        #[cfg(target_arch = "x86_64")]
        // SAFETY: a prefetch reads nothing; `s` is a slot of the table
        unsafe {
            core::arch::x86_64::_mm_prefetch(self.slots.as_ptr().add(s) as *const i8, core::arch::x86_64::_MM_HINT_T0)
        };
        #[cfg(not(target_arch = "x86_64"))]
        let _ = s;
    }

    /// (last index, count) of the group of the key `k`, present.
    fn entry(&self, k: u64) -> (u32, u32) {
        if self.is_small(k) {
            self.small[k as usize]
        } else {
            let (_, l, c) = self.slots[Self::slot(&self.slots, k)];
            (l, c)
        }
    }

    /// The groups of `other`, of indices all after those of `self`.
    pub fn merge(&mut self, other: &WordGroups) {
        for g in 0..other.first.len() {
            let k = other.keys[g];
            let (l, c) = other.entry(k);
            self.add_count(k, other.first[g], l, c);
        }
    }

    /// The three columns of `group`.
    pub fn columns(self) -> alloc::vec::Vec<i64> {
        let m = self.first.len();
        // (first, last, count) per group, in the order of the first indices:
        // `add_block` makes the groups of its small keys before those of the
        // hashed keys of the same chunk (D253), so the order of making is not
        // always that of the first occurrences
        let mut groups: alloc::vec::Vec<(u32, u32, u32)> = self.first.iter().zip(&self.keys).map(|(&f, &k)| {
            let (l, c) = self.entry(k);
            (f, l, c)
        }).collect();
        if !groups.windows(2).all(|w| w[0].0 < w[1].0) {
            groups.sort_unstable_by_key(|g| g.0);
        }
        let mut out = alloc::vec::Vec::with_capacity(3 * m);
        out.extend(groups.iter().map(|g| g.0 as i64));
        out.extend(groups.iter().map(|g| g.1 as i64));
        out.extend(groups.iter().map(|g| g.2 as i64));
        out
    }
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

/// The counts of the groups of integer keys (as their 64 bits) for
/// `$groupCounts` (D317), which needs no last index: each key with its
/// count in one word of its slot, `key << bits | count` (`bits` those of the
/// number of keys, so that no count reaches them; 0 an empty slot), where
/// `WordGroups` keeps a key, its last index and its count (16 bytes: a
/// table of 140000 keys twice larger than the L2 cache, D253 scattered it
/// into lists counted again). With `direct`, the keys below DIRECT_MAX
/// counted by their value; a key too large to share its word (beyond `64 -
/// bits` bits: a negative key, sign extended) in a `WordGroups` of its own.
/// The groups in the order of their first occurrence (`first`, `keys`);
/// the tables of consecutive ranges merged in their order.
pub struct CountTable {
    /// the slots by lines of 8, a key looked for in the line of its hash and
    /// the lines after it (D317)
    lines: alloc::vec::Vec<Line>,
    used: usize,
    bits: u32,
    small: alloc::vec::Vec<u32>,
    direct: bool,
    first: alloc::vec::Vec<u32>,
    keys: alloc::vec::Vec<u64>,
    wide: Option<alloc::boxed::Box<WordGroups>>,
}

/// 8 slots of a `CountTable` in one line of the cache.
#[derive(Clone, Copy, Default)]
#[repr(C, align(64))]
struct Line([u64; 8]);

impl CountTable {
    /// The keys of a range of at most `n` keys in all (with the ranges it
    /// will be merged with): its counts below 2^bits.
    pub fn new(direct: bool, n: usize) -> CountTable {
        let bits = (usize::BITS - n.max(1).leading_zeros()).max(1);
        CountTable { lines: alloc::vec![Line::default(); 128], used: 0, bits, small: alloc::vec::Vec::new(), direct, first: alloc::vec::Vec::new(), keys: alloc::vec::Vec::new(), wide: None }
    }

    /// The key `k` fits in a slot with its count.
    #[inline(always)]
    fn packs(&self, k: u64) -> bool {
        k >> (64 - self.bits) == 0
    }

    /// The slots of the line `l` that hold `k`, and the empty ones, as
    /// masks of 8 bits: the 8 slots compared at once (in vectors), one
    /// branch on the result, where a probe slot by slot mispredicted its
    /// end about once a key (D317: 208 million mispredictions against 27
    /// million in 3 counts of 125 million keys).
    #[inline(always)]
    fn masks(line: &Line, k: u64, bits: u32) -> (u32, u32) {
        let (mut found, mut empty) = (0u32, 0u32);
        for (j, &e) in line.0.iter().enumerate() {
            found |= ((e >> bits == k) as u32 & (e != 0) as u32) << j;
            empty |= ((e == 0) as u32) << j;
        }
        (found, empty)
    }

    /// `count` more for the key `k` of the slot table (`packs`), first at
    /// `first` if new.
    #[inline(always)]
    fn bump(&mut self, k: u64, h: u64, first: u32, count: u32) {
        let mask = self.lines.len() - 1;
        let mut l = (h as usize) & mask;
        loop {
            // SAFETY: `l` is masked by the length less one (a power of two)
            let line = unsafe { self.lines.get_unchecked_mut(l) };
            let (found, empty) = Self::masks(line, k, self.bits);
            if found != 0 {
                line.0[found.trailing_zeros() as usize] += count as u64;
                return;
            }
            if empty != 0 {
                line.0[empty.trailing_zeros() as usize] = k << self.bits | count as u64;
                self.first.push(first);
                self.keys.push(k);
                self.used += 1;
                // at most 3/4 full
                if self.used * 4 > self.lines.len() * 8 * 3 {
                    self.grow();
                }
                return;
            }
            l = (l + 1) & mask;
        }
    }

    #[cold]
    fn grow(&mut self) {
        let old = core::mem::take(&mut self.lines);
        self.lines = alloc::vec![Line::default(); old.len() * 2];
        let mask = self.lines.len() - 1;
        for e in old.iter().flat_map(|l| l.0).filter(|&e| e != 0) {
            let mut l = (hash_word(e >> self.bits) as usize) & mask;
            loop {
                let (_, empty) = Self::masks(&self.lines[l], u64::MAX, self.bits);
                if empty != 0 {
                    self.lines[l].0[empty.trailing_zeros() as usize] = e;
                    break;
                }
                l = (l + 1) & mask;
            }
        }
    }

    /// `count` more for the key `k`, first at `first` if new.
    fn add_count(&mut self, k: u64, first: u32, count: u32) {
        if self.direct && k < WordGroups::DIRECT_MAX {
            if k as usize >= self.small.len() {
                self.small.resize((k as usize + 1).next_power_of_two().max(256), 0);
            }
            let c = &mut self.small[k as usize];
            if *c == 0 {
                self.first.push(first);
                self.keys.push(k);
            }
            *c += count;
        } else if self.packs(k) {
            self.bump(k, hash_word(k), first, count);
        } else {
            self.wide.get_or_insert_with(|| alloc::boxed::Box::new(WordGroups::new(false))).add_count(k, first, first, count);
        }
    }

    /// The keys `keys` at the indices `start ..` (after every index added
    /// before), in order, by chunks of 1024: a chunk of small keys counted
    /// by their value; a chunk whose keys all pack (its largest tells) in
    /// one loop that prefetches the line of the key AHEAD places later, its
    /// hash computed there (no array of hashes written and read again), a
    /// small key among them counted by its value (k-nucleotide, k = 12:
    /// about 4 keys of a chunk); the other chunks key by key.
    pub fn add_block(&mut self, keys: &[u64], start: usize) {
        const CHUNK: usize = 1024;
        const AHEAD: usize = 16;
        let mut hs = [0u64; CHUNK];
        for (c, chunk) in keys.chunks(CHUNK).enumerate() {
            let base = (start + c * CHUNK) as u32;
            let top = chunk.iter().copied().max().unwrap_or(0);
            if self.direct && top < WordGroups::DIRECT_MAX {
                if top as usize >= self.small.len() {
                    self.small.resize((top as usize + 1).next_power_of_two().max(256), 0);
                }
                let small = &mut self.small[..];
                for (j, &k) in chunk.iter().enumerate() {
                    let e = &mut small[k as usize];
                    if *e == 0 {
                        self.first.push(base + j as u32);
                        self.keys.push(k);
                    }
                    *e += 1;
                }
                continue;
            }
            if self.packs(top) {
                // the hashes of the chunk first, in a loop of their own that
                // the compiler makes vector code (D417: each key hashed once,
                // where the key AHEAD places later was hashed again for its
                // prefetch)
                for (h, &k) in hs.iter_mut().zip(chunk) {
                    *h = hash_word(k);
                }
                let (n, bits, direct) = (chunk.len(), self.bits, self.direct);
                let mut j = 0;
                while j < n {
                    // the keys found in their lines, counted with the table,
                    // its mask and `bits` held here (nothing else of `self`
                    // written); a new key or a small one by `bump` or
                    // `add_count` below, the table read again after it (it
                    // may have grown) (D417)
                    let lines = &mut self.lines[..];
                    let mask = lines.len() - 1;
                    while j < n {
                        let k = chunk[j];
                        if direct && k < WordGroups::DIRECT_MAX {
                            break;
                        }
                        if j + AHEAD < n {
                            let l = (hs[j + AHEAD] as usize) & mask;
                            #[cfg(target_arch = "x86_64")]
                            // SAFETY: a prefetch reads nothing; `l` is a line of the table
                            unsafe {
                                core::arch::x86_64::_mm_prefetch(lines.as_ptr().add(l) as *const i8, core::arch::x86_64::_MM_HINT_T0)
                            };
                            #[cfg(not(target_arch = "x86_64"))]
                            let _ = l;
                        }
                        let mut l = (hs[j] as usize) & mask;
                        let hit = loop {
                            let (found, empty) = Self::masks(&lines[l], k, bits);
                            if found != 0 {
                                lines[l].0[found.trailing_zeros() as usize] += 1;
                                break true;
                            }
                            if empty != 0 {
                                break false;
                            }
                            l = (l + 1) & mask;
                        };
                        if !hit {
                            break;
                        }
                        j += 1;
                    }
                    if j < n {
                        self.add_count(chunk[j], base + j as u32, 1);
                        j += 1;
                    }
                }
                continue;
            }
            for (j, &k) in chunk.iter().enumerate() {
                self.add_count(k, base + j as u32, 1);
            }
        }
    }

    /// The count of the key `k` of a group.
    fn count(&self, k: u64) -> u32 {
        if self.direct && k < WordGroups::DIRECT_MAX {
            return self.small[k as usize];
        }
        let mask = self.lines.len() - 1;
        let mut l = (hash_word(k) as usize) & mask;
        loop {
            let (found, _) = Self::masks(&self.lines[l], k, self.bits);
            if found != 0 {
                return (self.lines[l].0[found.trailing_zeros() as usize] & ((1u64 << self.bits) - 1)) as u32;
            }
            l = (l + 1) & mask;
        }
    }

    /// The groups of `other`, of indices all after those of `self`.
    pub fn merge(&mut self, other: &CountTable) {
        for g in 0..other.first.len() {
            let k = other.keys[g];
            self.add_count(k, other.first[g], other.count(k));
        }
        if let Some(w) = &other.wide {
            self.wide.get_or_insert_with(|| alloc::boxed::Box::new(WordGroups::new(false))).merge(w);
        }
    }

    /// The columns of `group` (D167) without the last indices (zeros): the
    /// first index of each group in the order of the first indices, 0, its
    /// count.
    pub fn columns(self) -> alloc::vec::Vec<i64> {
        let mut groups: alloc::vec::Vec<(u32, u32)> = self.first.iter().zip(&self.keys).map(|(&f, &k)| (f, self.count(k))).collect();
        if let Some(w) = self.wide {
            let c = w.columns();
            let m = c.len() / 3;
            groups.extend((0..m).map(|g| (c[g] as u32, c[2 * m + g] as u32)));
        }
        if !groups.windows(2).all(|w| w[0].0 < w[1].0) {
            groups.sort_unstable_by_key(|g| g.0);
        }
        let m = groups.len();
        let mut out = alloc::vec::Vec::with_capacity(3 * m);
        out.extend(groups.iter().map(|g| g.0 as i64));
        out.extend(core::iter::repeat(0i64).take(m));
        out.extend(groups.iter().map(|g| g.1 as i64));
        out
    }
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
