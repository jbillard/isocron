//! Differential testing (SPEC section 15): random well-typed programs,
//! compiled and interpreted, must print exactly the same thing.

mod common;
use common::*;

struct G {
    r: Rng,
}

impl G {
    fn pick(&mut self, n: u64) -> u64 {
        self.r.next() % n
    }

    fn flit(&mut self) -> String {
        let choices = ["1.25", "-3.0", "0.001", "2.0", "0.5", "1e3", "-0.75", "7.0"];
        choices[self.pick(choices.len() as u64) as usize].to_string()
    }

    fn ilit(&mut self) -> String {
        let choices = ["3", "-7", "1000", "12", "-1", "123456789"];
        choices[self.pick(choices.len() as u64) as usize].to_string()
    }

    /// An element-wise function body over `v: f64`.
    fn fbody(&mut self, v: &str) -> String {
        let c = self.flit();
        match self.pick(7) {
            0 => format!("{v} * {v}"),
            1 => format!("{v} + {c}"),
            2 => format!("{v} > {c} ? {v} : {c} * {v}"),
            3 => format!("sqrt({v} * {v} + 1.0)"),
            4 => format!("-{v}"),
            5 => format!("{v} ** 3"),
            _ => format!("{v} / {c} - {v}"),
        }
    }

    fn farr(&mut self, d: u32) -> String {
        if d == 0 {
            return ["a", "b"][self.pick(2) as usize].to_string();
        }
        match self.pick(9) {
            0 => format!("({} + {})", self.farr(d - 1), self.farr(d - 1)),
            1 => format!("({} - {})", self.farr(d - 1), self.farr(d - 1)),
            2 => format!("({} * {})", self.farr(d - 1), self.farr(d - 1)),
            3 => format!("({} * {})", self.farr(d - 1), self.fsc(d - 1)),
            4 => format!("({} - {})", self.fsc(d - 1), self.farr(d - 1)),
            5 => {
                let a = self.farr(d - 1);
                let b = self.fbody("v");
                format!("map({}, (v) => {})", a, b)
            }
            6 => format!("({} ** 2)", self.farr(d - 1)),
            7 => format!("(-{})", self.farr(d - 1)),
            _ => format!("map(u, (c) => f64(c) * {})", self.flit()),
        }
    }

    fn op(&mut self) -> String {
        ["(+)", "(p, q) => p * 0.5 + q", "(p, q) => p > q ? p : q", "(p, q) => p - q", "(*)"]
            [self.pick(5) as usize]
            .to_string()
    }

    fn fsc(&mut self, d: u32) -> String {
        if d == 0 {
            return match self.pick(3) {
                0 => self.flit(),
                1 => format!("a[{}]", self.pick(32)),
                _ => format!("b[{}]", self.pick(32)),
            };
        }
        match self.pick(12) {
            0 => format!("sum({})", self.farr(d - 1)),
            1 => format!("dot({}, {})", self.farr(d - 1), self.farr(d - 1)),
            2 => format!("mean({})", self.farr(d - 1)),
            3 => {
                let a = self.farr(d - 1);
                let o = self.op();
                let e = self.flit();
                format!("reduce({}, {}, {})", a, o, e)
            }
            4 => format!("fold({}, {}, (acc, v) => acc * 0.5 + v)", self.farr(d - 1), self.fsc(d - 1)),
            5 => format!("({} + {})", self.fsc(d - 1), self.fsc(d - 1)),
            6 => format!("({} * {})", self.fsc(d - 1), self.fsc(d - 1)),
            7 => format!("({} / {})", self.fsc(d - 1), self.fsc(d - 1)),
            8 => format!("sqrt(({}) ** 2)", self.fsc(d - 1)),
            9 => format!("f64({})", self.isc(d - 1)),
            10 => format!("({} ? {} : {})", self.bsc(d - 1), self.fsc(d - 1), self.fsc(d - 1)),
            _ => format!("{}[{}]", self.farr(d - 1), self.pick(32)),
        }
    }

