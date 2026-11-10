//! The mathematical functions written in Isocron (D628): `exp`, `log`,
//! `sin`, `cos`, `tan`, `asin`, `acos`, `atan`, `atan2` and `pow` of `f64`
//! and of `f32` are functions of the prelude, musl's algorithms as the
//! crate `libm` 0.2.16 writes them. They give its bits, NaN included, for
//! every input: compiled natively, compiled for WebAssembly, interpreted
//! node by node, and under `iscc run` (their native form, D616). The
//! reference is `libm` itself, called here through `isc_runtime_core`, the
//! code the runtime called before (D108).
//!
//! `math_exhaustive` (ignored by default, some minutes on 4 threads:
//! `cargo test -p iscc --test math -- --ignored`) runs every one of the
//! 2^32 inputs of each function of one `f32`, and 10^7 inputs of each
//! function of `f64` and of the two of two `f32`, natively and as
//! WebAssembly; the others run in the suite.

mod common;
use common::*;
use std::path::{Path, PathBuf};

/// The functions, in the order of the programs below.
const NAMES: [&str; 10] = ["exp", "log", "sin", "cos", "tan", "asin", "acos", "atan", "atan2", "pow"];

fn libm64(f: usize, x: f64, y: f64) -> f64 {
    use isc_runtime_core as m;
    match f {
        0 => m::exp_f64(x),
        1 => m::log_f64(x),
        2 => m::sin_f64(x),
        3 => m::cos_f64(x),
        4 => m::tan_f64(x),
        5 => m::asin_f64(x),
        6 => m::acos_f64(x),
        7 => m::atan_f64(x),
        8 => m::atan2_f64(x, y),
        _ => m::pow_f64(x, y),
    }
}

fn libm32(f: usize, x: f32, y: f32) -> f32 {
    use isc_runtime_core as m;
    match f {
        0 => m::exp_f32(x),
        1 => m::log_f32(x),
        2 => m::sin_f32(x),
        3 => m::cos_f32(x),
        4 => m::tan_f32(x),
        5 => m::asin_f32(x),
        6 => m::acos_f32(x),
        7 => m::atan_f32(x),
        8 => m::atan2_f32(x, y),
        _ => m::pow_f32(x, y),
    }
}

/// The bits of each result, read as a part of the prelude (which can read
/// the bits of a float: a program cannot, D614). The function `f` (0 to 9:
/// of `f64`, 10 to 19: of `f32`), the files of the first and the second
/// arguments, the file of the bits of the results (8 bytes each for a
/// `f64`, 4 for a `f32`).
const BITS: &str = "function value64(f: i64, x: f64, y: f64): f64 {
    return f == 0 ? exp(x) : f == 1 ? log(x) : f == 2 ? sin(x) : f == 3 ? cos(x) : f == 4 ? tan(x) : f == 5 ? asin(x) : f == 6 ? acos(x) : f == 7 ? atan(x) : f == 8 ? atan2(x, y) : pow(x, y);
}

function value32(f: i64, x: f32, y: f32): f32 {
    return f == 0 ? exp(x) : f == 1 ? log(x) : f == 2 ? sin(x) : f == 3 ? cos(x) : f == 4 ? tan(x) : f == 5 ? asin(x) : f == 6 ? acos(x) : f == 7 ? atan(x) : f == 8 ? atan2(x, y) : pow(x, y);
}

action main(args: string[k]): i32 {
    const f = parseI64(args[0]);
    if (f < 10) {
        const xs = readF64(args[1]);
        const ys = readF64(args[2]);
        const r = map(iota(xs.length), (i) => $floatBits(value64(f, xs[i], ys[i])));
        writeBytes(args[3], map(iota(8 * xs.length), (j) => u8(r[j >> 3] >> u64((j & 7) * 8))));
    } else {
        const xs = readF32(args[1]);
        const ys = readF32(args[2]);
        const r = map(iota(xs.length), (i) => $floatBits(value32(f - 10, xs[i], ys[i])));
        writeBytes(args[3], map(iota(4 * xs.length), (j) => u8(r[j >> 2] >> u64((j & 3) * 8))));
    }
    return 0;
}
";

/// Every input of a function of one `f32`: for each block of 2^20
/// consecutive patterns of bits from the block `first`, the sum of the
/// bits of the results mixed with their inputs (`mix`, the same here).
const SUMS32: &str = "function mix(w: u64, r: u64): u64 {
    const z = (r ^ (w * 0x9E3779B97F4A7C15u64)) * 0xBF58476D1CE4E5B9u64;
    return z ^ (z >> 31u64);
}

