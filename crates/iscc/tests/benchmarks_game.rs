//! The ten programs of the Benchmarks Game (examples/benchmarks_game,
//! D160) against independent implementations written here in Rust, at
//! small sizes, in the interpreter and compiled, and under Valgrind. The
//! programs that read a FASTA file read the output of our `fasta`. Their
//! reference outputs (bench/benchmarks_game, in Docker) are the other
//! check, at the sizes of the Benchmarks Game.

mod common;
use common::*;
use std::collections::HashMap;
use std::io::Write as _;

fn source(name: &str) -> String {
    let path = format!("{}/../../examples/benchmarks_game/{}.isc", env!("CARGO_MANIFEST_DIR"), name);
    std::fs::read_to_string(path).unwrap()
}

fn run(bin: &std::path::Path, args: &[&str], input: &[u8]) -> Vec<u8> {
    let mut c = std::process::Command::new(bin)
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    c.stdin.take().unwrap().write_all(input).unwrap();
    let o = c.wait_with_output().unwrap();
    assert_eq!(o.status.code(), Some(0), "{:?}", bin);
    o.stdout
}

fn valgrind_clean(bin: &std::path::Path, args: &[&str], input: &[u8]) {
    let vg = std::path::PathBuf::from("/usr/bin/valgrind");
    if !vg.is_file() {
        return;
    }
    let mut c = std::process::Command::new(vg)
        .args(["--error-exitcode=9", "--leak-check=full", "--errors-for-leak-kinds=definite,indirect", "-q"])
        .arg(bin)
        .args(args)
        .env("ISC_ALLOC", "glibc")
        .env("ISC_THREADS", "2")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    c.stdin.take().unwrap().write_all(input).unwrap();
    let o = c.wait_with_output().unwrap();
    assert_eq!(o.status.code(), Some(0), "{:?} under valgrind: {}", bin, String::from_utf8_lossy(&o.stderr));
}

/// The program `name` with `arg` and `input`: the interpreter, the
/// compiled program, Valgrind; its output.
fn output(name: &str, arg: &str, input: &[u8]) -> Vec<u8> {
    let src = source(name);
    let p = iscc::frontend(&format!("{}.isc", name), &src).unwrap();
    *isc_interp::rt::STDIN.lock().unwrap() = Some(input.to_vec());
    let mut out = Vec::new();
    assert_eq!(isc_interp::run(&p, &[arg.to_string()], &mut out).unwrap(), 0, "{}", name);
    let opts = isc_codegen::Options { isa: isc_codegen::Isa::Avx2, ..isc_codegen::Options::default() };
    let bin = compile_with(&src, &format!("bg_{}", name), opts.clone());
    assert_eq!(run(&bin, &[arg], input), out, "{}: compiled and interpreted", name);
    // under Valgrind, linked with glibc (its allocator, D289)
    let vg = compile_glibc(&src, &format!("bg_{}_glibc", name), opts);
    assert_eq!(run(&vg, &[arg], input), out, "{}: compiled with glibc and interpreted", name);
    valgrind_clean(&vg, &[arg], input);
    out
}

// ---------------------------------------------------------------------
// the reference implementations

const ALU: &[u8] = b"GGCCGGGCGCGGTGGCTCACGCCTGTAATCCCAGCACTTTGGGAGGCCGAGGCGGGCGGATCACCTGAGGTCAGGAGTTCGAGACCAGCCTGGCCAACATGGTGAAACCCCGTCTCTACTAAAAATACAAAAATTAGCCGGGCGTGGTGGCGCGCGCCTGTAATCCCAGCTACTCGGGAGGCTGAGGCAGGAGAATCGCTTGAACCCGGGAGGCGGAGGTTGCAGTGAGCCGAGATCGCGCCACTGCACTCCAGCCTGGGCGACAGAGCGAGACTCCGTCTCAAAAA";

fn fasta(n: usize) -> Vec<u8> {
    let mut out = b">ONE Homo sapiens alu\n".to_vec();
    let lines = |out: &mut Vec<u8>, m: usize, base: &mut dyn FnMut(usize) -> u8| {
        let mut i = 0;
        while i < m {
            let e = (i + 60).min(m);
            for j in i..e {
                out.push(base(j));
            }
            out.push(b'\n');
            i = e;
        }
    };
    lines(&mut out, 2 * n, &mut |i| ALU[i % ALU.len()]);
    let mut last: i64 = 42;
    let mut random = move || {
        last = (last * 3877 + 29573) % 139968;
        last as f64 / 139968.0
    };
    let mut select = |codes: &[u8], p: &[f64], out: &mut Vec<u8>, m: usize| {
        let mut cp = Vec::new();
        let mut c = 0.0;
        for x in p {
            c += x;
            cp.push(c);
        }
        lines(out, m, &mut |_| {
            let r = random();
            codes[cp.iter().position(|&c| r < c).unwrap_or(codes.len() - 1)]
        });
    };
    out.extend(b">TWO IUB ambiguity codes\n");
    select(b"acgtBDHKMNRSVWY", &[0.27, 0.12, 0.12, 0.27, 0.02, 0.02, 0.02, 0.02, 0.02, 0.02, 0.02, 0.02, 0.02, 0.02, 0.02], &mut out, 3 * n);
    out.extend(b">THREE Homo sapiens frequency\n");
    select(b"acgt", &[0.3029549426680, 0.1979883004921, 0.1975473066391, 0.3015094502008], &mut out, 5 * n);
    out
}

