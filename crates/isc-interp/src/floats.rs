//! The texts of floats of the interpreter, in Rust: the shortest decimal
//! that reads back to a float (`print`, `toString`: `f64text`, the text of
//! `Display`) and `toFixed`. A compiled program writes these texts by the
//! functions of the prelude (`$rtF64Text`, `$rtF32Text`, `$rtFixedText`):
//! this code, which the runtime had before them, is their reference in the
//! tests.

use std::fmt::{self, Write};

use isc_runtime_core::Panic;

use crate::f64text;

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

/// Shortest decimal form that reads back to the same value (the `Display`
/// algorithm of `core`); NaN without sign, `inf`, `-inf`.
pub fn write_f64(w: &mut dyn Write, v: f64) -> fmt::Result {
    if v.is_nan() {
        w.write_str("NaN")
    } else if v.is_infinite() {
        w.write_str(if v > 0.0 { "inf" } else { "-inf" })
    } else {
        // Schubfach, the text of `Display` (D467)
        let mut b = [0u8; f64text::F64_TEXT_MAX];
        let n = f64text::write_shortest(v, &mut b);
        w.write_str(core::str::from_utf8(&b[..n]).map_err(|_| fmt::Error)?)
    }
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
    if d <= 18 {
        if let Some((buf, n)) = fixed_short(x, d) {
            // ASCII: digits, `-` and `.`
            return w.write_str(core::str::from_utf8(&buf[..n]).unwrap_or(""));
        }
    }
    fixed_exact(w, x, d)
}

/// `x.toFixed(d)` for `d` at most 18 and a finite `|x|` below 1e21 (D487):
/// the first `d + 1` decimals of the exact value of `|x|` from its integer
/// `m` and its power of two `e`, `m 10^(d+1) / 2^-e` in 128 bits, rounded
/// half away from zero, then its digits and its point written at once
/// (`fixed_exact` wrote the 1100 decimals of `|x|` each time: 1156 ns a
/// call). `None` when the product leaves 128 bits (`fixed_exact` then).
fn fixed_short(x: f64, d: usize) -> Option<([u8; 64], usize)> {
    let bits = x.to_bits();
    let be = ((bits >> 52) & 0x7ff) as i32;
    let mut m = bits & ((1u64 << 52) - 1);
    let e = if be == 0 {
        -1074
    } else {
        m |= 1u64 << 52;
        be - 1075
    };
    let p = 10u128.pow(d as u32);
    // |x| 10^d, rounded half away from zero; divided in 64 bits when it fits
    // (a division of 128 bits is a call)
    let q: u128 = if e >= 0 {
        ((m as u128) << e).checked_mul(p)?
    } else {
        // m < 2^53 and 10^(d+1) <= 10^19 < 2^64: in 117 bits
        let t = (m as u128) * p * 10;
        let n = if -e >= 128 { 0 } else { t >> (-e) };
        match u64::try_from(n) {
            Ok(n) => u128::from(n / 10 + u64::from(n % 10 >= 5)),
            Err(_) => n / 10 + u128::from(n % 10 >= 5),
        }
    };
    // its digits from the end, at least d + 1 of them, the point before
    // the last d
    let mut digits = [0u8; 48];
    let mut k = digits.len();
    let mut v = q;
    while v >> 64 != 0 {
        k -= 1;
        digits[k] = b'0' + (v % 10) as u8;
        v /= 10;
    }
    let mut v = v as u64;
    while v != 0 || digits.len() - k < d + 1 {
        k -= 1;
        digits[k] = b'0' + (v % 10) as u8;
        v /= 10;
    }
    let nd = digits.len() - k;
    let mut out = [0u8; 64];
    let mut o = 0;
    if x < 0.0 {
        out[0] = b'-';
        o = 1;
    }
    let int = nd - d;
    out[o..o + int].copy_from_slice(&digits[k..k + int]);
    o += int;
    if d > 0 {
        out[o] = b'.';
        o += 1;
        out[o..o + d].copy_from_slice(&digits[k + int..]);
        o += d;
    }
    Some((out, o))
}

/// `x.toFixed(d)` by the exact decimal value of `|x|` (a double has at
/// most 1074 decimals): for `d` above 18, and the reference of
/// `fixed_short` in its test.
fn fixed_exact(w: &mut dyn Write, x: f64, d: usize) -> fmt::Result {
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

#[cfg(test)]
mod tests {
    use super::*;

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

    /// D487: `fixed_short` writes the text of `fixed_exact` (the exact
    /// decimal value), for 200000 doubles of every size from 1e-30 to
    /// 1e21 and both signs, halves and the edges (`1.005`, `9.995`, `0.125`,
    /// `-0`, the subnormals, `1e20`, the powers of two), each with `d` from 0
    /// to 18.
    #[test]
    // no unsafe code in what it tests: under Miri, nothing more to find
    // than the test itself finds (D574)
    #[cfg_attr(miri, ignore)]
    fn fixed_short_is_fixed_exact() {
        let mut xs = vec![0.0, -0.0, 0.5, 1.5, 2.5, -2.5, 0.125, 0.375, 1.005, 9.995, 0.045, 1e20, 9.99e20, -9.99e20, 999999999999999.9, 0.000001, 5e-324, -5e-324, 2.2250738585072014e-308, 123456789.987654321];
        for k in (-60..70).step_by(if cfg!(miri) { 13 } else { 1 }) {
            xs.push(libm_pow2(k));
            xs.push(-libm_pow2(k) * 1.5);
        }
        let mut z = 0x9e3779b97f4a7c15u64;
        for _ in 0..if cfg!(miri) { 2000 } else { 200000 } {
            z ^= z << 13;
            z ^= z >> 7;
            z ^= z << 17;
            // a mantissa and a decimal exponent from -30 to 21
            let mant = (z >> 11) as f64 / (1u64 << 53) as f64;
            let ex = (z % 52) as i32 - 30;
            let mut v = mant;
            for _ in 0..ex.unsigned_abs() {
                v = if ex > 0 { v * 10.0 } else { v / 10.0 };
            }
            xs.push(if z & 1 == 0 { v } else { -v });
        }
        for &x in &xs {
            if !(x.abs() < 1e21) {
                continue;
            }
            for d in 0..=18usize {
                let mut want = String::new();
                fixed_exact(&mut want, x, d).unwrap();
                let got = fixed_short(x, d).map(|(b, n)| String::from_utf8(b[..n].to_vec()).unwrap());
                if let Some(got) = got {
                    assert_eq!(got, want, "{:e} with {} decimals", x, d);
                }
            }
        }
    }

    fn libm_pow2(k: i32) -> f64 {
        f64::from_bits(((1023 + k) as u64) << 52)
    }
}
