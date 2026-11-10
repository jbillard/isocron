//! A double as the shortest decimal that reads back to it (D467): Schubfach
//! (R. Giulietti, "The Schubfach way to render doubles", the algorithm of
//! Java's `Double.toString`), ported from the floor of `json` (D415), laid
//! out as Rust's `Display` writes it, the text of Isocron (no exponent,
//! `-0`, the digits then zeros, or a point). `core::fmt` took Grisu with its
//! fallback and its formatter: a third of `json.stringify`.

use crate::pow10_g::POW10_G;

const K_MIN: i32 = -324;

#[inline]
fn mulhi(a: u64, b: u64) -> u64 {
    ((a as u128 * b as u128) >> 64) as u64
}

#[inline]
fn flog10pow2(q: i32) -> i32 {
    ((q as i64 * 661971961083) >> 41) as i32
}

#[inline]
fn flog10three4(q: i32) -> i32 {
    ((q as i64 * 661971961083 - 274743187321) >> 41) as i32
}

#[inline]
fn flog2pow10(e: i32) -> i32 {
    ((e as i64 * 913124641741) >> 38) as i32
}

#[inline]
fn rop(g1: u64, g0: u64, cp: u64) -> u64 {
    const M63: u64 = 0x7fff_ffff_ffff_ffff;
    let x1 = mulhi(g0, cp);
    let y0 = g1.wrapping_mul(cp);
    let y1 = mulhi(g1, cp);
    let z = (y0 >> 1).wrapping_add(x1);
    let vbp = y1.wrapping_add(z >> 63);
    vbp | ((z & M63).wrapping_add(M63) >> 63)
}

/// The digits and the exponent of the shortest decimal of `c` 2^`q`.
fn decimal(q: i32, c: u64) -> (u64, i32) {
    const C_MIN: u64 = 1 << 52;
    let out = c & 1;
    let cb = c << 2;
    let cbr = cb + 2;
    let (cbl, k) = if c != C_MIN || q == -1074 { (cb - 2, flog10pow2(q)) } else { (cb - 1, flog10three4(q)) };
    let h = q + flog2pow10(-k) + 2;
    let [g1, g0] = POW10_G[(k - K_MIN) as usize];
    let vb = rop(g1, g0, cb << h);
    let vbl = rop(g1, g0, cbl << h);
    let vbr = rop(g1, g0, cbr << h);
    let s = vb >> 2;
    if s >= 100 {
        let sp10 = 10u64.wrapping_mul(mulhi(s, 115292150460684698u64 << 4));
        let tp10 = sp10.wrapping_add(10);
        let upin = vbl.wrapping_add(out) <= sp10 << 2;
        let wpin = (tp10 << 2).wrapping_add(out) <= vbr;
        if upin != wpin {
            return (if upin { sp10 } else { tp10 }, k);
        }
    }
    let t = s + 1;
    let uin = vbl.wrapping_add(out) <= s << 2;
    let win = (t << 2).wrapping_add(out) <= vbr;
    if uin != win {
        return (if uin { s } else { t }, k);
    }
    // a value halfway between two texts of its length: the larger, as
    // Rust's `Display` (Schubfach takes the even one: 2^-25 ends by 312
    // there, by 313 for `Display`)
    let cmp = vb.wrapping_sub((s + t) << 1) as i64;
    (if cmp < 0 { s } else { t }, k)
}

/// `d` 10^`e` read back as a double.
fn read_back(d: u64, e: i32) -> f64 {
    let mut b = [0u8; 32];
    let mut n = 0;
    let mut x = d;
    let mut digits = [0u8; 20];
    let mut nd = 0;
    loop {
        digits[nd] = b'0' + (x % 10) as u8;
        nd += 1;
        x /= 10;
        if x == 0 {
            break;
        }
    }
    for i in (0..nd).rev() {
        b[n] = digits[i];
        n += 1;
    }
    b[n] = b'e';
    n += 1;
    if e < 0 {
        b[n] = b'-';
        n += 1;
    }
    let mut ex = [0u8; 4];
    let mut ne = 0;
    let mut y = e.unsigned_abs();
    loop {
        ex[ne] = b'0' + (y % 10) as u8;
        ne += 1;
        y /= 10;
        if y == 0 {
            break;
        }
    }
    for i in (0..ne).rev() {
        b[n] = ex[i];
        n += 1;
    }
    core::str::from_utf8(&b[..n]).ok().and_then(|t| t.parse::<f64>().ok()).unwrap_or(f64::NAN)
}

/// The digits `f` and exponent `e` of a subnormal `v` of a few bits, with as
/// few digits as read back to `v`: Schubfach tries a digit less only when
/// its scaled value reaches 100, so that it keeps two where one may do
/// (20 times the least double: 99e-324 where `Display` writes 1e-322). Of
/// two that read back, the closer, the larger when both are as close, as
/// `Display`.
fn shorter(v: f64, (mut f, mut e): (u64, i32)) -> (u64, i32) {
    let v = v.abs();
    while f >= 10 {
        let lo = f / 10;
        let hi = lo + 1;
        let (a, b) = (read_back(lo, e + 1) == v, read_back(hi, e + 1) == v);
        let d = match (a, b) {
            (true, true) => {
                if f - lo * 10 < hi * 10 - f {
                    lo
                } else {
                    hi
                }
            }
            (true, false) => lo,
            (false, true) => hi,
            (false, false) => break,
        };
        f = d;
        e += 1;
    }
    (f, e)
}