function value32(f: i64, x: f32): f32 {
    return f == 0 ? exp(x) : f == 1 ? log(x) : f == 2 ? sin(x) : f == 3 ? cos(x) : f == 4 ? tan(x) : f == 5 ? asin(x) : f == 6 ? acos(x) : atan(x);
}

action main(args: string[k]): i32 {
    const f = parseI64(args[0]);
    const first = parseI64(args[1]);
    const count = parseI64(args[2]);
    print(map(iota(count), (b) => sum(map(iota(1048576), (i) => {
        const w = u64((first + b) * 1048576 + i);
        return mix(w, $floatBits(value32(f, $bitsFloat32(w))));
    }))));
    return 0;
}
";

fn mix(w: u64, r: u64) -> u64 {
    let z = (r ^ w.wrapping_mul(0x9E3779B97F4A7C15)).wrapping_mul(0xBF58476D1CE4E5B9);
    z ^ (z >> 31)
}

/// The sum of a block of 2^20 inputs of the function `f` of `f32`, by libm.
fn block_sum32(f: usize, b: u64) -> u64 {
    let mut s = 0u64;
    for i in 0..1u64 << 20 {
        let w = (b << 20) | i;
        s = s.wrapping_add(mix(w, libm32(f, f32::from_bits(w as u32), 0.0).to_bits() as u64));
    }
    s
}

/// The values where a function changes its path, as `libm` tests them:
/// the high words of its tests (and the next ones) with several low words,
/// both signs; the zeros, the subnormals, the least and largest floats, the
/// infinities and NaNs (quiet, signaling, of both signs).
fn specials64() -> Vec<f64> {
    let mut v = vec![
        0.0,
        -0.0,
        1.0,
        -1.0,
        0.5,
        -0.5,
        2.0,
        -2.0,
        3.0,
        -3.0,
        2.5,
        -2.5,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::from_bits(0x7FF8000000000000),
        f64::from_bits(0xFFF8000000000000),
        f64::from_bits(0x7FF0000000000001),
        f64::from_bits(0xFFF4000000000123),
        f64::MIN_POSITIVE,
        -f64::MIN_POSITIVE,
        f64::from_bits(1),
        f64::from_bits(0x8000000000000001),
        f64::from_bits(0x000FFFFFFFFFFFFF),
        f64::MAX,
        -f64::MAX,
        f64::EPSILON,
        709.782712893383973096,
        709.7827128933841,
        -745.13321910194110842,
        -745.1332191019412,
        -708.39641853226410622,
        708.3964185322641,
        1e300,
        1e-300,
        6381956970095103.0 * 2f64.powi(797),
    ];
    let words: [u32; 49] = [
        0x4086232B, 0x3E300000, 0x3FD62E42, 0x3FF0A2B2, 0x00100000, 0x7FF00000, 0x3FF00000, 0x3FE6A09E, 0x3FE921FB, 0x3E500000, 0x3E46A09E, 0x3E400000, 0x400F6A7A,
        0x4002D97C, 0x401C463B, 0x4015FDBC, 0x4012D97C, 0x401921FB, 0x413921FB, 0x3FE59428, 0x3FE00000, 0x3FEF3333, 0x3C600000, 0x44100000, 0x3FDC0000, 0x3FF30000,
        0x3FE60000, 0x40038000, 0x41E00000, 0x43F00000, 0x3FEFFFFF, 0x43400000, 0x40900000, 0x4090CC00, 0x3FF3988E, 0x3FFBB67A, 0x3FF921FB, 0x400921FB, 0x401921FB,
        0x3FF6A09E, 0x40862E42, 0x4086232A, 0x41D00000, 0x43300000, 0x3FE62E42, 0x3FC00000, 0x3E700000, 0x40000000, 0x40F00000,
    ];
    for w in words {
        for d in [-1i64, 0, 1] {
            for lo in [0u64, 1, 0x54442D18, 0x80000000, 0xFFFFFFFF] {
                for sign in [0u64, 1] {
                    let hi = (w as i64 + d) as u64 & 0x7FFFFFFF;
                    v.push(f64::from_bits(sign << 63 | hi << 32 | lo));
                }
            }
        }
    }
    v
}