fn records(input: &[u8]) -> Vec<(Vec<u8>, Vec<u8>)> {
    let mut out: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    for line in input.split(|&b| b == b'\n') {
        if line.first() == Some(&b'>') {
            out.push((line.to_vec(), Vec::new()));
        } else if let Some(r) = out.last_mut() {
            r.1.extend_from_slice(line);
        }
    }
    out
}

fn revcomp(input: &[u8]) -> Vec<u8> {
    let from = b"ACGTUMRWSYKVHDBNacgtumrwsykvhdbn";
    let to = b"TGCAAKYWSRMBDHVNTGCAAKYWSRMBDHVN";
    let mut out = Vec::new();
    for (head, seq) in records(input) {
        out.extend(&head);
        out.push(b'\n');
        let rc: Vec<u8> = seq.iter().rev().map(|b| from.iter().position(|f| f == b).map_or(*b, |i| to[i])).collect();
        for chunk in rc.chunks(60) {
            out.extend(chunk);
            out.push(b'\n');
        }
    }
    out
}

fn knucleotide(input: &[u8]) -> String {
    let seq: Vec<u8> = records(input).into_iter().find(|r| r.0.starts_with(b">THREE")).unwrap().1.to_ascii_uppercase();
    let mut out = String::new();
    for k in [1, 2] {
        let mut counts: HashMap<&[u8], usize> = HashMap::new();
        for w in seq.windows(k) {
            *counts.entry(w).or_default() += 1;
        }
        let mut v: Vec<(&[u8], usize)> = counts.into_iter().collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        let total = (seq.len() - k + 1) as f64;
        for (w, c) in v {
            out += &format!("{} {:.3}\n", String::from_utf8_lossy(w), 100.0 * c as f64 / total);
        }
        out += "\n";
    }
    for q in ["GGT", "GGTA", "GGTATT", "GGTATTTTAATT", "GGTATTTTAATTTATAGT"] {
        let c = seq.windows(q.len()).filter(|w| *w == q.as_bytes()).count();
        out += &format!("{}\t{}\n", c, q);
    }
    out
}

fn nbody(n: usize) -> String {
    let pi = 3.141592653589793;
    let sm = 4.0 * pi * pi;
    let d = 365.24;
    let mut b: [[f64; 7]; 5] = [
        [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, sm],
        [4.84143144246472090e+00, -1.16032004402742839e+00, -1.03622044471123109e-01, 1.66007664274403694e-03 * d, 7.69901118419740425e-03 * d, -6.90460016972063023e-05 * d, 9.54791938424326609e-04 * sm],
        [8.34336671824457987e+00, 4.12479856412430479e+00, -4.03523417114321381e-01, -2.76742510726862411e-03 * d, 4.99852801234917238e-03 * d, 2.30417297573763929e-05 * d, 2.85885980666130812e-04 * sm],
        [1.28943695621391310e+01, -1.51111514016986312e+01, -2.23307578892655734e-01, 2.96460137564761618e-03 * d, 2.37847173959480950e-03 * d, -2.96589568540237556e-05 * d, 4.36624404335156298e-05 * sm],
        [1.53796971148509165e+01, -2.59193146099879641e+01, 1.79258772950371181e-01, 2.68067772490389322e-03 * d, 1.62824170038242295e-03 * d, -9.51592254519715870e-05 * d, 5.15138902046611451e-05 * sm],
    ];
    let mut p = [0.0; 3];
    for x in &b {
        for k in 0..3 {
            p[k] += x[3 + k] * x[6];
        }
    }
    for k in 0..3 {
        b[0][3 + k] = -p[k] / sm;
    }
    let energy = |b: &[[f64; 7]; 5]| {
        let mut e = 0.0;
        for i in 0..5 {
            e += 0.5 * b[i][6] * (b[i][3] * b[i][3] + b[i][4] * b[i][4] + b[i][5] * b[i][5]);
            for j in i + 1..5 {
                let (dx, dy, dz) = (b[i][0] - b[j][0], b[i][1] - b[j][1], b[i][2] - b[j][2]);
                e -= b[i][6] * b[j][6] / (dx * dx + dy * dy + dz * dz).sqrt();
            }
        }
        e
    };
    let mut out = format!("{:.9}\n", energy(&b));
    for _ in 0..n {
        for i in 0..5 {
            for j in i + 1..5 {
                let (dx, dy, dz) = (b[i][0] - b[j][0], b[i][1] - b[j][1], b[i][2] - b[j][2]);
                let dist = (dx * dx + dy * dy + dz * dz).sqrt();
                let mag = 0.01 / (dist * dist * dist);
                let (mi, mj) = (b[i][6], b[j][6]);
                b[i][3] -= dx * mj * mag;
                b[i][4] -= dy * mj * mag;
                b[i][5] -= dz * mj * mag;
                b[j][3] += dx * mi * mag;
                b[j][4] += dy * mi * mag;
                b[j][5] += dz * mi * mag;
            }
        }
        for x in b.iter_mut() {
            for k in 0..3 {
                x[k] += 0.01 * x[3 + k];
            }
        }
    }
    out += &format!("{:.9}\n", energy(&b));
    out
}