/// The longest text: `-0.`, 323 zeros and 17 digits.
pub const F64_TEXT_MAX: usize = 352;

/// Writes the text of a finite `v` at the start of `p`; its length.
pub fn write_shortest(v: f64, p: &mut [u8; F64_TEXT_MAX]) -> usize {
    let bits = v.to_bits();
    let mut n = 0;
    if bits >> 63 != 0 {
        p[n] = b'-';
        n += 1;
    }
    let t = bits & ((1u64 << 52) - 1);
    let bq = ((bits >> 52) & 0x7ff) as i32;
    if bq == 0 && t == 0 {
        p[n] = b'0';
        return n + 1;
    }
    let (mut f, mut e) = if bq != 0 {
        let mq = bq - 1075;
        let c = (1u64 << 52) | t;
        if mq < 0 && -mq < 53 && ((c >> -mq) << -mq) == c {
            (c >> -mq, 0)
        } else {
            decimal(mq, c)
        }
    } else if t < 3 {
        // the two smallest values: Schubfach gives them two digits there (as
        // Java writes 4.9E-324); their shortest forms are 5e-324 and 1e-323
        if t == 1 {
            (5, -324)
        } else {
            (1, -323)
        }
    } else {
        shorter(v, decimal(-1074, t))
    };
    while f % 10 == 0 {
        f /= 10;
        e += 1;
    }
    let mut d = [0u8; 20];
    let mut nd = 0usize;
    let mut x = f;
    while x != 0 {
        d[nd] = b'0' + (x % 10) as u8;
        nd += 1;
        x /= 10;
    }
    if e >= 0 {
        for i in (0..nd).rev() {
            p[n] = d[i];
            n += 1;
        }
        for _ in 0..e {
            p[n] = b'0';
            n += 1;
        }
    } else if nd as i32 + e > 0 {
        let frac = (-e) as usize;
        for i in (frac..nd).rev() {
            p[n] = d[i];
            n += 1;
        }
        p[n] = b'.';
        n += 1;
        for i in (0..frac).rev() {
            p[n] = d[i];
            n += 1;
        }
    } else {
        p[n] = b'0';
        p[n + 1] = b'.';
        n += 2;
        for _ in 0..(-(nd as i32 + e)) {
            p[n] = b'0';
            n += 1;
        }
        for i in (0..nd).rev() {
            p[n] = d[i];
            n += 1;
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::{write_shortest, F64_TEXT_MAX};
    extern crate alloc;
    use alloc::format;
    use alloc::vec::Vec;

    fn same(v: f64) {
        let mut b = [0u8; F64_TEXT_MAX];
        let n = write_shortest(v, &mut b);
        let want = format!("{}", v);
        assert_eq!(core::str::from_utf8(&b[..n]).unwrap(), want, "bits {:#x}", v.to_bits());
    }

    /// D467: the text of Rust's `Display` for zeros, the subnormals of up to
    /// 100000 times the least, the powers of 2 and 10, the integers around
    /// 2^53, the largest and smallest values, and two million values of
    /// random bits (a tenth of these without the optimizations).
    #[test]
    // no unsafe code in what it tests: under Miri, nothing more to find
    // than the test itself finds (D574)
    #[cfg_attr(miri, ignore)]
    fn shortest_as_display() {
        let mut edges: Vec<f64> = alloc::vec![0.0, -0.0, f64::MIN_POSITIVE, f64::MAX, f64::MIN, f64::EPSILON, 0.1, 0.2, 0.3, 1.0 / 3.0, 5e-324, 1e-323, 1.5e-323, 2.2250738585072009e-308, 9007199254740992.0, 9007199254740993.0, 123456789012345680.0, 1e21, 1e22, 1e23, 0.5, 1.5, 100.0, 1234.5678];
        for e in (-1074..1024).step_by(if cfg!(miri) { 41 } else { 1 }) {
            // (a subnormal power by its bit: `powi` gives zero below 2^-1023)
            let p = if e < -1022 { f64::from_bits(1u64 << (e + 1074)) } else { 2f64.powi(e) };
            edges.push(p);
            edges.push(-p);
        }
        for e in (-323..309).step_by(if cfg!(miri) { 23 } else { 1 }) {
            edges.push(format!("1e{}", e).parse().unwrap());
            edges.push(format!("9.999999999999999e{}", e).parse().unwrap_or(1.0));
        }
        for i in 0..if cfg!(miri) { 40u64 } else { 2000 } {
            edges.push(f64::from_bits(i));
            edges.push((9007199254740992u64 - 1000 + i) as f64);
            edges.push(i as f64 * 0.5);
        }
        for v in edges {
            if v.is_finite() {
                same(v);
            }
        }
        // every subnormal of up to 100000 times the least (where the
        // digits are shortened by reading them back)
        let subs = if cfg!(debug_assertions) { 20_000 } else { 100_000 };
        for i in 0..subs {
            same(f64::from_bits(i));
        }
        let count = if cfg!(debug_assertions) { 200_000 } else { 2_000_000 };
        let mut x: u64 = 0x9e37_79b9_7f4a_7c15;
        for _ in 0..count {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            let v = f64::from_bits(x);
            if v.is_finite() {
                same(v);
            }
        }
    }
}