/// The same for `f32`.
fn specials32() -> Vec<f32> {
    let mut v = vec![0.0f32, -0.0, 1.0, -1.0, 0.5, -0.5, 2.0, -2.0, 3.0, -3.0, 2.5, -2.5, f32::INFINITY, f32::NEG_INFINITY, f32::MIN_POSITIVE, f32::MAX, -f32::MAX];
    for b in [0x7FC00000u32, 0xFFC00000, 0x7F800001, 0xFFA00123, 1, 0x80000001, 0x007FFFFF] {
        v.push(f32::from_bits(b));
    }
    let words: [u32; 40] = [
        0x42AEAC50, 0x39000000, 0x3EB17218, 0x3F851592, 0x42B17218, 0x42CFF1B5, 0x00800000, 0x7F800000, 0x3F800000, 0x3F3504F3, 0x3F490FDA, 0x39800000, 0x407B53D1,
        0x4016CBE3, 0x40E231D5, 0x40AFEDDF, 0x4DC90FDB, 0x3F000000, 0x32800000, 0x4C800000, 0x3EE00000, 0x3F980000, 0x3F300000, 0x401C0000, 0x4D000000, 0x3F7FFFF8,
        0x3F800007, 0x4B800000, 0x43000000, 0x43160000, 0x3FC90FDB, 0x40490FDB, 0x40C90FDB, 0x5D000000, 0x7F000000, 0x40000000, 0x3E800000, 0x3D000000, 0x4E000000,
        0x501C463B,
    ];
    for w in words {
        for d in [-2i64, -1, 0, 1, 2] {
            for sign in [0u32, 1] {
                v.push(f32::from_bits(sign << 31 | ((w as i64 + d) as u32 & 0x7FFFFFFF)));
            }
        }
    }
    v
}

/// The arguments of libm's hardest reductions: doubles near multiples of
/// pi/2 (from the continued fractions of 2^e 2/pi, the least distance
/// 2^-61 at 6381956970095103 2^797), from pi/4 to the largest.
fn hard64() -> Vec<f64> {
    let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/math_hard64.txt")).unwrap();
    let mut v = Vec::new();
    for l in text.lines().filter(|l| !l.starts_with('#')) {
        let x = f64::from_bits(u64::from_str_radix(l.trim(), 16).unwrap());
        v.push(x);
        v.push(-x);
        // and their neighbours
        v.push(f64::from_bits(x.to_bits() + 1));
        v.push(f64::from_bits(x.to_bits() - 1));
    }
    v
}

/// A double of any sign whose biased exponent is uniform in `lo..=hi`.
fn by_exponent64(r: &mut Rng, lo: u64, hi: u64) -> f64 {
    let e = lo + r.next() % (hi - lo + 1);
    let m = r.next() & 0x000FFFFFFFFFFFFF;
    let s = r.next() & 1;
    f64::from_bits(s << 63 | e << 52 | m)
}

fn by_exponent32(r: &mut Rng, lo: u32, hi: u32) -> f32 {
    let e = lo + (r.next() % (hi - lo + 1) as u64) as u32;
    let m = r.next() as u32 & 0x007FFFFF;
    let s = r.next() as u32 & 1;
    f32::from_bits(s << 31 | e << 23 | m)
}