fn spectralnorm(n: usize) -> String {
    let a = |i: usize, j: usize| 1.0 / (((i + j) * (i + j + 1) / 2 + i + 1) as f64);
    let av = |u: &[f64], t: bool| -> Vec<f64> { (0..n).map(|i| (0..n).map(|j| if t { a(j, i) } else { a(i, j) } * u[j]).sum()).collect() };
    let ata = |u: &[f64]| av(&av(u, false), true);
    let mut u = vec![1.0; n];
    let mut v = Vec::new();
    for _ in 0..10 {
        v = ata(&u);
        u = ata(&v);
    }
    let vbv: f64 = u.iter().zip(&v).map(|(a, b)| a * b).sum();
    let vv: f64 = v.iter().map(|x| x * x).sum();
    format!("{:.9}\n", (vbv / vv).sqrt())
}

fn fannkuch(n: usize) -> String {
    let mut perm1: Vec<usize> = (0..n).collect();
    let mut count = vec![0usize; n];
    let (mut r, mut perm_count, mut checksum, mut max_flips) = (n, 0i64, 0i64, 0i64);
    loop {
        while r != 1 {
            count[r - 1] = r;
            r -= 1;
        }
        let mut perm = perm1.clone();
        let mut flips = 0i64;
        while perm[0] != 0 {
            let k = perm[0];
            perm[..=k].reverse();
            flips += 1;
        }
        max_flips = max_flips.max(flips);
        checksum += if perm_count % 2 == 0 { flips } else { -flips };
        loop {
            if r == n {
                return format!("{}\nPfannkuchen({}) = {}\n", checksum, n, max_flips);
            }
            let p0 = perm1[0];
            for i in 0..r {
                perm1[i] = perm1[i + 1];
            }
            perm1[r] = p0;
            count[r] -= 1;
            if count[r] > 0 {
                break;
            }
            r += 1;
        }
        perm_count += 1;
    }
}

fn binarytrees(n: i64) -> String {
    let nodes = |d: i64| (1i64 << (d + 1)) - 1;
    let max = n.max(6);
    let mut out = format!("stretch tree of depth {}\t check: {}\n", max + 1, nodes(max + 1));
    let mut d = 4;
    while d <= max {
        let it = 1i64 << (max - d + 4);
        out += &format!("{}\t trees of depth {}\t check: {}\n", it, d, it * nodes(d));
        d += 2;
    }
    out += &format!("long lived tree of depth {}\t check: {}\n", max, nodes(max));
    out
}

// ---------------------------------------------------------------------

#[test]
fn computing_programs() {
    assert_eq!(String::from_utf8(output("binarytrees", "8", b"")).unwrap(), binarytrees(8));
    assert_eq!(String::from_utf8(output("fannkuchredux", "7", b"")).unwrap(), fannkuch(7));
    assert_eq!(String::from_utf8(output("nbody", "1000", b"")).unwrap(), nbody(1000));
    assert_eq!(String::from_utf8(output("spectralnorm", "100", b"")).unwrap(), spectralnorm(100));
    // the first 27 digits of pi, 10 per line
    assert_eq!(String::from_utf8(output("pidigits", "27", b"")).unwrap(), "3141592653\t:10\n5897932384\t:20\n6264338   \t:27\n");
}

#[test]
fn fasta_programs() {
    // one test: the standard input of the interpreter is a global
    let dna = output("fasta", "1000", b"");
    assert!(dna == fasta(1000), "fasta");
    assert!(output("revcomp", "0", &dna) == revcomp(&dna), "revcomp");
    assert_eq!(String::from_utf8(output("knucleotide", "0", &dna)).unwrap(), knucleotide(&dna));
    // regexredux: the interpreter and the compiled program agree; the
    // lengths of the input and of the cleaned text
    let out = String::from_utf8(output("regexredux", "0", &dna)).unwrap();
    let cleaned: usize = records(&dna).iter().map(|r| r.1.len()).sum();
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 13, "{}", out);
    assert_eq!(lines[9..12], ["".to_string(), dna.len().to_string(), cleaned.to_string()], "{}", out);
}
