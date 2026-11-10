//! The memory of a loop does not grow with its rounds: a family of
//! functions at a time (the pads, `join`, the cases, `toFixed`, the texts
//! of floats, the reading of floats with texts refused, arrays of floats,
//! `split`, `lines` and `replace`, a sort of texts, a text grown), each
//! making its values, reading them and freeing them a round. Since the
//! allocator of the generated code and the one of the runtime live side by
//! side (D605), a block that one makes and the other frees must come back
//! to the one that made it: the first wiring let it go (1.2 GiB against 11
//! MiB), with every output right and the suite green. Built by that
//! compiler, five of these ten loops grow tenfold between 60 and 600
//! rounds (the sort: 41 784 to 379 772 KiB); the bound below refuses them.

mod common;
use common::*;

const LOOPS: &str = "function third(i: i64): f64 {
    return (i % 2 == 0 ? 1.0 : -1.0) * f64(i * i * 7 + 1) / 3.0;
}

// one round of a family: its values made, read, freed
action round(kind: string, r: i64, xs: f64[n], ws: string[m]): i64 {
    if (kind == \"pad\") {
        const ts = map(iota(2000), (i) => toString(i + r).padStart(12, \"*\"));
        return sum(map(ts, (t) => t.length)) + ts[r % 2000].length;
    }
    if (kind == \"join\") {
        const ts = map(iota(2000), (i) => toString(i * 7 + r));
        return join(ts, \";\").length + ts[r % 2000].length;
    }
    if (kind == \"case\") {
        const ts = map(ws, (w) => w.toUpperCase() + toString(r));
        return sum(map(ts, (t) => t.length)) + ts[r % 2000].length;
    }
    if (kind == \"fixed\") {
        const ts = map(xs, (x) => (x + f64(r)).toFixed(3));
        return sum(map(ts, (t) => t.length)) + ts[r % 4096].length;
    }
    if (kind == \"float\") {
        const ts = map(xs, (x) => toString(x + f64(r)));
        return sum(map(ts, (t) => t.length)) + ts[r % 4096].length;
    }
    if (kind == \"parse\") {
        const ts = map(iota(2000), (i) => i % 3 == 0 ? \"x\" + toString(i) : toString(third(i + r)));
        return i64(sum(map(ts, (t) => match (tryParseF64(t)) {
            Ok(v) => v > 0.0 ? 1.0 : 2.0,
            Err(e) => f64(e.length),
        })));
    }
    if (kind == \"floor\") {
        const ys = map(xs, (x) => floor(x + f64(r)));
        return (ys[r % 4096] > 0.0 ? 1 : 2) + ys.length;
    }
    if (kind == \"split\") {
        const t = join(map(iota(500), (i) => toString(i + r)), \",\");
        const parts = split(t, \",\");
        const ls = lines(replace(t, \",\", \"\\n\"));
        return parts.length + ls.length + parts[r % 500].length;
    }
    if (kind == \"sort\") {
        const ts = sortBy(map(iota(5000), (i) => toString(i * 7 + r)), (t) => 0 - t.length);
        return ts[r % 5000].length;
    }
    if (kind == \"concat\") {
        return fold(iota(300), \"\", (acc, i) => acc + toString(i + r)).length;
    }
    return 0;
}

action main(args: string[k]): i32 {
    const kind = args[0];
    const count = parseI64(args[1]);
    const xs = map(iota(4096), (i) => third(i));
    const ws = map(iota(2000), (i) => \"word\" + toString(i));
    const total = sum(map(iota(count), (r) => round(kind, r, xs, ws)));
    const status = lines(readText(\"/proc/self/status\"));
    print(filter(status, (l) => l.startsWith(\"VmHWM\")));
    print(total > 0 ? 1 : 0);
    return 0;
}
";

/// The most memory of the program (`VmHWM`, in KiB) as it prints it itself.
fn most(bin: &std::path::Path, kind: &str, rounds: usize) -> u64 {
    let r = run_bin_threads(bin, &[kind, &rounds.to_string()], Some(4));
    assert_eq!(r.code, 0, "{} {}: {}", kind, rounds, r.stderr);
    let mut lines = r.stdout.lines();
    let first = lines.next().unwrap_or("");
    assert_eq!(lines.next(), Some("1"), "{} {}: {}", kind, rounds, r.stdout);
    let digits: String = first.chars().filter(|c| c.is_ascii_digit()).collect();
    assert!(first.contains("VmHWM") && !digits.is_empty(), "{} {}: {}", kind, rounds, first);
    digits.parse().unwrap()
}

#[test]
fn the_memory_of_a_loop_does_not_grow_with_its_rounds() {
    // (under Valgrind the memory of a program is Valgrind's; the allocator
    // that checks, `ISC_ALLOC=check`, keeps the blocks freed in a quarantine
    // of 64 MiB, which arrays of 32 KiB fill in 2000 rounds: 5 MiB after
    // 60 rounds, 26 after 600, 81 after 3000 and after 12 000. It has no
    // arena and no second allocator: not the case this test is for)
    if memcheck() || std::env::var("ISC_ALLOC").is_ok_and(|v| v == "check") {
        return;
    }
    // linked with the module of the prelude compiled once (D611), and whole
    for (bin, how) in [(compile(LOOPS, "bounded_loops"), "with the module"), (compile_whole(LOOPS, "bounded_loops_whole"), "whole")] {
        for kind in ["pad", "join", "case", "fixed", "float", "parse", "floor", "split", "sort", "concat"] {
            let (few, many) = (most(&bin, kind, 60), most(&bin, kind, 600));
            assert!(few > 1000, "{} {}: {} KiB", kind, how, few);
            // (the caches of four threads fill a little more in ten times the rounds)
            assert!(many <= few + few / 7 + 256, "{} {}: {} KiB after 60 rounds, {} after 600", kind, how, few, many);
        }
    }
}