/// `n` inputs of the function `f` of `f64`: the special values, then half
/// over the whole range by exponent, half over the range where it computes
/// (its biased exponents), and for `pow` and `atan2` the pairs of special
/// values (those of C99 and IEEE 754 among them) and pairs whose results
/// are finite, near the overflow and the underflow, of negative bases.
fn inputs64(f: usize, n: usize, seed: u64) -> (Vec<f64>, Vec<f64>) {
    let mut r = Rng(seed * 7919 + f as u64 + 1);
    let sp = specials64();
    let (mut xs, mut ys) = (Vec::with_capacity(n), Vec::with_capacity(n));
    if f >= 8 {
        let pairs = [0.0, -0.0, 1.0, -1.0, 0.5, -0.5, 2.0, -2.0, 3.0, -3.0, 2.5, -2.5, 1e-310, -1e-310, 1e300, -1e300, 0.9999999999999999, 1.0000000000000002, f64::INFINITY, f64::NEG_INFINITY, f64::NAN, -f64::NAN, 1023.0, 1024.0, -1074.0, -1075.0, 0.25, 4.0, 1e-20, 9007199254740993.0, 2147483648.0, -2147483649.0, 3.0e9, 1e19, -1e19];
        for &x in &pairs {
            for &y in &pairs {
                xs.push(x);
                ys.push(y);
            }
        }
        for &x in &sp {
            xs.push(x);
            ys.push(sp[(r.next() % sp.len() as u64) as usize]);
            xs.push(sp[(r.next() % sp.len() as u64) as usize]);
            ys.push(x);
        }
    } else {
        xs.extend_from_slice(&sp);
        if (2..=4).contains(&f) {
            xs.extend(hard64());
        }
        ys.resize(xs.len(), 0.0);
    }
    while xs.len() < n {
        let k = xs.len();
        let (x, y) = match (f, k % 6) {
            (8 | 9, 0 | 1) => (by_exponent64(&mut r, 0, 2047), by_exponent64(&mut r, 0, 2047)),
            (8, _) => (by_exponent64(&mut r, 900, 1150), by_exponent64(&mut r, 900, 1150)),
            // pow: finite results; negative bases with integers; near 1
            // with large exponents; near the overflow and the underflow
            (9, 2) => (f64::abs(by_exponent64(&mut r, 1003, 1043)), by_exponent64(&mut r, 1000, 1029)),
            (9, 3) => (by_exponent64(&mut r, 1013, 1033), (r.next() % 200) as f64 - 100.0),
            (9, 4) => (1.0 + by_exponent64(&mut r, 950, 1000), by_exponent64(&mut r, 1040, 1100)),
            (9, _) => {
                let x = f64::abs(by_exponent64(&mut r, 1, 2046));
                let l = x.log2();
                let t = [1023.0, 1024.0, -1022.0, -1074.0, -1075.0, -1060.0][(r.next() % 6) as usize];
                (x, t / l * (1.0 + ((r.next() % 2001) as f64 - 1000.0) * 1e-15))
            }
            (_, 0..=2) => (by_exponent64(&mut r, 0, 2047), 0.0),
            // the range where each computes
            (0, _) => (by_exponent64(&mut r, 900, 1032), 0.0),
            (1, _) => (f64::abs(by_exponent64(&mut r, 0, 2046)), 0.0),
            (2..=4, 3 | 4) => (by_exponent64(&mut r, 1000, 1043), 0.0),
            (2..=4, _) => (by_exponent64(&mut r, 1043, 2046), 0.0),
            (5 | 6, _) => (by_exponent64(&mut r, 900, 1022), 0.0),
            _ => (by_exponent64(&mut r, 950, 1100), 0.0),
        };
        xs.push(x);
        ys.push(y);
    }
    (xs, ys)
}

/// The same for `atan2` and `pow` of `f32` (the others are run on all
/// their inputs).
fn inputs32(f: usize, n: usize, seed: u64) -> (Vec<f32>, Vec<f32>) {
    let mut r = Rng(seed * 104729 + f as u64 + 11);
    let sp = specials32();
    let (mut xs, mut ys) = (Vec::with_capacity(n), Vec::with_capacity(n));
    if f >= 8 {
        let pairs = [0.0f32, -0.0, 1.0, -1.0, 0.5, -0.5, 2.0, -2.0, 3.0, -3.0, 2.5, -2.5, 1e-40, -1e-40, 1e30, -1e30, 0.99999994, 1.0000001, f32::INFINITY, f32::NEG_INFINITY, f32::NAN, -f32::NAN, 127.0, 128.0, -149.0, -150.0, 16777217.0, 2.0e8, 0.25];
        for &x in &pairs {
            for &y in &pairs {
                xs.push(x);
                ys.push(y);
            }
        }
        for &x in &sp {
            xs.push(x);
            ys.push(sp[(r.next() % sp.len() as u64) as usize]);
            xs.push(sp[(r.next() % sp.len() as u64) as usize]);
            ys.push(x);
        }
    } else {
        xs.extend_from_slice(&sp);
        ys.resize(xs.len(), 0.0);
    }
    while xs.len() < n {
        let k = xs.len();
        let (x, y) = match (f, k % 6) {
            (_, 0 | 1) => (by_exponent32(&mut r, 0, 255), by_exponent32(&mut r, 0, 255)),
            (8, _) => (by_exponent32(&mut r, 100, 155), by_exponent32(&mut r, 100, 155)),
            (_, 2) => (f32::abs(by_exponent32(&mut r, 117, 137)), by_exponent32(&mut r, 115, 133)),
            (_, 3) => (by_exponent32(&mut r, 120, 133), (r.next() % 200) as f32 - 100.0),
            (_, 4) => (1.0 + by_exponent32(&mut r, 100, 120), by_exponent32(&mut r, 140, 160)),
            _ => {
                let x = f32::abs(by_exponent32(&mut r, 1, 254));
                let l = x.log2();
                let t = [127.0f32, 128.0, -126.0, -149.0, -150.0, -140.0][(r.next() % 6) as usize];
                (x, t / l * (1.0 + ((r.next() % 201) as f32 - 100.0) * 1e-7))
            }
        };
        xs.push(x);
        ys.push(y);
    }
    (xs, ys)
}

