// The same walk as walk.isc, in idiomatic Rust: the state lent mutably
// (`&mut State`), the hit returned by value.
#[derive(Clone, Copy)]
struct State { a: u64, b: u64, c: u64, d: u64, e: u64, f: u64, g: u64, h: u64, i: u64, j: u64, k: u64, l: u64, m: u64 }

#[derive(Clone, Copy)]
struct Mat { kind: i64, r: f64, g: f64, b: f64, fuzz: f64 }

#[derive(Clone, Copy)]
struct Hit { px: f64, py: f64, pz: f64, nx: f64, ny: f64, nz: f64, t: f64, u: f64, v: f64, id: i64, mat: Mat }

enum Tree { Leaf(f64, f64, i64), Node(Box<Tree>, Box<Tree>, f64, f64) }

fn seeded(q: u64) -> State {
    let a = q.wrapping_mul(0x9E3779B97F4A7C15).wrapping_add(1);
    State { a, b: a ^ 1, c: a ^ 2, d: a ^ 3, e: a ^ 4, f: a ^ 5, g: a ^ 6, h: a ^ 7, i: a ^ 8, j: a ^ 9, k: a ^ 10, l: a ^ 11, m: 0 }
}

fn draw(s: &mut State) -> f64 {
    let x1 = s.a ^ (s.a >> 12);
    let x2 = x1 ^ (x1 << 25);
    let x3 = x2 ^ (x2 >> 27);
    let r = x3.wrapping_mul(0x2545F4914F6CDD1D);
    s.a = x3;
    s.b = s.b.wrapping_add(r);
    s.m += 1;
    (r >> 11) as f64 * (1.0 / 9007199254740992.0)
}

fn material(id: i64) -> Mat {
    Mat { kind: id % 4, r: (id % 7) as f64 * 0.1, g: (id % 11) as f64 * 0.05, b: (id % 13) as f64 * 0.02, fuzz: 0.5 }
}

fn walk(t: &Tree, x: f64, tmax: f64, s: &mut State) -> Option<Hit> {
    match t {
        Tree::Leaf(lo, hi, id) => {
            if x < *lo || x > *hi {
                return None;
            }
            let r = draw(s);
            let tt = (x - lo) * (1.0 + r);
            if tt >= tmax {
                return None;
            }
            Some(Hit { px: x, py: *lo, pz: *hi, nx: r, ny: 1.0 - r, nz: 0.5, t: tt, u: x - lo, v: hi - x, id: *id, mat: material(*id) })
        }
        Tree::Node(left, right, lo, hi) => {
            if x < *lo || x > *hi {
                return None;
            }
            let h1 = walk(left, x, tmax, s);
            let t1 = match h1 { Some(h) => h.t, None => tmax };
            let h2 = walk(right, x, t1, s);
            if h2.is_some() { h2 } else { h1 }
        }
    }
}

fn built(starts: &[f64], from: usize, to: usize) -> Tree {
    if to - from == 1 {
        return Tree::Leaf(starts[from], starts[from] + 0.01, from as i64);
    }
    let mid = (from + to) / 2;
    Tree::Node(Box::new(built(starts, from, mid)), Box::new(built(starts, mid, to)), starts[from], starts[to - 1] + 0.01)
}

fn query(t: &Tree, q: i64) -> i64 {
    let mut s = seeded(q as u64);
    let x = draw(&mut s);
    match walk(t, x, 1.0e9, &mut s) {
        Some(hit) => (hit.t * 1.0e9) as i64 + (hit.mat.r * 1.0e6) as i64 + (hit.u * 1.0e9) as i64 + hit.id + s.m as i64,
        None => s.m as i64,
    }
}


// the same walk in the style of Isocron: the state given by value, given
// back with the hit
fn draw_v(s: State) -> (f64, State) {
    let mut t = s;
    let r = draw(&mut t);
    (r, t)
}

fn walk_v(t: &Tree, x: f64, tmax: f64, s: State) -> (Option<Hit>, State) {
    match t {
        Tree::Leaf(lo, hi, id) => {
            if x < *lo || x > *hi {
                return (None, s);
            }
            let (r, s1) = draw_v(s);
            let tt = (x - lo) * (1.0 + r);
            if tt >= tmax {
                return (None, s1);
            }
            (Some(Hit { px: x, py: *lo, pz: *hi, nx: r, ny: 1.0 - r, nz: 0.5, t: tt, u: x - lo, v: hi - x, id: *id, mat: material(*id) }), s1)
        }
        Tree::Node(left, right, lo, hi) => {
            if x < *lo || x > *hi {
                return (None, s);
            }
            let (h1, s1) = walk_v(left, x, tmax, s);
            let t1 = match h1 { Some(h) => h.t, None => tmax };
            let (h2, s2) = walk_v(right, x, t1, s1);
            (if h2.is_some() { h2 } else { h1 }, s2)
        }
    }
}

fn query_v(t: &Tree, q: i64) -> i64 {
    let (x, s0) = draw_v(seeded(q as u64));
    let (h, s1) = walk_v(t, x, 1.0e9, s0);
    match h {
        Some(hit) => (hit.t * 1.0e9) as i64 + (hit.mat.r * 1.0e6) as i64 + (hit.u * 1.0e9) as i64 + hit.id + s1.m as i64,
        None => s1.m as i64,
    }
}

fn main() {
    let queries: i64 = std::env::args().nth(1).map(|a| a.parse().unwrap()).unwrap_or(1000000);
    let starts: Vec<f64> = (0..4096).map(|i| i as f64 / 4096.0).collect();
    let t = built(&starts, 0, 4096);
    let t0 = std::time::Instant::now();
    let by_value = std::env::args().nth(2).as_deref() == Some("value");
    let total: i64 = if by_value { (0..queries).map(|q| query_v(&t, q)).sum() } else { (0..queries).map(|q| query(&t, q)).sum() };
    let ms = t0.elapsed().as_millis();
    println!("{}", total);
    println!("{} ms", ms);
}