    fn isc(&mut self, d: u32) -> String {
        if d == 0 {
            return match self.pick(2) {
                0 => self.ilit(),
                _ => format!("i64(u[{}])", self.pick(32)),
            };
        }
        match self.pick(9) {
            0 => "sum(map(u, (c) => i64(c)))".to_string(),
            1 => format!("i64({})", self.fsc(d - 1)),
            2 => format!("({} + {})", self.isc(d - 1), self.isc(d - 1)),
            3 => format!("({} * {})", self.isc(d - 1), self.isc(d - 1)),
            4 => format!("({} / {})", self.isc(d - 1), ["3", "-2", "7"][self.pick(3) as usize]),
            5 => format!("({} % {})", self.isc(d - 1), ["3", "-2", "7"][self.pick(3) as usize]),
            6 => "reduce(map(u, (c) => i64(c) * 3 - 100), (p, q) => p - q, 0)".to_string(),
            7 => "fold(u, 0, (acc, c) => acc * 31 + i64(c))".to_string(),
            _ => format!("i64(fold(u, fill(8, 0u32), (h, c) => h.with(c % 8u8, h[c % 8u8] + 1u32))[{}])", self.pick(8)),
        }
    }

    fn bsc(&mut self, d: u32) -> String {
        if d == 0 {
            return ["true", "false"][self.pick(2) as usize].to_string();
        }
        match self.pick(5) {
            0 => format!("({} < {})", self.fsc(d - 1), self.fsc(d - 1)),
            1 => format!("({} == {})", self.isc(d - 1), self.isc(d - 1)),
            2 => format!("(!{})", self.bsc(d - 1)),
            3 => format!("({} && {})", self.bsc(d - 1), self.bsc(d - 1)),
            _ => format!("({} == {})", self.farr(d - 1), self.farr(d - 1)),
        }
    }

    fn program(&mut self) -> String {
        let mut s = String::from(
            "action main(args: string[k]): i32 {\n    const a = read_f64(args[0]);\n    const b = read_f64(args[1]);\n    const u = read_bytes(args[2]);\n",
        );
        for _ in 0..8 {
            let d = 1 + self.pick(3) as u32;
            let e = match self.pick(4) {
                0 => self.isc(d),
                1 => self.bsc(d),
                _ => self.fsc(d),
            };
            s.push_str(&format!("    print({});\n", e));
        }
        s.push_str("    return 0;\n}\n");
        s
    }
}

#[test]
fn random_programs_agree() {
    // ISC_DIFF_PROGRAMS and ISC_DIFF_SEED widen the campaign.
    let seed = std::env::var("ISC_DIFF_SEED").ok().and_then(|s| s.parse().ok()).unwrap_or(0x5EED);
    let mut g = G { r: Rng(seed) };
    let mut inputs = Vec::new();
    for n in [33usize, 5000] {
        let a = random_f64s(n as u64, n);
        let b = random_f64s(n as u64 + 1, n);
        let mut r = Rng(n as u64 + 2);
        let u: Vec<u8> = (0..n).map(|_| (r.next() % 256) as u8).collect();
        let pa = write_f64s(&format!("d_a{}.f64", n), &a);
        let pb = write_f64s(&format!("d_b{}.f64", n), &b);
        let pu = tmp(&format!("d_u{}.u8", n));
        std::fs::write(&pu, &u).unwrap();
        inputs.push((pa, pb, pu.to_string_lossy().to_string()));
    }
    let programs: usize = std::env::var("ISC_DIFF_PROGRAMS").ok().and_then(|s| s.parse().ok()).unwrap_or(30);
    for i in 0..programs {
        let src = g.program();
        let bin = compile(&src, &format!("d_prog{}", i));
        for (pa, pb, pu) in &inputs {
            let args = [pa.as_str(), pb.as_str(), pu.as_str()];
            // with one thread and with three (blocks shared unevenly)
            let r = run_bin_threads(&bin, &args, Some(1));
            let r3 = run_bin_threads(&bin, &args, Some(3));
            assert_eq!((r.code, &r.stdout, &r.stderr), (r3.code, &r3.stdout, &r3.stderr), "program {} depends on the number of threads:\n{}", i, src);
            match interp(&src, &args) {
                Outcome::Exit(code, out) => {
                    assert_eq!((r.code, &r.stdout), (code, &out), "program {}:\n{}\nstderr: {}", i, src, r.stderr)
                }
                Outcome::Panic(m, out) => {
                    assert_eq!(r.code, 101, "program {}:\n{}", i, src);
                    assert_eq!((r.stderr.trim_end(), r.stdout.as_str()), (m.as_str(), out.as_str()), "program {}:\n{}", i, src);
                }
            }
        }
    }
}