fn write_bits64(name: &str, xs: &[f64]) -> String {
    let p = tmp(name);
    let b: Vec<u8> = xs.iter().flat_map(|x| x.to_bits().to_le_bytes()).collect();
    std::fs::write(&p, b).unwrap();
    p.to_string_lossy().to_string()
}

fn write_bits32(name: &str, xs: &[f32]) -> String {
    let p = tmp(name);
    let b: Vec<u8> = xs.iter().flat_map(|x| x.to_bits().to_le_bytes()).collect();
    std::fs::write(&p, b).unwrap();
    p.to_string_lossy().to_string()
}

/// A program of the prelude built for `isa`.
fn build_prelude(src: &str, name: &str, isa: isc_codegen::Isa) -> PathBuf {
    let p = iscc::frontend_as_prelude(&format!("{}.isc", name), src).unwrap_or_else(|e| panic!("{}", e));
    let mut o = isc_codegen::Options::default();
    o.isa = isa;
    let out = tmp(if isa.is_wasm() { format!("{}.wasm", name) } else { name.to_string() }.as_str());
    let opts = iscc::BuildOptions { output: out.clone(), emit_llvm: None, emit_asm: None, debug: false, codegen: o, link: Vec::new(), glibc: false };
    iscc::build(&p, &opts).unwrap_or_else(|e| panic!("{}: {}", name, e));
    out
}

/// How a program of the bits is run.
enum Runner<'a> {
    Native(&'a Path),
    Wasm(&'a Path, &'a Path),
    Interp(&'a isc_ir::Program),
}

/// The bits of the results of the function `fi` (0 to 19) on these inputs.
fn run_bits(how: &Runner, fi: usize, x: &str, y: &str, tag: &str) -> Vec<u64> {
    let out = tmp(&format!("math_bits_{}_{}.out", tag, fi)).to_string_lossy().to_string();
    let args = [fi.to_string(), x.to_string(), y.to_string(), out.clone()];
    let args: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    match how {
        Runner::Native(bin) => {
            let r = run_bin_threads(bin, &args, Some(4));
            assert_eq!(r.code, 0, "{}", r.stderr);
        }
        Runner::Wasm(runner, wasm) => {
            let r = run_wasm(runner, wasm, &args);
            assert_eq!(r.code, 0, "{}", r.stderr);
        }
        Runner::Interp(p) => {
            let owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();
            let mut sink = Vec::new();
            assert_eq!(isc_interp::run(p, &owned, &mut sink).unwrap_or_else(|e| panic!("{}", e)), 0);
        }
    }
    let b = std::fs::read(&out).unwrap();
    if fi < 10 {
        b.chunks_exact(8).map(|c| u64::from_le_bytes(c.try_into().unwrap())).collect()
    } else {
        b.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap()) as u64).collect()
    }
}

/// The inputs of `fi` whose results differ from libm's (the first ones
/// printed); `xs`, `ys` as bits.
fn differences(fi: usize, xs: &[u64], ys: &[u64], got: &[u64], tag: &str) -> usize {
    assert_eq!(xs.len(), got.len(), "{} {}: the count of the results", tag, fi);
    let mut bad = 0;
    for i in 0..xs.len() {
        let want = if fi < 10 { libm64(fi, f64::from_bits(xs[i]), f64::from_bits(ys[i])).to_bits() } else { libm32(fi - 10, f32::from_bits(xs[i] as u32), f32::from_bits(ys[i] as u32)).to_bits() as u64 };
        if want != got[i] {
            bad += 1;
            if bad <= 8 {
                eprintln!("{} {}{}({:#x}, {:#x}): {:#x}, libm {:#x}", tag, NAMES[fi % 10], if fi < 10 { "" } else { "f" }, xs[i], ys[i], got[i], want);
            }
        }
    }
    bad
}

