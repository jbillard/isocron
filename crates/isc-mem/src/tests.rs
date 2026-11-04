//! Every path of the functions against the standard library (the C library
//! of the machine under it): every length up to 600 and some larger ones,
//! at places of every alignment, the copies of bytes that overlap at every
//! distance up to 40 and some more, both ways, the paths past the caches
//! called themselves (their length depends on the cache of the machine).

use super::*;
use std::vec::Vec;

fn bytes(n: usize, seed: u64) -> Vec<u8> {
    let mut x = seed | 1;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect()
}

const PLACES: [(usize, usize); 5] = [(0, 0), (1, 3), (7, 0), (15, 33), (31, 1)];

fn lengths() -> impl Iterator<Item = usize> {
    (0..600).chain([1000, 2047, 2048, 2049, 2111, 2112, 2113, 4095, 4096, 4097, 20000, 70001])
}

#[test]
fn copies_are_those_of_the_standard_library() {
    let src = bytes(80_000, 1);
    for n in lengths() {
        for (so, dof) in PLACES {
            let mut x = bytes(n + 64, 2);
            let mut y = x.clone();
            copy(&mut x[dof..dof + n], &src[so..so + n]);
            y[dof..dof + n].copy_from_slice(&src[so..so + n]);
            assert!(x == y, "copy of {} at {}, {}", n, so, dof);
        }
    }
}

#[test]
fn fills_are_those_of_the_standard_library() {
    for n in lengths() {
        for (_, dof) in PLACES {
            let mut x = bytes(n + 64, 3);
            let mut y = x.clone();
            fill(&mut x[dof..dof + n], 0xa5);
            y[dof..dof + n].fill(0xa5);
            assert!(x == y, "fill of {} at {}", n, dof);
        }
    }
}

#[test]
fn moves_that_overlap_are_those_of_the_standard_library() {
    for n in lengths().filter(|&n| n <= 5000) {
        for dist in (0..40).chain([63, 64, 100, 129, 300, 2200]) {
            let base = bytes(n + 2300, 4);
            for up in [false, true] {
                let (from, to) = if up { (0, dist) } else { (dist, 0) };
                let mut x = base.clone();
                let mut y = base.clone();
                copy_within(&mut x, from, to, n);
                y.copy_within(from..from + n, to);
                assert!(x == y, "move of {} from {} to {}", n, from, to);
            }
        }
    }
}

#[test]
fn comparisons_are_those_of_the_standard_library() {
    let a = bytes(80_000, 5);
    for n in lengths() {
        for (so, _) in PLACES {
            let x = &a[so..so + n];
            let mut b = x.to_vec();
            assert_eq!(compare(x, &b), 0, "equal {}", n);
            for k in [0, 1, 2, 3, 4, 7, 8, 15, 16, 17, 31, 32, 33, 63, 64, 100, 128, 129, 255, 256, 257, 500, 1023, 1024].into_iter().chain([n.wrapping_sub(1), n / 2]) {
                if k >= n {
                    continue;
                }
                for delta in [1u8, 255, 128] {
                    let old = b[k];
                    b[k] = old.wrapping_add(delta);
                    let want = x.cmp(&b[..]) as i32;
                    assert_eq!(compare(x, &b).signum(), want, "compare of {} at {}", n, k);
                    assert_eq!(compare(&b, x).signum(), -want, "compare of {} at {}, swapped", n, k);
                    b[k] = old;
                }
            }
        }
    }
}

#[test]
fn copies_and_fills_past_the_caches() {
    // 4 pages at a time, then the loop of 4 vectors, at odd places
    let src = bytes(5 * PAGE * 4 + 4096, 6);
    for n in [4 * PAGE * 4 + 77, 4 * PAGE * 5 - 13, 4 * PAGE * 2 + 4 * VEC + 1, 9 * VEC] {
        for (so, dof) in PLACES {
            let mut x = bytes(n + 64, 7);
            let mut y = x.clone();
            copy_streaming(&mut Apart::new(&mut x[dof..dof + n], &src[so..so + n]));
            y[dof..dof + n].copy_from_slice(&src[so..so + n]);
            assert!(x == y, "copy past the caches of {} at {}, {}", n, so, dof);
            fill_vectors(&mut x[dof..dof + n], 0x3c, false);
            y[dof..dof + n].fill(0x3c);
            assert!(x == y, "fill past the caches of {} at {}", n, dof);
        }
    }
}

#[test]
fn the_length_of_strings() {
    for n in [0usize, 1, 5, 100, 5000] {
        let mut s = std::vec![b'x'; n];
        s.push(0);
        // SAFETY: `s` ends with a zero byte
        assert_eq!(unsafe { length(s.as_ptr()) }, n);
    }
}

#[test]
fn the_threshold_past_the_caches_is_half_the_last_cache() {
    let t = non_temporal_min();
    assert!(t >= 512 << 10, "{}", t);
    assert_eq!(non_temporal_min(), t);
}