/// Each function on `n` inputs (`inputs64`, `inputs32`; for the one of one
/// `f32`, its special values and `n` by exponent), run each way of `ways`:
/// the count of the inputs compared, and of those that differ.
fn compare(ways: &[(&str, Runner)], n: usize, seed: u64) -> (usize, usize) {
    let (mut count, mut bad) = (0, 0);
    for fi in 0..20 {
        let (xb, yb, x, y): (Vec<u64>, Vec<u64>, String, String) = if fi < 10 {
            let (xs, ys) = inputs64(fi, n, seed);
            let (x, y) = (write_bits64(&format!("math_x_{}_{}", seed, fi), &xs), write_bits64(&format!("math_y_{}_{}", seed, fi), &ys));
            (xs.iter().map(|v| v.to_bits()).collect(), ys.iter().map(|v| v.to_bits()).collect(), x, y)
        } else {
            let (mut xs, mut ys) = inputs32(fi - 10, n, seed);
            if fi < 18 {
                let mut r = Rng(seed + fi as u64);
                xs.truncate(specials32().len());
                while xs.len() < n {
                    xs.push(by_exponent32(&mut r, 0, 255));
                }
                ys = vec![0.0; xs.len()];
            }
            let (x, y) = (write_bits32(&format!("math_x_{}_{}", seed, fi), &xs), write_bits32(&format!("math_y_{}_{}", seed, fi), &ys));
            (xs.iter().map(|v| v.to_bits() as u64).collect(), ys.iter().map(|v| v.to_bits() as u64).collect(), x, y)
        };
        for (tag, how) in ways {
            let got = run_bits(how, fi, &x, &y, tag);
            let d = differences(fi, &xb, &yb, &got, tag);
            count += xb.len();
            bad += d;
            eprintln!("{}: {}{} {} inputs, {} different", tag, NAMES[fi % 10], if fi < 10 { "" } else { "f" }, xb.len(), d);
        }
    }
    (count, bad)
}

/// The suite's share: every function on its special values, the arguments
/// of the hardest reductions and inputs by exponent, natively and as
/// WebAssembly; interpreted node by node on fewer.
#[test]
fn math_bits_as_libm() {
    let bin = build_prelude(BITS, "math_bits", isc_codegen::Isa::Native);
    let (count, bad) = compare(&[("native", Runner::Native(&bin))], 30_000, 1);
    assert_eq!(bad, 0, "{} of {} inputs differ natively", bad, count);
    let p = iscc::frontend_as_prelude("math_bits.isc", BITS).unwrap();
    let (count, bad) = compare(&[("interpreted", Runner::Interp(&p))], 300, 2);
    assert_eq!(bad, 0, "{} of {} inputs differ interpreted", bad, count);
    match wasi_runner() {
        Some(runner) => {
            let wasm = build_prelude(BITS, "math_bits", isc_codegen::Isa::Wasm32);
            let (count, bad) = compare(&[("wasm", Runner::Wasm(&runner, &wasm))], 30_000, 3);
            assert_eq!(bad, 0, "{} of {} inputs differ as WebAssembly", bad, count);
        }
        None => eprintln!("no WASI engine (wasmtime): not run as WebAssembly"),
    }
}

/// libm's large reduction in `$mRpGeneral` (its loop, line by line) gives
/// what the common cases of `$mRp` give: the general path is run here on
/// every argument, not only on the rare ones that need it.
#[test]
fn math_large_reduction_general_path() {
    const SRC: &str = "action main(args: string[k]): i32 {
    const xs = readF64(args[0]);
    const r = map(xs, (x) => {
        const b = $floatBits(x);
        const m = (b & 0x000FFFFFFFFFFFFFu64) | 0x0010000000000000u64;
        const e0 = i32((b >> 52u64) & 0x7FFu64) - 1046i32;
        const t0 = i64(m >> 29u64);
        const t1 = i64((m >> 5u64) & 0xFFFFFFu64);
        const t2 = i64((m & 31u64) << 19u64);
        const jv0 = i64(e0 - 3i32) / 24;
        const jv = jv0 < 0 ? 0 : jv0;
        const q0 = i64(e0) - 24 * (jv + 1);
        const [n, y0, y1] = $mRp(t0, t1, t2, e0, 4, true);
        const [m2, z0, z1] = $mRpGeneral(t0, t1, t2, jv, q0, 4, true);
        const t = m & 0xFFFFFFu64;
        const [n3, w0, w1] = $mRp(i64(t | 0x800000u64), 0, 0, e0 + 29i32, 3, false);
        const [m3, v0, v1] = $mRpGeneral(i64(t | 0x800000u64), 0, 0, (i64(e0 + 26i32) / 24 < 0 ? 0 : i64(e0 + 26i32) / 24), i64(e0 + 29i32) - 24 * ((i64(e0 + 26i32) / 24 < 0 ? 0 : i64(e0 + 26i32) / 24) + 1), 3, false);
        return (n == m2 && $floatBits(y0) == $floatBits(z0) && $floatBits(y1) == $floatBits(z1) ? 0 : 1) + (n3 == m3 && $floatBits(w0) == $floatBits(v0) ? 0 : 2);
    });
    print(sum(r));
    print(r.length);
    return 0;
}
";
    let mut rng = Rng(77);
    let mut xs: Vec<f64> = (0..20_000).map(|_| f64::abs(by_exponent64(&mut rng, 1043, 2046))).collect();
    xs.extend(hard64().into_iter().map(f64::abs).filter(|x| *x >= 1.6470993291652855e6));
    let path = write_bits64("math_general.f64", &xs);
    let bin = build_prelude(SRC, "math_general", isc_codegen::Isa::Native);
    let r = run_bin(&bin, &[&path]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(r.stdout, format!("0\n{}\n", xs.len()), "the general path differs from the common ones");
}

/// A program of the language: compiled, interpreted node by node (the
/// prelude's code) and under `iscc run` (the native forms), the same text.
const VALUES: &str = include_str!("data/math_values.isc");

#[test]
fn math_run_and_interpreter_as_compiled() {
    let compiled = run_bin(&compile(VALUES, "math_values"), &[]);
    assert_eq!(compiled.code, 0, "{}", compiled.stderr);
    assert_eq!(interp_ok(VALUES, &[]), compiled.stdout, "interpreted node by node");
    let p = iscc::frontend_for_run("math_values.isc", VALUES).unwrap();
    let mut out = Vec::new();
    assert_eq!(isc_interp::run(&p, &[], &mut out).unwrap(), 0);
    assert_eq!(String::from_utf8(out).unwrap(), compiled.stdout, "under iscc run");
    // (the functions are called natively there: their names in the program)
    let native = p.fns.iter().filter(|f| f.name.starts_with(isc_interp::NATIVE)).count();
    assert!(native >= 18, "{} functions in a native form", native);
}

/// A program may name its own functions as those the prelude calls: the
/// mathematical functions of the prelude call `Math.abs` and `Math.sqrt`,
/// which a program cannot declare again (the whole prelude is checked
/// with each program: a call of `abs` there would be the C function of
/// this one).
#[test]
fn math_with_a_program_that_names_abs_and_sqrt() {
    const SRC: &str = "declare function abs(v: i32): i32;

function sqrt(x: i64): i64 {
    return x / 2;
}

action main(args: string[k]): i32 {
    const x = f64(abs(-3i32)) + 0.25;
    print([pow(-x, 3.0), asin(x / 4.0), acos(-x / 4.0), atan2(x, -1.0), f64(pow(f32(x), 0.5f32)), f64(sqrt(10))]);
    return 0;
}
";
    let r = run_bin(&compile(SRC, "math_names"), &[]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    // (the output of the compiler of before, the runtime's `libm`)
    assert_eq!(r.stdout, "[-34.328125, 0.948427838239876, 2.519224165034773, 1.869295258381076, 1.8027756214141846, 5]\n");
}

/// The generated code calls no function of the runtime for them, and a
/// function that relaxes its arithmetic (`@allowReassoc`) relaxes none of
/// their operations: the same bits inside it as outside.
#[test]
fn math_code_and_reassoc() {
    const SRC: &str = "@allowReassoc
function relaxed(k: i64, x: f64): f64 {
    return k == 0 ? exp(x) : k == 1 ? sin(x) : k == 2 ? cos(x) : k == 3 ? pow(x, 1.5) : k == 4 ? log(x) : k == 5 ? tan(x) : k == 6 ? atan2(x, 2.0) : k == 7 ? asin(x / 8.0) : k == 8 ? acos(x / 8.0) : k == 9 ? atan(x)
        : k == 10 ? f64(exp(f32(x))) : k == 11 ? f64(sin(f32(x))) : k == 12 ? f64(pow(f32(x), 1.5f32)) : f64(atan2(f32(x), -2.0f32));
}

function strict(k: i64, x: f64): f64 {
    return k == 0 ? exp(x) : k == 1 ? sin(x) : k == 2 ? cos(x) : k == 3 ? pow(x, 1.5) : k == 4 ? log(x) : k == 5 ? tan(x) : k == 6 ? atan2(x, 2.0) : k == 7 ? asin(x / 8.0) : k == 8 ? acos(x / 8.0) : k == 9 ? atan(x)
        : k == 10 ? f64(exp(f32(x))) : k == 11 ? f64(sin(f32(x))) : k == 12 ? f64(pow(f32(x), 1.5f32)) : f64(atan2(f32(x), -2.0f32));
}

action main(args: string[k]): i32 {
    const xs = map(iota(2000), (i) => f64(i) * 0.003 + 0.001);
    print(sum(map(iota(14), (k) => map(xs, (x) => relaxed(k, x) == strict(k, x) ? 1 : 0).sum())));
    return 0;
}
";
    let bin = compile(SRC, "math_code");
    let r = run_bin(&bin, &[]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(r.stdout, "28000\n", "relaxed and strict differ");
    let ll = std::fs::read_to_string(bin.with_extension("ll")).unwrap();
    for gone in ["@isc_exp", "@isc_log", "@isc_sin", "@isc_cos", "@isc_tan", "@isc_asin", "@isc_acos", "@isc_atan", "@isc_pow"] {
        assert!(!ll.contains(gone), "{} called", gone);
    }
}

/// Every input of each function of one `f32` (2^32), natively and as
/// WebAssembly, by sums of blocks of 2^20 against libm's (a block that
/// differs is then run input by input); 10^7 inputs of each function of
/// `f64` and of `atan2` and `pow` of `f32`, natively and as WebAssembly;
/// 3000 of each interpreted node by node. Its counts are printed.
#[test]
#[ignore]
fn math_exhaustive() {
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(4);
    let native = build_prelude(SUMS32, "math_sums32", isc_codegen::Isa::Native);
    let runner = wasi_runner();
    let wasm = runner.as_ref().map(|_| build_prelude(SUMS32, "math_sums32", isc_codegen::Isa::Wasm32Threads));
    let mut total = 0u64;
    for f in 0..8 {
        // libm's sums, on the threads of this test
        let want: Vec<u64> = std::thread::scope(|s| {
            let hs: Vec<_> = (0..threads as u64).map(|t| s.spawn(move || (0..4096u64).filter(|b| b % threads as u64 == t).map(|b| (b, block_sum32(f, b))).collect::<Vec<_>>())).collect();
            let mut v = vec![0u64; 4096];
            for h in hs {
                for (b, x) in h.join().unwrap() {
                    v[b as usize] = x;
                }
            }
            v
        });
        let mut ways: Vec<(&str, Vec<u64>)> = Vec::new();
        let r = run_bin_threads(&native, &[&f.to_string(), "0", "4096"], Some(threads));
        assert_eq!(r.code, 0, "{}", r.stderr);
        ways.push(("native", parse_list(&r.stdout)));
        if let (Some(runner), Some(wasm)) = (&runner, &wasm) {
            let r = run_wasm_threads(runner, wasm, &[&f.to_string(), "0", "4096"], Some(threads));
            assert_eq!(r.code, 0, "{}", r.stderr);
            ways.push(("wasm", parse_list(&r.stdout)));
        }
        for (tag, got) in &ways {
            let bad: Vec<usize> = (0..4096).filter(|&b| got[b] != want[b]).collect();
            eprintln!("{}: {}f, 2^32 inputs, {} blocks of 2^20 different", tag, NAMES[f], bad.len());
            // the inputs that differ in the first blocks, one by one (natively)
            for &b in bad.iter().take(2) {
                let xs: Vec<f32> = (0..1u32 << 20).map(|i| f32::from_bits((b as u32) << 20 | i)).collect();
                let (x, y) = (write_bits32("math_block_x", &xs), write_bits32("math_block_y", &vec![0.0; xs.len()]));
                let bin = build_prelude(BITS, "math_bits", isc_codegen::Isa::Native);
                let got = run_bits(&Runner::Native(&bin), 10 + f, &x, &y, "block");
                let xb: Vec<u64> = xs.iter().map(|v| v.to_bits() as u64).collect();
                differences(10 + f, &xb, &vec![0; xb.len()], &got, tag);
            }
            assert!(bad.is_empty(), "{}: {}f differs in blocks {:?}", tag, NAMES[f], &bad[..bad.len().min(8)]);
            total += 1 << 32;
        }
    }
    let bin = build_prelude(BITS, "math_bits", isc_codegen::Isa::Native);
    let mut ways = vec![("native", Runner::Native(&bin))];
    let wasm_bits = runner.as_ref().map(|_| build_prelude(BITS, "math_bits", isc_codegen::Isa::Wasm32));
    if let (Some(runner), Some(w)) = (&runner, &wasm_bits) {
        ways.push(("wasm", Runner::Wasm(runner, w)));
    }
    let (count, bad) = compare(&ways, 10_000_000, 9);
    eprintln!("f64 and two f32: {} inputs compared, {} different", count, bad);
    assert_eq!(bad, 0);
    let p = iscc::frontend_as_prelude("math_bits.isc", BITS).unwrap();
    let (icount, ibad) = compare(&[("interpreted", Runner::Interp(&p))], 3000, 10);
    eprintln!("interpreted: {} inputs compared, {} different", icount, ibad);
    assert_eq!(ibad, 0);
    eprintln!("one f32: {} inputs compared", total);
}

fn parse_list(s: &str) -> Vec<u64> {
    let t = s.trim().trim_start_matches('[').trim_end_matches(']');
    t.split(", ").map(|v| v.parse::<u64>().unwrap_or_else(|_| panic!("{}", v))).collect()
}
