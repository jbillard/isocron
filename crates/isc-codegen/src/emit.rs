//! IR to textual LLVM IR.
//!
//! * Fusion (rule F1): a producer (`map`, element-wise operator, `iota`,
//!   `fill`, `zeros`) in a consumer slot is never stored: its elements are
//!   computed inside the consumer's loop (`Src`).
//! * `reduce` follows the fixed form of section 10. Its V = 32 interleaved
//!   accumulators are one `<32 x T>` value (several passes over the block
//!   when the target has too few registers, `reduce_passes`), so the main
//!   loop is SIMD code by construction; the final tree is `shufflevector`s of
//!   halves.
//! * Element-wise loops are emitted 16 elements at a time with vector types
//!   when every operation of the element computation has a vector form,
//!   followed by a scalar tail loop.
//! * Floating-point instructions carry no fast-math flag (section 13).
//! * `bench` calls its body through a function pointer laundered by an empty
//!   `asm`, so that LLVM can neither hoist, merge nor delete the repetitions
//!   (section 12).

use std::collections::{BTreeSet, HashMap, HashSet};

use isc_ir::*;
use isc_syntax::Span;

/// V, the number of interleaved accumulators of the fixed reduction form
/// (SPEC section 10). Other values are an experiment (`iscc --lanes`), not
/// the language.
pub const LANES: u32 = 32;
pub const BLOCK: i64 = 4096;

/// A quotient and a remainder of the index of a loop by the constant `c`,
/// in two slots that each iteration moves on (D180).
#[derive(Clone)]
struct DivMod {
    var: VarId,
    c: i64,
    q: String,
    r: String,
    /// The lanes of the slots: 1, or the width of a vector loop (D186).
    w: u32,
}

/// The pairs (x, c) of the divisions `x / c` and `x % c` of `e`, `x` one
/// of `vars` (an `i64` index, never negative) and `c` a positive integer
/// literal.
fn index_divisions(e: &Expr, vars: &[VarId], out: &mut Vec<(VarId, i64)>) {
    if let ExprKind::Binary(BinOp::Div | BinOp::Rem, a, b) = &e.kind {
        if let (ExprKind::Var(x, _), ExprKind::Lit(Lit::Int(c))) = (&a.kind, &b.kind) {
            if *c > 0 && vars.contains(x) && a.ty == Type::Int(IntKind::I64) && !out.contains(&(*x, *c)) {
                out.push((*x, *c));
            }
        }
    }
    for c in e.children() {
        index_divisions(c, vars, out);
    }
}

/// A `map` over an index `q` (an `iota`) whose body divides it by one
/// constant `c` of at least `w` (D214): `c`, the `q / c` and the `q % c`.
fn row_split(src: &Src, w: u32) -> Option<(i64, Vec<*const Expr>, Vec<*const Expr>)> {
    fn walk(e: &Expr, q: VarId, cs: &mut Vec<i64>, divs: &mut Vec<*const Expr>, rems: &mut Vec<*const Expr>) {
        if let ExprKind::Binary(op @ (BinOp::Div | BinOp::Rem), a, b) = &e.kind {
            if let (ExprKind::Var(x, _), ExprKind::Lit(Lit::Int(c))) = (&a.kind, &b.kind) {
                if *x == q && *c > 0 && a.ty == Type::Int(IntKind::I64) {
                    cs.push(*c);
                    if *op == BinOp::Div {
                        divs.push(e as *const Expr);
                    } else {
                        rems.push(e as *const Expr);
                    }
                    return;
                }
            }
        }
        for c in e.children() {
            walk(c, q, cs, divs, rems);
        }
    }
    let Src::Map(srcs, l) = src else { return None };
    // an element that is a loop or a call costs more than the arithmetic of
    // its index, and the last vector of a row computes some elements twice
    // (fasta: 3 of 61 elements of `pick` again, 0.174 s -> 0.178 s)
    if srcs.len() != 1 || !matches!(srcs[0], Src::Iota) || l.params.len() != 1 || inner_loop(&l.body) {
        return None;
    }
    let (mut cs, mut divs, mut rems) = (Vec::new(), Vec::new(), Vec::new());
    walk(&l.body, l.params[0], &mut cs, &mut divs, &mut rems);
    let c = *cs.first()?;
    if cs.iter().any(|&x| x != c) || c < w as i64 {
        return None;
    }
    Some((c, divs, rems))
}

/// Levels of the reduction of a stream (`reduce_stream`): B^5 = 2^60
/// elements kept.
const STREAM_LEVELS: i64 = 5;
/// At most this many blocks: the results of the tasks of a `reduce` are
/// each in their own 128 bytes (see `reduce`).
const PAD_BLOCKS: i64 = 32;
/// The LLVM type of an array of arrays (D95).
const NESTED: &str = "{ ptr, i64, i64, i64 }";

/// The lambda of a `scan` is a predefined associative operator of SPEC
/// section 6 on integers (`+`, `*`, `min`, `max`) or booleans (`and`, `or`,
/// lowered to `if`): `(a, b) -> a op b`, or `b op a` (they commute).
fn assoc_scan_op(l: &Lambda, t: &Type) -> bool {
    if l.params.len() != 2 {
        return false;
    }
    let (p, q) = (l.params[0], l.params[1]);
    let var = |e: &Expr, v: VarId| matches!(&e.kind, ExprKind::Var(w, _) if *w == v);
    let pair = |a: &Expr, b: &Expr| (var(a, p) && var(b, q)) || (var(a, q) && var(b, p));
    match (t, &l.body.kind) {
        (Type::Int(_), ExprKind::Binary(op, a, b)) => {
            matches!(op, BinOp::Add | BinOp::Mul | BinOp::Min | BinOp::Max) && pair(a, b)
        }
        (Type::Bool, ExprKind::If(c, a, b)) => {
            let lit = |e: &Expr, x: bool| matches!(&e.kind, ExprKind::Lit(Lit::Bool(y)) if *y == x);
            // `c and a` is `if c then a else false`, `c or b` is `if c then true else b`
            (lit(b, false) && pair(c, a)) || (lit(a, true) && pair(c, b))
        }
        _ => false,
    }
}

/// The operator of a `scan` of integers that can be computed in a vector
/// register: `+`, `*`, `min`, `max` (exact, associative and commutative),
/// with the value that leaves the other operand unchanged.
fn int_scan_op(l: &Lambda, t: &Type) -> Option<(BinOp, i128)> {
    let Type::Int(k) = t else { return None };
    if !assoc_scan_op(l, t) {
        return None;
    }
    let ExprKind::Binary(op, _, _) = &l.body.kind else { return None };
    let b = k.bits();
    let (lo, hi) = if k.signed() { (-(1i128 << (b - 1)), (1i128 << (b - 1)) - 1) } else { (0, (1i128 << b) - 1) };
    // an unsigned maximum is written as -1 (all bits set) in LLVM
    let hi = if k.signed() { hi } else { -1 };
    let neutral = match op {
        BinOp::Add => 0,
        BinOp::Mul => 1,
        BinOp::Min => hi,
        BinOp::Max => lo,
        _ => return None,
    };
    Some((*op, neutral))
}

/// Largest array of block results of a `reduce` on the
/// stack; beyond, on the heap. 32 KiB: 4096 results of `f64`, inputs up to
/// 2^24 elements, a small part of the default stack of a thread (DECISIONS
/// D86).
const STACK_SCRATCH: i64 = 32 << 10;
/// From this size on, the array of a `reduce` is read from memory, not from
/// the caches (twice the 16 MiB of L3 of xeon), and the loop of its blocks
/// takes STREAM_UNROLL rows per iteration: large `sum`, one core, 3483863.0
/// against 3527408.4 ns; slower from the caches (small `sum` 1866.1 against
/// 1760.5 ns, 8 MB 176172.2 against 174722.3 ns; DECISIONS D86).
const STREAM_BYTES: i64 = 32 << 20;
const STREAM_UNROLL: i64 = 4;
/// Loops of a constant count up to this are unrolled whole (D205).
const UNROLL_FULL: i64 = 16;
/// The first identifier of their metadata, after those of the scalar loops.
const UNROLL_IDS: u64 = 1 << 20;
/// Width of the element-wise loops (speed only, never results).
pub const MAP_WIDTH: u32 = 16;
/// Chunks per thread of a `map` whose elements are loops (D121, D143).
const HEAVY_CHUNKS: i64 = 8;

/// Instruction set targeted by the back-end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Isa {
    /// The host CPU with LLVM's default tuning (on recent Intel CPUs with
    /// AVX-512, LLVM prefers 256-bit vectors).
    Native,
    /// The host CPU without any AVX-512 feature.
    Avx2,
    /// The host CPU, 512-bit vectors allowed and preferred.
    Avx512,
    /// SSE4.2 without AVX, the instruction set of the Celeron J3455 of the
    /// POC (LLVM CPU `goldmont`: 16 `xmm` registers of 2 doubles).
    Sse,
    /// AVX without AVX2 or FMA: Ivy Bridge (LLVM CPU `ivybridge`), the
    /// i5-3330 of the Benchmarks Game; 16 `ymm` registers of 4 doubles.
    Avx,
    /// WebAssembly with WASI (`wasm32-wasi`), 128-bit SIMD (`simd128`).
    Wasm32,
    /// The same with threads (`wasm32-wasi-threads`: wasi-threads, shared
    /// memory, atomics).
    Wasm32Threads,
}

impl Isa {
    pub fn is_wasm(self) -> bool {
        matches!(self, Isa::Wasm32 | Isa::Wasm32Threads)
    }

    /// The vector registers the back-end allocates for this instruction set:
    /// (bytes per register, number of registers). With AVX-512, LLVM's
    /// default tuning (`Native`) uses 256-bit vectors, in 32 registers.
    pub fn vector_registers(self) -> (u32, u32) {
        match self {
            // the engine maps `v128` values to 128-bit registers, 16 on x86-64
            Isa::Sse | Isa::Wasm32 | Isa::Wasm32Threads => (16, 16),
            Isa::Avx2 | Isa::Avx => (32, 16),
            Isa::Avx512 => (64, 32),
            Isa::Native => {
                let f = crate::backend::host_features();
                let has = |name: &str| f.split(',').any(|x| x == format!("+{}", name));
                if has("avx512vl") {
                    (32, 32)
                } else if has("avx") {
                    (32, 16)
                } else {
                    (16, 16)
                }
            }
        }
    }
}

/// Number of passes over a block for `w` accumulators of `elem_bytes` bytes
/// whose elements read `loads` arrays: the registers of the accumulators of
/// one pass, plus one register per array read, must fit in the vector
/// registers of the target, otherwise LLVM spills accumulators inside the
/// loop. With AVX (registers of 32 bytes or more), one of the loads is folded
/// into the arithmetic instruction and needs no register; SSE cannot fold an
/// unaligned load. Pass p updates the accumulators p·w/passes ..
/// (p+1)·w/passes - 1, each with its elements in increasing order, so the
/// result is the fixed form whatever the number of passes (speed only).
pub fn reduce_passes(w: u32, elem_bytes: u32, loads: u32, regs: (u32, u32)) -> u32 {
    let (bytes, count) = regs;
    let temps = if bytes >= 32 { loads.saturating_sub(1) } else { loads };
    let mut passes = 1;
    while passes < w && (w / passes * elem_bytes).div_ceil(bytes) + temps > count {
        passes *= 2;
    }
    passes
}

/// Elements per chunk of the passes of `reduce_passes`: the chunk of every
/// array read takes at most 16 KiB together (L1 holds 24 KiB or more on the
/// targets measured), and is a whole number of rows of `w` elements.
pub fn reduce_chunk(w: u32, elem_bytes: u32, loads: u32) -> i64 {
    let mut c = BLOCK;
    while c > w as i64 && c * (elem_bytes * loads) as i64 > 16 * 1024 {
        c /= 2;
    }
    c
}

impl Src<'_> {
    /// Number of arrays read to compute one element.
    fn loads(&self) -> u32 {
        match self {
            Src::Mat { .. } | Src::Rows { .. } => 1,
            Src::Iota | Src::Uniform(_) => 0,
            Src::Map(srcs, _) => srcs.iter().map(Src::loads).sum(),
            Src::Offset { inner, .. } => inner.loads(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Options {
    pub lanes: u32,
    pub isa: Isa,
    /// Size from which the array of a `reduce` is read four rows at a time
    /// (STREAM_BYTES; the tests lower it to run that loop on small arrays).
    pub stream_bytes: i64,
    /// Largest scratch array of a `reduce` on the stack (STACK_SCRATCH; the
    /// tests lower it to run the heap path).
    pub stack_scratch: i64,
}

impl Default for Options {
    fn default() -> Options {
        Options { lanes: LANES, isa: Isa::Native, stream_bytes: STREAM_BYTES, stack_scratch: STACK_SCRATCH }
    }
}

type R<T> = Result<T, String>;

#[derive(Clone, Debug)]
pub struct V {
    /// SSA name or constant.
    pub r: String,
    /// LLVM type.
    pub t: String,
    /// The same value for every lane (a scalar usable at any width).
    pub uni: bool,
}

/// A scalar-typed value is the same in every lane of a vector loop.
fn uni(x: &V) -> bool {
    !x.t.starts_with('<')
}

fn v(r: impl Into<String>, t: impl Into<String>) -> V {
    V { r: r.into(), t: t.into(), uni: true }
}

pub fn lt(t: &Type) -> String {
    match t {
        Type::Int(k) => format!("i{}", k.bits()),
        Type::F64 => "double".into(),
        Type::F32 => "float".into(),
        Type::Bool | Type::Unit => "i1".into(),
        // an array of arrays: its block, rows, columns, and the distance in
        // elements from a row to the next (the columns, or 1 for `window`)
        Type::Array(e, _) if e.is_array() => NESTED.into(),
        Type::Str | Type::Big | Type::Array(_, _) => "{ ptr, i64 }".into(),
        // a node of a recursive type (D103)
        Type::Rec(_) => "ptr".into(),
        Type::Tuple(ts) => {
            if ts.is_empty() {
                "{}".into()
            } else {
                format!("{{ {} }}", ts.iter().map(lt).collect::<Vec<_>>().join(", "))
            }
        }
    }
}

/// Type of an array element in memory.
fn mt(t: &Type) -> String {
    match t {
        Type::Bool => "i8".into(),
        t => lt(t),
    }
}

fn esize(t: &Type) -> i64 {
    match t {
        Type::Int(k) => (k.bits() / 8) as i64,
        Type::F64 => 8,
        Type::F32 => 4,
        Type::Bool | Type::Unit => 1,
        Type::Str | Type::Big | Type::Array(_, _) => 16,
        Type::Rec(_) => 8,
        Type::Tuple(ts) => ts.iter().map(esize).sum::<i64>().max(1),
    }
}

/// The scalar leaves of an element type, in memory order, with the path of
/// tuple indices that leads to each (a scalar is its own single leaf).
fn leaves(t: &Type) -> Vec<(Vec<u32>, Type)> {
    match t {
        Type::Tuple(ts) => {
            let mut out = Vec::new();
            for (i, c) in ts.iter().enumerate() {
                for (mut p, l) in leaves(c) {
                    p.insert(0, i as u32);
                    out.push((p, l));
                }
            }
            out
        }
        t => vec![(Vec::new(), t.clone())],
    }
}

fn vt(t: &Type, w: u32) -> String {
    if w == 1 {
        lt(t)
    } else {
        format!("<{} x {}>", w, lt(t))
    }
}

/// A `float` constant: LLVM writes it as the `double` of the same value.
fn fconst32(f: f32) -> String {
    format!("0x{:016X}", (f as f64).to_bits())
}

fn fconst(f: f64) -> String {
    format!("0x{:016X}", f.to_bits())
}

fn llvm_string(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_graphic() && b != b'"' && b != b'\\' || b == b' ' {
            out.push(b as char);
        } else {
            out.push_str(&format!("\\{:02X}", b));
        }
    }
    out
}

/// The element source of a fused loop.
/// The suffix of a scalar type in the names of LLVM intrinsics.
fn ty_suffix(t: &Type) -> String {
    match t {
        Type::F64 => "f64".into(),
        Type::F32 => "f32".into(),
        Type::Int(k) => format!("i{}", k.bits()),
        _ => "i1".into(),
    }
}

/// A `filter` and the `map`s that follow it (`prepare_stream`).
struct Stream<'e> {
    base: Src<'e>,
    n: String,
    pred: &'e Lambda,
    posts: Vec<&'e Lambda>,
    /// The source, the predicate and the maps work on vectors, and the maps
    /// cannot panic (computed for the rejected elements too): the vector
    /// path of `reduce_stream` (D110).
    vec: bool,
}

enum Src<'e> {
    /// A materialized array: `len` is its length (the layout of its columns).
    Mat { ptr: String, len: String, elem: Type },
    Iota,
    Uniform(V),
    Map(Vec<Src<'e>>, &'e Lambda),
    /// The rows of an array of arrays (each a view `{ ptr, i64 }`).
    Rows { ptr: String, cols: String, stride: String, elem: Type },
    /// The elements of `inner` from `start` on: a slice of a producer (D101).
    Offset { inner: Box<Src<'e>>, start: String },
}

#[derive(Default)]
struct Func {
    allocas: Vec<String>,
    body: Vec<String>,
    tmp: u32,
    /// Vectors of consecutive indices (the index of a loop, by lanes): each
    /// one's first index, a scalar, and its step, 1 or -1 (D181, D185).
    contig: HashMap<String, (String, i64)>,
    /// The aggregates whose length (field 1, or the columns at 2) a type
    /// fixed (D204): their reads are that constant (D205).
    known_len: HashMap<(String, u32), u32>,
    /// The tables of bytes of the vector loop being emitted, their 8 parts
    /// of 32 bytes loaded once before it (D198).
    table_parts: HashMap<VarId, Vec<String>>,
}

pub struct Gen<'p> {
    p: &'p Program,
    opts: Options,
    /// In vector code: the lanes on the current path (a `<w x i1>`), None
    /// for all of them; the entry mask of a SPMD call (D140).
    mask: Option<String>,
    /// The condition of an `if` of a SPMD body, computed by the guard that
    /// tells whether it is uniform (D146).
    spmd_cond: Option<V>,
    /// `Isa::vector_registers` of the target.
    vregs: (u32, u32),
    /// Inside a task of a parallel loop: nested loops stay sequential.
    in_task: bool,
    /// The quotients and remainders of the index of the loop being emitted
    /// by constants, carried from one iteration to the next (D180).
    divmod: Vec<DivMod>,
    /// In vector code read by `elem_replayed`: the block that computes the
    /// elements of the vector again one at a time, where a failed check
    /// jumps, so that the panic is the one of the scalar loop (D185).
    vec_fail: Option<String>,
    /// The array literals put on the stack (D191): never freed.
    stack_vars: HashSet<VarId>,
    /// While the sizes of a `CheckSize` are computed: no length fixed by a
    /// type is put into a value (D204), the test reads the real ones.
    no_fix: bool,
    /// The recursive functions that build trees of new nodes only (D200).
    builders: HashSet<u32>,
    /// In a variant of a builder: the register of its region.
    region: Option<String>,
    /// The trees built in a region: their region, freed at their drop.
    region_vars: HashMap<VarId, String>,
    /// The variants of builders to emit.
    rg_needed: BTreeSet<u32>,
    /// The recursive functions whose body is `if (c) base else ...` (or the
    /// other way), `c` and `base` simple: computed by the caller (D202).
    base_cases: HashMap<u32, (&'p Expr, &'p Expr, bool)>,
    /// In a task: the recursive types whose nodes may be shared with other
    /// threads (reached from what the task captures or returns); None: all
    /// of them (D175).
    task_shared: Option<HashSet<u32>>,
    /// While the first level of a `reduce` is emitted: an `i1`, true when its
    /// array has at least STREAM_BYTES bytes (see `reduce_block`).
    stream: Option<String>,
    /// Loops of `for_range_scalar` so far (their metadata ends the module).
    scalar_loops: u32,
    /// Loops of a constant count of at most UNROLL_FULL, unrolled (D205).
    unrolled_loops: u32,
    ntask: u32,
    decls: BTreeSet<String>,
    globals: Vec<String>,
    funcs: Vec<String>,
    strs: HashMap<String, (String, usize)>,
    nbench: u32,
    f: Func,
    env: HashMap<VarId, V>,
    /// The function being emitted: its direct tail calls, turned into jumps
    /// to `TailLoop::head` (D102).
    tail: Option<TailLoop>,
    /// The functions that free or print the nodes of each recursive type
    /// (`(type, atomic)` for the frees), emitted at the end (D103).
    rec_drops: BTreeSet<(u32, bool)>,
    rec_prints: BTreeSet<u32>,
    /// The equality functions of recursive types to emit (D129).
    rec_eqs: BTreeSet<u32>,
    /// The operation being emitted is inside `allowReassoc` (D109).
    fast_math: bool,
    /// The tail calls of the function being emitted to other functions,
    /// with the temporaries to free before them (D113).
    other_tails: HashMap<*const Expr, Vec<VarId>>,
    /// Expressions whose value is already computed: the `reduce` of each
    /// window in the cascade of `window_cascade` (D115).
    pre: HashMap<*const Expr, V>,
    /// The next outlined task prefers 512-bit vectors (`#1`), set by
    /// `window_cascade` (D115).
    wide_task: bool,
    /// The attribute group `#1` is used.
    wide_used: bool,
}

/// A value of this type holds nodes of a recursive type (D130).
/// An element of this type owns memory of its own: a node (D130) or a
/// string (D134); arrays of them free and copy each element.
fn holds_owned(t: &Type) -> bool {
    match t {
        Type::Rec(_) | Type::Str | Type::Big => true,
        Type::Tuple(ts) => ts.iter().any(holds_owned),
        Type::Array(e, _) => holds_owned(e),
        _ => false,
    }
}

/// `e` contains a loop of its own (`reduce`, `fold`, `scan`, `filter`):
/// an element of a `map` that costs much more than an operation.
fn inner_loop(e: &Expr) -> bool {
    // a search of a regular expression or a replacement in a text is a loop
    // over the text too (D174): a `map` of a few of them is shared out
    matches!(e.kind, ExprKind::Reduce(..) | ExprKind::Fold(..) | ExprKind::Scan(..) | ExprKind::Filter(..) | ExprKind::Call(..))
        || matches!(&e.kind, ExprKind::StrOp(StrOp::Rx(_) | StrOp::ReplaceAll | StrOp::ReplaceFirst, _))
        || e.children().into_iter().any(inner_loop)
}

// ---------------------------------------------------------------------
// SPMD calls (D140): a recursive function whose recursive calls are all
// tail calls to itself, of scalars, that cannot panic whatever its
// arguments, is computed for the lanes of vector code at once: each lane
// has its state, a lane that reaches a result keeps it and stops, the loop
// goes on while a lane is active (the lanes not on the call's path start
// stopped).

thread_local! {
    /// The functions of the program computed lane by lane in vector code.
    static SPMD: std::cell::RefCell<HashSet<u32>> = std::cell::RefCell::new(HashSet::new());
    /// While `lam_ok_replay` runs: the rules of vector code read by
    /// `elem_replayed` (D185).
    static REPLAY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// The target permutes bytes (AVX-512 VBMI): an index into a table of
    /// bytes is vector code (D185).
    static BYTE_LOOKUP: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// The target has the 52-bit products of AVX-512 IFMA (D192).
    static IFMA: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// The target compresses vectors of 32 and 64 bits (AVX-512 F and VL),
    /// and of bytes (VBMI2): the elements a `filter` keeps (D195).
    static COMPRESS: std::cell::Cell<(bool, bool)> = const { std::cell::Cell::new((false, false)) };
    /// The parameters of the lambdas being checked bound to an `iota`: the
    /// index of the loop, consecutive in the lanes (D185).
    static INDEX_VARS: std::cell::RefCell<HashSet<VarId>> = std::cell::RefCell::new(HashSet::new());
    /// While `src_vec_ok_lookup` runs: an index into an array of bytes is
    /// vector code, its loop choosing at run time (`lookup_tables`, D185).
    static LOOKUP_OK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The largest number of parameters of the SPMD functions called in `e`,
/// if it calls one: the state each lane keeps.
fn spmd_state(p: &Program, e: &Expr) -> Option<usize> {
    let own = match &e.kind {
        ExprKind::Call(f, _) if SPMD.with(|s| s.borrow().contains(f)) => Some(p.fns[*f as usize].params.len()),
        _ => None,
    };
    e.children().into_iter().filter_map(|c| spmd_state(p, c)).chain(own).max()
}

/// The tail calls of `f` in its body (the leaves of `spmd_shape`).
fn spmd_tail_calls(e: &Expr, f: u32, out: &mut Vec<*const Vec<Expr>>) {
    match &e.kind {
        ExprKind::Let { body, .. } => spmd_tail_calls(body, f, out),
        ExprKind::If(_, t, x) => {
            spmd_tail_calls(t, f, out);
            spmd_tail_calls(x, f, out);
        }
        ExprKind::Call(g, args) if *g == f => out.push(args as *const Vec<Expr>),
        _ => {}
    }
}

/// The parameters of a SPMD function that keep one value for all the active
/// lanes (D140): uniform at the call (`entry`), and given by every tail call
/// the same expression of such parameters and literals (a counter `i + 1`).
/// The lanes start together, so they stay together: these are scalars.
fn spmd_uniform(fun: &isc_ir::Function, f: u32, entry: &[bool]) -> Vec<bool> {
    let mut calls = Vec::new();
    spmd_tail_calls(&fun.body, f, &mut calls);
    // SAFETY: the pointers are into `fun.body`, borrowed for this function
    let calls: Vec<&Vec<Expr>> = calls.into_iter().map(|c| unsafe { &*c }).collect();
    fn only(e: &Expr, params: &[VarId], u: &[bool]) -> bool {
        match &e.kind {
            ExprKind::Var(v, _) => params.iter().position(|p| p == v).map(|k| u[k]).unwrap_or(false),
            ExprKind::Lit(_) | ExprKind::Binary(..) | ExprKind::Unary(..) | ExprKind::Convert(_) => {
                e.children().into_iter().all(|c| only(c, params, u))
            }
            _ => false,
        }
    }
    let mut u = entry.to_vec();
    loop {
        let mut changed = false;
        for k in 0..u.len() {
            if !u[k] {
                continue;
            }
            let ok = calls.iter().all(|c| same_expr(&c[k], &calls[0][k]) && only(&c[k], &fun.params, &u));
            if !ok {
                u[k] = false;
                changed = true;
            }
        }
        if !changed {
            return u;
        }
    }
}

fn spmd_fns(p: &Program) -> HashSet<u32> {
    let mut out = HashSet::new();
    for (i, f) in p.fns.iter().enumerate() {
        let i = i as u32;
        let scalars = f.params.iter().all(|v| p.vars[*v as usize].ty.is_scalar()) && f.body.ty.is_scalar();
        if !scalars || !isc_ir::passes::cannot_panic(&f.body) {
            continue;
        }
        let mut varying: HashSet<VarId> = f.params.iter().copied().collect();
        if spmd_shape(&f.body, i, &mut varying) {
            out.insert(i);
        }
    }
    out
}

/// The body of a SPMD function: `let`s and `if`s whose leaves are values
/// of vector code or tail calls of the function itself.
fn spmd_shape(e: &Expr, f: u32, varying: &mut HashSet<VarId>) -> bool {
    match &e.kind {
        ExprKind::Let { var, init, body, .. } => {
            if !vec_ok(init, varying) {
                return false;
            }
            varying.insert(*var);
            spmd_shape(body, f, varying)
        }
        ExprKind::If(c, t, x) => vec_ok(c, varying) && spmd_shape(t, f, varying) && spmd_shape(x, f, varying),
        ExprKind::Call(g, args) if *g == f => args.iter().all(|a| vec_ok(a, varying)),
        _ => vec_ok(e, varying),
    }
}

/// The `reduce` of the row in `(w) -> ... reduce(w, op, z) ...`, when the
/// row is used nowhere else (D115).
fn window_reduce_of(l: &Lambda) -> Option<&Expr> {
    fn uses(e: &Expr, w: VarId) -> usize {
        let own = usize::from(matches!(e.kind, ExprKind::Var(v, _) if v == w));
        own + e.children().into_iter().map(|c| uses(c, w)).sum::<usize>()
    }
    fn find(e: &Expr, w: VarId) -> Option<&Expr> {
        if let ExprKind::Reduce(x, _, _) = &e.kind {
            if matches!(x.kind, ExprKind::Var(v, _) if v == w) && e.ty.is_scalar() {
                return Some(e);
            }
        }
        e.children().into_iter().find_map(|c| find(c, w))
    }
    let w = *l.params.first()?;
    if l.params.len() != 1 || uses(&l.body, w) != 1 {
        return None;
    }
    find(&l.body, w)
}

fn rec_drop_name(i: u32, atomic: bool) -> String {
    if atomic {
        format!("isc_rec_drop_{}_atomic", i)
    } else {
        format!("isc_rec_drop_{}", i)
    }
}

/// A recursive function whose direct tail calls are a loop.
struct TailLoop {
    calls: HashMap<*const Expr, isc_ir::passes::TailCall>,
    head: String,
    /// Per parameter: its slot, and for an array the slot of a flag set when
    /// the loop owns its value (a temporary moved into it by a tail call).
    slots: Vec<(String, Option<String>, Type)>,
}


const RUNTIME_DECLS: &[&str] = &[
    "declare noalias ptr @isc_alloc(i64)",
    "declare noalias ptr @isc_alloc_zeroed(i64)",
    "declare void @isc_free(ptr)",
    "declare ptr @isc_node_alloc(i64)",
    "declare void @isc_node_free(ptr, i64)",
    "declare void @isc_read_f64(ptr, ptr, i64, i32, i32)",
    "declare void @isc_read_f32(ptr, ptr, i64, i32, i32)",
    "declare void @isc_read_bytes(ptr, ptr, i64, i32, i32)",
    "declare void @isc_write_f64(ptr, i64, ptr, i64, i32, i32)",
    "declare void @isc_write_f32(ptr, i64, ptr, i64, i32, i32)",
    "declare void @isc_write_bytes(ptr, i64, ptr, i64, i32, i32)",
    "declare void @isc_print_i64(i64)",
    "declare void @isc_print_u64(i64)",
    "declare void @isc_print_f64(double)",
    "declare void @isc_print_f32(float)",
    "declare void @isc_print_bool(i32)",
    "declare void @isc_print_str(ptr, i64)",
    "declare void @isc_print_newline()",
    "declare void @isc_print_bytes(ptr, i64)",
    "declare void @isc_flush()",
    "declare void @isc_try_parse_i64(ptr, ptr, ptr, i64)",
    "declare void @isc_try_parse_f64(ptr, ptr, ptr, i64)",
    "declare void @isc_try_read_bytes(ptr, ptr, ptr, i64)",
    "declare void @isc_try_read_f64(ptr, ptr, ptr, i64)",
    "declare void @isc_try_read_text(ptr, ptr, ptr, i64)",
    "declare void @isc_try_read_csv_f64(ptr, ptr, ptr, i64, i64)",
    "declare void @isc_try_write_bytes(ptr, ptr, i64, ptr, i64)",
    "declare i64 @isc_clock_ns()",
    "declare i64 @isc_parse_i64(ptr, i64, i32, i32)",
    "declare double @isc_parse_f64(ptr, i64, i32, i32)",
    "declare void @isc_str_new(ptr, ptr, i64)",
    "declare void @isc_str_concat(ptr, ptr, i64, ptr, i64)",
    "declare i32 @isc_str_cmp(ptr, i64, ptr, i64)",
    "declare void @isc_str_from_bytes(ptr, ptr, i64, i32, i32)",
    "declare void @isc_to_string_i64(ptr, i64)",
    "declare void @isc_to_string_u64(ptr, i64)",
    "declare void @isc_to_string_f64(ptr, double)",
    "declare void @isc_to_string_f32(ptr, float)",
    "declare void @isc_to_string_bool(ptr, i32)",
    "declare void @isc_read_text(ptr, ptr, i64, i32, i32)",
    "declare void @isc_write_text(ptr, i64, ptr, i64, i32, i32)",
    "declare void @isc_lines(ptr, ptr, i64)",
    "declare void @isc_split(ptr, ptr, i64, ptr, i64, i32, i32)",
    "declare void @isc_trim(ptr, ptr, i64)",
    "declare i64 @isc_index_of(ptr, i64, ptr, i64)",
    "declare i32 @isc_str_test(ptr, i64, ptr, i64, i32)",
    "declare void @isc_replace_all(ptr, ptr, i64, ptr, i64, ptr, i64, i32)",
    "declare void @isc_rx_new(ptr, ptr, i64, ptr, i64, i32, i32)",
    "declare i64 @isc_rx_int(ptr, i64, ptr, i64, i32)",
    "declare void @isc_rx_matches(ptr, ptr, i64, ptr, i64)",
    "declare void @isc_rx_replace(ptr, ptr, i64, ptr, i64, ptr, i64, i32, i32, i32)",
    "declare void @isc_to_case(ptr, ptr, i64, i32)",
    "declare void @isc_pad(ptr, ptr, i64, i64, ptr, i64, i32)",
    "declare void @isc_repeat(ptr, ptr, i64, i64, i32, i32)",
    "declare void @isc_trim_side(ptr, ptr, i64, i32)",
    "declare void @isc_str_slice(ptr, ptr, i64, i64, i64, i32, i32)",
    "declare void @isc_to_fixed(ptr, double, i64, i32, i32)",
    "declare void @isc_read_stdin(ptr, i32, i32)",
    "declare void @isc_argsort(ptr, ptr, i64, i32)",
    "declare void @isc_group(ptr, ptr, i64, i32)",
    "declare double @isc_js_number(ptr, i64, i32, i64)",
    "declare void @isc_print_big(ptr, i64)",
    "declare void @isc_big_op(ptr, ptr, i64, ptr, i64, i32, i32, i32)",
    "declare i32 @isc_big_cmp(ptr, i64, ptr, i64)",
    "declare void @isc_big_from_i64(ptr, i64)",
    "declare void @isc_big_from_u64(ptr, i64)",
    "declare void @isc_big_parse(ptr, ptr, i64, i32, i32)",
    "declare i64 @isc_big_to_i64(ptr, i64)",
    "declare double @isc_big_to_f64(ptr, i64)",
    "declare void @isc_big_to_string(ptr, ptr, i64)",
    "declare void @isc_read_stdin_text(ptr, i32, i32)",
    "declare void @isc_join(ptr, ptr, i64, ptr, i64)",
    "declare void @isc_parse_csv_f64(ptr, ptr, i64, i64, i32, i32)",
    "declare void @isc_read_csv_f64(ptr, ptr, i64, i64, i32, i32)",
    "declare void @isc_args(ptr, i32, ptr)",
    "declare void @isc_panic_bounds(i64, i64, i32, i32, i32) noreturn",
    "declare void @isc_panic_size(i64, i64, i32, i32) noreturn",
    "declare void @isc_panic_slice(i64, i64, i64, i32, i32) noreturn",
    "declare void @isc_panic_window(i64, i64, i32, i32) noreturn",
    "declare void @isc_panic_negsize(i64, i32, i32) noreturn",
    "declare void @isc_panic_div0(i32, i32) noreturn",
    "declare void @isc_panic_negexp(i64, i32, i32) noreturn",
    "declare void @isc_panic_bench(i64, i32, i32) noreturn",
    "declare void @llvm.memcpy.p0.p0.i64(ptr, ptr, i64, i1)",
    "declare ptr @llvm.stacksave.p0()",
    "declare void @llvm.stackrestore.p0(ptr)",
    "declare void @isc_parallel_for(i64, ptr, ptr, i64)",
    "declare void @isc_parallel_for_dynamic(i64, ptr, ptr, i64)",
    "declare i64 @isc_thread_count()",
    "declare ptr @isc_cstr(ptr, i64, ptr, i64, i32, i32)",
];

/// Emits the LLVM module of a program.
pub fn emit(p: &Program) -> R<String> {
    emit_with(p, &Options::default())
}

/// Emits the LLVM module of a program with explicit options.
pub fn emit_with(p: &Program, opts: &Options) -> R<String> {
    if !opts.lanes.is_power_of_two() || opts.lanes < 2 || BLOCK % opts.lanes as i64 != 0 {
        return Err(format!("--lanes {}: V must be a power of two dividing {}", opts.lanes, BLOCK));
    }
    SPMD.with(|s| *s.borrow_mut() = spmd_fns(p));
    BYTE_LOOKUP.with(|b| {
        b.set(matches!(opts.isa, Isa::Native | Isa::Avx512) && crate::backend::host_features().split(',').any(|f| f == "+avx512vbmi"))
    });
    IFMA.with(|b| {
        let f = crate::backend::host_features();
        b.set(matches!(opts.isa, Isa::Native | Isa::Avx512) && ["+avx512ifma", "+avx512vl"].iter().all(|x| f.split(',').any(|y| y == *x)))
    });
    COMPRESS.with(|b| {
        let f = crate::backend::host_features();
        let has = |x: &str| f.split(',').any(|y| y == x);
        let wide = matches!(opts.isa, Isa::Native | Isa::Avx512) && has("+avx512f") && has("+avx512vl");
        b.set((wide, wide && has("+avx512vbmi2") && has("+avx512bw")))
    });
    let mut g = Gen {
        p,
        opts: *opts,
        mask: None,
        spmd_cond: None,
        vregs: opts.isa.vector_registers(),
        in_task: false,
        task_shared: None,
        stream: None,
        scalar_loops: 0,
        unrolled_loops: 0,
        ntask: 0,
        decls: BTreeSet::new(),
        globals: Vec::new(),
        funcs: Vec::new(),
        strs: HashMap::new(),
        nbench: 0,
        f: Func::default(),
        env: HashMap::new(),
        divmod: Vec::new(),
        vec_fail: None,
        stack_vars: HashSet::new(),
        no_fix: false,
        builders: if opts.isa.is_wasm() { HashSet::new() } else { builders(p) },
        region: None,
        region_vars: HashMap::new(),
        rg_needed: BTreeSet::new(),
        base_cases: base_cases(p),
        tail: None,
        rec_drops: BTreeSet::new(),
        rec_prints: BTreeSet::new(),
        rec_eqs: BTreeSet::new(),
        fast_math: false,
        other_tails: HashMap::new(),
        pre: HashMap::new(),
        wide_task: false,
        wide_used: false,
    };
    // the recursive functions (D102)
    for i in 0..p.fns.len() {
        g.emit_fn(i, false)?;
    }
    let slot = g.alloca("{ ptr, i64 }");
    g.emit(format!("call void @isc_args(ptr {}, i32 %argc, ptr %argv)", slot));
    let args = g.ins(format!("load {{ ptr, i64 }}, ptr {}", slot));
    g.env.insert(p.args, v(args.clone(), "{ ptr, i64 }"));
    let r = g.ex(&p.body, 1)?;
    // the array of the arguments and their strings (copies, D134), owned
    // by `main` unless it gave it away (D96)
    if p.drop_args {
        let t = Type::Array(Box::new(Type::Str), isc_ir::Fixed::ANY);
        g.drop_value(&v(args.clone(), "{ ptr, i64 }"), &t);
    }
    g.emit("call void @isc_flush()");
    g.emit(format!("ret i32 {}", r.r));
    let attrs = g.attrs();
    // the C library of WASI calls `__main_argc_argv`, the name clang gives to
    // `main(argc, argv)` on WebAssembly
    let main_name = if opts.isa.is_wasm() { "__main_argc_argv" } else { "main" };
    // with recursive functions, the body of `main` runs on a thread with a
    // large stack (`isc_run_big_stack`, D102)
    let big_stack = !p.fns.is_empty() && !opts.isa.is_wasm();
    let body_name = if big_stack { "isc_main_body" } else { main_name };
    let mut main = g.finish(&format!("define i32 @{}(i32 %argc, ptr %argv){}", body_name, attrs));
    // the variants of the builders whose trees are made in a region (D200),
    // each of which may ask for others
    let mut done: HashSet<u32> = HashSet::new();
    while let Some(f) = g.rg_needed.iter().copied().find(|f| !done.contains(f)) {
        done.insert(f);
        g.emit_fn(f as usize, true)?;
    }
    g.emit_rec_helpers()?;
    if big_stack {
        g.decls.insert("declare i32 @isc_run_big_stack(ptr, i32, ptr)".into());
        main.push_str(&format!(
            "\ndefine i32 @{}(i32 %argc, ptr %argv) {{\nentry:\n  %r = call i32 @isc_run_big_stack(ptr @isc_main_body, i32 %argc, ptr %argv)\n  ret i32 %r\n}}\n",
            main_name
        ));
    }
    let mut out = String::new();
    out.push_str("; Isocron module\n\n");
    for d in RUNTIME_DECLS {
        out.push_str(d);
        out.push('\n');
    }
    for d in &g.decls {
        out.push_str(d);
        out.push('\n');
    }
    out.push('\n');
    for gl in &g.globals {
        out.push_str(gl);
        out.push('\n');
    }
    out.push('\n');
    // The generated code starts on a page (D212): its place modulo 4096, so
    // the alignment of its loops, no longer depends on the size of the code
    // the linker puts before it (the cold functions of the runtime: a change
    // of the runtime moved binary-trees by 848 bytes and made it 5 % slower,
    // the same instructions, D188, D211). A first function, empty and kept
    // (not internal), aligned; the others follow it in the same section.
    if !opts.isa.is_wasm() {
        out.push_str("define hidden void @isc_code_start() align 4096 {\nentry:\n  ret void\n}\n\n");
    }
    for f in &g.funcs {
        out.push_str(f);
        out.push('\n');
    }
    out.push_str(&main);
    if opts.isa == Isa::Wasm32Threads {
        // the waits of the thread pool of the runtime (unstable in Rust)
        out.push_str(
            "\ndeclare i32 @llvm.wasm.memory.atomic.wait32(ptr, i32, i64)\n\
             declare i32 @llvm.wasm.memory.atomic.notify(ptr, i32)\n\
             define void @isc_wasm_wait32(ptr %p, i32 %v) {\n  \
             %r = call i32 @llvm.wasm.memory.atomic.wait32(ptr %p, i32 %v, i64 -1)\n  ret void\n}\n\
             define void @isc_wasm_notify(ptr %p) {\n  \
             %r = call i32 @llvm.wasm.memory.atomic.notify(ptr %p, i32 -1)\n  ret void\n}\n",
        );
    }
    if g.unrolled_loops > 0 {
        out.push_str(&format!("\n!{} = !{{!\"llvm.loop.unroll.full\"}}\n", UNROLL_IDS));
        for k in 1..=g.unrolled_loops as u64 {
            out.push_str(&format!("!{0} = distinct !{{!{0}, !{1}}}\n", UNROLL_IDS + k, UNROLL_IDS));
        }
    }
    if g.scalar_loops > 0 {
        out.push_str("\n!0 = !{!\"llvm.loop.vectorize.enable\", i1 false}\n!1 = !{!\"llvm.loop.unroll.disable\"}\n");
        for k in 0..g.scalar_loops {
            out.push_str(&format!("!{0} = distinct !{{!{0}, !0, !1}}\n", k + 2));
        }
    }
    if opts.isa == Isa::Avx512 {
        // explicit 512-bit vectors stay in zmm registers instead of being split
        out.push_str("\nattributes #0 = { \"min-legal-vector-width\"=\"512\" \"prefer-vector-width\"=\"512\" }\n");
    }
    if g.wide_used {
        out.push_str("\nattributes #1 = { \"min-legal-vector-width\"=\"512\" \"prefer-vector-width\"=\"512\" }\n");
    }

    Ok(out)
}

fn free_vars(e: &Expr, bound: &mut HashSet<VarId>, out: &mut Vec<VarId>) {
    match &e.kind {
        ExprKind::Var(v, _) => {
            if !bound.contains(v) && !out.contains(v) {
                out.push(*v);
            }
        }
        ExprKind::Let { var, init, body, .. } => {
            free_vars(init, bound, out);
            bound.insert(*var);
            free_vars(body, bound, out);
        }
        _ => {
            match &e.kind {
                ExprKind::Map(_, l) | ExprKind::Reduce(_, l, _) | ExprKind::Fold(_, _, l) | ExprKind::Scan(_, l, _) | ExprKind::Filter(_, l) => {
                    for p in &l.params {
                        bound.insert(*p);
                    }
                }
                _ => {}
            }
            for c in e.children() {
                free_vars(c, bound, out);
            }
        }
    }
}

fn mentions(e: &Expr, vars: &HashSet<VarId>) -> bool {
    if let ExprKind::Var(v, _) = e.kind {
        return vars.contains(&v);
    }
    e.children().into_iter().any(|c| mentions(c, vars))
}

/// A length of an array fixed by the type `t` (D204).
fn has_fixed(t: &Type) -> bool {
    match t {
        Type::Array(e, f) => f.0.is_some() || matches!(&**e, Type::Array(_, g) if g.0.is_some()),
        Type::Tuple(ts) => ts.iter().any(has_fixed),
        _ => false,
    }
}

/// The lengths fixed by `t` (D204): the path of each in the aggregate of a
/// value of `t` (`lt`), and the length. An array: its length at 1; an
/// array of arrays (`NESTED`): its rows at 1, its columns at 2.
fn fixed_lengths(t: &Type, path: &mut Vec<u32>, out: &mut Vec<(Vec<u32>, u32)>) {
    match t {
        Type::Array(e, f) => {
            if let Some(n) = f.0 {
                path.push(1);
                out.push((path.clone(), n));
                path.pop();
            }
            if let Type::Array(_, g) = &**e {
                if let Some(m) = g.0 {
                    path.push(2);
                    out.push((path.clone(), m));
                    path.pop();
                }
            }
        }
        Type::Tuple(ts) => {
            for (k, c) in ts.iter().enumerate() {
                path.push(k as u32);
                fixed_lengths(c, path, out);
                path.pop();
            }
        }
        _ => {}
    }
}

/// The recursive functions whose body is `if (c) a else b` with `c` and one
/// branch simple (D202): no call, no allocation, no panic, a scalar or a
/// constructor without payload as result. A caller computes `c` and that
/// branch itself and calls the function only for the other one: the leaves
/// of a tree cost no call. The parameters of a recursive function are
/// borrowed (its caller owns them), so nothing changes hands.
fn base_cases(p: &Program) -> HashMap<u32, (&Expr, &Expr, bool)> {
    fn payloadless(e: &Expr) -> bool {
        match &e.kind {
            ExprKind::Box(x) => match &x.kind {
                ExprKind::Tuple(es) => {
                    matches!(es.first().map(|t| &t.kind), Some(ExprKind::Lit(_))) && es.iter().skip(1).all(|c| matches!(c.kind, ExprKind::Zero))
                }
                _ => false,
            },
            _ => false,
        }
    }
    fn simple(e: &Expr) -> bool {
        use ExprKind::*;
        match &e.kind {
            // (reading a variable does nothing; what a base case gives back
            // is checked by `base`)
            Lit(_) | Zero | Var(..) => true,
            Unary(_, x) => simple(x),
            Binary(op, a, b) => {
                !matches!(op, BinOp::Div | BinOp::Rem) && !a.ty.has_heap() && !b.ty.has_heap() && simple(a) && simple(b)
            }
            If(c, t, f) => simple(c) && simple(t) && simple(f),
            Let { init, body, drop: false, .. } => simple(init) && simple(body),
            Proj(x, _) | Unbox(x) => simple(x),
            Box(_) => payloadless(e),
            _ => false,
        }
    }
    fn base(e: &Expr) -> bool {
        simple(e) && (e.ty.is_scalar() || payloadless(e))
    }
    let mut out = HashMap::new();
    for (i, f) in p.fns.iter().enumerate() {
        if let ExprKind::If(c, t, e) = &f.body.kind {
            if !simple(c) {
                continue;
            }
            match (base(t), base(e)) {
                (true, false) => {
                    out.insert(i as u32, (&**c, &**t, true));
                }
                (false, true) => {
                    out.insert(i as u32, (&**c, &**e, false));
                }
                _ => {}
            }
        }
    }
    out
}

/// The recursive functions that build trees of new nodes only (D200): no
/// parameter holds a recursive type, the result is one, and the body
/// (`builds`) makes its nodes from scalars, constructors without payload
/// and the results of builders, never reading, sharing or freeing a node.
fn builders(p: &Program) -> HashSet<u32> {
    fn holds_rec(t: &Type) -> bool {
        match t {
            Type::Rec(_) => true,
            Type::Array(e, _) => holds_rec(e),
            Type::Tuple(ts) => ts.iter().any(holds_rec),
            _ => false,
        }
    }
    // no value of a recursive type anywhere in `e`
    fn plain(e: &Expr) -> bool {
        !holds_rec(&e.ty) && e.children().into_iter().all(plain)
    }
    fn builds(e: &Expr, set: &HashSet<u32>) -> bool {
        match &e.kind {
            ExprKind::Box(x) => match &x.kind {
                ExprKind::Tuple(es) => es.iter().all(|c| matches!(c.kind, ExprKind::Zero) || plain(c) || builds(c, set)),
                _ => false,
            },
            ExprKind::Call(g, args) => set.contains(g) && args.iter().all(plain),
            ExprKind::If(c, t, f) => plain(c) && result(t, set) && result(f, set),
            _ => false,
        }
    }
    // the result: a node made here (not a call: a tail call would be a loop)
    fn result(e: &Expr, set: &HashSet<u32>) -> bool {
        match &e.kind {
            ExprKind::Box(_) => builds(e, set),
            ExprKind::If(c, t, f) => plain(c) && result(t, set) && result(f, set),
            ExprKind::Let { init, body, .. } => plain(init) && result(body, set),
            ExprKind::CheckSize(a, b, body) => plain(a) && plain(b) && result(body, set),
            ExprKind::CheckNonNegative(k, body) => plain(k) && result(body, set),
            _ => false,
        }
    }
    let mut set: HashSet<u32> = (0..p.fns.len() as u32)
        .filter(|&i| {
            let f = &p.fns[i as usize];
            matches!(f.body.ty, Type::Rec(_)) && f.params.iter().all(|v| !holds_rec(&p.vars[*v as usize].ty))
        })
        .collect();
    loop {
        let keep: HashSet<u32> = set.iter().copied().filter(|&i| result(&p.fns[i as usize].body, &set)).collect();
        if keep.len() == set.len() {
            return set;
        }
        set = keep;
    }
}

/// Every use of the tree `t` in `e` lends it, borrowed, to a function whose
/// result holds no recursive value (it can keep no node of it): the tree
/// dies whole at the end of its `let` (D200).
fn region_use_ok(t: VarId, e: &Expr) -> bool {
    fn holds_rec(ty: &Type) -> bool {
        match ty {
            Type::Rec(_) => true,
            Type::Array(e, _) => holds_rec(e),
            Type::Tuple(ts) => ts.iter().any(holds_rec),
            _ => false,
        }
    }
    fn ok(e: &Expr, t: VarId) -> bool {
        use ExprKind::*;
        match &e.kind {
            Var(x, _) => *x != t,
            Free(x, b) => *x != t && ok(b, t),
            Call(_, args) if !holds_rec(&e.ty) => {
                args.iter().all(|a| matches!(a.kind, Var(x, Use::Borrow) if x == t) || ok(a, t))
            }
            _ => e.children().into_iter().all(|c| ok(c, t)),
        }
    }
    ok(e, t)
}

/// The elements of an array literal `e` ends in, through `let`s of scalars
/// (the body of an inlined function, D191).
fn literal_tail(e: &Expr) -> Option<&Vec<Expr>> {
    match &e.kind {
        ExprKind::ArrayLit(es) => Some(es),
        ExprKind::Let { init, body, stmt: false, drop: false, .. } if init.ty.is_scalar() => literal_tail(body),
        _ => None,
    }
}

/// Every use of the array `v` in `e` reads it, borrowed: an element, its
/// length, the source of a `map`, `reduce`, `fold`, `filter` or `scan`, a
/// copy; never moved, freed, viewed (a slice, a window), stored or passed
/// to a function (a tail call would carry it into the loop). It may then
/// live on the stack (D191).
fn stack_array(v: VarId, e: &Expr) -> bool {
    fn is_v(e: &Expr, v: VarId) -> bool {
        matches!(e.kind, ExprKind::Var(x, Use::Borrow) if x == v)
    }
    fn src(x: &Expr, v: VarId) -> bool {
        is_v(x, v) || ok(x, v)
    }
    fn ok(e: &Expr, v: VarId) -> bool {
        use ExprKind::*;
        match &e.kind {
            Var(x, _) => *x != v,
            Free(x, b) => *x != v && ok(b, v),
            Index(a, i) if is_v(a, v) => ok(i, v),
            Len(a) if is_v(a, v) => true,
            Copy(x) if is_v(x, v) => true,
            Map(ins, l) => ins.iter().all(|x| src(x, v)) && ok(&l.body, v),
            Reduce(x, l, z) | Scan(x, l, z) => src(x, v) && ok(&l.body, v) && ok(z, v),
            Fold(x, init, l) => src(x, v) && ok(init, v) && ok(&l.body, v),
            Filter(x, l) => src(x, v) && ok(&l.body, v),
            _ => e.children().into_iter().all(|c| ok(c, v)),
        }
    }
    ok(e, v)
}

/// `e` frees `v` somewhere (`Free`, D105).
fn frees(e: &Expr, v: VarId) -> bool {
    matches!(e.kind, ExprKind::Free(w, _) if w == v) || e.children().into_iter().any(|c| frees(c, v))
}

/// No panic and no effect: may be evaluated speculatively (`select`).
fn safe(e: &Expr) -> bool {
    isc_ir::passes::cannot_panic(e)
}

fn is_signed(t: &Type) -> bool {
    matches!(t, Type::Int(k) if k.signed())
}

/// Can this element computation be emitted with vector types?
fn vec_ok(e: &Expr, varying: &mut HashSet<VarId>) -> bool {
    vec_ok_given(e, varying, std::ptr::null()) && index_vec_ok(e)
}

/// The sites of `e` that can panic (SPEC section 5): an index, an integer
/// division by a variable, a power by a signed exponent, ...
fn panic_sites(e: &Expr) -> usize {
    use ExprKind::*;
    let here = match &e.kind {
        // a size of every lane (vector code: not varying) fails for all the
        // lanes at once (D183)
        CheckNonNegative(..) => 0,
        Index(..) | Update { .. } | Slice(..) | Window(..) | CheckSize(..) | Action(..) | Bench(..) => 1,
        Binary(BinOp::Div | BinOp::Rem, a, b) if matches!(a.ty, Type::Int(_)) && !matches!(b.kind, Lit(isc_ir::Lit::Int(d)) if d != 0) => 1,
        Pow(_, k) if !(matches!(k.kind, Lit(isc_ir::Lit::Int(n)) if n >= 0) || !matches!(k.ty, Type::Int(ik) if ik.signed())) => 1,
        _ => 0,
    };
    here + e.children().into_iter().map(panic_sites).sum::<usize>()
}

fn has_index(e: &Expr) -> bool {
    matches!(e.kind, ExprKind::Index(..)) || e.children().into_iter().any(has_index)
}

/// Vector code of a body with an index (D181): its only site that can
/// panic, so that the first lane out of bounds is the element at which the
/// scalar loop panics; any number when a failed check is replayed one
/// element at a time (D185).
fn index_vec_ok(e: &Expr) -> bool {
    !has_index(e) || panic_sites(e) == 1 || REPLAY.with(|r| r.get())
}

/// The sites of `e` that can panic are indexings, whose checks and loads
/// are restricted to the lanes of a mask (D185).
fn maskable(e: &Expr) -> bool {
    use ExprKind::*;
    match &e.kind {
        Binary(BinOp::Div | BinOp::Rem, a, b) if matches!(a.ty, Type::Int(_)) => {
            matches!(b.kind, Lit(isc_ir::Lit::Int(d)) if d != 0) && maskable(a)
        }
        // (a size check of vector code is of every lane: `uniform_check`)
        Update { .. } | Slice(..) | Window(..) | Action(..) | Bench(..) => false,
        Pow(..) => isc_ir::passes::cannot_panic(e),
        _ => e.children().into_iter().all(maskable),
    }
}

/// `i` is the index of the loop (a parameter bound to an `iota`) plus or
/// minus values of every lane, or a value of every lane minus it:
/// consecutive indices (D185).
fn affine_index(i: &Expr, varying: &HashSet<VarId>) -> bool {
    match &i.kind {
        ExprKind::Var(v, _) => INDEX_VARS.with(|s| s.borrow().contains(v)),
        ExprKind::Binary(BinOp::Add, a, b) => {
            (affine_index(a, varying) && !mentions(b, varying)) || (affine_index(b, varying) && !mentions(a, varying))
        }
        ExprKind::Binary(BinOp::Sub, a, b) => {
            (affine_index(a, varying) && !mentions(b, varying)) || (!mentions(a, varying) && affine_index(b, varying))
        }
        _ => false,
    }
}

/// The elements of `s` are computed by a lambda that can panic (D185).
fn src_checks(s: &Src) -> bool {
    match s {
        Src::Map(srcs, l) => !isc_ir::passes::cannot_panic(&l.body) || srcs.iter().any(src_checks),
        Src::Offset { inner, .. } => src_checks(inner),
        _ => false,
    }
}

/// `lam_ok` for vector code read by `elem_replayed` (D185).
fn lam_ok_replay(l: &Lambda) -> bool {
    let before = REPLAY.with(|r| r.replace(true));
    let ok = lam_ok(l);
    REPLAY.with(|r| r.set(before));
    ok
}

/// `src_vec_ok_replay`, an index into an array of bytes allowed: for
/// `fill_range`, whose loop is vector code only when each such array has at
/// most 256 bytes (D185).
/// `let x = map(...) in printBytes(x)`: an array of bytes made only to be
/// written, by `materialize` (not a stream, not rows), its elements without
/// a loop of their own (D225).
fn print_stream_ok(var: VarId, init: &Expr, body: &Expr) -> bool {
    matches!(&init.kind, ExprKind::Map(..))
        && init.ty.is_array()
        && *init.ty.elem() == Type::Int(IntKind::U8)
        && !isc_ir::passes::is_stream(init)
        && !matches!(&init.kind, ExprKind::Map(_, l) if inner_loop(&l.body))
        // an input moved into it would no longer be freed with the result
        && !matches!(&init.kind, ExprKind::Map(ins, _) if ins.iter().any(|i| matches!(i.kind, ExprKind::Var(_, Use::Move))))
        && matches!(&body.kind, ExprKind::Action(ActionOp::PrintBytes, args) if args.len() == 1 && matches!(args[0].kind, ExprKind::Var(x, _) if x == var))
}

fn src_vec_ok_lookup(e: &Expr) -> bool {
    let before = LOOKUP_OK.with(|b| b.replace(true));
    let ok = src_vec_ok_replay(e);
    LOOKUP_OK.with(|b| b.set(before));
    ok
}

/// The arrays of bytes that the body of `l` indexes by an index read from
/// another array: permuted in registers by vector code (D185); an index
/// computed from the index of the loop alone reads a window (D187).
fn lookup_tables(l: &Lambda) -> Vec<VarId> {
    fn walk(e: &Expr, out: &mut Vec<VarId>) {
        if let ExprKind::Index(x, i) = &e.kind {
            if let ExprKind::Var(v, _) = x.kind {
                let none = HashSet::new();
                if esize(x.ty.elem()) == 1 && !affine_index(i, &none) && has_index(i) && !out.contains(&v) {
                    out.push(v);
                }
            }
        }
        for c in e.children() {
            walk(c, out);
        }
    }
    let mut out = Vec::new();
    walk(&l.body, &mut out);
    out
}

/// `src_vec_ok` for vector code read by `elem_replayed` (D185).
fn src_vec_ok_replay(e: &Expr) -> bool {
    let before = REPLAY.with(|r| r.replace(true));
    let ok = src_vec_ok(e);
    REPLAY.with(|r| r.set(before));
    ok
}

/// `vec_ok`, the expression `given` (a `reduce` whose value is computed
/// elsewhere, D115) counting as a varying scalar.
fn vec_ok_given(e: &Expr, varying: &mut HashSet<VarId>, given: *const Expr) -> bool {
    if std::ptr::eq(e, given) {
        return true;
    }
    if !e.ty.is_scalar() {
        return false;
    }
    let has_given = |x: &Expr| -> bool {
        fn find(x: &Expr, g: *const Expr) -> bool {
            std::ptr::eq(x, g) || x.children().into_iter().any(|c| find(c, g))
        }
        !given.is_null() && find(x, given)
    };
    match &e.kind {
        ExprKind::Lit(_) | ExprKind::Var(..) => true,
        ExprKind::Let { var, init, body, .. } => {
            if !vec_ok_given(init, varying, given) {
                return false;
            }
            if mentions(init, varying) || has_given(init) {
                varying.insert(*var);
            }
            vec_ok_given(body, varying, given)
        }
        ExprKind::Unary(_, x) | ExprKind::Convert(x) | ExprKind::Sqrt(x) => vec_ok_given(x, varying, given),
        // `fma` is an instruction; the other functions are calls of the
        // runtime, one element at a time
        ExprKind::Math(MathFn::Fma, xs) => xs.iter().all(|x| vec_ok_given(x, varying, given)),
        ExprKind::Binary(_, a, b) => vec_ok_given(a, varying, given) && vec_ok_given(b, varying, given),
        ExprKind::Pow(b, k) => {
            vec_ok_given(b, varying, given) && !mentions(k, varying) && !has_given(k) && vec_ok_given(k, varying, given)
        }
        // both branches computed; under the mask of its lanes, a branch may
        // index arrays or divide integers when a failed check is replayed
        // one element at a time (D185)
        ExprKind::If(c, t, f) => {
            vec_ok_given(c, varying, given)
                && vec_ok_given(t, varying, given)
                && vec_ok_given(f, varying, given)
                && ((safe(t) && safe(f)) || (REPLAY.with(|r| r.get()) && maskable(t) && maskable(f)))
        }
        // a SPMD call (D140)
        ExprKind::Call(f, args) => SPMD.with(|s| s.borrow().contains(f)) && args.iter().all(|a| vec_ok_given(a, varying, given)),
        // the length of an array of the context, two sizes of every lane
        // compared, the reduction (order free) of an array of the context
        // (D185)
        ExprKind::Len(x) => !mentions(x, varying) && !has_given(x),
        ExprKind::CheckSize(a, b, body) => {
            !mentions(a, varying)
                && !mentions(b, varying)
                && !has_given(a)
                && !has_given(b)
                && vec_ok_given(a, varying, given)
                && vec_ok_given(b, varying, given)
                && vec_ok_given(body, varying, given)
        }
        ExprKind::Reduce(x, op, z) => {
            isc_ir::passes::order_free(op, z)
                && match &x.kind {
                    ExprKind::Var(..) => !mentions(x, varying) && x.ty.elem().is_scalar(),
                    ExprKind::Map(ins, m) if ins.len() == 1 && matches!(ins[0].kind, ExprKind::Var(..)) => {
                        !mentions(&ins[0], varying) && ins[0].ty.elem().is_scalar() && {
                            varying.remove(&m.params[0]);
                            vec_ok_given(&m.body, varying, given)
                        }
                    }
                    _ => false,
                }
        }
        // a size checked once for every lane (`iota(k)` in a `fold`, D183)
        ExprKind::CheckNonNegative(k, b) => {
            !mentions(k, varying) && !has_given(k) && vec_ok_given(k, varying, given) && vec_ok_given(b, varying, given)
        }
        // an element of an array of the context, by an index that cannot
        // panic (D181): a load of consecutive elements or a gather
        ExprKind::Index(x, i) => {
            matches!(x.kind, ExprKind::Var(..))
                && !mentions(x, varying)
                && !has_given(x)
                && matches!(x.ty.elem(), Type::Int(_) | Type::F64 | Type::F32)
                && (isc_ir::passes::cannot_panic(i) || REPLAY.with(|r| r.get()))
                && vec_ok_given(i, varying, given)
                // elements of fewer than 32 bits: no gather instruction; their
                // index consecutive (a load), or bytes permuted (D185)
                && (esize(x.ty.elem()) >= 4
                    || affine_index(i, varying)
                    || (esize(x.ty.elem()) == 1 && BYTE_LOOKUP.with(|b| b.get()) && LOOKUP_OK.with(|b| b.get())))
        }
        // a `fold` over the same elements in every lane (`iota(8)`, an
        // array of the context): a scalar loop of a vector accumulator
        ExprKind::Fold(x, init, l) => {
            matches!(x.kind, ExprKind::Iota(_) | ExprKind::Var(..))
                && x.ty.elem().is_scalar()
                && !mentions(x, varying)
                && !has_given(x)
                && vec_ok_given(init, varying, given)
                && {
                    varying.insert(l.params[0]);
                    varying.remove(&l.params[1]);
                    vec_ok_given(&l.body, varying, given)
                }
        }
        _ => false,
    }
}

fn lam_ok(l: &Lambda) -> bool {
    let mut varying: HashSet<VarId> = l.params.iter().copied().collect();
    vec_ok(&l.body, &mut varying)
}

fn src_vec_ok(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Map(ins, l) => {
            // the parameters bound to an `iota` are the index of the loop
            INDEX_VARS.with(|s| {
                let mut s = s.borrow_mut();
                for (x, p) in ins.iter().zip(&l.params) {
                    if matches!(x.kind, ExprKind::Iota(_)) {
                        s.insert(*p);
                    }
                }
            });
            ins.iter().all(src_vec_ok) && lam_ok(l)
        }
        ExprKind::Slice(x, _, _) if isc_ir::passes::is_producer(x) => src_vec_ok(x),
        _ => e.ty.is_array() && e.ty.elem().is_scalar(),
    }
}

/// Structural equality of two expressions, ignoring positions and moves
/// (enough for the index expressions of rule P).
fn same_expr(a: &Expr, b: &Expr) -> bool {
    use ExprKind::*;
    if a.ty != b.ty {
        return false;
    }
    match (&a.kind, &b.kind) {
        (Lit(x), Lit(y)) => x == y,
        (Var(x, _), Var(y, _)) => x == y,
        (Unary(o, x), Unary(p, y)) => o == p && same_expr(x, y),
        (Binary(o, x1, x2), Binary(p, y1, y2)) => o == p && same_expr(x1, y1) && same_expr(x2, y2),
        (Convert(x), Convert(y)) => same_expr(x, y),
        (Index(x1, x2), Index(y1, y2)) => same_expr(x1, y1) && same_expr(x2, y2),
        _ => false,
    }
}

/// Rule P of the privatized accumulation (DECISIONS D50): the function of a
/// `fold` is `(c, v) -> c with [g(v)] = c[g(v)] + h(v)` (or `h(v) + c[g(v)]`)
/// with an integer array accumulator, and neither `g` nor `h` uses `c`.
/// Integer addition modulo 2^bits is associative and commutative, so the fold
/// may be computed by chunks into private zero copies added to `init` at the
/// end: same result for any chunking. Returns `(g(v), h(v), position of the
/// read c[g(v)])`: the read is evaluated before the update (section 8, rule
/// 2), so it is the one that reports an index out of bounds.
fn additive_update(l: &Lambda) -> Option<(&Expr, &Expr, Span)> {
    if l.params.len() != 2 {
        return None;
    }
    let c = l.params[0];
    if !matches!(&l.body.ty, Type::Array(e, _) if matches!(**e, Type::Int(_))) {
        return None;
    }
    let (target, index, value) = match &l.body.kind {
        ExprKind::Update { target, index, value, .. } => (target, index, value),
        _ => return None,
    };
    if !matches!(target.kind, ExprKind::Var(t, _) if t == c) {
        return None;
    }
    let reads = |e: &Expr| matches!(&e.kind, ExprKind::Index(a, i) if matches!(a.kind, ExprKind::Var(t, _) if t == c) && same_expr(i, index));
    let (h, read) = match &value.kind {
        ExprKind::Binary(BinOp::Add, a, b) if reads(a) => (b, a.span),
        ExprKind::Binary(BinOp::Add, a, b) if reads(b) => (a, b.span),
        _ => return None,
    };
    let cs: HashSet<VarId> = [c].into_iter().collect();
    if mentions(index, &cs) || mentions(h, &cs) {
        return None;
    }
    Some((index, h, read))
}

impl<'p> Gen<'p> {
    // ----- emission helpers ------------------------------------------------

    fn emit(&mut self, s: impl Into<String>) {
        self.f.body.push(format!("  {}", s.into()));
    }

    fn tmp(&mut self) -> String {
        self.f.tmp += 1;
        format!("%t{}", self.f.tmp)
    }

    fn ins(&mut self, rhs: String) -> String {
        // the length of an array that its type fixed: the constant (D205),
        // so that the loops over it have a known count here too
        if rhs.starts_with("extractvalue {") {
            if let Some((head, field)) = rhs.rsplit_once(", ") {
                if let (Ok(k), Some(reg)) = (field.parse::<u32>(), head.rsplit(' ').next()) {
                    if let Some(n) = self.f.known_len.get(&(reg.to_string(), k)) {
                        return n.to_string();
                    }
                }
            }
        }
        let t = self.tmp();
        self.emit(format!("{} = {}", t, rhs));
        t
    }

    fn label(&mut self) -> String {
        self.f.tmp += 1;
        format!("L{}", self.f.tmp)
    }

    fn block(&mut self, l: &str) {
        self.f.body.push(format!("{}:", l));
    }

    fn br(&mut self, l: &str) {
        self.emit(format!("br label %{}", l));
    }

    fn cbr(&mut self, c: &str, a: &str, b: &str) {
        self.emit(format!("br i1 {}, label %{}, label %{}", c, a, b));
    }

    fn alloca(&mut self, ty: &str) -> String {
        let t = self.tmp();
        self.f.allocas.push(format!("  {} = alloca {}", t, ty));
        t
    }


    /// Function attributes (the attribute group of `Isa::Avx512`).
    fn attrs(&self) -> &'static str {
        if self.opts.isa == Isa::Avx512 {
            " #0"
        } else {
            ""
        }
    }

    fn finish(&mut self, header: &str) -> String {
        let f = std::mem::take(&mut self.f);
        let mut s = format!("{} {{\nentry:\n", header);
        for a in &f.allocas {
            s.push_str(a);
            s.push('\n');
        }
        for b in &f.body {
            s.push_str(b);
            s.push('\n');
        }
        s.push_str("}\n");
        s
    }

    fn pos(span: Span) -> String {
        format!("i32 {}, i32 {}", span.line, span.col)
    }

    /// A recursive function: `define internal <result> @isc_fn_<i>(<params>)`
    /// (D102). Its parameters are borrowed, its result is new.
    fn emit_fn(&mut self, i: usize, in_region: bool) -> R<()> {
        let f = &self.p.fns[i];
        // the variant of a builder (D200): its nodes cut from the region of
        // its first parameter
        let saved_region = std::mem::replace(&mut self.region, if in_region { Some("%rg".to_string()) } else { None });
        let saved_f = std::mem::take(&mut self.f);
        let saved_env = std::mem::take(&mut self.env);
        let saved_divmod = std::mem::take(&mut self.divmod);
        let saved_vec_fail = self.vec_fail.take();
        let saved_task = std::mem::replace(&mut self.in_task, false);
        let mut ps = Vec::new();
        if in_region {
            ps.push("ptr %rg".to_string());
        }
        for (k, var) in f.params.iter().enumerate() {
            let t = lt(&self.p.vars[*var as usize].ty);
            ps.push(format!("{} %a{}", t, k));
            self.env.insert(*var, v(format!("%a{}", k), t));
        }
        if !self.opts.isa.is_wasm() {
            self.stack_check(&f.name, f.span);
        }
        // the state of a loop (D206): lent by the caller, copied once here;
        // its body takes it apart, each iteration receives a new one
        let mut entry: Vec<String> = (0..f.params.len()).map(|k| format!("%a{}", k)).collect();
        for (k, var) in f.params.iter().enumerate() {
            if f.consumed.get(k).copied().unwrap_or(false) {
                let ty = self.p.vars[*var as usize].ty.clone();
                let t = lt(&ty);
                let x = self.copy_value(&v(entry[k].clone(), t.clone()), &ty);
                entry[k] = x.r.clone();
                self.env.insert(*var, v(x.r, t));
            }
        }
        // direct tail calls: the parameters in slots, the body a loop
        let calls = isc_ir::passes::tail_calls(i as u32, f);
        // the tail calls to other functions, unless the loop of this one owns
        // an array (it must free it after such a call)
        let moves = calls.values().any(|c| !c.moved.is_empty());
        // (WebAssembly: no other calling convention than C's, so no such call)
        let others =
            if moves || self.opts.isa.is_wasm() { HashMap::new() } else { isc_ir::passes::other_tail_calls(i as u32, f) };
        let saved_others = std::mem::replace(&mut self.other_tails, others);
        let saved_tail = self.tail.take();
        if !calls.is_empty() {
            let mut slots = Vec::new();
            for (k, var) in f.params.iter().enumerate() {
                let ty = self.p.vars[*var as usize].ty.clone();
                let t = lt(&ty);
                let s = self.alloca(&t);
                self.emit(format!("store {} {}, ptr {}", t, entry[k], s));
                // a state is consumed by each iteration: never freed by the
                // loop (no flag)
                let consumed = f.consumed.get(k).copied().unwrap_or(false);
                let owned = if ty.has_heap() && !consumed {
                    let o = self.alloca("i1");
                    self.emit(format!("store i1 false, ptr {}", o));
                    Some(o)
                } else {
                    None
                };
                slots.push((s, owned, ty));
            }
            let head = self.label();
            self.br(&head);
            self.block(&head);
            for (var, (s, _, ty)) in f.params.iter().zip(&slots) {
                let t = lt(ty);
                let x = self.ins(format!("load {}, ptr {}", t, s));
                self.env.insert(*var, v(x, t));
            }
            self.tail = Some(TailLoop { calls, head, slots });
        }
        let ret = lt(&f.body.ty);
        let r = self.ex(&f.body, 1);
        if let Ok(x) = &r {
            // the parameters the loop owns are freed on return
            if let Some(tl) = self.tail.take() {
                for (s, owned, ty) in &tl.slots {
                    let Some(o) = owned else { continue };
                    let own = self.ins(format!("load i1, ptr {}", o));
                    let lf = self.label();
                    let ld = self.label();
                    self.cbr(&own, &lf, &ld);
                    self.block(&lf);
                    let t = lt(ty);
                    let val = self.ins(format!("load {}, ptr {}", t, s));
                    self.drop_value(&v(val, t), ty);
                    self.br(&ld);
                    self.block(&ld);
                }
            }
            self.emit(format!("ret {} {}", ret, x.r));
        }
        self.tail = saved_tail;
        let attrs = self.attrs();
        // `tailcc`: a call marked `tail` and followed by `ret` reuses the frame
        // (mutual tail recursion, D113)
        let cc = self.fn_cc();
        let name = if in_region { format!("{}_rg", i) } else { i.to_string() };
        // on 64 bytes (D213): a small function called at each node of a
        // tree stays whole in the cache of decoded instructions
        let text = self.finish(&format!("define internal {}{} @isc_fn_{}({}){} align 64", cc, ret, name, ps.join(", "), attrs));
        self.region = saved_region;
        self.other_tails = saved_others;
        self.f = saved_f;
        self.env = saved_env;
        self.divmod = saved_divmod;
        self.vec_fail = saved_vec_fail;
        self.in_task = saved_task;
        r?;
        self.funcs.push(text);
        Ok(())
    }

    /// A direct tail call: the new parameters in their slots, then a jump to
    /// the start of the body. A temporary passed as an array parameter is
    /// moved into the loop (the old value freed if the loop owned it).
    fn tail_jump(&mut self, e: &Expr, args: &[Expr]) -> R<V> {
        let mut vals = Vec::new();
        for a in args {
            vals.push(self.ex(a, 1)?);
        }
        let tl = self.tail.take().unwrap();
        let tc = &tl.calls[&(e as *const Expr)];
        for var in &tc.free_first {
            if self.stack_vars.contains(var) {
                continue;
            }
            if let Some(rg) = self.region_vars.get(var).cloned() {
                self.emit(format!("call void @isc_region_free(ptr {})", rg));
                continue;
            }
            let ty = self.p.vars[*var as usize].ty.clone();
            let x = self.env[var].clone();
            self.drop_value(&x, &ty);
        }
        for (k, x) in vals.iter().enumerate() {
            let (s, owned, ty) = &tl.slots[k];
            let t = lt(ty);
            if let (Some((_, _)), Some(o)) = (tc.moved.iter().find(|(j, _)| *j == k), owned.as_ref()) {
                let own = self.ins(format!("load i1, ptr {}", o));
                let lf = self.label();
                let ld = self.label();
                self.cbr(&own, &lf, &ld);
                self.block(&lf);
                let old = self.ins(format!("load {}, ptr {}", t, s));
                self.drop_value(&v(old, t.clone()), ty);
                self.br(&ld);
                self.block(&ld);
                self.emit(format!("store i1 true, ptr {}", o));
            }
            self.emit(format!("store {} {}, ptr {}", t, x.r, s));
        }
        let head = tl.head.clone();
        self.tail = Some(tl);
        self.br(&head);
        // the rest of the expression is never reached
        let dead = self.label();
        self.block(&dead);
        let ret = lt(&e.ty);
        Ok(V { r: "poison".into(), t: ret, uni: true })
    }

    /// The calling convention of the recursive functions: `tailcc` (D113),
    /// C's on WebAssembly.
    fn fn_cc(&self) -> &'static str {
        if self.opts.isa.is_wasm() {
            ""
        } else {
            "tailcc "
        }
    }

    /// At the entry of a recursive function: a panic when the frame is
    /// below the floor of the thread's stack (`isc_stack_floor`, computed
    /// once per thread, kept in a thread-local variable).
    fn stack_check(&mut self, name: &str, span: Span) {
        self.decls.insert("declare i64 @isc_stack_floor()".into());
        self.decls.insert("declare void @isc_panic_stack(ptr, i64, i32, i32) noreturn".into());
        self.decls.insert("declare ptr @llvm.frameaddress.p0(i32)".into());
        let tls = "@isc_stack_floor_tls = internal thread_local global i64 0".to_string();
        if !self.globals.contains(&tls) {
            self.globals.push(tls);
        }
        let slot = self.alloca("i64");
        let fl = self.ins("load i64, ptr @isc_stack_floor_tls".to_string());
        self.emit(format!("store i64 {}, ptr {}", fl, slot));
        let unset = self.ins(format!("icmp eq i64 {}, 0", fl));
        let linit = self.label();
        let ljoin = self.label();
        self.cbr(&unset, &linit, &ljoin);
        self.block(&linit);
        let x = self.ins("call i64 @isc_stack_floor()".to_string());
        // unknown (0): 1, so that it is not asked again and never stops
        let z = self.ins(format!("icmp eq i64 {}, 0", x));
        let x1 = self.ins(format!("select i1 {}, i64 1, i64 {}", z, x));
        self.emit(format!("store i64 {}, ptr @isc_stack_floor_tls", x1));
        self.emit(format!("store i64 {}, ptr {}", x1, slot));
        self.br(&ljoin);
        self.block(&ljoin);
        let floor = self.ins(format!("load i64, ptr {}", slot));
        let fa = self.ins("call ptr @llvm.frameaddress.p0(i32 0)".to_string());
        let fi = self.ins(format!("ptrtoint ptr {} to i64", fa));
        let ok = self.ins(format!("icmp uge i64 {}, {}", fi, floor));
        let (sname, n) = self.str_const(name);
        self.guard(&ok, format!("call void @isc_panic_stack(ptr {}, i64 {}, {})", sname, n, Self::pos(span)));
    }

    /// Branches to a panic call unless `ok` holds.
    fn guard(&mut self, ok: &str, call: String) {
        let lok = self.label();
        let lfail = self.label();
        self.cbr(ok, &lok, &lfail);
        self.block(&lfail);
        self.emit(call);
        self.emit("unreachable");
        self.block(&lok);
    }

    /// A constant of any bytes (a `bigint`, D168).
    fn bytes_const(&mut self, b: &[u8]) -> (String, usize) {
        let key = format!("\u{0}bytes:{}", b.iter().map(|x| format!("{:02x}", x)).collect::<String>());
        if let Some(x) = self.strs.get(&key) {
            return x.clone();
        }
        let name = format!("@.str.{}", self.strs.len());
        let n = b.len();
        let text = if n == 0 { "\\00".to_string() } else { b.iter().map(|x| format!("\\{:02X}", x)).collect::<String>() };
        self.globals.push(format!("{} = private unnamed_addr constant [{} x i8] c\"{}\"", name, n.max(1), text));
        self.strs.insert(key, (name.clone(), n));
        (name, n)
    }

    fn str_const(&mut self, s: &str) -> (String, usize) {
        if let Some(x) = self.strs.get(s) {
            return x.clone();
        }
        let name = format!("@.str.{}", self.strs.len());
        let n = s.len();
        self.globals.push(format!(
            "{} = private unnamed_addr constant [{} x i8] c\"{}\"",
            name,
            n.max(1),
            if n == 0 { "\\00".to_string() } else { llvm_string(s) }
        ));
        self.strs.insert(s.to_string(), (name.clone(), n));
        (name, n)
    }

    /// `for (i = lo; i < hi - (step - 1); i += step) body(i)`
    fn for_range(&mut self, lo: &str, hi: &str, step: i64, body: &mut dyn FnMut(&mut Self, &str) -> R<()>) -> R<()> {
        self.for_range_md(lo, hi, step, false, body)
    }

    /// `for_range` that LLVM neither vectorizes nor unrolls: for loops of a
    /// few iterations, which LLVM would otherwise turn into a vector loop,
    /// its run-time checks and its epilogue (compilation time, D86).
    fn for_range_scalar(&mut self, lo: &str, hi: &str, step: i64, body: &mut dyn FnMut(&mut Self, &str) -> R<()>) -> R<()> {
        self.for_range_md(lo, hi, step, true, body)
    }

    fn for_range_md(&mut self, lo: &str, hi: &str, step: i64, scalar: bool, body: &mut dyn FnMut(&mut Self, &str) -> R<()>) -> R<()> {
        let ia = self.alloca("i64");
        self.emit(format!("store i64 {}, ptr {}", lo, ia));
        let limit = if step == 1 { hi.to_string() } else { self.ins(format!("sub i64 {}, {}", hi, step - 1)) };
        let head = self.label();
        let lbody = self.label();
        let exit = self.label();
        self.br(&head);
        self.block(&head);
        let i = self.ins(format!("load i64, ptr {}", ia));
        let c = self.ins(format!("icmp slt i64 {}, {}", i, limit));
        self.cbr(&c, &lbody, &exit);
        self.block(&lbody);
        body(self, &i)?;
        let i2 = self.ins(format!("add i64 {}, {}", i, step));
        self.emit(format!("store i64 {}, ptr {}", i2, ia));
        // a constant count of at most UNROLL_FULL: unrolled whole, whatever
        // the size of the body (LLVM's threshold kept the 10 pairs of n-body
        // a loop, its roots and divisions one after the other, D205)
        let count = match (lo.parse::<i64>(), hi.parse::<i64>()) {
            (Ok(a), Ok(b)) if step > 0 && b > a => Some((b - a + step - 1) / step),
            _ => None,
        };
        if scalar {
            // a distinct loop identifier, its properties !0 and !1 (see `emit_with`)
            self.scalar_loops += 1;
            self.emit(format!("br label %{}, !llvm.loop !{}", head, self.scalar_loops + 1));
        } else if count.is_some_and(|c| (2..=UNROLL_FULL).contains(&c)) {
            self.unrolled_loops += 1;
            self.emit(format!("br label %{}, !llvm.loop !{}", head, UNROLL_IDS + self.unrolled_loops as u64));
        } else {
            self.br(&head);
        }
        self.block(&exit);
        Ok(())
    }

    /// Repeats a uniform value in every lane.
    fn widen(&mut self, x: &V, w: u32) -> V {
        if w == 1 || !uni(&x) {
            return x.clone();
        }
        let vty = format!("<{} x {}>", w, x.t);
        let a = self.ins(format!("insertelement {} poison, {} {}, i64 0", vty, x.t, x.r));
        let b = self.ins(format!("shufflevector {} {}, {} poison, <{} x i32> zeroinitializer", vty, a, vty, w));
        V { r: b, t: vty, uni: false }
    }

    /// A constant `c` of scalar type `t`, at width `w`.
    fn cst(&self, t: &str, c: &str, w: u32) -> String {
        if w == 1 {
            c.to_string()
        } else {
            format!("<{}>", vec![format!("{} {}", t, c); w as usize].join(", "))
        }
    }

    /// The block, rows, columns and stride of an array of arrays.
    fn nested_parts(&mut self, x: &V) -> (String, String, String, String) {
        let f: Vec<String> = (0..4).map(|i| self.ins(format!("extractvalue {} {}, {}", NESTED, x.r, i))).collect();
        (f[0].clone(), f[1].clone(), f[2].clone(), f[3].clone())
    }

    fn nested(&mut self, ptr: &str, rows: &str, cols: &str, stride: &str) -> V {
        let mut acc = "poison".to_string();
        for (i, (t, x)) in [("ptr", ptr), ("i64", rows), ("i64", cols), ("i64", stride)].iter().enumerate() {
            acc = self.ins(format!("insertvalue {} {}, {} {}, {}", NESTED, acc, t, x, i));
        }
        v(acc, NESTED)
    }

    /// Row `idx` of an array of arrays: a view `{ ptr, i64 }` of its block.
    fn row_view(&mut self, ptr: &str, cols: &str, stride: &str, idx: &str, elem: &Type) -> V {
        let off = self.ins(format!("mul i64 {}, {}", idx, stride));
        let q = self.ins(format!("getelementptr inbounds {}, ptr {}, i64 {}", lt(elem), ptr, off));
        self.arr(&q, cols)
    }

    fn arr(&mut self, ptr: &str, len: &str) -> V {
        let a = self.ins(format!("insertvalue {{ ptr, i64 }} poison, ptr {}, 0", ptr));
        let b = self.ins(format!("insertvalue {{ ptr, i64 }} {}, i64 {}, 1", a, len));
        v(b, "{ ptr, i64 }")
    }

    // ----- arrays of tuples: one column per scalar leaf (SPEC section 4) -----
    //
    // An array of `n` tuples is one block: the column of leaf c starts at the
    // sum of the previous columns, each rounded up to 64 bytes (a cache line,
    // and the alignment of every vector access); the array value is still
    // `{ ptr, i64 }`. An array of scalars is one column.

    /// Bytes of the column of a leaf of `size` bytes, as an IR value.
    fn column_bytes(&mut self, n: &str, size: i64) -> String {
        let b = self.ins(format!("mul i64 {}, {}", n, size));
        let r = self.ins(format!("add i64 {}, 63", b));
        self.ins(format!("and i64 {}, -64", r))
    }

    /// Bytes of an array of `n` elements.
    fn array_bytes(&mut self, n: &str, elem: &Type) -> String {
        if !matches!(elem, Type::Tuple(_)) {
            return self.ins(format!("mul i64 {}, {}", n, esize(elem)));
        }
        let mut total = "0".to_string();
        for (_, leaf) in leaves(elem) {
            let c = self.column_bytes(n, esize(&leaf));
            total = self.ins(format!("add i64 {}, {}", total, c));
        }
        total
    }

    fn alloc_elems(&mut self, n: &str, elem: &Type) -> String {
        let bytes = self.array_bytes(n, elem);
        self.ins(format!("call ptr @isc_alloc(i64 {})", bytes))
    }

    /// A scratch array of `n` elements: on the stack, aligned to 128 bytes,
    /// when it takes at most STACK_SCRATCH bytes, else on the heap. `heap` is
    /// an `i1` slot set to whether it is on the heap (see `free_scratch`);
    /// the caller saves the stack before (`llvm.stacksave`) and restores it
    /// once the array is dead.
    fn alloc_scratch(&mut self, n: &str, elem: &Type, heap: &str) -> String {
        // the stack part is always reserved, 0 bytes for a large array: one
        // branch only, to the heap (less code to compile)
        let bytes = self.array_bytes(n, elem);
        let pp = self.alloca("ptr");
        let small = self.ins(format!("icmp ule i64 {}, {}", bytes, self.opts.stack_scratch));
        let b1 = self.ins(format!("add i64 {}, 127", bytes));
        let b2 = self.ins(format!("and i64 {}, -128", b1));
        let b3 = self.ins(format!("select i1 {}, i64 {}, i64 0", small, b2));
        let s = self.ins(format!("alloca i8, i64 {}, align 128", b3));
        self.emit(format!("store ptr {}, ptr {}", s, pp));
        let large = self.ins(format!("xor i1 {}, true", small));
        self.emit(format!("store i1 {}, ptr {}", large, heap));
        let lheap = self.label();
        let ljoin = self.label();
        self.cbr(&small, &ljoin, &lheap);
        self.block(&lheap);
        let h = self.ins(format!("call ptr @isc_alloc(i64 {})", bytes));
        self.emit(format!("store ptr {}, ptr {}", h, pp));
        self.br(&ljoin);
        self.block(&ljoin);
        self.ins(format!("load ptr, ptr {}", pp))
    }

    /// `i1`: an array of `n` elements of `elem` has at least STREAM_BYTES bytes.
    fn stream_flag(&mut self, n: &str, elem: &Type) -> String {
        let bytes = self.array_bytes(n, elem);
        self.ins(format!("icmp sge i64 {}, {}", bytes, self.opts.stream_bytes))
    }

    /// Frees a scratch array of `alloc_scratch` when it is on the heap.
    fn free_scratch(&mut self, p: &str, heap: &str) {
        let on_heap = self.ins(format!("load i1, ptr {}", heap));
        let lfree = self.label();
        let lend = self.label();
        self.cbr(&on_heap, &lfree, &lend);
        self.block(&lfree);
        self.emit(format!("call void @isc_free(ptr {})", p));
        self.br(&lend);
        self.block(&lend);
    }

    /// Start of the column of each leaf of a tuple element type.
    fn columns(&mut self, ptr: &str, n: &str, elem: &Type) -> Vec<(Vec<u32>, Type, String)> {
        let mut out = Vec::new();
        let mut off = "0".to_string();
        for (path, leaf) in leaves(elem) {
            let base = self.ins(format!("getelementptr inbounds i8, ptr {}, i64 {}", ptr, off));
            let c = self.column_bytes(n, esize(&leaf));
            off = self.ins(format!("add i64 {}, {}", off, c));
            out.push((path, leaf, base));
        }
        out
    }

    /// Element `idx` of an array of `n` elements at `ptr` (`w` consecutive
    /// elements as a vector; tuples are loaded one at a time).
    fn load_elem(&mut self, ptr: &str, n: &str, idx: &str, elem: &Type, w: u32) -> V {
        if let Type::Tuple(_) = elem {
            let ty = lt(elem);
            let mut acc = "poison".to_string();
            for (path, leaf, base) in self.columns(ptr, n, elem) {
                let x = self.load_scalar(&base, idx, &leaf, 1);
                let path: Vec<String> = path.iter().map(|i| i.to_string()).collect();
                acc = self.ins(format!("insertvalue {} {}, {} {}, {}", ty, acc, x.t, x.r, path.join(", ")));
            }
            return V { r: acc, t: ty, uni: false };
        }
        self.load_scalar(ptr, idx, elem, w)
    }

    fn store_elem(&mut self, ptr: &str, n: &str, idx: &str, elem: &Type, x: &V, w: u32) {
        if let Type::Tuple(_) = elem {
            for (path, leaf, base) in self.columns(ptr, n, elem) {
                let path: Vec<String> = path.iter().map(|i| i.to_string()).collect();
                let f = self.ins(format!("extractvalue {} {}, {}", x.t, x.r, path.join(", ")));
                self.store_scalar(&base, idx, &leaf, &v(f, lt(&leaf)), 1);
            }
            return;
        }
        self.store_scalar(ptr, idx, elem, x, w)
    }

    fn load_scalar(&mut self, ptr: &str, idx: &str, elem: &Type, w: u32) -> V {
        let m = mt(elem);
        let g = self.ins(format!("getelementptr inbounds {}, ptr {}, i64 {}", m, ptr, idx));
        if w == 1 {
            let x = self.ins(format!("load {}, ptr {}", m, g));
            if *elem == Type::Bool {
                let b = self.ins(format!("trunc i8 {} to i1", x));
                return V { r: b, t: "i1".into(), uni: false };
            }
            return V { r: x, t: m, uni: false };
        }
        let vm = format!("<{} x {}>", w, m);
        let x = self.ins(format!("load {}, ptr {}, align {}", vm, g, esize(elem)));
        if *elem == Type::Bool {
            let b = self.ins(format!("trunc {} {} to <{} x i1>", vm, x, w));
            return V { r: b, t: format!("<{} x i1>", w), uni: false };
        }
        V { r: x, t: vm, uni: false }
    }

    fn store_scalar(&mut self, ptr: &str, idx: &str, elem: &Type, x: &V, w: u32) {
        let m = mt(elem);
        let g = self.ins(format!("getelementptr inbounds {}, ptr {}, i64 {}", m, ptr, idx));
        let x = self.widen(x, w);
        if w == 1 {
            let val = if *elem == Type::Bool { self.ins(format!("zext i1 {} to i8", x.r)) } else { x.r.clone() };
            self.emit(format!("store {} {}, ptr {}", m, val, g));
        } else {
            let vm = format!("<{} x {}>", w, m);
            let val = if *elem == Type::Bool {
                self.ins(format!("zext <{} x i1> {} to {}", w, x.r, vm))
            } else {
                x.r.clone()
            };
            self.emit(format!("store {} {}, ptr {}, align {}", vm, val, g, esize(elem)));
        }
    }

    fn zero(t: &Type) -> String {
        match t {
            Type::F64 => fconst(0.0),
            Type::F32 => fconst32(0.0),
            Type::Bool | Type::Unit => "false".into(),
            Type::Int(_) => "0".into(),
            Type::Rec(_) => "null".into(),
            _ => "zeroinitializer".into(),
        }
    }

    // ----- memory management -----------------------------------------------

    fn drop_value(&mut self, x: &V, t: &Type) {
        match t {
            // one owner less: the node is freed by the last one (D103)
            Type::Rec(i) => {
                let atomic = self.rec_atomic(*i);
                self.rec_drops.insert((*i, atomic));
                self.emit(format!("call void @{}(ptr {})", rec_drop_name(*i, atomic), x.r));
            }
            Type::Array(e, _) if e.is_array() => {
                let p = self.ins(format!("extractvalue {} {}, 0", NESTED, x.r));
                // each string of the rows (D154): an owned array of arrays
                // is contiguous (its stride is its columns)
                let inner = e.elem().clone();
                if holds_owned(&inner) {
                    let (_, rows, cols, _) = self.nested_parts(x);
                    let total = self.ins(format!("mul i64 {}, {}", rows, cols));
                    self.for_range_scalar("0", &total, 1, &mut |g, i| {
                        let el = g.load_elem(&p, &total, i, &inner, 1);
                        g.drop_value(&el, &inner);
                        Ok(())
                    })
                    .expect("the drop of the elements of the rows");
                }
                self.emit(format!("call void @isc_free(ptr {})", p));
            }
            // a string: its bytes (D134)
            Type::Str | Type::Big => {
                let p = self.ins(format!("extractvalue {{ ptr, i64 }} {}, 0", x.r));
                self.emit(format!("call void @isc_free(ptr {})", p));
            }
            Type::Array(e, _) => {
                let p = self.ins(format!("extractvalue {{ ptr, i64 }} {}, 0", x.r));
                // each node of the elements: one owner less (D130)
                if holds_owned(e) {
                    let n = self.ins(format!("extractvalue {{ ptr, i64 }} {}, 1", x.r));
                    let et = (**e).clone();
                    self.for_range_scalar("0", &n, 1, &mut |g, i| {
                        let el = g.load_elem(&p, &n, i, &et, 1);
                        g.drop_value(&el, &et);
                        Ok(())
                    })
                    .expect("the drop of the elements");
                }
                self.emit(format!("call void @isc_free(ptr {})", p));
            }
            Type::Tuple(ts) => {
                for (i, c) in ts.iter().enumerate() {
                    if c.has_heap() {
                        let f = self.ins(format!("extractvalue {} {}, {}", x.t, x.r, i));
                        self.drop_value(&v(f, lt(c)), c);
                    }
                }
            }
            _ => {}
        }
    }

    fn copy_value(&mut self, x: &V, t: &Type) -> V {
        match t {
            Type::Array(e, _) if e.is_array() => {
                // into one block, rows contiguous (a window becomes a matrix)
                let inner = e.elem().clone();
                let (p, rows, cols, stride) = self.nested_parts(x);
                let total = self.ins(format!("mul i64 {}, {}", rows, cols));
                let q = self.alloc_elems(&total, &inner);
                let es_sz = esize(&inner);
                self.for_range("0", &rows, 1, &mut |g, i| {
                    let so = g.ins(format!("mul i64 {}, {}", i, stride));
                    let from = g.ins(format!("getelementptr inbounds {}, ptr {}, i64 {}", lt(&inner), p, so));
                    let dof = g.ins(format!("mul i64 {}, {}", i, cols));
                    let to = g.ins(format!("getelementptr inbounds {}, ptr {}, i64 {}", lt(&inner), q, dof));
                    if holds_owned(&inner) {
                        // each string: a copy of its own (D154)
                        g.for_range_scalar("0", &cols, 1, &mut |g, j| {
                            let el = g.load_elem(&from, &cols, j, &inner, 1);
                            let c = g.copy_value(&el, &inner);
                            g.store_elem(&to, &cols, j, &inner, &c, 1);
                            Ok(())
                        })?;
                    } else {
                        let bytes = g.ins(format!("mul i64 {}, {}", cols, es_sz));
                        g.emit(format!("call void @llvm.memcpy.p0.p0.i64(ptr {}, ptr {}, i64 {}, i1 false)", to, from, bytes));
                    }
                    Ok(())
                })
                .expect("copy of rows");
                self.nested(&q, &rows, &cols, &cols)
            }
            Type::Array(e, _) => {
                let p = self.ins(format!("extractvalue {{ ptr, i64 }} {}, 0", x.r));
                let n = self.ins(format!("extractvalue {{ ptr, i64 }} {}, 1", x.r));
                let bytes = self.array_bytes(&n, e);
                let q = self.ins(format!("call ptr @isc_alloc(i64 {})", bytes));
                self.emit(format!("call void @llvm.memcpy.p0.p0.i64(ptr {}, ptr {}, i64 {}, i1 false)", q, p, bytes));
                // each node of the elements: one owner more (D130); each
                // string: a copy of its own (D134)
                if holds_owned(e) {
                    let et = (**e).clone();
                    self.for_range_scalar("0", &n, 1, &mut |g, i| {
                        let el = g.load_elem(&p, &n, i, &et, 1);
                        let c = g.copy_value(&el, &et);
                        g.store_elem(&q, &n, i, &et, &c, 1);
                        Ok(())
                    })
                    .expect("the copy of the elements");
                }
                self.arr(&q, &n)
            }
            // one owner more (D103)
            Type::Rec(i) => {
                self.rec_incref(&x.r, *i);
                x.clone()
            }
            // a string: a new one (D134)
            Type::Str | Type::Big => {
                let (p, n) = self.str_parts(x);
                let out = self.alloca("{ ptr, i64 }");
                self.emit(format!("call void @isc_str_new(ptr {}, ptr {}, i64 {})", out, p, n));
                v(self.ins(format!("load {{ ptr, i64 }}, ptr {}", out)), "{ ptr, i64 }")
            }
            Type::Tuple(ts) => {
                let mut acc = "poison".to_string();
                for (i, c) in ts.iter().enumerate() {
                    let f = self.ins(format!("extractvalue {} {}, {}", x.t, x.r, i));
                    let fc = self.copy_value(&v(f, lt(c)), c);
                    acc = self.ins(format!("insertvalue {} {}, {} {}, {}", x.t, acc, fc.t, fc.r, i));
                }
                v(acc, x.t.clone())
            }
            _ => x.clone(),
        }
    }

    /// The type of a node in memory: its count, then its tuple.
    fn rec_node_ty(&self, i: u32) -> String {
        format!("{{ i64, {} }}", lt(&self.p.recs[i as usize].layout))
    }

    /// The cells of nodes in the code itself (D176): the free list of the
    /// thread (`isc_node_thread` of the runtime, `heap.rs`) when the
    /// allocator of Isocron is in use and the node takes at most 256
    /// bytes; the runtime otherwise (glibc, WebAssembly, a refill).
    fn node_decls(&mut self) {
        self.decls.insert("@isc_heap_mode = external global i8".into());
        self.decls.insert("@isc_node_thread = external hidden thread_local(localexec) global [32 x ptr], align 64".into());
    }

    /// `i1`: the free list of the thread serves nodes of `bytes` bytes.
    fn node_fast(&mut self, bytes: &str) -> String {
        self.node_decls();
        let mode = self.ins("load i8, ptr @isc_heap_mode".to_string());
        let isc = self.ins(format!("icmp eq i8 {}, 2", mode));
        let small = self.ins(format!("icmp ule i64 {}, 256", bytes));
        self.ins(format!("and i1 {}, {}", isc, small))
    }

    /// The slot of the free list of nodes of `bytes` bytes.
    fn node_slot(&mut self, bytes: &str) -> String {
        let c1 = self.ins(format!("add i64 {}, 15", bytes));
        let c2 = self.ins(format!("lshr i64 {}, 4", c1));
        let c = self.ins(format!("sub i64 {}, 1", c2));
        self.ins(format!("getelementptr inbounds [32 x ptr], ptr @isc_node_thread, i64 0, i64 {}", c))
    }

    /// A cell of `bytes` bytes of the region `rg` (D200): its bump, the
    /// runtime when the chunk is full (or with glibc: no chunk).
    fn region_alloc(&mut self, rg: &str, bytes: &str) -> String {
        self.decls.insert("declare ptr @isc_region_refill(ptr, i64)".into());
        let b = self.ins(format!("load ptr, ptr {}", rg));
        let ep = self.ins(format!("getelementptr inbounds ptr, ptr {}, i64 1", rg));
        let e = self.ins(format!("load ptr, ptr {}", ep));
        let n = self.ins(format!("getelementptr i8, ptr {}, i64 {}", b, bytes));
        let fits = self.ins(format!("icmp ule ptr {}, {}", n, e));
        let nonnull = self.ins(format!("icmp ne ptr {}, null", b));
        let ok = self.ins(format!("and i1 {}, {}", fits, nonnull));
        let (lfast, lslow, lj) = (self.label(), self.label(), self.label());
        self.cbr(&ok, &lfast, &lslow);
        self.block(&lfast);
        self.emit(format!("store ptr {}, ptr {}", n, rg));
        self.br(&lj);
        self.block(&lslow);
        let q = self.ins(format!("call ptr @isc_region_refill(ptr {}, i64 {})", rg, bytes));
        self.br(&lj);
        self.block(&lj);
        self.ins(format!("phi ptr [ {}, %{} ], [ {}, %{} ]", b, lfast, q, lslow))
    }

    fn node_alloc(&mut self, bytes: &str) -> String {
        if self.opts.isa.is_wasm() {
            return self.ins(format!("call ptr @isc_node_alloc(i64 {})", bytes));
        }
        let ok = self.node_fast(bytes);
        let (lfast, lpop, lslow, lj) = (self.label(), self.label(), self.label(), self.label());
        self.cbr(&ok, &lfast, &lslow);
        self.block(&lfast);
        let slot = self.node_slot(bytes);
        let h = self.ins(format!("load ptr, ptr {}", slot));
        let some = self.ins(format!("icmp ne ptr {}, null", h));
        self.cbr(&some, &lpop, &lslow);
        self.block(&lpop);
        let next = self.ins(format!("load ptr, ptr {}", h));
        self.emit(format!("store ptr {}, ptr {}", next, slot));
        self.br(&lj);
        self.block(&lslow);
        let q = self.ins(format!("call ptr @isc_node_alloc(i64 {})", bytes));
        self.br(&lj);
        self.block(&lj);
        self.ins(format!("phi ptr [ {}, %{} ], [ {}, %{} ]", h, lpop, q, lslow))
    }

    fn node_free(&mut self, p: &str, bytes: &str) {
        if self.opts.isa.is_wasm() {
            self.emit(format!("call void @isc_node_free(ptr {}, i64 {})", p, bytes));
            return;
        }
        let ok = self.node_fast(bytes);
        let (lfast, lslow, lj) = (self.label(), self.label(), self.label());
        self.cbr(&ok, &lfast, &lslow);
        self.block(&lfast);
        let slot = self.node_slot(bytes);
        let old = self.ins(format!("load ptr, ptr {}", slot));
        self.emit(format!("store ptr {}, ptr {}", old, p));
        self.emit(format!("store ptr {}, ptr {}", p, slot));
        self.br(&lj);
        self.block(&lslow);
        self.emit(format!("call void @isc_node_free(ptr {}, i64 {})", p, bytes));
        self.br(&lj);
        self.block(&lj);
    }

    /// `i1`: the value of a recursive type is a node in memory (not null,
    /// not a constructor without payload, D169).
    fn is_node(&mut self, p: &str) -> String {
        let pi = self.ins(format!("ptrtoint ptr {} to i64", p));
        let low = self.ins(format!("and i64 {}, 1", pi));
        let even = self.ins(format!("icmp eq i64 {}, 0", low));
        let some = self.ins(format!("icmp ne i64 {}, 0", pi));
        self.ins(format!("and i1 {}, {}", even, some))
    }

    /// The tuple of a value of the recursive type `i`: loaded from its node,
    /// or, for a constructor without payload (an odd value, D169), its tag
    /// and zeros.
    fn node_tuple(&mut self, p: &str, i: u32) -> String {
        let nt = self.rec_node_ty(i);
        let lay = lt(&self.p.recs[i as usize].layout);
        let pi = self.ins(format!("ptrtoint ptr {} to i64", p));
        let low = self.ins(format!("and i64 {}, 1", pi));
        let odd = self.ins(format!("icmp ne i64 {}, 0", low));
        let ltag = self.label();
        let lload = self.label();
        let lj = self.label();
        self.cbr(&odd, &ltag, &lload);
        self.block(&ltag);
        let t = self.ins(format!("lshr i64 {}, 1", pi));
        let t32 = self.ins(format!("trunc i64 {} to i32", t));
        let tagged = self.ins(format!("insertvalue {} zeroinitializer, i32 {}, 0", lay, t32));
        self.br(&lj);
        self.block(&lload);
        let pt = self.ins(format!("getelementptr inbounds {}, ptr {}, i32 0, i32 1", nt, p));
        let loaded = self.ins(format!("load {}, ptr {}", lay, pt));
        self.br(&lj);
        self.block(&lj);
        self.ins(format!("phi {} [ {}, %{} ], [ {}, %{} ]", lay, tagged, ltag, loaded, lload))
    }

    /// The recursive types reached from a type, through the layouts of
    /// their nodes.
    fn rec_types_in(&self, t: &Type, out: &mut HashSet<u32>) {
        match t {
            Type::Rec(i) => {
                if out.insert(*i) {
                    let l = self.p.recs[*i as usize].layout.clone();
                    self.rec_types_in(&l, out);
                }
            }
            Type::Tuple(ts) => ts.iter().for_each(|x| self.rec_types_in(x, out)),
            Type::Array(e, _) => self.rec_types_in(e, out),
            _ => {}
        }
    }

    /// The counts of the nodes of the recursive type `i` must be atomic:
    /// in a task, unless no type reached from `i` can be shared with
    /// another thread (D175).
    fn rec_atomic(&self, i: u32) -> bool {
        if !self.in_task {
            return false;
        }
        let Some(shared) = &self.task_shared else { return true };
        let mut reach = HashSet::new();
        self.rec_types_in(&Type::Rec(i), &mut reach);
        reach.iter().any(|r| shared.contains(r))
    }

    /// Adds one owner to a node: atomic in a task (other threads may hold
    /// it), a plain increment otherwise (SPEC section 8); nothing for a
    /// constructor without payload (D169).
    fn rec_incref(&mut self, p: &str, i: u32) {
        let node = self.is_node(p);
        let linc = self.label();
        let lj = self.label();
        self.cbr(&node, &linc, &lj);
        self.block(&linc);
        if self.rec_atomic(i) {
            self.emit(format!("atomicrmw add ptr {}, i64 1 monotonic", p));
        } else {
            let c = self.ins(format!("load i64, ptr {}", p));
            let c1 = self.ins(format!("add i64 {}, 1", c));
            self.emit(format!("store i64 {}, ptr {}", c1, p));
        }
        self.br(&lj);
        self.block(&lj);
    }

    /// The frees and prints of nodes asked for so far (each may ask for
    /// others: until none is left).
    fn emit_rec_helpers(&mut self) -> R<()> {
        let mut done_drops = BTreeSet::new();
        let mut done_prints = BTreeSet::new();
        let mut done_eqs = BTreeSet::new();
        loop {
            if let Some(&i) = self.rec_eqs.iter().find(|i| !done_eqs.contains(*i)) {
                done_eqs.insert(i);
                self.emit_rec_eq(i)?;
                continue;
            }
            if let Some(&k) = self.rec_drops.iter().find(|k| !done_drops.contains(*k)) {
                done_drops.insert(k);
                self.emit_rec_drop(k.0, k.1)?;
                continue;
            }
            if let Some(&i) = self.rec_prints.iter().find(|i| !done_prints.contains(*i)) {
                done_prints.insert(i);
                self.emit_rec_print(i)?;
                continue;
            }
            return Ok(());
        }
    }

    /// `void @isc_rec_drop_<i>(ptr %p)`: one owner less; the last one frees
    /// the parts of the node, then the node. The last part that is a node is
    /// freed by a tail call, so that freeing a long list does not use the
    /// stack.
    fn emit_rec_drop(&mut self, i: u32, atomic: bool) -> R<()> {
        let saved_f = std::mem::take(&mut self.f);
        let saved_env = std::mem::take(&mut self.env);
        let saved_divmod = std::mem::take(&mut self.divmod);
        let saved_vec_fail = self.vec_fail.take();
        let saved_task = std::mem::replace(&mut self.in_task, atomic);
        // inside an atomic drop, every count is atomic (D175)
        let saved_shared = self.task_shared.take();
        let nt = self.rec_node_ty(i);
        let layout = self.p.recs[i as usize].layout.clone();
        let lay = lt(&layout);
        let lnull = self.label();
        let lgo = self.label();
        // null, or a constructor without payload (D169): nothing to free
        let node = self.is_node("%p");
        self.cbr(&node, &lgo, &lnull);
        self.block(&lnull);
        self.emit("ret void");
        self.block(&lgo);
        let last = if atomic {
            let old = self.ins("atomicrmw sub ptr %p, i64 1 acq_rel".to_string());
            self.ins(format!("icmp eq i64 {}, 1", old))
        } else {
            let c = self.ins("load i64, ptr %p".to_string());
            let c1 = self.ins(format!("sub i64 {}, 1", c));
            self.emit(format!("store i64 {}, ptr %p", c1));
            self.ins(format!("icmp eq i64 {}, 0", c1))
        };
        let lfree = self.label();
        let lkeep = self.label();
        self.cbr(&last, &lfree, &lkeep);
        self.block(&lkeep);
        self.emit("ret void");
        self.block(&lfree);
        let pt = self.ins(format!("getelementptr inbounds {}, ptr %p, i32 0, i32 1", nt));
        let t = self.ins(format!("load {}, ptr {}", lay, pt));
        let comps = match &layout {
            Type::Tuple(ts) => ts.clone(),
            _ => Vec::new(),
        };
        let tail = comps.iter().rposition(|c| matches!(c, Type::Rec(_)));
        for (k, c) in comps.iter().enumerate() {
            if c.has_heap() && Some(k) != tail {
                let f = self.ins(format!("extractvalue {} {}, {}", lay, t, k));
                self.drop_value(&v(f, lt(c)), c);
            }
        }
        let rest = tail.map(|k| (self.ins(format!("extractvalue {} {}, {}", lay, t, k)), comps[k].clone()));
        let end = self.ins(format!("getelementptr {}, ptr null, i32 1", nt));
        let bytes = self.ins(format!("ptrtoint ptr {} to i64", end));
        self.node_free("%p", &bytes);
        if let Some((f, Type::Rec(j))) = rest {
            self.rec_drops.insert((j, atomic));
            self.emit(format!("tail call void @{}(ptr {})", rec_drop_name(j, atomic), f));
        }
        self.emit("ret void");
        let text = self.finish(&format!("define internal void @{}(ptr %p)", rec_drop_name(i, atomic)));
        self.f = saved_f;
        self.env = saved_env;
        self.divmod = saved_divmod;
        self.vec_fail = saved_vec_fail;
        self.in_task = saved_task;
        self.task_shared = saved_shared;
        self.funcs.push(text);
        Ok(())
    }

    /// `i1 @isc_rec_eq_<i>(ptr %p, ptr %q)`: the structural equality of two
    /// nodes (D129): component by component, the tag first, IEEE equality
    /// of the floats (no shortcut for the same node: NaN differs from
    /// itself); the last component that is a node by a tail call, so that
    /// comparing long lists does not use the stack.
    fn emit_rec_eq(&mut self, i: u32) -> R<()> {
        let saved_f = std::mem::take(&mut self.f);
        let saved_env = std::mem::take(&mut self.env);
        let saved_divmod = std::mem::take(&mut self.divmod);
        let saved_vec_fail = self.vec_fail.take();
        let saved_task = std::mem::replace(&mut self.in_task, false);
        let nt = self.rec_node_ty(i);
        let layout = self.p.recs[i as usize].layout.clone();
        let lay = lt(&layout);
        let pn = self.ins("icmp eq ptr %p, null".to_string());
        let qn = self.ins("icmp eq ptr %q, null".to_string());
        let either = self.ins(format!("or i1 {}, {}", pn, qn));
        let lnull = self.label();
        let lgo = self.label();
        self.cbr(&either, &lnull, &lgo);
        self.block(&lnull);
        let both = self.ins(format!("and i1 {}, {}", pn, qn));
        self.emit(format!("ret i1 {}", both));
        self.block(&lgo);
        let _ = &nt;
        let tp = self.node_tuple("%p", i);
        let tq = self.node_tuple("%q", i);
        let comps = match &layout {
            Type::Tuple(ts) => ts.clone(),
            other => vec![other.clone()],
        };
        let tail = comps.iter().rposition(|c| matches!(c, Type::Rec(_)));
        let res = (|| -> R<()> {
            for (k, c) in comps.iter().enumerate() {
                if Some(k) == tail {
                    continue;
                }
                let (a, b) = if matches!(layout, Type::Tuple(_)) {
                    (self.ins(format!("extractvalue {} {}, {}", lay, tp, k)), self.ins(format!("extractvalue {} {}, {}", lay, tq, k)))
                } else {
                    (tp.clone(), tq.clone())
                };
                let eq = self.eq_value(&v(a, lt(c)), &v(b, lt(c)), c)?;
                let lnext = self.label();
                let lno = self.label();
                self.cbr(&eq, &lnext, &lno);
                self.block(&lno);
                self.emit("ret i1 false");
                self.block(&lnext);
            }
            if let Some(k) = tail {
                let a = self.ins(format!("extractvalue {} {}, {}", lay, tp, k));
                let b = self.ins(format!("extractvalue {} {}, {}", lay, tq, k));
                let Type::Rec(j) = comps[k] else { unreachable!() };
                self.rec_eqs.insert(j);
                let r = self.ins(format!("tail call i1 @isc_rec_eq_{}(ptr {}, ptr {})", j, a, b));
                self.emit(format!("ret i1 {}", r));
            } else {
                self.emit("ret i1 true");
            }
            Ok(())
        })();
        let text = self.finish(&format!("define internal i1 @isc_rec_eq_{}(ptr %p, ptr %q)", i));
        self.f = saved_f;
        self.env = saved_env;
        self.divmod = saved_divmod;
        self.vec_fail = saved_vec_fail;
        self.in_task = saved_task;
        res?;
        self.funcs.push(text);
        Ok(())
    }

    /// The equality of two values of type `t` (an `i1`): scalars (IEEE for
    /// floats), tuples, arrays element by element, nodes by their function.
    fn eq_value(&mut self, a: &V, b: &V, t: &Type) -> R<String> {
        Ok(match t {
            Type::Int(_) | Type::Bool => self.ins(format!("icmp eq {} {}, {}", lt(t), a.r, b.r)),
            Type::F64 | Type::F32 => self.ins(format!("fcmp oeq {} {}, {}", lt(t), a.r, b.r)),
            Type::Unit => "true".to_string(),
            Type::Str | Type::Big => {
                let (p, n) = self.str_parts(a);
                let (q, m) = self.str_parts(b);
                let c = self.ins(format!("call i32 @isc_str_cmp(ptr {}, i64 {}, ptr {}, i64 {})", p, n, q, m));
                self.ins(format!("icmp eq i32 {}, 0", c))
            }
            Type::Rec(j) => {
                self.rec_eqs.insert(*j);
                self.ins(format!("call i1 @isc_rec_eq_{}(ptr {}, ptr {})", j, a.r, b.r))
            }
            Type::Tuple(ts) => {
                let mut acc = "true".to_string();
                for (k, c) in ts.iter().enumerate() {
                    let x = self.ins(format!("extractvalue {} {}, {}", a.t, a.r, k));
                    let y = self.ins(format!("extractvalue {} {}, {}", b.t, b.r, k));
                    let e = self.eq_value(&v(x, lt(c)), &v(y, lt(c)), c)?;
                    acc = self.ins(format!("and i1 {}, {}", acc, e));
                }
                acc
            }
            Type::Array(et, _) if !et.is_array() => {
                let (pa, na) = self.str_parts(a);
                let (pb, nb) = self.str_parts(b);
                let same = self.ins(format!("icmp eq i64 {}, {}", na, nb));
                let flag = self.alloca("i1");
                self.emit(format!("store i1 {}, ptr {}", same, flag));
                let n = self.ins(format!("select i1 {}, i64 {}, i64 0", same, na));
                let et = (**et).clone();
                self.for_range_scalar("0", &n, 1, &mut |g, i| {
                    let x = g.load_elem(&pa, &na, i, &et, 1);
                    let y = g.load_elem(&pb, &nb, i, &et, 1);
                    let e = g.eq_value(&x, &y, &et)?;
                    let f = g.ins(format!("load i1, ptr {}", flag));
                    let f2 = g.ins(format!("and i1 {}, {}", f, e));
                    g.emit(format!("store i1 {}, ptr {}", f2, flag));
                    Ok(())
                })?;
                self.ins(format!("load i1, ptr {}", flag))
            }
            other => return Err(format!("codegen: equality of {:?} in a recursive type", other)),
        })
    }

    /// `void @isc_rec_print_<i>(ptr %p)`: prints the tuple of the node.
    fn emit_rec_print(&mut self, i: u32) -> R<()> {
        let saved_f = std::mem::take(&mut self.f);
        let saved_env = std::mem::take(&mut self.env);
        let saved_divmod = std::mem::take(&mut self.divmod);
        let saved_vec_fail = self.vec_fail.take();
        let saved_task = std::mem::replace(&mut self.in_task, false);
        let nt = self.rec_node_ty(i);
        let r = self.p.recs[i as usize].clone();
        let lay = lt(&r.layout);
        let _ = &nt;
        let t = self.node_tuple("%p", i);
        let res = self.print_value(&v(t, lay), &r.layout, &r.shape);
        self.emit("ret void");
        let text = self.finish(&format!("define internal void @isc_rec_print_{}(ptr %p)", i));
        self.f = saved_f;
        self.env = saved_env;
        self.divmod = saved_divmod;
        self.vec_fail = saved_vec_fail;
        self.in_task = saved_task;
        res?;
        self.funcs.push(text);
        Ok(())
    }

    fn black_box(&mut self, x: &V) {
        let slot = self.alloca(&x.t);
        if self.opts.isa.is_wasm() {
            // no inline assembly in WebAssembly: a volatile store is kept too
            self.emit(format!("store volatile {} {}, ptr {}", x.t, x.r, slot));
            return;
        }
        self.emit(format!("store {} {}, ptr {}", x.t, x.r, slot));
        self.emit(format!("call void asm sideeffect \"\", \"r,~{{memory}}\"(ptr {})", slot));
    }

    // ----- expressions -------------------------------------------------------

    fn var_ty(&self, v: VarId) -> &Type {
        &self.p.vars[v as usize].ty
    }

    fn bind(&mut self, var: VarId, x: V) {
        self.env.insert(var, x);
    }

    /// Compiles an expression; `w > 1` only for element computations
    /// accepted by `vec_ok`. A value whose type fixes the length of its
    /// arrays (`f64[5]`, D204) gets that length as a constant in its
    /// aggregate: LLVM folds every read of it (bounds checks against a
    /// constant, loops of a known count). Only results, never bindings: a
    /// parameter whose size is tested on entry is not fixed before its test.
    fn ex(&mut self, e: &Expr, w: u32) -> R<V> {
        let x = self.ex_any(e, w)?;
        if w == 1 && !self.no_fix && has_fixed(&e.ty) {
            return Ok(self.fix_len(x, &e.ty));
        }
        Ok(x)
    }

    /// `x` with the lengths its type `t` fixes (D204), when it has the
    /// layout of `t` (not the lanes of vector code).
    fn fix_len(&mut self, x: V, t: &Type) -> V {
        let lay = lt(t);
        if x.t != lay {
            return x;
        }
        let mut ls = Vec::new();
        fixed_lengths(t, &mut Vec::new(), &mut ls);
        let mut r = x.r.clone();
        let mut top = Vec::new();
        for (path, n) in ls {
            let idx = path.iter().map(|k| k.to_string()).collect::<Vec<_>>().join(", ");
            r = self.ins(format!("insertvalue {} {}, i64 {}, {}", lay, r, n, idx));
            if path.len() == 1 {
                top.push((path[0], n));
            }
        }
        for (k, n) in top {
            self.f.known_len.insert((r.clone(), k), n);
        }
        V { r, ..x }
    }

    fn ex_any(&mut self, e: &Expr, w: u32) -> R<V> {
        use ExprKind::*;
        let span = e.span;
        if !self.pre.is_empty() {
            if let Some(x) = self.pre.get(&(e as *const Expr)) {
                // (a vector given for vector code: the scalar replay of a
                // failed check computes the element itself, D214)
                if !(w == 1 && x.t.starts_with('<')) {
                    return Ok(x.clone());
                }
            }
        }
        match &e.kind {
            // a new string of the bytes of a constant (D134)
            StrLit(s) => {
                // a `bigint`: its bytes in the form of the compiled code
                // (base 256, D168), from the form of the IR (base 128)
                let (g, n) = if e.ty == Type::Big { self.bytes_const(&isc_runtime_core::big_to_256(s.as_bytes())) } else { self.str_const(s) };
                let out = self.alloca("{ ptr, i64 }");
                self.emit(format!("call void @isc_str_new(ptr {}, ptr {}, i64 {})", out, g, n));
                Ok(v(self.ins(format!("load {{ ptr, i64 }}, ptr {}", out)), "{ ptr, i64 }"))
            }
            StrOp(op, es) => self.str_op(e, *op, es),
            Lit(l) => Ok(match l {
                isc_ir::Lit::Int(i) => v(i.to_string(), lt(&e.ty)),
                isc_ir::Lit::F64(f) => v(fconst(*f), "double"),
                isc_ir::Lit::F32(f) => v(fconst32(*f), "float"),
                isc_ir::Lit::Bool(b) => v(b.to_string(), "i1"),
                isc_ir::Lit::Unit => v("false", "i1"),
            }),
            Var(x, _) => self.env.get(x).cloned().ok_or_else(|| format!("codegen: unbound variable %{}", x)),
            // a tree of new nodes (a builder's) that its body only lends to
            // functions whose results hold no tree: made in a region, freed
            // at once (D200)
            Let { var, init, body, drop: true, stmt }
                if w == 1
                    && matches!(init.kind, Call(f, _) if self.builders.contains(&f))
                    && region_use_ok(*var, body)
                    && !self.opts.isa.is_wasm() =>
            {
                self.decls.insert("declare ptr @isc_region_new()".into());
                self.decls.insert("declare void @isc_region_free(ptr)".into());
                let rg = self.ins("call ptr @isc_region_new()".to_string());
                let saved = std::mem::replace(&mut self.region, Some(rg.clone()));
                let iv = self.ex(init, 1);
                self.region = saved;
                let iv = iv?;
                self.bind(*var, iv.clone());
                if *stmt {
                    self.black_box(&iv);
                }
                // never read: freed at once, as in D117
                if !mentions(body, &HashSet::from([*var])) {
                    self.emit(format!("call void @isc_region_free(ptr {})", rg));
                    return self.ex(body, w);
                }
                self.region_vars.insert(*var, rg.clone());
                let r = self.ex(body, w);
                self.region_vars.remove(var);
                let r = r?;
                self.emit(format!("call void @isc_region_free(ptr {})", rg));
                Ok(r)
            }
            // an array of bytes made only to be written (`printBytes(map(...))`):
            // made and written by blocks that stay in the caches, never whole
            // (D225)
            Let { var, init, body, .. } if w == 1 && print_stream_ok(*var, init, body) => {
                self.print_stream(init)?;
                Ok(v("false", "i1"))
            }
            // a literal array of scalars that its body only reads: on the
            // stack, never freed (D191)
            Let { var, init, body, .. } if literal_tail(init).is_some() && init.ty.elem().is_scalar() && stack_array(*var, body) => {
                let n = literal_tail(init).unwrap().len();
                let et = init.ty.elem().clone();
                let slot = self.alloca(&format!("[{} x {}]", n.max(1), mt(&et)));
                self.stack_literal(init, &slot, &et, n)?;
                let arr = self.arr(&slot, &n.to_string());
                self.bind(*var, arr);
                self.stack_vars.insert(*var);
                self.ex(body, w)
            }
            Let { var, init, body, stmt, drop } => {
                let iv = self.ex(init, w)?;
                self.bind(*var, iv.clone());
                if *stmt && w == 1 {
                    self.black_box(&iv);
                }
                // a value the rest never reads is freed at once, not at the
                // end of its scope: its memory is given again to the next
                // allocation (`let (_, _) = bench(1, f)` before the timed
                // call, DECISIONS D117)
                let early = *drop && w == 1 && !mentions(body, &HashSet::from([*var])) && !frees(body, *var);
                if early {
                    let t = self.var_ty(*var).clone();
                    self.drop_value(&iv, &t);
                }
                let r = self.ex(body, w)?;
                if *drop && !early {
                    let t = self.var_ty(*var).clone();
                    self.drop_value(&iv, &t);
                }
                Ok(r)
            }
            Tuple(es) => {
                let t = lt(&e.ty);
                let mut acc = "poison".to_string();
                for (i, x) in es.iter().enumerate() {
                    let xv = self.ex(x, 1)?;
                    acc = self.ins(format!("insertvalue {} {}, {} {}, {}", t, acc, xv.t, xv.r, i));
                }
                if es.is_empty() {
                    acc = "zeroinitializer".into();
                }
                Ok(v(acc, t))
            }
            Proj(x, i) | Take(x, i) => {
                let xv = self.ex(x, 1)?;
                let r = self.ins(format!("extractvalue {} {}, {}", xv.t, xv.r, i));
                Ok(v(r, lt(&e.ty)))
            }
            Unary(op, x) => {
                let xv = self.ex(x, w)?;
                let ww = if uni(&xv) { 1 } else { w };
                let t = vt(&x.ty, ww);
                let r = match (op, &x.ty) {
                    (UnOp::Not, _) => {
                        let c = self.cst("i1", "true", ww);
                        self.ins(format!("xor {} {}, {}", t, xv.r, c))
                    }
                    (UnOp::Neg, Type::F64 | Type::F32) => self.ins(format!("fneg {} {}", t, xv.r)),
                    (UnOp::Neg, Type::Int(_)) => {
                        let z = if ww == 1 { "0".to_string() } else { "zeroinitializer".into() };
                        self.ins(format!("sub {} {}, {}", t, z, xv.r))
                    }
                    _ => return Err("codegen: bad unary operand".into()),
                };
                Ok(V { r, t, uni: uni(&xv) })
            }
            // two values of a recursive type: their equality function (D129)
            Binary(BinOp::Eq, a, b) if matches!(a.ty, Type::Rec(_)) => {
                let av = self.ex(a, 1)?;
                let bv = self.ex(b, 1)?;
                let Type::Rec(j) = a.ty else { unreachable!() };
                self.rec_eqs.insert(j);
                let r = self.ins(format!("call i1 @isc_rec_eq_{}(ptr {}, ptr {})", j, av.r, bv.r));
                Ok(v(r, "i1"))
            }
            Binary(op, a, b) if self.divmod_slot(*op, a, b, w).is_some() => {
                let slot = self.divmod_slot(*op, a, b, w).unwrap();
                let t = if w == 1 { "i64".to_string() } else { format!("<{} x i64>", w) };
                let r = self.ins(format!("load {}, ptr {}", t, slot));
                Ok(V { r, t, uni: false })
            }
            // a 64-bit division by a constant in vector code (D192)
            Binary(op @ (BinOp::Div | BinOp::Rem), a, b)
                // (a power of two: shifts, which LLVM already makes)
                if w > 1
                    && w % 4 == 0
                    && a.ty == Type::Int(IntKind::I64)
                    && matches!(b.kind, Lit(isc_ir::Lit::Int(c)) if c >= 3 && (c & (c - 1)) != 0)
                    && IFMA.with(|f| f.get()) =>
            {
                let Lit(isc_ir::Lit::Int(c)) = b.kind else { unreachable!() };
                let av = self.ex(a, w)?;
                if uni(&av) {
                    let bv = self.ex(b, 1)?;
                    return self.binop(*op, av, bv, &a.ty, w, span);
                }
                self.div_ifma(*op, &av, c, w, span)
            }
            Binary(op, a, b) => {
                let av = self.ex(a, w)?;
                let bv = self.ex(b, w)?;
                // consecutive indices plus or minus a value of every lane are
                // consecutive indices (D184, D185)
                // (a value of every lane minus them: consecutive downwards)
                let base = if matches!(op, BinOp::Add | BinOp::Sub) && a.ty == Type::Int(IntKind::I64) && w > 1 {
                    match (self.f.contig.get(&av.r).cloned(), self.f.contig.get(&bv.r).cloned()) {
                        (Some((x, d)), None) if uni(&bv) => Some((x, d, bv.r.clone(), false)),
                        (None, Some((y, d))) if uni(&av) => Some((y, d, av.r.clone(), *op == BinOp::Sub)),
                        _ => None,
                    }
                } else {
                    None
                };
                // consecutive indices against a value of every lane: at most
                // one lane equal, found from the first one (D217): a mask of
                // one bit instead of comparing the lanes
                if matches!(op, BinOp::Eq | BinOp::Ne) && a.ty == Type::Int(IntKind::I64) && w > 1 {
                    let one_side = match (self.f.contig.get(&av.r).cloned(), self.f.contig.get(&bv.r).cloned()) {
                        (Some((x, d)), None) if uni(&bv) => Some((x, d, bv.r.clone())),
                        (None, Some((y, d))) if uni(&av) => Some((y, d, av.r.clone())),
                        _ => None,
                    };
                    if let Some((first, d, u)) = one_side {
                        let diff = self.ins(format!("sub i64 {}, {}", u, first));
                        let k = if d == 1 { diff } else { self.ins(format!("sub i64 0, {}", diff)) };
                        let inside = self.ins(format!("icmp ult i64 {}, {}", k, w));
                        let kk = if w < 64 { self.ins(format!("trunc i64 {} to i{}", k, w)) } else { k };
                        let bit = self.ins(format!("shl i{} 1, {}", w, kk));
                        let mut bits = self.ins(format!("select i1 {}, i{} {}, i{} 0", inside, w, bit, w));
                        if *op == BinOp::Ne {
                            bits = self.ins(format!("xor i{} {}, -1", w, bits));
                        }
                        let m = self.ins(format!("bitcast i{} {} to <{} x i1>", w, bits, w));
                        return Ok(V { r: m, t: format!("<{} x i1>", w), uni: false });
                    }
                }
                // inside `allowReassoc` (D109)
                let saved = std::mem::replace(&mut self.fast_math, e.reassoc);
                let r = self.binop(*op, av, bv, &a.ty, w, span);
                self.fast_math = saved;
                if let (Some((x, d, u, flip)), Ok(rv)) = (base, &r) {
                    let (first, d) = if flip {
                        (self.ins(format!("sub i64 {}, {}", u, x)), -d)
                    } else if *op == BinOp::Add {
                        (self.ins(format!("add i64 {}, {}", x, u)), d)
                    } else {
                        (self.ins(format!("sub i64 {}, {}", x, u)), d)
                    };
                    self.f.contig.insert(rv.r.clone(), (first, d));
                }
                r
            }
            Pow(b, k) => {
                // inside `allowReassoc`, its multiplications too (D109,
                // D123: `(x - m) ^ 2` contracted with the sum into an FMA)
                let saved = std::mem::replace(&mut self.fast_math, e.reassoc);
                let r = self.pow(b, k, w, span);
                self.fast_math = saved;
                r
            }
            Convert(x) => {
                let xv = self.ex(x, w)?;
                self.convert(xv, &x.ty, &e.ty, w)
            }
            // one rounding: the instruction (D108)
            Math(MathFn::Fma, xs) => {
                let vs: Vec<V> = xs.iter().map(|x| self.ex(x, w)).collect::<R<_>>()?;
                let all_uni = vs.iter().all(uni);
                let ww = if all_uni { 1 } else { w };
                let vs: Vec<V> = vs.iter().map(|x| self.widen(x, ww)).collect();
                let (s, f) = if e.ty == Type::F32 { ("float", "f32") } else { ("double", "f64") };
                let (t, name) = if ww == 1 {
                    (s.to_string(), format!("llvm.fma.{}", f))
                } else {
                    (format!("<{} x {}>", ww, s), format!("llvm.fma.v{}{}", ww, f))
                };
                self.decls.insert(format!("declare {} @{}({}, {}, {})", t, name, t, t, t));
                let r = self.ins(format!("call {} @{}({} {}, {} {}, {} {})", t, name, t, vs[0].r, t, vs[1].r, t, vs[2].r));
                Ok(V { r, t, uni: all_uni })
            }
            // a function of the runtime, the same everywhere (D108)
            Math(f, xs) => {
                let vs: Vec<V> = xs.iter().map(|x| self.ex(x, 1)).collect::<R<_>>()?;
                let (s, suffix) = if e.ty == Type::F32 { ("float", "f") } else { ("double", "") };
                let name = format!("isc_{}{}", f.name(), suffix);
                let ps = vec![s; vs.len()].join(", ");
                self.decls.insert(format!("declare {} @{}({})", s, name, ps));
                let args: Vec<String> = vs.iter().map(|x| format!("{} {}", s, x.r)).collect();
                let r = self.ins(format!("call {} @{}({})", s, name, args.join(", ")));
                Ok(V { r, t: s.to_string(), uni: vs.iter().all(uni) })
            }
            Sqrt(x) => {
                let xv = self.ex(x, w)?;
                let ww = if uni(&xv) { 1 } else { w };
                let (s, f) = if x.ty == Type::F32 { ("float", "f32") } else { ("double", "f64") };
                let (t, name) = if ww == 1 {
                    (s.to_string(), format!("llvm.sqrt.{}", f))
                } else {
                    (format!("<{} x {}>", ww, s), format!("llvm.sqrt.v{}{}", ww, f))
                };
                self.decls.insert(format!("declare {} @{}({})", t, name, t));
                let r = self.ins(format!("call {} @{}({} {})", t, name, t, xv.r));
                Ok(V { r, t, uni: uni(&xv) })
            }
            If(c, t, f) => {
                if w > 1 {
                    let cv = self.ex(c, w)?;
                    // the lanes of each branch, for the SPMD calls in them
                    let saved = self.mask.clone();
                    let (mt, mf) = self.branch_masks(&cv, w);
                    self.mask = Some(mt);
                    let tv = self.ex(t, w);
                    self.mask = Some(mf);
                    let fv = self.ex(f, w);
                    self.mask = saved;
                    let (tv, fv) = (tv?, fv?);
                    let all_uni = uni(&cv) && uni(&tv) && uni(&fv);
                    if all_uni {
                        let r = self.ins(format!("select i1 {}, {} {}, {} {}", cv.r, tv.t, tv.r, fv.t, fv.r));
                        return Ok(V { r, t: tv.t, uni: true });
                    }
                    let tv = self.widen(&tv, w);
                    let fv = self.widen(&fv, w);
                    let cv = self.widen(&cv, w);
                    let r = self.ins(format!("select {} {}, {} {}, {} {}", cv.t, cv.r, tv.t, tv.r, fv.t, fv.r));
                    return Ok(V { r, t: tv.t, uni: false });
                }
                let cv = self.ex(c, 1)?;
                let lt_ = self.label();
                let lf = self.label();
                let lj = self.label();
                self.cbr(&cv.r, &lt_, &lf);
                self.block(&lt_);
                let tv = self.ex(t, 1)?;
                let tend = self.current_label(&lt_);
                self.br(&lj);
                self.block(&lf);
                let fv = self.ex(f, 1)?;
                let fend = self.current_label(&lf);
                self.br(&lj);
                self.block(&lj);
                let ty = lt(&e.ty);
                let r = self.ins(format!("phi {} [ {}, %{} ], [ {}, %{} ]", ty, tv.r, tend, fv.r, fend));
                Ok(v(r, ty))
            }
            Len(x) => {
                let (_, n) = self.prepare_src(x)?;
                Ok(v(n, "i64"))
            }
            ArrayLit(es) if e.ty.elem().is_array() => {
                // rows copied into one block (they have the same length: a
                // run-time test when it is not known at compilation)
                let inner = e.ty.elem().elem().clone();
                let rows = es.len().to_string();
                let mut vals = Vec::new();
                for x in es {
                    vals.push(self.ex(x, 1)?);
                }
                let cols = if vals.is_empty() {
                    "0".to_string()
                } else {
                    self.ins(format!("extractvalue {{ ptr, i64 }} {}, 1", vals[0].r))
                };
                let total = self.ins(format!("mul i64 {}, {}", rows, cols));
                let p = self.alloc_elems(&total, &inner);
                let es_sz = esize(&inner);
                for (i, xv) in vals.iter().enumerate() {
                    let q = self.ins(format!("extractvalue {{ ptr, i64 }} {}, 0", xv.r));
                    let m = self.ins(format!("extractvalue {{ ptr, i64 }} {}, 1", xv.r));
                    let same = self.ins(format!("icmp eq i64 {}, {}", m, cols));
                    self.guard(&same, format!("call void @isc_panic_size(i64 {}, i64 {}, {})", m, cols, Self::pos(span)));
                    let off = self.ins(format!("mul i64 {}, {}", i, cols));
                    let dst = self.ins(format!("getelementptr inbounds {}, ptr {}, i64 {}", lt(&inner), p, off));
                    let bytes = self.ins(format!("mul i64 {}, {}", cols, es_sz));
                    self.emit(format!("call void @llvm.memcpy.p0.p0.i64(ptr {}, ptr {}, i64 {}, i1 false)", dst, q, bytes));
                    // the elements of a literal are its own (the ownership
                    // pass copies an element still used elsewhere): the row,
                    // copied into the block, is freed
                    self.emit(format!("call void @isc_free(ptr {})", q));
                }
                Ok(self.nested(&p, &rows, &cols, &cols))
            }
            // data of the program, aligned as an array (D173)
            ConstArray(ls) => {
                let et = e.ty.elem().clone();
                let m = mt(&et);
                let vals: Vec<String> = ls
                    .iter()
                    .map(|l| match l {
                        isc_ir::Lit::Int(i) => format!("{} {}", m, i),
                        isc_ir::Lit::F64(f) => format!("double {}", fconst(*f)),
                        isc_ir::Lit::F32(f) => format!("float {}", fconst32(*f)),
                        isc_ir::Lit::Bool(b) => format!("i8 {}", *b as u8),
                        isc_ir::Lit::Unit => "i8 0".to_string(),
                    })
                    .collect();
                let name = format!("@.arr.{}", self.globals.len());
                self.globals.push(format!("{} = private unnamed_addr constant [{} x {}] [{}], align 64", name, ls.len(), m, vals.join(", ")));
                Ok(self.arr(&name, &ls.len().to_string()))
            }
            ArrayLit(es) => {
                let et = e.ty.elem().clone();
                let n = es.len().to_string();
                let p = self.alloc_elems(&n, &et);
                for (i, x) in es.iter().enumerate() {
                    let xv = self.ex(x, 1)?;
                    self.store_elem(&p, &n, &i.to_string(), &et, &xv, 1);
                }
                Ok(self.arr(&p, &n))
            }
            Index(a, i) if w > 1 => self.index_vec(a, i, w, span),
            Index(a, i) => {
                let (src, n) = self.prepare_src(a)?;
                let iv = self.ex(i, 1)?;
                let idx = self.index64(&iv, &i.ty);
                self.bounds(&idx, &n, &i.ty, span);
                self.elem(&src, &idx, 1)
            }
            Window(x, k) => {
                // a view of `x`: rows `n - k + 1`, columns `k`, stride 1
                let xv = self.ex(x, 1)?;
                let p = self.ins(format!("extractvalue {{ ptr, i64 }} {}, 0", xv.r));
                let n = self.ins(format!("extractvalue {{ ptr, i64 }} {}, 1", xv.r));
                let kv = self.ex(k, 1)?;
                let ok = self.ins(format!("icmp sle i64 {}, {}", kv.r, n));
                self.guard(&ok, format!("call void @isc_panic_window(i64 {}, i64 {}, {})", kv.r, n, Self::pos(span)));
                let d = self.ins(format!("sub i64 {}, {}", n, kv.r));
                let rows = self.ins(format!("add i64 {}, 1", d));
                Ok(self.nested(&p, &rows, &kv.r, "1"))
            }
            // a slice of a producer: only its elements computed, into new
            // memory (D101)
            // rows a .. b of an array of arrays of scalars: a new block
            // (not a view: `slice_is_view`), each row copied (D193)
            Slice(x, a, b) if x.ty.elem().is_array() => {
                let et = x.ty.elem().elem().clone();
                if !et.is_scalar() {
                    return Err(format!("{}:{}: a slice of an array of arrays of `{:?}` is not supported", span.line, span.col, et));
                }
                let xv = self.ex(x, 1)?;
                let (p, rows, cols, stride) = self.nested_parts(&xv);
                let av = self.ex(a, 1)?;
                let bv = self.ex(b, 1)?;
                self.slice_check(&av.r, &bv.r, &rows, span);
                let m = self.ins(format!("sub i64 {}, {}", bv.r, av.r));
                let total = self.ins(format!("mul i64 {}, {}", m, cols));
                let dst = self.alloc_elems(&total, &et);
                let sz = esize(&et);
                let row_bytes = self.ins(format!("mul i64 {}, {}", cols, sz));
                self.for_range("0", &m, 1, &mut |g, r| {
                    let from_row = g.ins(format!("add i64 {}, {}", av.r, r));
                    let off = g.ins(format!("mul i64 {}, {}", from_row, stride));
                    let src = g.ins(format!("getelementptr inbounds {}, ptr {}, i64 {}", mt(&et), p, off));
                    let doff = g.ins(format!("mul i64 {}, {}", r, cols));
                    let d = g.ins(format!("getelementptr inbounds {}, ptr {}, i64 {}", mt(&et), dst, doff));
                    g.emit(format!("call void @llvm.memcpy.p0.p0.i64(ptr {}, ptr {}, i64 {}, i1 false)", d, src, row_bytes));
                    Ok(())
                })?;
                // a producer was made for the slice: freed after it
                if isc_ir::passes::is_producer(x) {
                    self.drop_value(&xv, &x.ty);
                }
                Ok(self.nested(&dst, &m, &cols, &cols))
            }
            Slice(x, ..) if isc_ir::passes::is_producer(x) => self.materialize(e),
            Slice(x, a, b) => {
                // a view of scalars, or a new array of the elements a .. b - 1,
                // column by column for an array of tuples
                let xv = self.ex(x, 1)?;
                let p = self.ins(format!("extractvalue {{ ptr, i64 }} {}, 0", xv.r));
                let n = self.ins(format!("extractvalue {{ ptr, i64 }} {}, 1", xv.r));
                let av = self.ex(a, 1)?;
                let bv = self.ex(b, 1)?;
                self.slice_check(&av.r, &bv.r, &n, span);
                let m = self.ins(format!("sub i64 {}, {}", bv.r, av.r));
                let elem_ty = e.ty.elem().clone();
                if e.ty.slice_is_view() {
                    // a view of the elements a .. b - 1 (rule F4, D97)
                    let from = self.ins(format!("getelementptr inbounds {}, ptr {}, i64 {}", lt(&elem_ty), p, av.r));
                    let r0 = self.ins(format!("insertvalue {{ ptr, i64 }} poison, ptr {}, 0", from));
                    let r = self.ins(format!("insertvalue {{ ptr, i64 }} {}, i64 {}, 1", r0, m));
                    return Ok(V { r, t: "{ ptr, i64 }".into(), uni: true });
                }
                let dst = self.alloc_elems(&m, &elem_ty);
                let leaf_tys: Vec<Type> = if matches!(elem_ty, Type::Tuple(_)) {
                    leaves(&elem_ty).into_iter().map(|(_, l)| l).collect()
                } else {
                    vec![elem_ty.clone()]
                };
                let (srcs, dsts): (Vec<String>, Vec<String>) = if matches!(elem_ty, Type::Tuple(_)) {
                    let s = self.columns(&p, &n, &elem_ty).into_iter().map(|(_, _, b)| b).collect();
                    let d = self.columns(&dst, &m, &elem_ty).into_iter().map(|(_, _, b)| b).collect();
                    (s, d)
                } else {
                    (vec![p.clone()], vec![dst.clone()])
                };
                for ((leaf, s), d) in leaf_tys.iter().zip(srcs).zip(dsts) {
                    let sz = esize(leaf);
                    let off = self.ins(format!("mul i64 {}, {}", av.r, sz));
                    let from = self.ins(format!("getelementptr inbounds i8, ptr {}, i64 {}", s, off));
                    let bytes = self.ins(format!("mul i64 {}, {}", m, sz));
                    self.emit(format!("call void @llvm.memcpy.p0.p0.i64(ptr {}, ptr {}, i64 {}, i1 false)", d, from, bytes));
                }
                let r0 = self.ins(format!("insertvalue {{ ptr, i64 }} poison, ptr {}, 0", dst));
                let r = self.ins(format!("insertvalue {{ ptr, i64 }} {}, i64 {}, 1", r0, m));
                Ok(V { r, t: "{ ptr, i64 }".into(), uni: true })
            }
            Update { target, index, value, .. } => {
                let iv = self.ex(index, 1)?;
                let vv = self.ex(value, 1)?;
                let av = self.ex(target, 1)?;
                let p = self.ins(format!("extractvalue {{ ptr, i64 }} {}, 0", av.r));
                let n = self.ins(format!("extractvalue {{ ptr, i64 }} {}, 1", av.r));
                let idx = self.index64(&iv, &index.ty);
                self.bounds(&idx, &n, &index.ty, span);
                let et = e.ty.elem().clone();
                // the element replaced loses its owner: a node (D130)
                if holds_owned(&et) {
                    let old = self.load_elem(&p, &n, &idx, &et, 1);
                    self.drop_value(&old, &et);
                }
                self.store_elem(&p, &n, &idx, &et, &vv, 1);
                Ok(av)
            }
            Map(..) if isc_ir::passes::is_stream(e) => self.materialize_stream(e),
            // the reductions of sliding windows, level by level (D115)
            Map(ins, l) if ins.len() == 1 && ins[0].ty.elem().is_array() && window_reduce_of(l).is_some() => {
                self.window_cascade(e)
            }
            // a `map` whose elements are arrays: an array of arrays (D126)
            Map(..) if e.ty.elem().is_array() => self.materialize_rows(e),
            Map(..) | Iota(_) | Fill(..) | Zeros(_) => self.materialize(e),
            // values that own memory (strings, nodes, D134)
            Reduce(x, l, z) if w > 1 => self.reduce_vec(x, l, z, w),
            Reduce(x, l, z) if holds_owned(&e.ty) => self.reduce_owned(x, l, z),
            Scan(x, l, z) if holds_owned(e.ty.elem()) => self.scan_owned(x, l, z),
            Reduce(x, l, z) if isc_ir::passes::is_stream(x) => self.reduce_stream(x, l, z),
            Reduce(x, l, z) if Self::columns_of(x, l, z).is_some() => {
                let cols = Self::columns_of(x, l, z).unwrap();
                self.reduce_columns(e, &cols)
            }
            Reduce(x, l, z) => self.reduce(e, x, l, z),
            Scan(x, l, z) => self.scan(e, x, l, z),
            Filter(x, l) => self.filter(e, x, l),
            Fold(x, init, l) if w > 1 => self.fold_vec(x, init, l, w),
            Fold(x, init, l) if isc_ir::passes::is_stream(x) => self.fold_stream(x, init, l),
            Fold(x, init, l) => match additive_update(l) {
                Some((gi, h, read)) if !self.in_task => self.fold_private(e, x, init, l, gi, h, read),
                _ => self.fold(x, init, l),
            },
            CheckSize(a, b, body) => {
                // the real sizes, not those the types expect (D204)
                let saved = std::mem::replace(&mut self.no_fix, true);
                let av = self.ex(a, 1);
                let bv = self.ex(b, 1);
                self.no_fix = saved;
                let (av, bv) = (av?, bv?);
                let ok = self.ins(format!("icmp eq i64 {}, {}", av.r, bv.r));
                self.uniform_check(&ok, format!("call void @isc_panic_size(i64 {}, i64 {}, {})", av.r, bv.r, Self::pos(span)), w);
                self.ex(body, w)
            }
            CheckNonNegative(k, body) => {
                let kv = self.ex(k, 1)?;
                let ok = self.ins(format!("icmp sge i64 {}, 0", kv.r));
                self.uniform_check(&ok, format!("call void @isc_panic_negsize(i64 {}, {})", kv.r, Self::pos(span)), w);
                self.ex(body, w)
            }
            Zero => Ok(v(Self::zero(&e.ty), lt(&e.ty))),
            Copy(x) => {
                let xv = self.ex(x, 1)?;
                Ok(self.copy_value(&xv, &x.ty))
            }
            Action(op, args) => self.action(*op, args, e),
            Bench(r, body) => self.bench(r, body, e),
            // the body, then a variable freed (D105)
            Free(var, body) => {
                let r = self.ex(body, w)?;
                let x = self.env[var].clone();
                let ty = self.p.vars[*var as usize].ty.clone();
                self.drop_value(&x, &ty);
                Ok(r)
            }
            // a new node: one owner, its tuple (D103)
            Box(x) => {
                let Type::Rec(i) = e.ty else { return Err("internal error: box of a non-recursive type".into()) };
                // a constructor without payload (its tag, zeros): no node,
                // the odd value 2 * tag + 1 (D169)
                if let ExprKind::Tuple(es) = &x.kind {
                    if let (Some(ExprKind::Lit(isc_ir::Lit::Int(tag))), true) =
                        (es.first().map(|t| &t.kind), es.iter().skip(1).all(|c| matches!(c.kind, ExprKind::Zero)))
                    {
                        let p = self.ins(format!("inttoptr i64 {} to ptr", 2 * *tag + 1));
                        return Ok(v(p, "ptr"));
                    }
                }
                let t = self.ex(x, 1)?;
                let nt = self.rec_node_ty(i);
                let end = self.ins(format!("getelementptr {}, ptr null, i32 1", nt));
                let bytes = self.ins(format!("ptrtoint ptr {} to i64", end));
                // a cell of the region of a builder (D200), else of the
                // allocator of nodes (D166, D176)
                let p = match self.region.clone() {
                    Some(rg) => self.region_alloc(&rg, &bytes),
                    None => self.node_alloc(&bytes),
                };
                self.emit(format!("store i64 1, ptr {}", p));
                let pt = self.ins(format!("getelementptr inbounds {}, ptr {}, i32 0, i32 1", nt, p));
                self.emit(format!("store {} {}, ptr {}", t.t, t.r, pt));
                Ok(v(p, "ptr"))
            }
            Unbox(x) => {
                let Type::Rec(i) = x.ty else { return Err("internal error: unbox of a non-recursive type".into()) };
                let p = self.ex(x, 1)?;
                let lay = lt(&e.ty);
                let t = self.node_tuple(&p.r, i);
                Ok(v(t, lay))
            }
            // a recursive function (D102)
            Call(f, args) if w > 1 => self.spmd_call(*f, args, w),
            Call(f, args) => {
                let key = e as *const Expr;
                if self.tail.as_ref().map(|t| t.calls.contains_key(&key)).unwrap_or(false) {
                    return self.tail_jump(e, args);
                }
                let mut vals = Vec::new();
                let mut xs = Vec::new();
                for a in args {
                    let x = self.ex(a, 1)?;
                    vals.push(format!("{} {}", x.t, x.r));
                    xs.push(x);
                }
                let ret = lt(&e.ty);
                // a tail call to another function: its frame replaces this
                // one (D113)
                if let Some(first) = self.other_tails.get(&key).cloned() {
                    for var in &first {
                        // (an array on the stack is never freed, D191; a tree
                        // of a region with its region, D200)
                        if self.stack_vars.contains(var) {
                            continue;
                        }
                        if let Some(rg) = self.region_vars.get(var).cloned() {
                            self.emit(format!("call void @isc_region_free(ptr {})", rg));
                            continue;
                        }
                        let ty = self.p.vars[*var as usize].ty.clone();
                        let x = self.env[var].clone();
                        self.drop_value(&x, &ty);
                    }
                    let r = self.ins(format!("tail call tailcc {} @isc_fn_{}({})", ret, f, vals.join(", ")));
                    self.emit(format!("ret {} {}", ret, r));
                    let dead = self.label();
                    self.block(&dead);
                    return Ok(V { r: "poison".into(), t: ret, uni: true });
                }
                // the base case of the callee here, the call only for the
                // other branch (D202)
                let inline_base = self.base_cases.get(f).copied();
                let mut base_join = None;
                if let Some((c, base, then_base)) = inline_base {
                    let params = self.p.fns[*f as usize].params.clone();
                    let saved: Vec<(VarId, Option<V>)> = params.iter().map(|q| (*q, self.env.get(q).cloned())).collect();
                    for (q, x) in params.iter().zip(xs.iter()) {
                        self.bind(*q, x.clone());
                    }
                    let r = (|| -> R<(String, String, String)> {
                        let cv = self.ex(c, 1)?;
                        let (lbase, lcall) = (self.label(), self.label());
                        if then_base {
                            self.cbr(&cv.r, &lbase, &lcall);
                        } else {
                            self.cbr(&cv.r, &lcall, &lbase);
                        }
                        self.block(&lbase);
                        let bv = self.ex(base, 1)?;
                        let bend = self.current_label(&lbase);
                        let lj = self.label();
                        self.br(&lj);
                        self.block(&lcall);
                        Ok((bv.r, bend, lj))
                    })();
                    for (q, old) in saved {
                        match old {
                            Some(x) => {
                                self.env.insert(q, x);
                            }
                            None => {
                                self.env.remove(&q);
                            }
                        }
                    }
                    base_join = Some(r?);
                }
                let cc = self.fn_cc();
                // a builder called while a region is made: its variant
                let r = if let (Some(rg), true) = (self.region.clone(), self.builders.contains(f)) {
                    self.rg_needed.insert(*f);
                    self.ins(format!("call {}{} @isc_fn_{}_rg(ptr {}, {})", cc, ret, f, rg, vals.join(", ")))
                } else {
                    self.ins(format!("call {}{} @isc_fn_{}({})", cc, ret, f, vals.join(", ")))
                };
                if let Some((bv, bend, lj)) = base_join {
                    let cend = self.current_label("");
                    self.br(&lj);
                    self.block(&lj);
                    let phi = self.ins(format!("phi {} [ {}, %{} ], [ {}, %{} ]", ret, bv, bend, r, cend));
                    return Ok(V { r: phi, t: ret, uni: true });
                }
                Ok(V { r, t: ret, uni: true })
            }
        }
    }

    /// The label of the block currently being filled.
    fn current_label(&self, default: &str) -> String {
        for line in self.f.body.iter().rev() {
            if !line.starts_with("  ") && line.ends_with(':') {
                return line.trim_end_matches(':').to_string();
            }
        }
        default.to_string()
    }

    fn index64(&mut self, iv: &V, t: &Type) -> String {
        match t {
            Type::Int(k) if k.bits() < 64 => {
                let op = if k.signed() { "sext" } else { "zext" };
                self.ins(format!("{} i{} {} to i64", op, k.bits(), iv.r))
            }
            _ => iv.r.clone(),
        }
    }

    /// `x / c` or `x % c` of the 64-bit lanes of `x` by a constant `c >= 2`
    /// (D192): x86 has no 64-bit vector product high, so that LLVM divides
    /// one lane at a time. When every lane is in `0 .. 2^52 / c`, the high
    /// 52 bits of `x * m`, `m = ceil(2^52 / c)` (`vpmadd52huq`, AVX-512
    /// IFMA), are `x / c`: `m` exceeds `2^52 / c` by less than 1, so that
    /// the product exceeds `x / c * 2^52` by less than `x`, less than
    /// `2^52 / c`; the remainder is `x - q * c`. Otherwise the division of
    /// LLVM.
    fn div_ifma(&mut self, op: BinOp, x: &V, c: i64, w: u32, span: Span) -> R<V> {
        let vt = format!("<{} x i64>", w);
        let bound = (1i64 << 52) / c;
        let lim = self.cst("i64", &bound.to_string(), w);
        let out = self.ins(format!("icmp uge {} {}, {}", vt, x.r, lim));
        let bits = self.ins(format!("bitcast <{} x i1> {} to i{}", w, out, w));
        let ok = self.ins(format!("icmp eq i{} {}, 0", w, bits));
        let lfast = self.label();
        let lslow = self.label();
        let ljoin = self.label();
        self.cbr(&ok, &lfast, &lslow);
        self.block(&lfast);
        let m = (((1i128 << 52) + c as i128 - 1) / c as i128) as i64;
        self.decls.insert("declare <4 x i64> @llvm.x86.avx512.vpmadd52h.uq.256(<4 x i64>, <4 x i64>, <4 x i64>)".into());
        let mv = format!("<i64 {m}, i64 {m}, i64 {m}, i64 {m}>", m = m);
        // by 4 lanes, then the quotients put together two by two
        let mut parts = Vec::new();
        for k in 0..w / 4 {
            let lanes = (0..4).map(|j| format!("i32 {}", 4 * k + j)).collect::<Vec<_>>().join(", ");
            let part = self.ins(format!("shufflevector {} {}, {} poison, <4 x i32> <{}>", vt, x.r, vt, lanes));
            parts.push(self.ins(format!("call <4 x i64> @llvm.x86.avx512.vpmadd52h.uq.256(<4 x i64> zeroinitializer, <4 x i64> {}, <4 x i64> {})", part, mv)));
        }
        let mut n = 4;
        while parts.len() > 1 {
            let lanes = (0..2 * n).map(|j| format!("i32 {}", j)).collect::<Vec<_>>().join(", ");
            let mut next = Vec::new();
            for pair in parts.chunks(2) {
                next.push(self.ins(format!("shufflevector <{n} x i64> {}, <{n} x i64> {}, <{} x i32> <{}>", pair[0], pair[1], 2 * n, lanes, n = n)));
            }
            parts = next;
            n *= 2;
        }
        let q = parts.pop().unwrap();
        let r_fast = if op == BinOp::Div {
            q.clone()
        } else {
            let cv = self.cst("i64", &c.to_string(), w);
            let qc = self.ins(format!("mul {} {}, {}", vt, q, cv));
            self.ins(format!("sub {} {}, {}", vt, x.r, qc))
        };
        let l1 = self.current_label(&lfast);
        self.br(&ljoin);
        self.block(&lslow);
        let cv = v(self.cst("i64", &c.to_string(), w), vt.clone());
        let r_slow = self.binop(op, x.clone(), cv, &Type::Int(IntKind::I64), w, span)?;
        let l2 = self.current_label(&lslow);
        self.br(&ljoin);
        self.block(&ljoin);
        let r = self.ins(format!("phi {} [ {}, %{} ], [ {}, %{} ]", vt, r_fast, l1, r_slow.r, l2));
        Ok(V { r, t: vt, uni: false })
    }

    /// A check of a value of every lane: in vector code under a mask, a
    /// failure only when a lane is active (a branch of an `if`); a jump to
    /// the replay of the vector when there is one (D185).
    fn uniform_check(&mut self, ok: &str, panic_call: String, w: u32) {
        let fine = match self.mask.clone() {
            Some(m) if w > 1 => {
                let name = format!("llvm.vector.reduce.or.v{}i1", w);
                self.decls.insert(format!("declare i1 @{}(<{} x i1>)", name, w));
                let active = self.ins(format!("call i1 @{}(<{} x i1> {})", name, w, m));
                let out = self.ins(format!("xor i1 {}, true", ok));
                let fails = self.ins(format!("and i1 {}, {}", out, active));
                self.ins(format!("xor i1 {}, true", fails))
            }
            _ => ok.to_string(),
        };
        if w > 1 {
            self.vec_check(&fine, panic_call);
        } else {
            self.guard(&fine, panic_call);
        }
    }

    /// A check in vector code: a jump to the replay of the vector when there
    /// is one (D185), the panic otherwise.
    fn vec_check(&mut self, ok: &str, panic_call: String) {
        match self.vec_fail.clone() {
            Some(lf) => {
                let lok = self.label();
                self.cbr(ok, &lok, &lf);
                self.block(&lok);
            }
            None => self.guard(ok, panic_call),
        }
    }

    /// The `let`s of scalars of `e`, then its array literal stored at `slot`
    /// (`literal_tail`, D191).
    fn stack_literal(&mut self, e: &Expr, slot: &str, et: &Type, n: usize) -> R<()> {
        match &e.kind {
            ExprKind::Let { var, init, body, .. } => {
                let iv = self.ex(init, 1)?;
                self.bind(*var, iv);
                self.stack_literal(body, slot, et, n)
            }
            ExprKind::ArrayLit(es) => {
                for (i, x) in es.iter().enumerate() {
                    let xv = self.ex(x, 1)?;
                    self.store_elem(slot, &n.to_string(), &i.to_string(), et, &xv, 1);
                }
                Ok(())
            }
            _ => Err("internal error: not an array literal".into()),
        }
    }

    /// `a[i]` in vector code (D181): `a` an array of the context, `i` a
    /// vector of indices. Every lane checked at once; the first lane out of
    /// bounds gives the panic of the scalar loop (the body has no other site
    /// that can panic, `index_vec_ok`). Consecutive indices (the index of
    /// the loop): one vector load; others: a gather.
    fn index_vec(&mut self, a: &Expr, i: &Expr, w: u32, span: Span) -> R<V> {
        let (src, n) = self.prepare_src(a)?;
        let iv = self.ex(i, w)?;
        let unsigned = if is_signed(&i.ty) { 0 } else { 1 };
        if uni(&iv) {
            let idx = self.index64(&iv, &i.ty);
            let ok = self.ins(format!("icmp ult i64 {}, {}", idx, n));
            let Some(m) = self.mask.clone() else {
                self.vec_check(&ok, format!("call void @isc_panic_bounds(i64 {}, i64 {}, i32 {}, {})", idx, n, unsigned, Self::pos(span)));
                return self.elem(&src, &idx, 1);
            };
            // under a mask (a branch of an `if`, D185): a failure only when a
            // lane is active, the load only when in bounds
            let name = format!("llvm.vector.reduce.or.v{}i1", w);
            self.decls.insert(format!("declare i1 @{}(<{} x i1>)", name, w));
            let active = self.ins(format!("call i1 @{}(<{} x i1> {})", name, w, m));
            let out = self.ins(format!("xor i1 {}, true", ok));
            let fails = self.ins(format!("and i1 {}, {}", out, active));
            let fine = self.ins(format!("xor i1 {}, true", fails));
            self.vec_check(&fine, format!("call void @isc_panic_bounds(i64 {}, i64 {}, i32 {}, {})", idx, n, unsigned, Self::pos(span)));
            let lload = self.label();
            let lskip = self.label();
            let ljoin = self.label();
            self.cbr(&ok, &lload, &lskip);
            self.block(&lload);
            let x = self.elem(&src, &idx, 1)?;
            let lend = self.current_label(&lload);
            self.br(&ljoin);
            self.block(&lskip);
            self.br(&ljoin);
            self.block(&ljoin);
            let et = a.ty.elem().clone();
            let r = self.ins(format!("phi {} [ {}, %{} ], [ {}, %{} ]", x.t, x.r, lend, Self::zero(&et), lskip));
            return Ok(V { r, t: x.t, uni: true });
        }
        let Src::Mat { ptr, elem, .. } = &src else {
            return Err("internal error: an index of vector code into a computed array".into());
        };
        let (ptr, elem) = (ptr.clone(), elem.clone());
        let idx = match &i.ty {
            Type::Int(k) if k.bits() < 64 => {
                let op = if k.signed() { "sext" } else { "zext" };
                self.ins(format!("{} <{} x i{}> {} to <{} x i64>", op, w, k.bits(), iv.r, w))
            }
            _ => iv.r.clone(),
        };
        let lok = self.label();
        // consecutive indices: all in bounds when the first and the last are
        // (D217); the test of every lane only when they are not (the first
        // lane out, for the message). Under a mask too (a branch of an `if`,
        // D226): every lane in bounds, the active ones are; the lanes left
        // out may be past the end, then the test of the active lanes.
        if let (Some((first, d)), true) = (self.f.contig.get(&iv.r).cloned(), idx == iv.r) {
            let last = self.ins(format!("add i64 {}, {}", first, d * (w as i64 - 1)));
            let a1 = self.ins(format!("icmp ult i64 {}, {}", first, n));
            let a2 = self.ins(format!("icmp ult i64 {}, {}", last, n));
            let fine = self.ins(format!("and i1 {}, {}", a1, a2));
            let lslow = self.label();
            self.cbr(&fine, &lok, &lslow);
            self.block(&lslow);
        }
        let nv = self.widen(&v(n.clone(), "i64"), w);
        let bad0 = self.ins(format!("icmp uge <{} x i64> {}, {}", w, idx, nv.r));
        let bad = match self.mask.clone() {
            Some(m) => self.ins(format!("and <{} x i1> {}, {}", w, bad0, m)),
            None => bad0,
        };
        let bits = self.ins(format!("bitcast <{} x i1> {} to i{}", w, bad, w));
        let ok = self.ins(format!("icmp eq i{} {}, 0", w, bits));
        if let Some(lf) = self.vec_fail.clone() {
            // the elements of the vector again, one at a time (D185)
            self.cbr(&ok, &lok, &lf);
        } else {
            let lfail = self.label();
            self.cbr(&ok, &lok, &lfail);
            self.block(&lfail);
            let first = self.ins(format!("call i{w} @llvm.cttz.i{w}(i{w} {bits}, i1 true)", w = w, bits = bits));
            self.decls.insert(format!("declare i{w} @llvm.cttz.i{w}(i{w}, i1)", w = w));
            let at = self.ins(format!("extractelement <{} x i64> {}, i{} {}", w, idx, w, first));
            self.emit(format!("call void @isc_panic_bounds(i64 {}, i64 {}, i32 {}, {})", at, n, unsigned, Self::pos(span)));
            self.emit("unreachable");
        }
        self.block(&lok);
        let m = mt(&elem);
        let vm = format!("<{} x {}>", w, m);
        let suffix = match m.as_str() {
            "double" => "f64".to_string(),
            "float" => "f32".to_string(),
            t => t.to_string(),
        };
        if let Some((base, dir)) = self.f.contig.get(&iv.r).cloned() {
            // downwards: the same load from the lowest index, its lanes
            // reversed (D185)
            let rev = format!("<{}>", (0..w).rev().map(|j| format!("i32 {}", j)).collect::<Vec<_>>().join(", "));
            let low = if dir == 1 { base } else { self.ins(format!("sub i64 {}, {}", base, w - 1)) };
            let x = match self.mask.clone() {
                None => self.load_scalar(&ptr, &low, &elem, w).r,
                Some(mask) => {
                    // the lanes of a mask only: the others may be past the end
                    let mask = if dir == 1 { mask } else { self.ins(format!("shufflevector <{} x i1> {}, <{} x i1> poison, <{} x i32> {}", w, mask, w, w, rev)) };
                    let g = self.ins(format!("getelementptr inbounds {}, ptr {}, i64 {}", m, ptr, low));
                    let name = format!("llvm.masked.load.v{}{}.p0", w, suffix);
                    self.decls.insert(format!("declare {} @{}(ptr, i32, <{} x i1>, {})", vm, name, w, vm));
                    self.ins(format!("call {} @{}(ptr {}, i32 {}, <{} x i1> {}, {} zeroinitializer)", vm, name, g, esize(&elem), w, mask, vm))
                }
            };
            let x = if dir == 1 { x } else { self.ins(format!("shufflevector {} {}, {} poison, <{} x i32> {}", vm, x, vm, w, rev)) };
            return Ok(V { r: x, t: vm, uni: false });
        }
        if esize(&elem) == 1 && matches!(elem, Type::Int(_)) && w <= 32 && BYTE_LOOKUP.with(|b| b.get()) {
            // a table of at most 256 bytes: permutations of its bytes in
            // registers (AVX-512 VBMI), a gather beyond (D185)
            let small = self.ins(format!("icmp ule i64 {}, 256", n));
            let llook = self.label();
            let lgather = self.label();
            let ljoin = self.label();
            self.cbr(&small, &llook, &lgather);
            self.block(&llook);
            let parts = match a.kind {
                ExprKind::Var(v, _) => self.f.table_parts.get(&v).cloned(),
                _ => None,
            };
            let r1 = self.byte_lookup(&ptr, &n, &idx, w, parts);
            let l1 = self.current_label(&llook);
            self.br(&ljoin);
            self.block(&lgather);
            // an index computed from the index of the loop alone: its lanes
            // close together, one window of 64 bytes permuted (D187)
            let r2 = if has_index(i) { self.gather(&ptr, &idx, &elem, w) } else { self.byte_window(&ptr, &n, &idx, &elem, w) };
            let l2 = self.current_label(&lgather);
            self.br(&ljoin);
            self.block(&ljoin);
            let r = self.ins(format!("phi {} [ {}, %{} ], [ {}, %{} ]", vm, r1, l1, r2, l2));
            return Ok(V { r, t: vm, uni: false });
        }
        let x = self.gather(&ptr, &idx, &elem, w);
        Ok(V { r: x, t: vm, uni: false })
    }

    /// The elements of `ptr` at the lanes of `idx` (`<w x i64>`), those of
    /// the lanes of the mask.
    fn gather(&mut self, ptr: &str, idx: &str, elem: &Type, w: u32) -> String {
        let m = mt(elem);
        let vm = format!("<{} x {}>", w, m);
        let suffix = match m.as_str() {
            "double" => "f64".to_string(),
            "float" => "f32".to_string(),
            t => t.to_string(),
        };
        let ptrs = self.ins(format!("getelementptr inbounds {}, ptr {}, <{} x i64> {}", m, ptr, w, idx));
        let name = format!("llvm.masked.gather.v{}{}.v{}p0", w, suffix, w);
        self.decls.insert(format!("declare {} @{}(<{} x ptr>, i32, <{} x i1>, {})", vm, name, w, w, vm));
        let lanes = self.mask.clone().unwrap_or_else(|| format!("<{}>", (0..w).map(|_| "i1 true").collect::<Vec<_>>().join(", ")));
        self.ins(format!("call {} @{}(<{} x ptr> {}, i32 {}, <{} x i1> {}, {} zeroinitializer)", vm, name, w, ptrs, esize(elem), w, lanes, vm))
    }

    /// The bytes of `ptr` at the lanes of `idx` (those of the mask, in
    /// bounds) when they lie in a window of 64 bytes (D187): two masked
    /// loads of 32 bytes from the lowest index (nothing past the end) and
    /// one `vpermi2b` of the indices relative to it; a gather otherwise.
    fn byte_window(&mut self, ptr: &str, n: &str, idx: &str, elem: &Type, w: u32) -> String {
        let vt = format!("<{} x i64>", w);
        let (lo_in, hi_in) = match self.mask.clone() {
            Some(m) => (
                self.ins(format!("select <{} x i1> {}, {} {}, {} {}", w, m, vt, idx, vt, self.cst("i64", "-1", w))),
                self.ins(format!("select <{} x i1> {}, {} {}, {} zeroinitializer", w, m, vt, idx, vt)),
            ),
            None => (idx.to_string(), idx.to_string()),
        };
        for f in ["umin", "umax"] {
            self.decls.insert(format!("declare i64 @llvm.vector.reduce.{}.v{}i64({})", f, w, vt));
        }
        let lo = self.ins(format!("call i64 @llvm.vector.reduce.umin.v{}i64({} {})", w, vt, lo_in));
        let hi = self.ins(format!("call i64 @llvm.vector.reduce.umax.v{}i64({} {})", w, vt, hi_in));
        let span = self.ins(format!("sub i64 {}, {}", hi, lo));
        let near = self.ins(format!("icmp ult i64 {}, 64", span));
        let lwin = self.label();
        let lgat = self.label();
        let ljoin = self.label();
        self.cbr(&near, &lwin, &lgat);
        self.block(&lwin);
        self.decls.insert("declare <32 x i8> @llvm.masked.load.v32i8.p0(ptr, i32, <32 x i1>, <32 x i8>)".into());
        self.decls.insert("declare <32 x i8> @llvm.x86.avx512.vpermi2var.qi.256(<32 x i8>, <32 x i8>, <32 x i8>)".into());
        let mut parts = Vec::new();
        for k in 0..2 {
            let from = self.ins(format!("add i64 {}, {}", lo, 32 * k));
            let left = self.ins(format!("sub i64 {}, {}", n, from));
            let neg = self.ins(format!("icmp slt i64 {}, 0", left));
            let l0 = self.ins(format!("select i1 {}, i64 0, i64 {}", neg, left));
            let big = self.ins(format!("icmp sgt i64 {}, 32", l0));
            let bits = self.ins(format!("select i1 {}, i64 32, i64 {}", big, l0));
            let one = self.ins(format!("shl i64 1, {}", bits));
            let m64 = self.ins(format!("sub i64 {}, 1", one));
            let m32 = self.ins(format!("trunc i64 {} to i32", m64));
            let mv = self.ins(format!("bitcast i32 {} to <32 x i1>", m32));
            let p = self.ins(format!("getelementptr inbounds i8, ptr {}, i64 {}", ptr, from));
            parts.push(self.ins(format!("call <32 x i8> @llvm.masked.load.v32i8.p0(ptr {}, i32 1, <32 x i1> {}, <32 x i8> zeroinitializer)", p, mv)));
        }
        let lov = self.widen(&v(lo.clone(), "i64"), w);
        let rel = self.ins(format!("sub {} {}, {}", vt, idx, lov.r));
        let i8v = self.ins(format!("trunc {} {} to <{} x i8>", vt, rel, w));
        let iv = if w == 32 {
            i8v
        } else {
            let lanes = (0..32).map(|j| if j < w { format!("i32 {}", j) } else { "i32 poison".to_string() }).collect::<Vec<_>>().join(", ");
            self.ins(format!("shufflevector <{} x i8> {}, <{} x i8> poison, <32 x i32> <{}>", w, i8v, w, lanes))
        };
        let r = self.ins(format!("call <32 x i8> @llvm.x86.avx512.vpermi2var.qi.256(<32 x i8> {}, <32 x i8> {}, <32 x i8> {})", parts[0], iv, parts[1]));
        let r1 = if w == 32 {
            r
        } else {
            let lanes = (0..w).map(|j| format!("i32 {}", j)).collect::<Vec<_>>().join(", ");
            self.ins(format!("shufflevector <32 x i8> {}, <32 x i8> poison, <{} x i32> <{}>", r, w, lanes))
        };
        let l1 = self.current_label(&lwin);
        self.br(&ljoin);
        self.block(&lgat);
        let r2 = self.gather(ptr, idx, elem, w);
        let l2 = self.current_label(&lgat);
        self.br(&ljoin);
        self.block(&ljoin);
        self.ins(format!("phi <{} x i8> [ {}, %{} ], [ {}, %{} ]", w, r1, l1, r2, l2))
    }

    /// The bytes of a table of `n` <= 256 bytes at the lanes of `idx` (in
    /// bounds): its 8 parts of 32 bytes in registers (masked loads: nothing
    /// read past its end), 4 permutations of 64 of them (`vpermi2b`, bits 0
    /// to 5 of each index), the one of bits 6 and 7 (D185).
    fn byte_lookup(&mut self, ptr: &str, n: &str, idx: &str, w: u32, loaded: Option<Vec<String>>) -> String {
        self.decls.insert("declare <32 x i8> @llvm.x86.avx512.vpermi2var.qi.256(<32 x i8>, <32 x i8>, <32 x i8>)".into());
        let parts = match loaded {
            Some(p) => p,
            None => self.table_registers(ptr, n),
        };
        self.permute_bytes(&parts, idx, w)
    }

    /// The 8 parts of 32 bytes of a table of `n` <= 256 bytes, the bytes
    /// past its end zero (masked loads: nothing read there).
    fn table_registers(&mut self, ptr: &str, n: &str) -> Vec<String> {
        self.decls.insert("declare <32 x i8> @llvm.masked.load.v32i8.p0(ptr, i32, <32 x i1>, <32 x i8>)".into());
        let mut parts = Vec::new();
        for k in 0..8 {
            let left = self.ins(format!("sub i64 {}, {}", n, 32 * k));
            let neg = self.ins(format!("icmp slt i64 {}, 0", left));
            let l0 = self.ins(format!("select i1 {}, i64 0, i64 {}", neg, left));
            let big = self.ins(format!("icmp sgt i64 {}, 32", l0));
            let bits = self.ins(format!("select i1 {}, i64 32, i64 {}", big, l0));
            let one = self.ins(format!("shl i64 1, {}", bits));
            let m64 = self.ins(format!("sub i64 {}, 1", one));
            let m32 = self.ins(format!("trunc i64 {} to i32", m64));
            let mv = self.ins(format!("bitcast i32 {} to <32 x i1>", m32));
            let p = self.ins(format!("getelementptr inbounds i8, ptr {}, i64 {}", ptr, 32 * k));
            parts.push(self.ins(format!("call <32 x i8> @llvm.masked.load.v32i8.p0(ptr {}, i32 1, <32 x i1> {}, <32 x i8> zeroinitializer)", p, mv)));
        }
        parts
    }

    /// The bytes of the table of `parts` at the lanes of `idx`.
    fn permute_bytes(&mut self, parts: &[String], idx: &str, w: u32) -> String {
        let i8v = self.ins(format!("trunc <{} x i64> {} to <{} x i8>", w, idx, w));
        let iv = if w == 32 {
            i8v
        } else {
            let lanes = (0..32).map(|j| if j < w { format!("i32 {}", j) } else { "i32 poison".to_string() }).collect::<Vec<_>>().join(", ");
            self.ins(format!("shufflevector <{} x i8> {}, <{} x i8> poison, <32 x i32> <{}>", w, i8v, w, lanes))
        };
        let mut q = Vec::new();
        for j in 0..4 {
            q.push(self.ins(format!("call <32 x i8> @llvm.x86.avx512.vpermi2var.qi.256(<32 x i8> {}, <32 x i8> {}, <32 x i8> {})", parts[2 * j], iv, parts[2 * j + 1])));
        }
        let splat64 = format!("<{}>", (0..32).map(|_| "i8 64").collect::<Vec<_>>().join(", "));
        let b6 = self.ins(format!("and <32 x i8> {}, {}", iv, splat64));
        let s6 = self.ins(format!("icmp ne <32 x i8> {}, zeroinitializer", b6));
        let s7 = self.ins(format!("icmp slt <32 x i8> {}, zeroinitializer", iv));
        let lo = self.ins(format!("select <32 x i1> {}, <32 x i8> {}, <32 x i8> {}", s6, q[1], q[0]));
        let hi = self.ins(format!("select <32 x i1> {}, <32 x i8> {}, <32 x i8> {}", s6, q[3], q[2]));
        let r = self.ins(format!("select <32 x i1> {}, <32 x i8> {}, <32 x i8> {}", s7, hi, lo));
        if w == 32 {
            return r;
        }
        let lanes = (0..w).map(|j| format!("i32 {}", j)).collect::<Vec<_>>().join(", ");
        self.ins(format!("shufflevector <32 x i8> {}, <32 x i8> poison, <{} x i32> <{}>", r, w, lanes))
    }

    /// 0 <= a <= b <= n, or the panic of a slice.
    fn slice_check(&mut self, a: &str, b: &str, n: &str, span: Span) {
        let c0 = self.ins(format!("icmp sge i64 {}, 0", a));
        let c1 = self.ins(format!("icmp sle i64 {}, {}", a, b));
        let c2 = self.ins(format!("icmp sle i64 {}, {}", b, n));
        let c01 = self.ins(format!("and i1 {}, {}", c0, c1));
        let ok = self.ins(format!("and i1 {}, {}", c01, c2));
        self.guard(&ok, format!("call void @isc_panic_slice(i64 {}, i64 {}, i64 {}, {})", a, b, n, Self::pos(span)));
    }

    fn bounds(&mut self, idx: &str, n: &str, t: &Type, span: Span) {
        let ok = self.ins(format!("icmp ult i64 {}, {}", idx, n));
        let unsigned = if is_signed(t) { 0 } else { 1 };
        self.guard(&ok, format!("call void @isc_panic_bounds(i64 {}, i64 {}, i32 {}, {})", idx, n, unsigned, Self::pos(span)));
    }

    fn binop(&mut self, op: BinOp, a: V, b: V, ty: &Type, w: u32, span: Span) -> R<V> {
        let all_uni = uni(&a) && uni(&b);
        let ww = if all_uni { 1 } else { w };
        let a = self.widen(&a, ww);
        let b = self.widen(&b, ww);
        let t = vt(ty, ww);
        let bool_t = vt(&Type::Bool, ww);
        use BinOp::*;
        if matches!(op, Min | Max) {
            // IEEE 754-2019 `minimum` / `maximum` for floats (SPEC section 10:
            // NaN gives NaN, -0 below +0), signed or unsigned for integers
            let (scalar, name) = match ty {
                Type::F64 => ("f64".to_string(), if op == Min { "minimum" } else { "maximum" }),
                Type::F32 => ("f32".to_string(), if op == Min { "minimum" } else { "maximum" }),
                Type::Int(k) => {
                    let n = match (op, k.signed()) {
                        (Min, true) => "smin",
                        (Min, false) => "umin",
                        (_, true) => "smax",
                        (_, false) => "umax",
                    };
                    (format!("i{}", k.bits()), n)
                }
                _ => return Err(format!("codegen: {} on {:?}", op.text(), ty)),
            };
            let f = if ww == 1 { format!("llvm.{}.{}", name, scalar) } else { format!("llvm.{}.v{}{}", name, ww, scalar) };
            self.decls.insert(format!("declare {} @{}({}, {})", t, f, t, t));
            let r = self.ins(format!("call {} @{}({} {}, {} {})", t, f, t, a.r, t, b.r));
            return Ok(V { r, t, uni: all_uni });
        }
        let (r, rt) = match ty {
            Type::F64 | Type::F32 => {
                let (ins, cmp) = match op {
                    Add => ("fadd", false),
                    Sub => ("fsub", false),
                    Mul => ("fmul", false),
                    Div => ("fdiv", false),
                    Rem => ("frem", false),
                    Eq => ("fcmp oeq", true),
                    Ne => ("fcmp une", true),
                    Lt => ("fcmp olt", true),
                    Le => ("fcmp ole", true),
                    Gt => ("fcmp ogt", true),
                    Ge => ("fcmp oge", true),
                    And | Or | Min | Max | BitAnd | BitOr | BitXor | Shl | Shr => return Err("codegen: logic on floats".into()),
                };
                // `allowReassoc`: what `-fassociative-math` allows (SPEC
                // section 10), not `-ffast-math`
                let flags = if self.fast_math && !cmp { " reassoc nsz contract" } else { "" };
                let r = self.ins(format!("{}{} {} {}, {}", ins, flags, t, a.r, b.r));
                (r, if cmp { bool_t } else { t.clone() })
            }
            Type::Int(k) => {
                let s = k.signed();
                let cmp = |o: BinOp| -> Option<&'static str> {
                    Some(match o {
                        Eq => "eq",
                        Ne => "ne",
                        Lt => if s { "slt" } else { "ult" },
                        Le => if s { "sle" } else { "ule" },
                        Gt => if s { "sgt" } else { "ugt" },
                        Ge => if s { "sge" } else { "uge" },
                        _ => return None,
                    })
                };
                if let Some(c) = cmp(op) {
                    let r = self.ins(format!("icmp {} {} {}, {}", c, t, a.r, b.r));
                    (r, bool_t)
                } else {
                    match op {
                        Add | Sub | Mul => {
                            let ins = match op {
                                Add => "add",
                                Sub => "sub",
                                _ => "mul",
                            };
                            (self.ins(format!("{} {} {}, {}", ins, t, a.r, b.r)), t.clone())
                        }
                        Div | Rem => (self.intdiv(op, &a, &b, *k, ww, span), t.clone()),
                        // bitwise (D136); the shift amount modulo the bits
                        BitAnd | BitOr | BitXor => {
                            let ins = match op {
                                BitAnd => "and",
                                BitOr => "or",
                                _ => "xor",
                            };
                            (self.ins(format!("{} {} {}, {}", ins, t, a.r, b.r)), t.clone())
                        }
                        Shl | Shr => {
                            let m = self.cst(&format!("i{}", k.bits()), &(k.bits() - 1).to_string(), ww);
                            let amount = self.ins(format!("and {} {}, {}", t, b.r, m));
                            let ins = match (op, s) {
                                (Shl, _) => "shl",
                                (_, true) => "ashr",
                                _ => "lshr",
                            };
                            (self.ins(format!("{} {} {}, {}", ins, t, a.r, amount)), t.clone())
                        }
                        _ => return Err("codegen: logic on integers".into()),
                    }
                }
            }
            Type::Bool => {
                let c = match op {
                    Eq => "eq",
                    Ne => "ne",
                    Lt => "ult",
                    Le => "ule",
                    Gt => "ugt",
                    Ge => "uge",
                    _ => return Err("codegen: arithmetic on booleans".into()),
                };
                (self.ins(format!("icmp {} {} {}, {}", c, t, a.r, b.r)), bool_t)
            }
            _ => return Err(format!("codegen: binary operator on {:?}", ty)),
        };
        Ok(V { r, t: rt, uni: all_uni })
    }

    /// Integer division (section 4): truncation toward zero, panic on zero,
    /// `MIN / -1 = MIN` and `MIN % -1 = 0`.
    fn intdiv(&mut self, op: BinOp, a: &V, b: &V, k: IntKind, w: u32, span: Span) -> String {
        let it = format!("i{}", k.bits());
        let t = if w == 1 { it.clone() } else { format!("<{} x {}>", w, it) };
        let z = if w == 1 { "0".to_string() } else { "zeroinitializer".to_string() };
        let isz = self.ins(format!("icmp eq {} {}, {}", t, b.r, z));
        let any = if w == 1 {
            isz
        } else {
            let name = format!("llvm.vector.reduce.or.v{}i1", w);
            self.decls.insert(format!("declare i1 @{}(<{} x i1>)", name, w));
            self.ins(format!("call i1 @{}(<{} x i1> {})", name, w, isz))
        };
        let ok = self.ins(format!("xor i1 {}, true", any));
        self.guard(&ok, format!("call void @isc_panic_div0({})", Self::pos(span)));
        if !k.signed() {
            let ins = if op == BinOp::Div { "udiv" } else { "urem" };
            return self.ins(format!("{} {} {}, {}", ins, t, a.r, b.r));
        }
        let m1 = self.cst(&it, "-1", w);
        let one = self.ct_one(&it, w);
        let bt = if w == 1 { "i1".to_string() } else { format!("<{} x i1>", w) };
        let ism1 = self.ins(format!("icmp eq {} {}, {}", t, b.r, m1));
        let safe_b = self.ins(format!("select {} {}, {} {}, {} {}", bt, ism1, t, one, t, b.r));
        if op == BinOp::Div {
            let q = self.ins(format!("sdiv {} {}, {}", t, a.r, safe_b));
            let neg = self.ins(format!("sub {} {}, {}", t, z, a.r));
            self.ins(format!("select {} {}, {} {}, {} {}", bt, ism1, t, neg, t, q))
        } else {
            let q = self.ins(format!("srem {} {}, {}", t, a.r, safe_b));
            self.ins(format!("select {} {}, {} {}, {} {}", bt, ism1, t, z, t, q))
        }
    }

    fn ct_one(&self, it: &str, w: u32) -> String {
        self.cst(it, "1", w)
    }

    fn mul(&mut self, a: &V, b: &V, ty: &Type, w: u32) -> V {
        let all_uni = uni(&a) && uni(&b);
        let ww = if all_uni { 1 } else { w };
        let a = self.widen(a, ww);
        let b = self.widen(b, ww);
        let t = vt(ty, ww);
        let ins = if ty.is_float() { "fmul" } else { "mul" };
        let flags = if self.fast_math && ty.is_float() { " reassoc nsz contract" } else { "" };
        let r = self.ins(format!("{}{} {} {}, {}", ins, flags, t, a.r, b.r));
        V { r, t, uni: all_uni }
    }

    /// `x ^ k` (section 10): binary exponentiation, most significant bit
    /// first; `x ^ 2` is exactly `x * x`.
    fn pow(&mut self, b: &Expr, k: &Expr, w: u32, span: Span) -> R<V> {
        let x = self.ex(b, w)?;
        let one = match &b.ty {
            Type::F64 => v(fconst(1.0), "double"),
            Type::F32 => v(fconst32(1.0), "float"),
            Type::Int(_) => v("1", lt(&b.ty)),
            _ => return Err("codegen: bad pow base".into()),
        };
        if let ExprKind::Lit(Lit::Int(n)) = k.kind {
            if !(is_signed(&k.ty) && n < 0) {
                let e = n as u64;
                if e == 0 {
                    return Ok(one);
                }
                let top = 63 - e.leading_zeros();
                let mut r = x.clone();
                for bit in (0..top).rev() {
                    r = self.mul(&r, &r, &b.ty, w);
                    if (e >> bit) & 1 == 1 {
                        r = self.mul(&r, &x, &b.ty, w);
                    }
                }
                return Ok(r);
            }
        }
        let kv = self.ex(k, 1)?;
        let e = self.index64(&kv, &k.ty);
        if is_signed(&k.ty) {
            let ok = self.ins(format!("icmp sge i64 {}, 0", e));
            self.guard(&ok, format!("call void @isc_panic_negexp(i64 {}, {})", e, Self::pos(span)));
        }
        // result for e == 0 is 1; otherwise loop over the bits below the top one
        let rt = if uni(&x) { lt(&b.ty) } else { vt(&b.ty, w) };
        let ra = self.alloca(&rt);
        let onew = self.widen(&one, if uni(&x) { 1 } else { w });
        self.emit(format!("store {} {}, ptr {}", rt, onew.r, ra));
        let lz = self.label();
        let lnz = self.label();
        let ldone = self.label();
        let isz = self.ins(format!("icmp eq i64 {}, 0", e));
        self.cbr(&isz, &lz, &lnz);
        self.block(&lnz);
        self.decls.insert("declare i64 @llvm.ctlz.i64(i64, i1)".into());
        let lzc = self.ins(format!("call i64 @llvm.ctlz.i64(i64 {}, i1 true)", e));
        let top = self.ins(format!("sub i64 63, {}", lzc));
        self.emit(format!("store {} {}, ptr {}", rt, x.r, ra));
        let ba = self.alloca("i64");
        self.emit(format!("store i64 {}, ptr {}", top, ba));
        let head = self.label();
        let body = self.label();
        self.br(&head);
        self.block(&head);
        let bit = self.ins(format!("load i64, ptr {}", ba));
        let more = self.ins(format!("icmp sgt i64 {}, 0", bit));
        self.cbr(&more, &body, &ldone);
        self.block(&body);
        let bit1 = self.ins(format!("sub i64 {}, 1", bit));
        self.emit(format!("store i64 {}, ptr {}", bit1, ba));
        let r0 = self.ins(format!("load {}, ptr {}", rt, ra));
        let rv = V { r: r0, t: rt.clone(), uni: uni(&x) };
        let sq = self.mul(&rv, &rv, &b.ty, w);
        let sx = self.mul(&sq, &x, &b.ty, w);
        let sh = self.ins(format!("lshr i64 {}, {}", e, bit1));
        let lb = self.ins(format!("and i64 {}, 1", sh));
        let set = self.ins(format!("icmp eq i64 {}, 1", lb));
        let sel = self.ins(format!("select i1 {}, {} {}, {} {}", set, rt, sx.r, rt, sq.r));
        self.emit(format!("store {} {}, ptr {}", rt, sel, ra));
        self.br(&head);
        self.block(&lz);
        self.br(&ldone);
        self.block(&ldone);
        let r = self.ins(format!("load {}, ptr {}", rt, ra));
        Ok(V { r, t: rt, uni: uni(&x) })
    }

    fn convert(&mut self, x: V, from: &Type, to: &Type, w: u32) -> R<V> {
        let ww = if uni(&x) { 1 } else { w };
        let ft = vt(from, ww);
        let tt = vt(to, ww);
        let r = match (from, to) {
            (Type::Int(a), Type::Int(b)) => {
                if a.bits() == b.bits() {
                    let u = uni(&x);
                    return Ok(V { r: x.r, t: tt, uni: u });
                } else if b.bits() < a.bits() {
                    self.ins(format!("trunc {} {} to {}", ft, x.r, tt))
                } else if a.signed() {
                    self.ins(format!("sext {} {} to {}", ft, x.r, tt))
                } else {
                    self.ins(format!("zext {} {} to {}", ft, x.r, tt))
                }
            }
            (Type::Int(a), Type::F64 | Type::F32) => {
                let op = if a.signed() { "sitofp" } else { "uitofp" };
                self.ins(format!("{} {} {} to {}", op, ft, x.r, tt))
            }
            (Type::F64 | Type::F32, Type::Int(b)) => {
                let kind = if b.signed() { "fptosi" } else { "fptoui" };
                let f = if *from == Type::F32 { "f32" } else { "f64" };
                let name = if ww == 1 {
                    format!("llvm.{}.sat.i{}.{}", kind, b.bits(), f)
                } else {
                    format!("llvm.{}.sat.v{}i{}.v{}{}", kind, ww, b.bits(), ww, f)
                };
                self.decls.insert(format!("declare {} @{}({})", tt, name, ft));
                self.ins(format!("call {} @{}({} {})", tt, name, ft, x.r))
            }
            (Type::F32, Type::F64) => self.ins(format!("fpext {} {} to {}", ft, x.r, tt)),
            (Type::F64, Type::F32) => self.ins(format!("fptrunc {} {} to {}", ft, x.r, tt)),
            (Type::F64, Type::F64) | (Type::F32, Type::F32) => return Ok(x),
            _ => return Err(format!("codegen: conversion {:?} -> {:?}", from, to)),
        };
        Ok(V { r, t: tt, uni: uni(&x) })
    }

    // ----- loops -------------------------------------------------------------

    /// Evaluates the leaves of a producer tree; returns the source and the
    /// length of the loop.
    fn prepare_src<'e>(&mut self, e: &'e Expr) -> R<(Src<'e>, String)> {
        match &e.kind {
            // read by index: bound to a `let` by `place_streams` (D99)
            ExprKind::Filter(..) => Err("internal error: a `filter` read by index".into()),
            ExprKind::Map(ins, l) => {
                let mut srcs = Vec::new();
                let mut len = None;
                for i in ins {
                    let (s, n) = self.prepare_src(i)?;
                    if len.is_none() {
                        len = Some(n);
                    }
                    srcs.push(s);
                }
                Ok((Src::Map(srcs, l), len.unwrap()))
            }
            ExprKind::Iota(n) => {
                let nv = self.ex(n, 1)?;
                Ok((Src::Iota, nv.r))
            }
            // a slice of a producer: its elements from `a`, computed when read
            ExprKind::Slice(x, a, b) if isc_ir::passes::is_producer(x) => {
                let (inner, n) = self.prepare_src(x)?;
                let av = self.ex(a, 1)?;
                let bv = self.ex(b, 1)?;
                self.slice_check(&av.r, &bv.r, &n, e.span);
                let m = self.ins(format!("sub i64 {}, {}", bv.r, av.r));
                Ok((Src::Offset { inner: Box::new(inner), start: av.r }, m))
            }
            ExprKind::Fill(n, x) => {
                let nv = self.ex(n, 1)?;
                let xv = self.ex(x, 1)?;
                Ok((Src::Uniform(xv), nv.r))
            }
            ExprKind::Zeros(n) => {
                let nv = self.ex(n, 1)?;
                let et = e.ty.elem();
                Ok((Src::Uniform(v(Self::zero(et), lt(et))), nv.r))
            }
            _ if e.ty.elem().is_array() => {
                let a = self.ex(e, 1)?;
                let (p, rows, cols, stride) = self.nested_parts(&a);
                Ok((Src::Rows { ptr: p, cols, stride, elem: e.ty.elem().elem().clone() }, rows))
            }
            _ => {
                let a = self.ex(e, 1)?;
                let p = self.ins(format!("extractvalue {{ ptr, i64 }} {}, 0", a.r));
                let n = self.ins(format!("extractvalue {{ ptr, i64 }} {}, 1", a.r));
                Ok((Src::Mat { ptr: p, len: n.clone(), elem: e.ty.elem().clone() }, n))
            }
        }
    }

    /// Element(s) `idx .. idx + w` of a source.
    fn elem(&mut self, s: &Src, idx: &str, w: u32) -> R<V> {
        match s {
            Src::Mat { ptr, len, elem } => Ok(self.load_elem(ptr, len, idx, elem, w)),
            Src::Iota => {
                if w == 1 {
                    return Ok(V { r: idx.to_string(), t: "i64".into(), uni: false });
                }
                let base = self.widen(&v(idx, "i64"), w);
                let steps: Vec<String> = (0..w).map(|i| format!("i64 {}", i)).collect();
                let r = self.ins(format!("add <{} x i64> {}, <{}>", w, base.r, steps.join(", ")));
                self.f.contig.insert(r.clone(), (idx.to_string(), 1));
                Ok(V { r, t: format!("<{} x i64>", w), uni: false })
            }
            Src::Uniform(x) => Ok(x.clone()),
            Src::Offset { inner, start } => {
                let j = self.ins(format!("add i64 {}, {}", start, idx));
                self.elem(inner, &j, w)
            }
            Src::Rows { ptr, cols, stride, elem } => Ok(self.row_view(ptr, cols, stride, idx, elem)),
            Src::Map(srcs, l) => {
                let mut args = Vec::new();
                for s in srcs {
                    args.push(self.elem(s, idx, w)?);
                }
                for (p, a) in l.params.iter().zip(args) {
                    self.bind(*p, a);
                }
                self.ex(&l.body, w)
            }
        }
    }

    fn materialize(&mut self, e: &Expr) -> R<V> {
        let et = e.ty.elem().clone();
        if let ExprKind::Zeros(n) = &e.kind {
            let nv = self.ex(n, 1)?;
            let bytes = self.array_bytes(&nv.r, &et);
            let p = self.ins(format!("call ptr @isc_alloc_zeroed(i64 {})", bytes));
            return Ok(self.arr(&p, &nv.r));
        }
        let spmd = match &e.kind {
            ExprKind::Map(_, l) => spmd_state(self.p, &l.body),
            _ => None,
        };
        // read by `fill_range`, whose vector loop replays a failed check and
        // permutes small tables of bytes (D185)
        let w = match (src_vec_ok_lookup(e), spmd) {
            (false, _) => 1,
            (true, Some(state)) => self.spmd_width(state),
            // bytes: 32 lanes, one `ymm` register of results (D199)
            (true, None) if esize(&et) == 1 && et.is_scalar() => 2 * MAP_WIDTH,
            (true, None) => MAP_WIDTH,
        };
        let (src, n) = self.prepare_src(e)?;
        // an input moved into the `map` (an array of the same scalars at its
        // last use, D207): the results written into it, no new array
        let reused = match &e.kind {
            ExprKind::Map(ins, _) => ins.iter().find_map(|i| match i.kind {
                ExprKind::Var(v, Use::Move) if i.ty.is_array() && *i.ty.elem() == et => self.env.get(&v).cloned(),
                _ => None,
            }),
            _ => None,
        };
        let p = match reused {
            Some(x) => self.ins(format!("extractvalue {{ ptr, i64 }} {}, 0", x.r)),
            None => self.alloc_elems(&n, &et),
        };
        // when each element is a loop of its own (a `reduce` per row, a
        // call), chunks taken in turn (D121, D143)
        let heavy = matches!(&e.kind, ExprKind::Map(_, l) if inner_loop(&l.body));
        if self.in_task {
            self.fill_range(&src, &p, &n, &et, "0", &n, w)?;
            return Ok(self.arr(&p, &n));
        }
        // in parallel, by blocks of B elements; when each element is a loop,
        // HEAVY_CHUNKS chunks per thread: the elements do not depend on the
        // cut (D121)
        // at most one block: filled here, without a task (D172)
        let ldone = if heavy {
            None
        } else {
            let small = self.ins(format!("icmp sle i64 {}, {}", n, BLOCK));
            let lsmall = self.label();
            let lpar = self.label();
            let ldone = self.label();
            self.cbr(&small, &lsmall, &lpar);
            self.block(&lsmall);
            self.fill_range(&src, &p, &n, &et, "0", &n, w)?;
            self.br(&ldone);
            self.block(&lpar);
            Some(ldone)
        };
        let bs = if heavy {
            // HEAVY_CHUNKS chunks per thread, taken in turn (`isc_parallel_for_dynamic`):
            // elements of very different costs (a loop whose length depends
            // on the element, D140) are shared out evenly (D143)
            let t0 = self.ins("call i64 @isc_thread_count()".to_string());
            let threads = self.ins(format!("mul i64 {}, {}", t0, HEAVY_CHUNKS));
            let t1 = self.ins(format!("add i64 {}, {}", n, threads));
            let t2 = self.ins(format!("sub i64 {}, 1", t1));
            let t3 = self.ins(format!("sdiv i64 {}, {}", t2, threads));
            let one = self.ins(format!("icmp slt i64 {}, 1", t3));
            self.ins(format!("select i1 {}, i64 1, i64 {}", one, t3))
        } else {
            BLOCK.to_string()
        };
        let n0 = self.ins(format!("add i64 {}, {}", n, bs));
        let n1 = self.ins(format!("sub i64 {}, 1", n0));
        let c = self.ins(format!("sdiv i64 {}, {}", n1, bs));
        let pv = v(p.clone(), "ptr");
        let bsv = v(bs.clone(), "i64");
        let (task, ctx, ctx_bytes) = self.outline_task(&[e], &[pv, bsv], &mut |g, extra, i| {
            let (src, n) = g.prepare_src(e)?;
            let start = g.ins(format!("mul i64 {}, {}", i, extra[1].r));
            let lim = g.ins(format!("add i64 {}, {}", start, extra[1].r));
            let short = g.ins(format!("icmp slt i64 {}, {}", n, lim));
            let end = g.ins(format!("select i1 {}, i64 {}, i64 {}", short, n, lim));
            if !heavy {
                return g.fill_range(&src, &extra[0].r, &n, &et, &start, &end, w);
            }
            // each chunk starts at a different place of its range (then
            // wraps): the threads do not read addresses a power of two
            // apart at the same time (D121)
            let len = g.ins(format!("sub i64 {}, {}", end, start));
            let k = g.ins(format!("mul i64 {}, 37", i));
            let empty = g.ins(format!("icmp slt i64 {}, 1", len));
            let len1 = g.ins(format!("select i1 {}, i64 1, i64 {}", empty, len));
            let rot = g.ins(format!("srem i64 {}, {}", k, len1));
            let mid = g.ins(format!("add i64 {}, {}", start, rot));
            g.fill_range(&src, &extra[0].r, &n, &et, &mid, &end, w)?;
            g.fill_range(&src, &extra[0].r, &n, &et, &start, &mid, w)
        })?;
        // the chunks of elements that are loops: taken by the threads as they
        // finish the one before (D145)
        let pf = if heavy { "isc_parallel_for_dynamic" } else { "isc_parallel_for" };
        self.emit(format!("call void @{}(i64 {}, ptr {}, ptr {}, i64 {})", pf, c, task, ctx, ctx_bytes));
        if let Some(l) = ldone {
            self.br(&l);
            self.block(&l);
        }
        Ok(self.arr(&p, &n))
    }

    /// `printBytes` of a `map` of bytes made only for it (D225): by blocks of
    /// STREAM bytes, each made in parallel as `materialize` makes the whole
    /// array (tasks of BLOCK elements, the same `fill_range`), into one
    /// buffer of STREAM bytes, then written; the buffer stays in the caches
    /// (a quarter of a MiB per thread), the bytes never go to memory nor to
    /// new pages. The element `i` of the block from `lo` is stored at
    /// `buffer - lo + i`: the code of the elements does not change. The
    /// same bytes, written in the same order.
    fn print_stream(&mut self, e: &Expr) -> R<()> {
        const STREAM: i64 = 1 << 20;
        let et = e.ty.elem().clone();
        let spmd = match &e.kind {
            ExprKind::Map(_, l) => spmd_state(self.p, &l.body),
            _ => None,
        };
        let w = match (src_vec_ok_lookup(e), spmd) {
            (false, _) => 1,
            (true, Some(state)) => self.spmd_width(state),
            (true, None) => 2 * MAP_WIDTH,
        };
        let (src, n) = self.prepare_src(e)?;
        let small = self.ins(format!("icmp slt i64 {}, {}", n, STREAM));
        let size = self.ins(format!("select i1 {}, i64 {}, i64 {}", small, n, STREAM));
        let buf = self.ins(format!("call ptr @isc_alloc(i64 {})", size));
        let in_task = self.in_task;
        // by the number of the block (`for_range` takes whole steps only)
        let nb0 = self.ins(format!("add i64 {}, {}", n, STREAM - 1));
        let nb = self.ins(format!("sdiv i64 {}, {}", nb0, STREAM));
        self.for_range_scalar("0", &nb, 1, &mut |g, k| {
            let lo = &g.ins(format!("mul i64 {}, {}", k, STREAM));
            let lim = g.ins(format!("add i64 {}, {}", lo, STREAM));
            let short = g.ins(format!("icmp slt i64 {}, {}", n, lim));
            let hi = g.ins(format!("select i1 {}, i64 {}, i64 {}", short, n, lim));
            let neg = g.ins(format!("sub i64 0, {}", lo));
            let dst = g.ins(format!("getelementptr i8, ptr {}, i64 {}", buf, neg));
            if in_task {
                g.fill_range(&src, &dst, &n, &et, lo, &hi, w)?;
            } else {
                let len = g.ins(format!("sub i64 {}, {}", hi, lo));
                let c0 = g.ins(format!("add i64 {}, {}", len, BLOCK - 1));
                let c = g.ins(format!("sdiv i64 {}, {}", c0, BLOCK));
                let extra = [v(dst.clone(), "ptr"), v(lo.to_string(), "i64"), v(hi.clone(), "i64")];
                let et2 = et.clone();
                let (task, ctx, ctx_bytes) = g.outline_task(&[e], &extra, &mut |t, x, i| {
                    let (src, n) = t.prepare_src(e)?;
                    let off = t.ins(format!("mul i64 {}, {}", i, BLOCK));
                    let start = t.ins(format!("add i64 {}, {}", x[1].r, off));
                    let lim = t.ins(format!("add i64 {}, {}", start, BLOCK));
                    let short = t.ins(format!("icmp slt i64 {}, {}", x[2].r, lim));
                    let end = t.ins(format!("select i1 {}, i64 {}, i64 {}", short, x[2].r, lim));
                    t.fill_range(&src, &x[0].r, &n, &et2, &start, &end, w)
                })?;
                g.emit(format!("call void @isc_parallel_for(i64 {}, ptr {}, ptr {}, i64 {})", c, task, ctx, ctx_bytes));
            }
            let len = g.ins(format!("sub i64 {}, {}", hi, lo));
            g.emit(format!("call void @isc_print_bytes(ptr {}, i64 {})", buf, len));
            Ok(())
        })?;
        self.emit(format!("call void @isc_free(ptr {})", buf));
        Ok(())
    }

    /// `map(rows, (w) -> ... reduce(w, op, z) ...)` over windows (a view of
    /// stride 1) of a width multiple of V, at most B: each window is one
    /// block of the fixed form (section 10), and its tree shares its levels
    /// with the windows next to it: the value of lane j is
    /// L(p) = op(...op(z, x[p]), x[p + V]...) at p = i + j, and after the
    /// level d the value of lane j is M_d(i + j) with
    /// M_d(p) = op(M_2d(p), M_2d(p + d)); the result of window i is M_1(i).
    /// So each level is computed once per position, by blocks of
    /// `CASCADE` windows in two buffers on the stack (in parallel, block by
    /// block, in a task that prefers 512-bit vectors on AVX-512), and the
    /// rest of the lambda gets M_1(i) for its `reduce`: L in one pass for
    /// lanes of at most 4 elements, the levels two at a time, the last one
    /// with the lambda. The same bits as one `reduce` per window, which the
    /// other windows (a stride other than 1, another width) still get
    /// (D115).
    fn window_cascade(&mut self, e: &Expr) -> R<V> {
        const CASCADE: i64 = 1024;
        let ExprKind::Map(ins, l) = &e.kind else { unreachable!() };
        let red = window_reduce_of(l).unwrap();
        let ExprKind::Reduce(_, op, z) = &red.kind else { unreachable!() };
        let et = e.ty.elem().clone();
        let rt = red.ty.clone();
        let rlt = lt(&rt);
        let lanes = self.opts.lanes as i64;
        let rows_v = self.ex(&ins[0], 1)?;
        let (_, rows, cols, stride) = self.nested_parts(&rows_v);
        let one = self.ins(format!("icmp eq i64 {}, 1", stride));
        let rem = self.ins(format!("srem i64 {}, {}", cols, lanes));
        let multiple = self.ins(format!("icmp eq i64 {}, 0", rem));
        let wide = self.ins(format!("icmp sge i64 {}, {}", cols, lanes));
        let small = self.ins(format!("icmp sle i64 {}, {}", cols, BLOCK));
        let c1 = self.ins(format!("and i1 {}, {}", one, multiple));
        let c2 = self.ins(format!("and i1 {}, {}", wide, small));
        let ok = self.ins(format!("and i1 {}, {}", c1, c2));
        let res = self.alloca("{ ptr, i64 }");
        let lfast = self.label();
        let lslow = self.label();
        let ljoin = self.label();
        self.cbr(&ok, &lfast, &lslow);
        self.block(&lslow);
        let slow = self.materialize(e)?;
        self.emit(format!("store {{ ptr, i64 }} {}, ptr {}", slow.r, res));
        self.br(&ljoin);
        self.block(&lfast);
        let out = self.alloc_elems(&rows, &et);
        let nb1 = self.ins(format!("add i64 {}, {}", rows, CASCADE - 1));
        let nb = self.ins(format!("sdiv i64 {}, {}", nb1, CASCADE));
        // one block of windows: its levels, then the rest of the lambda
        let chunk = |g: &mut Self, rows_v: &V, outp: &str, b: &str| -> R<()> {
            let (p, rows, cols, _) = g.nested_parts(rows_v);
            let zv = g.ex(z, 1)?;
            let bt = format!("[{} x {}]", CASCADE + lanes, rlt);
            let ba = g.alloca(&bt);
            let bb = g.alloca(&bt);
            let s = g.ins(format!("mul i64 {}, {}", b, CASCADE));
            let left = g.ins(format!("sub i64 {}, {}", rows, s));
            let short = g.ins(format!("icmp slt i64 {}, {}", left, CASCADE));
            let cnt = g.ins(format!("select i1 {}, i64 {}, i64 {}", short, left, CASCADE));
            let per_lane = g.ins(format!("sdiv i64 {}, {}", cols, lanes));
            // L(q) for q in 0 .. cnt + V - 1
            let nl = g.ins(format!("add i64 {}, {}", cnt, lanes - 1));
            let xs = g.ins(format!("getelementptr inbounds {}, ptr {}, i64 {}", rlt, p, s));
            // z everywhere, then one pass per element of a lane: for each q
            // the elements still come in their order, and each pass is a
            // plain loop over q (vectorized)
            // L(q) in one pass when a lane has at most FUSED_LANE elements
            // (windows of V to 4 V: one loop per count, chosen here); the m
            // elements of a lane in their order
            const FUSED_LANE: i64 = 4;
            let ldone = g.label();
            for m in 1..=FUSED_LANE {
                let is_m = g.ins(format!("icmp eq i64 {}, {}", per_lane, m));
                let lyes = g.label();
                let lno = g.label();
                g.cbr(&is_m, &lyes, &lno);
                g.block(&lyes);
                g.for_range("0", &nl, 1, &mut |g, q| {
                    let mut acc = zv.clone();
                    for t in 0..m {
                        let at = g.ins(format!("add i64 {}, {}", q, t * lanes));
                        let px = g.ins(format!("getelementptr inbounds {}, ptr {}, i64 {}", rlt, xs, at));
                        let xv = g.ins(format!("load {}, ptr {}", rlt, px));
                        acc = g.apply2(op, acc, v(xv, rlt.clone()), 1)?;
                    }
                    let pq = g.ins(format!("getelementptr inbounds {}, ptr {}, i64 0, i64 {}", bt, ba, q));
                    g.emit(format!("store {} {}, ptr {}", rlt, acc.r, pq));
                    Ok(())
                })?;
                g.br(&ldone);
                g.block(&lno);
            }
            // wider lanes: the first element of each lane with z, in one
            // pass, then one pass per element
            g.for_range("0", &nl, 1, &mut |g, q| {
                let px = g.ins(format!("getelementptr inbounds {}, ptr {}, i64 {}", rlt, xs, q));
                let xv = g.ins(format!("load {}, ptr {}", rlt, px));
                let r = g.apply2(op, zv.clone(), v(xv, rlt.clone()), 1)?;
                let pq = g.ins(format!("getelementptr inbounds {}, ptr {}, i64 0, i64 {}", bt, ba, q));
                g.emit(format!("store {} {}, ptr {}", rlt, r.r, pq));
                Ok(())
            })?;
            g.for_range_scalar("1", &per_lane, 1, &mut |g, t| {
                let off = g.ins(format!("mul i64 {}, {}", t, lanes));
                let xt = g.ins(format!("getelementptr inbounds {}, ptr {}, i64 {}", rlt, xs, off));
                g.for_range("0", &nl, 1, &mut |g, q| {
                    let px = g.ins(format!("getelementptr inbounds {}, ptr {}, i64 {}", rlt, xt, q));
                    let xv = g.ins(format!("load {}, ptr {}", rlt, px));
                    let pq = g.ins(format!("getelementptr inbounds {}, ptr {}, i64 0, i64 {}", bt, ba, q));
                    let a = g.ins(format!("load {}, ptr {}", rlt, pq));
                    let r = g.apply2(op, v(a, rlt.clone()), v(xv, rlt.clone()), 1)?;
                    g.emit(format!("store {} {}, ptr {}", rlt, r.r, pq));
                    Ok(())
                })
            })?;
            g.br(&ldone);
            g.block(&ldone);
            // the levels d = V/2, ..., 2, from one buffer to the other, two
            // at a time (d and d/2 in one pass: one store per position
            // instead of two, the operands of the tree unchanged); the last
            // one (d = 1) with the rest of the lambda
            let mut d = lanes / 2;
            let mut from = ba.clone();
            let mut to = bb.clone();
            while d >= 4 {
                let h = d / 2;
                let np = g.ins(format!("add i64 {}, {}", cnt, h - 1));
                let (f, t) = (from.clone(), to.clone());
                g.for_range("0", &np, 1, &mut |g, q| {
                    let at = |g: &mut Self, off: i64| -> V {
                        let qo = g.ins(format!("add i64 {}, {}", q, off));
                        let p = g.ins(format!("getelementptr inbounds {}, ptr {}, i64 0, i64 {}", bt, f, qo));
                        v(g.ins(format!("load {}, ptr {}", rlt, p)), rlt.clone())
                    };
                    let (a0, a1, b0, b1) = (at(g, 0), at(g, d), at(g, h), at(g, h + d));
                    let a = g.apply2(op, a0, a1, 1)?;
                    let b = g.apply2(op, b0, b1, 1)?;
                    let r = g.apply2(op, a, b, 1)?;
                    let pt = g.ins(format!("getelementptr inbounds {}, ptr {}, i64 0, i64 {}", bt, t, q));
                    g.emit(format!("store {} {}, ptr {}", rlt, r.r, pt));
                    Ok(())
                })?;
                std::mem::swap(&mut from, &mut to);
                d /= 4;
            }
            while d >= 2 {
                let np = g.ins(format!("add i64 {}, {}", cnt, d - 1));
                let (f, t) = (from.clone(), to.clone());
                g.for_range("0", &np, 1, &mut |g, q| {
                    let qd = g.ins(format!("add i64 {}, {}", q, d));
                    let pa = g.ins(format!("getelementptr inbounds {}, ptr {}, i64 0, i64 {}", bt, f, q));
                    let pb = g.ins(format!("getelementptr inbounds {}, ptr {}, i64 0, i64 {}", bt, f, qd));
                    let a = g.ins(format!("load {}, ptr {}", rlt, pa));
                    let bv = g.ins(format!("load {}, ptr {}", rlt, pb));
                    let r = g.apply2(op, v(a, rlt.clone()), v(bv, rlt.clone()), 1)?;
                    let pt = g.ins(format!("getelementptr inbounds {}, ptr {}, i64 0, i64 {}", bt, t, q));
                    g.emit(format!("store {} {}, ptr {}", rlt, r.r, pt));
                    Ok(())
                })?;
                std::mem::swap(&mut from, &mut to);
                d /= 2;
            }
            // the rest of the lambda, its `reduce` given: by vectors of 64
            // bytes, with non-temporal stores when the result is larger
            // than the caches (D122), then one at a time
            let fin = from.clone();
            let vw = (64 / esize(&rt)) as u32;
            let mut tail_from = "0".to_string();
            let body_vec = vec_ok_given(&l.body, &mut l.params.iter().copied().collect(), red as *const Expr) && !has_index(&l.body);
            if body_vec && et.is_scalar() && esize(&et) == esize(&rt) && vw > 1 {
                let full = g.ins(format!("and i64 {}, -{}", cnt, vw));
                let vty = format!("<{} x {}>", vw, rlt);
                let nt = !g.opts.isa.is_wasm();
                let bytes = g.ins(format!("mul i64 {}, {}", rows, esize(&et)));
                let huge = g.ins(format!("icmp sge i64 {}, {}", bytes, g.opts.stream_bytes));
                let lnt = g.label();
                let lplain = g.label();
                let lafter = g.label();
                if nt {
                    g.cbr(&huge, &lnt, &lplain);
                } else {
                    g.br(&lplain);
                }
                let modes: &[bool] = if nt { &[true, false] } else { &[false] };
                for &streaming in modes {
                    g.block(if streaming { &lnt } else { &lplain });
                    g.for_range_scalar("0", &full, vw as i64, &mut |g, q| {
                        let q1 = g.ins(format!("add i64 {}, 1", q));
                        let pa = g.ins(format!("getelementptr inbounds {}, ptr {}, i64 0, i64 {}", bt, fin, q));
                        let pb = g.ins(format!("getelementptr inbounds {}, ptr {}, i64 0, i64 {}", bt, fin, q1));
                        let a = g.ins(format!("load {}, ptr {}, align {}", vty, pa, esize(&rt)));
                        let bv = g.ins(format!("load {}, ptr {}, align {}", vty, pb, esize(&rt)));
                        let r = g.apply2(op, V { r: a, t: vty.clone(), uni: false }, V { r: bv, t: vty.clone(), uni: false }, vw)?;
                        g.pre.insert(red as *const Expr, r);
                        let y = g.ex(&l.body, vw);
                        g.pre.remove(&(red as *const Expr));
                        let y = g.widen(&y?, vw);
                        let i = g.ins(format!("add i64 {}, {}", s, q));
                        if streaming {
                            // 64-byte aligned: `s` a multiple of the block, `q` of vw
                            let po = g.ins(format!("getelementptr inbounds {}, ptr {}, i64 {}", lt(&et), outp, i));
                            g.emit(format!("store {} {}, ptr {}, align 64, !nontemporal !{{i32 1}}", y.t, y.r, po));
                        } else {
                            g.store_elem(outp, &rows, &i, &et, &y, vw);
                        }
                        Ok(())
                    })?;
                    if streaming {
                        g.emit("call void @llvm.x86.sse.sfence()".to_string());
                        g.decls.insert("declare void @llvm.x86.sse.sfence()".to_string());
                    }
                    g.br(&lafter);
                }
                g.block(&lafter);
                tail_from = full;
            }
            g.for_range(&tail_from, &cnt, 1, &mut |g, q| {
                let q1 = g.ins(format!("add i64 {}, 1", q));
                let pa = g.ins(format!("getelementptr inbounds {}, ptr {}, i64 0, i64 {}", bt, fin, q));
                let pb = g.ins(format!("getelementptr inbounds {}, ptr {}, i64 0, i64 {}", bt, fin, q1));
                let a = g.ins(format!("load {}, ptr {}", rlt, pa));
                let bv = g.ins(format!("load {}, ptr {}", rlt, pb));
                let r = g.apply2(op, v(a, rlt.clone()), v(bv, rlt.clone()), 1)?;
                g.pre.insert(red as *const Expr, r);
                let y = g.ex(&l.body, 1);
                g.pre.remove(&(red as *const Expr));
                let y = y?;
                let i = g.ins(format!("add i64 {}, {}", s, q));
                g.store_elem(outp, &rows, &i, &et, &y, 1);
                Ok(())
            })
        };
        if self.in_task {
            let rv = rows_v.clone();
            let o = out.clone();
            self.for_range_scalar("0", &nb, 1, &mut |g, b| chunk(g, &rv, &o, b))?;
        } else {
            let extra = [v(out.clone(), "ptr")];
            // passes over buffers in the L1 cache: 512-bit vectors halve
            // their instructions (measured: 32 768 `f64`, 28.0 µs with
            // 256-bit vectors, 19.8 µs with 512-bit ones, D115)
            self.wide_task = true;
            let (task, ctx, ctx_bytes) = self.outline_task(&[e], &extra, &mut |g, extra, b| {
                let rv = g.ex(&ins[0], 1)?;
                chunk(g, &rv, &extra[0].r, b)
            })?;
            self.emit(format!("call void @isc_parallel_for(i64 {}, ptr {}, ptr {}, i64 {})", nb, task, ctx, ctx_bytes));
        }
        let fast = self.arr(&out, &rows);
        self.emit(format!("store {{ ptr, i64 }} {}, ptr {}", fast.r, res));
        self.br(&ljoin);
        self.block(&ljoin);
        let r = self.ins(format!("load {{ ptr, i64 }}, ptr {}", res));
        Ok(V { r, t: "{ ptr, i64 }".into(), uni: true })
    }

    /// `map(x, f)` where `f` returns an array of scalars: an array of
    /// arrays, its rows of the same length (the type says so: their size
    /// does not depend on the element) in one block, row i written in its
    /// place by `row_into` (D126); in parallel, one chunk of rows per
    /// thread.
    fn materialize_rows(&mut self, e: &Expr) -> R<V> {
        let ExprKind::Map(ins, l) = &e.kind else { unreachable!() };
        let row_ty = e.ty.elem().clone();
        let et = row_ty.elem().clone();
        if !et.is_scalar() && et != Type::Str && et != Type::Big {
            return Err(format!("{}:{}: a `map` building an array of arrays of `{:?}` is not supported in the POC", e.span.line, e.span.col, et));
        }
        let mut srcs = Vec::new();
        let mut n = String::new();
        for x in ins {
            let (s, len) = self.prepare_src(x)?;
            srcs.push(s);
            n = len;
        }
        // the length of the rows: that of row 0 (none: 0)
        let some = self.ins(format!("icmp sgt i64 {}, 0", n));
        let mslot = self.alloca("i64");
        self.emit(format!("store i64 0, ptr {}", mslot));
        let lfirst = self.label();
        let ljoin = self.label();
        self.cbr(&some, &lfirst, &ljoin);
        self.block(&lfirst);
        for (s, p) in srcs.iter().zip(l.params.iter()) {
            let x = self.elem(s, "0", 1)?;
            self.bind(*p, x);
        }
        let m0 = self.row_into(&l.body, None, &et, None)?;
        self.emit(format!("store i64 {}, ptr {}", m0, mslot));
        self.br(&ljoin);
        self.block(&ljoin);
        let m = self.ins(format!("load i64, ptr {}", mslot));
        let total = self.ins(format!("mul i64 {}, {}", n, m));
        let block = self.alloc_elems(&total, &et);
        let esz = esize(&et);
        if self.in_task {
            let b = block.clone();
            let mm = m.clone();
            self.for_range_scalar("0", &n, 1, &mut |g, i| {
                for (s, p) in srcs.iter().zip(l.params.iter()) {
                    let x = g.elem(s, i, 1)?;
                    g.bind(*p, x);
                }
                let off = g.ins(format!("mul i64 {}, {}", i, mm));
                let off = g.ins(format!("mul i64 {}, {}", off, esz));
                let dest = g.ins(format!("getelementptr inbounds i8, ptr {}, i64 {}", b, off));
                g.row_into(&l.body, Some(&dest), &et, Some(&mm))?;
                Ok(())
            })?;
        } else {
            let threads = self.ins("call i64 @isc_thread_count()".to_string());
            let t1 = self.ins(format!("add i64 {}, {}", n, threads));
            let t2 = self.ins(format!("sub i64 {}, 1", t1));
            let t3 = self.ins(format!("sdiv i64 {}, {}", t2, threads));
            let small = self.ins(format!("icmp slt i64 {}, 1", t3));
            let bs = self.ins(format!("select i1 {}, i64 1, i64 {}", small, t3));
            let c0 = self.ins(format!("add i64 {}, {}", n, bs));
            let c1 = self.ins(format!("sub i64 {}, 1", c0));
            let chunks = self.ins(format!("sdiv i64 {}, {}", c1, bs));
            let extra = [v(block.clone(), "ptr"), v(m.clone(), "i64"), v(bs.clone(), "i64")];
            let (task, ctx, ctx_bytes) = self.outline_task(&[e], &extra, &mut |g, extra, c| {
                let mut srcs = Vec::new();
                let mut n = String::new();
                for x in ins {
                    let (s, len) = g.prepare_src(x)?;
                    srcs.push(s);
                    n = len;
                }
                let start = g.ins(format!("mul i64 {}, {}", c, extra[2].r));
                let lim = g.ins(format!("add i64 {}, {}", start, extra[2].r));
                let short = g.ins(format!("icmp slt i64 {}, {}", n, lim));
                let end = g.ins(format!("select i1 {}, i64 {}, i64 {}", short, n, lim));
                let (b, mm) = (extra[0].r.clone(), extra[1].r.clone());
                g.for_range_scalar(&start, &end, 1, &mut |g, i| {
                    for (s, p) in srcs.iter().zip(l.params.iter()) {
                        let x = g.elem(s, i, 1)?;
                        g.bind(*p, x);
                    }
                    let off = g.ins(format!("mul i64 {}, {}", i, mm));
                    let off = g.ins(format!("mul i64 {}, {}", off, esz));
                    let dest = g.ins(format!("getelementptr inbounds i8, ptr {}, i64 {}", b, off));
                    g.row_into(&l.body, Some(&dest), &et, Some(&mm))?;
                    Ok(())
                })
            })?;
            self.emit(format!("call void @isc_parallel_for(i64 {}, ptr {}, ptr {}, i64 {})", chunks, task, ctx, ctx_bytes));
        }
        Ok(self.nested(&block, &n, &m, &m))
    }

    /// The row `body` (an array of scalars or strings of type `et`) written
    /// at `dest` without a temporary array when it can be: a literal element
    /// by element, a producer filled in place, a view (a variable, a slice, a
    /// row of another array of arrays) copied, any other array computed then
    /// moved; returns its length, checked against `expect`, the length of
    /// the rows (D154: an array of arrays is regular). With no `dest`, only
    /// the length (computed without writing the elements when it can be).
    fn row_into(&mut self, body: &Expr, dest: Option<&str>, et: &Type, expect: Option<&str>) -> R<String> {
        use ExprKind::*;
        let span = body.span;
        let check = |g: &mut Self, n: &str| {
            if let (Some(m), Some(_)) = (expect, dest) {
                let same = g.ins(format!("icmp eq i64 {}, {}", n, m));
                g.guard(&same, format!("call void @isc_panic_size(i64 {}, i64 {}, {})", n, m, Self::pos(span)));
            }
        };
        let owned = holds_owned(et);
        match &body.kind {
            Let { var, init, body: b, drop, .. } => {
                let iv = self.ex(init, 1)?;
                self.bind(*var, iv.clone());
                let len = self.row_into(b, dest, et, expect)?;
                if *drop {
                    let t = self.var_ty(*var).clone();
                    self.drop_value(&iv, &t);
                }
                Ok(len)
            }
            ArrayLit(es) => {
                let n = es.len().to_string();
                check(self, &n);
                if let Some(d) = dest {
                    for (k, x) in es.iter().enumerate() {
                        let xv = self.ex(x, 1)?;
                        self.store_scalar(d, &k.to_string(), et, &xv, 1);
                    }
                }
                Ok(n)
            }
            Map(..) | Iota(_) | Fill(..) | Zeros(_) if !owned => {
                let (src, n) = self.prepare_src(body)?;
                check(self, &n);
                if let Some(d) = dest {
                    let w = if src_vec_ok(body) { MAP_WIDTH } else { 1 };
                    self.fill_range(&src, d, &n, et, "0", &n, w)?;
                }
                Ok(n)
            }
            // (an array of constants, data of the program, borrowed: copied,
            // never freed, D173, D193)
            Var(..) | Index(..) | Slice(..) | ConstArray(_) => {
                let moved = matches!(&body.kind, Var(_, Use::Move));
                let xv = self.ex(body, 1)?;
                let (p, n) = self.str_parts(&xv);
                check(self, &n);
                if let Some(d) = dest {
                    if owned && !moved {
                        // a view of strings: each one copied (D154)
                        self.for_range_scalar("0", &n, 1, &mut |g, j| {
                            let el = g.load_elem(&p, &n, j, et, 1);
                            let c = g.copy_value(&el, et);
                            g.store_elem(d, &n, j, et, &c, 1);
                            Ok(())
                        })?;
                    } else {
                        let bytes = self.ins(format!("mul i64 {}, {}", n, esize(et)));
                        self.emit(format!("call void @llvm.memcpy.p0.p0.i64(ptr {}, ptr {}, i64 {}, i1 false)", d, p, bytes));
                    }
                }
                // a variable moved here (a row computed by a `let` of the
                // function): copied, then freed (its block only when its
                // elements moved into the rows)
                if let Var(var, Use::Move) = &body.kind {
                    if owned && dest.is_some() {
                        self.emit(format!("call void @isc_free(ptr {})", p));
                    } else {
                        let t = self.var_ty(*var).clone();
                        self.drop_value(&xv, &t);
                    }
                }
                Ok(n)
            }
            // any other array: computed (new), then moved into the rows
            _ => {
                let xv = self.ex(body, 1)?;
                let (p, n) = self.str_parts(&xv);
                check(self, &n);
                match dest {
                    Some(d) => {
                        let bytes = self.ins(format!("mul i64 {}, {}", n, esize(et)));
                        self.emit(format!("call void @llvm.memcpy.p0.p0.i64(ptr {}, ptr {}, i64 {}, i1 false)", d, p, bytes));
                        self.emit(format!("call void @isc_free(ptr {})", p));
                    }
                    None => {
                        let t = body.ty.clone();
                        self.drop_value(&xv, &t);
                    }
                }
                Ok(n)
            }
        }
    }

    /// Stores the elements `start .. end` of a source into `p`.
    #[allow(clippy::too_many_arguments)]
    /// Elements `idx .. idx + w` of a source in vector code whose failed
    /// checks jump to a replay (D185): the elements computed again one at a
    /// time, in order, so that the panic is the one of the scalar loop (its
    /// element and its site); if none panics (a check of a lane that the
    /// scalar code would not have made), their values make the vector.
    fn elem_replayed(&mut self, src: &Src, idx: &str, w: u32) -> R<V> {
        if w == 1 || !src_checks(src) {
            return self.elem(src, idx, w);
        }
        let lfail = self.label();
        let lcont = self.label();
        let saved = std::mem::replace(&mut self.vec_fail, Some(lfail.clone()));
        let xv = self.elem(src, idx, w);
        self.vec_fail = saved;
        let xv = self.widen(&xv?, w);
        let lvec = self.current_label(&lfail);
        self.br(&lcont);
        self.block(&lfail);
        let slot = self.alloca(&xv.t);
        self.for_range_scalar("0", &w.to_string(), 1, &mut |g, j| {
            let k = g.ins(format!("add i64 {}, {}", idx, j));
            let x = g.elem(src, &k, 1)?;
            let cur = g.ins(format!("load {}, ptr {}", xv.t, slot));
            let nv = g.ins(format!("insertelement {} {}, {} {}, i64 {}", xv.t, cur, x.t, x.r, j));
            g.emit(format!("store {} {}, ptr {}", xv.t, nv, slot));
            Ok(())
        })?;
        let rv = self.ins(format!("load {}, ptr {}", xv.t, slot));
        let lrep = self.current_label(&lfail);
        self.br(&lcont);
        self.block(&lcont);
        let r = self.ins(format!("phi {} [ {}, %{} ], [ {}, %{} ]", xv.t, xv.r, lvec, rv, lrep));
        Ok(V { r, t: xv.t, uni: false })
    }

    fn fill_range(&mut self, src: &Src, p: &str, n: &str, et: &Type, start: &str, end: &str, w: u32) -> R<()> {
        // elements that own memory, lent by the source (the value of
        // `fill`, an array): each one copied (D154)
        fn lends(s: &Src) -> bool {
            match s {
                Src::Map(..) | Src::Iota => false,
                Src::Offset { inner, .. } => lends(inner),
                Src::Mat { .. } | Src::Uniform(_) | Src::Rows { .. } => true,
            }
        }
        if holds_owned(et) && lends(src) {
            return self.for_range(start, end, 1, &mut |g, i| {
                let x = g.elem(src, i, 1)?;
                let c = g.copy_value(&x, et);
                g.store_elem(p, n, i, et, &c, 1);
                Ok(())
            });
        }
        let mut tail_start = start.to_string();
        // arrays of bytes permuted in registers: the vector loop only when
        // each has at most 256 bytes, every element by the scalar loop
        // otherwise (the same values, D185)
        let tables = match src {
            Src::Map(_, l) if w > 1 && BYTE_LOOKUP.with(|b| b.get()) => lookup_tables(l),
            _ => Vec::new(),
        };
        let mut lscalar = None;
        if !tables.is_empty() {
            let mut all = "true".to_string();
            for t in &tables {
                let tv = self.env.get(t).cloned().ok_or("codegen: a table of bytes unbound")?;
                let tn = self.ins(format!("extractvalue {{ ptr, i64 }} {}, 1", tv.r));
                let small = self.ins(format!("icmp ule i64 {}, 256", tn));
                all = self.ins(format!("and i1 {}, {}", all, small));
            }
            let lvec = self.label();
            let lsc = self.label();
            self.cbr(&all, &lvec, &lsc);
            self.block(&lvec);
            lscalar = Some(lsc);
            // each table in registers once, before the loop (D198)
            for t in &tables {
                let tv = self.env.get(t).cloned().ok_or("codegen: a table of bytes unbound")?;
                let tp = self.ins(format!("extractvalue {{ ptr, i64 }} {}, 0", tv.r));
                let tn = self.ins(format!("extractvalue {{ ptr, i64 }} {}, 1", tv.r));
                let parts = self.table_registers(&tp, &tn);
                self.f.table_parts.insert(*t, parts);
            }
        }
        if let (true, Some((c, divs, rems))) = (w > 1, row_split(src, w)) {
            // by rows of the constant that divides the index (D214)
            self.fill_rows(src, p, n, et, start, end, w, c, &divs, &rems)?;
            for t in &tables {
                self.f.table_parts.remove(t);
            }
            tail_start = end.to_string();
        } else if w > 1 {
            let len = self.ins(format!("sub i64 {}, {}", end, start));
            let full = self.ins(format!("and i64 {}, -{}", len, w));
            let vend = self.ins(format!("add i64 {}, {}", start, full));
            let dmv = self.divmod_start_vec(src, start, w);
            self.for_range(start, &vend, w as i64, &mut |g, i| {
                let x = g.elem_replayed(src, i, w)?;
                g.store_elem(p, n, i, et, &x, w);
                g.divmod_step_vec(&dmv);
                Ok(())
            })?;
            let kept = self.divmod.len() - dmv.len();
            self.divmod.truncate(kept);
            for t in &tables {
                self.f.table_parts.remove(t);
            }
            tail_start = vend;
        }
        let ljoin = lscalar.as_ref().map(|_| self.label());
        let tail = match (&lscalar, &ljoin) {
            (Some(lsc), Some(lj)) => {
                // after the vector loop, its last elements; or all of them
                let lfrom = self.current_label(lsc);
                self.br(lj);
                self.block(lsc);
                self.br(lj);
                self.block(lj);
                self.ins(format!("phi i64 [ {}, %{} ], [ {}, %{} ]", tail_start, lfrom, start, lsc))
            }
            _ => tail_start,
        };
        let dm = self.divmod_start(src, &tail);
        self.for_range(&tail, end, 1, &mut |g, i| {
            let x = g.elem(src, i, 1)?;
            g.store_elem(p, n, i, et, &x, 1);
            g.divmod_step(&dm);
            Ok(())
        })?;
        let kept = self.divmod.len() - dm.len();
        self.divmod.truncate(kept);
        Ok(())
    }

    /// The elements `start .. end` of a `map` over an index `q` whose body
    /// divides it by the constant `c` (`q / c`, `q % c`, c >= w), by rows of
    /// `c` (D214): in a row, `q / c` is the row, the same for its lanes, and
    /// `q % c` the place in it, consecutive lanes; no division, and an index
    /// made of them is consecutive too (one load where a window, its bounds
    /// and a permutation were). Each row by vectors of `w` from its start,
    /// then one more ending at its end, over the one before (the elements
    /// of a `map` are values: computed twice, stored twice, the same); a row
    /// of fewer than `w` elements (the ends of `start .. end`) one at a time.
    #[allow(clippy::too_many_arguments)]
    fn fill_rows(
        &mut self,
        src: &Src,
        p: &str,
        n: &str,
        et: &Type,
        start: &str,
        end: &str,
        w: u32,
        c: i64,
        divs: &[*const Expr],
        rems: &[*const Expr],
    ) -> R<()> {
        self.decls.insert("declare i64 @llvm.smax.i64(i64, i64)".into());
        self.decls.insert("declare i64 @llvm.smin.i64(i64, i64)".into());
        let some = self.ins(format!("icmp slt i64 {}, {}", start, end));
        let (lrows, ldone) = (self.label(), self.label());
        self.cbr(&some, &lrows, &ldone);
        self.block(&lrows);
        let rlo = self.ins(format!("sdiv i64 {}, {}", start, c));
        let em1 = self.ins(format!("sub i64 {}, 1", end));
        let rhi = self.ins(format!("sdiv i64 {}, {}", em1, c));
        let rend = self.ins(format!("add i64 {}, 1", rhi));
        self.for_range(&rlo, &rend, 1, &mut |g, row| {
            let bq = g.ins(format!("mul i64 {}, {}", row, c));
            let a = g.ins(format!("sub i64 {}, {}", start, bq));
            let r_lo = g.ins(format!("call i64 @llvm.smax.i64(i64 {}, i64 0)", a));
            let b = g.ins(format!("sub i64 {}, {}", end, bq));
            let r_hi = g.ins(format!("call i64 @llvm.smin.i64(i64 {}, i64 {})", b, c));
            let len = g.ins(format!("sub i64 {}, {}", r_hi, r_lo));
            let wide = g.ins(format!("icmp sge i64 {}, {}", len, w));
            let (lvec, lsc, lj) = (g.label(), g.label(), g.label());
            g.cbr(&wide, &lvec, &lsc);
            g.block(&lvec);
            let rowv = V { r: row.to_string(), t: "i64".into(), uni: true };
            g.for_range(&r_lo, &r_hi, w as i64, &mut |g, r| g.row_vector(src, p, n, et, w, &rowv, &bq, r, divs, rems))?;
            let last = g.ins(format!("sub i64 {}, {}", r_hi, w));
            g.row_vector(src, p, n, et, w, &rowv, &bq, &last, divs, rems)?;
            g.br(&lj);
            g.block(&lsc);
            g.for_range(&r_lo, &r_hi, 1, &mut |g, r| {
                let idx = g.ins(format!("add i64 {}, {}", bq, r));
                for d in divs {
                    g.pre.insert(*d, rowv.clone());
                }
                for m in rems {
                    g.pre.insert(*m, V { r: r.to_string(), t: "i64".into(), uni: true });
                }
                let x = g.elem(src, &idx, 1);
                for e in divs.iter().chain(rems) {
                    g.pre.remove(e);
                }
                let x = x?;
                g.store_elem(p, n, &idx, et, &x, 1);
                Ok(())
            })?;
            g.br(&lj);
            g.block(&lj);
            Ok(())
        })?;
        self.br(&ldone);
        self.block(&ldone);
        Ok(())
    }

    /// One vector of a row of `fill_rows`: the elements `bq + r ..
    /// bq + r + w`, the row and the places in it given to the body.
    #[allow(clippy::too_many_arguments)]
    fn row_vector(
        &mut self,
        src: &Src,
        p: &str,
        n: &str,
        et: &Type,
        w: u32,
        row: &V,
        bq: &str,
        r: &str,
        divs: &[*const Expr],
        rems: &[*const Expr],
    ) -> R<()> {
        let idx = self.ins(format!("add i64 {}, {}", bq, r));
        let places = self.elem(&Src::Iota, r, w)?;
        for d in divs {
            self.pre.insert(*d, row.clone());
        }
        for m in rems {
            self.pre.insert(*m, places.clone());
        }
        let x = self.elem_replayed(src, &idx, w);
        for e in divs.iter().chain(rems) {
            self.pre.remove(e);
        }
        let x = x?;
        self.store_elem(p, n, &idx, et, &x, w);
        Ok(())
    }

    /// Before a loop over a `map` from index `start`, one element at a
    /// time: for each `x / c` and `x % c` of its body, `x` the index (a
    /// parameter bound to an `iota`) and `c` a positive integer literal,
    /// the quotient and the remainder of `start`, in two slots that each
    /// iteration moves on (`divmod_step`): an addition and a comparison
    /// instead of a division by a constant, about ten instructions (D180).
    fn divmod_start(&mut self, src: &Src, start: &str) -> Vec<DivMod> {
        let Src::Map(srcs, l) = src else { return Vec::new() };
        let vars: Vec<VarId> = srcs.iter().zip(&l.params).filter(|(s, _)| matches!(s, Src::Iota)).map(|(_, x)| *x).collect();
        let mut pairs = Vec::new();
        if !vars.is_empty() {
            index_divisions(&l.body, &vars, &mut pairs);
        }
        let mut dm = Vec::new();
        for (var, c) in pairs {
            let q = self.alloca("i64");
            let r = self.alloca("i64");
            let q0 = self.ins(format!("sdiv i64 {}, {}", start, c));
            let r0 = self.ins(format!("srem i64 {}, {}", start, c));
            self.emit(format!("store i64 {}, ptr {}", q0, q));
            self.emit(format!("store i64 {}, ptr {}", r0, r));
            dm.push(DivMod { var, c, q, r, w: 1 });
        }
        self.divmod.extend(dm.iter().cloned());
        dm
    }

    /// `divmod_start` for a vector loop of `w` lanes from `start` (D186): the
    /// quotients and remainders of the indices `start .. start + w`, each
    /// step adding `w % c` to the remainders and `w / c` to the quotients,
    /// one more where a remainder reaches `c` (`divmod_step_vec`).
    fn divmod_start_vec(&mut self, src: &Src, start: &str, w: u32) -> Vec<DivMod> {
        let Src::Map(srcs, l) = src else { return Vec::new() };
        let vars: Vec<VarId> = srcs.iter().zip(&l.params).filter(|(s, _)| matches!(s, Src::Iota)).map(|(_, x)| *x).collect();
        let mut pairs = Vec::new();
        if !vars.is_empty() {
            index_divisions(&l.body, &vars, &mut pairs);
        }
        let vt = format!("<{} x i64>", w);
        let mut dm = Vec::new();
        for (var, c) in pairs {
            let base = self.widen(&v(start, "i64"), w);
            let steps: Vec<String> = (0..w).map(|i| format!("i64 {}", i)).collect();
            let idx = self.ins(format!("add {} {}, <{}>", vt, base.r, steps.join(", ")));
            let cv = self.cst("i64", &c.to_string(), w);
            let q0 = self.ins(format!("sdiv {} {}, {}", vt, idx, cv));
            let r0 = self.ins(format!("srem {} {}, {}", vt, idx, cv));
            let q = self.alloca(&vt);
            let r = self.alloca(&vt);
            self.emit(format!("store {} {}, ptr {}", vt, q0, q));
            self.emit(format!("store {} {}, ptr {}", vt, r0, r));
            dm.push(DivMod { var, c, q, r, w });
        }
        self.divmod.extend(dm.iter().cloned());
        dm
    }

    fn divmod_step_vec(&mut self, dm: &[DivMod]) {
        for d in dm {
            let w = d.w;
            let vt = format!("<{} x i64>", w);
            let bt = format!("<{} x i1>", w);
            let (dq, dr) = ((w as i64) / d.c, (w as i64) % d.c);
            let r = self.ins(format!("load {}, ptr {}", vt, d.r));
            let q = self.ins(format!("load {}, ptr {}", vt, d.q));
            let r1 = self.ins(format!("add {} {}, {}", vt, r, self.cst("i64", &dr.to_string(), w)));
            let q1 = self.ins(format!("add {} {}, {}", vt, q, self.cst("i64", &dq.to_string(), w)));
            let cv = self.cst("i64", &d.c.to_string(), w);
            let wrap = self.ins(format!("icmp sge {} {}, {}", vt, r1, cv));
            let r2 = self.ins(format!("sub {} {}, {}", vt, r1, cv));
            let r3 = self.ins(format!("select {} {}, {} {}, {} {}", bt, wrap, vt, r2, vt, r1));
            let k = self.ins(format!("zext {} {} to {}", bt, wrap, vt));
            let q2 = self.ins(format!("add {} {}, {}", vt, q1, k));
            self.emit(format!("store {} {}, ptr {}", vt, r3, d.r));
            self.emit(format!("store {} {}, ptr {}", vt, q2, d.q));
        }
    }

    /// The next index: the remainder plus one, back to 0 at `c`, which
    /// adds one to the quotient.
    fn divmod_step(&mut self, dm: &[DivMod]) {
        for d in dm {
            let r = self.ins(format!("load i64, ptr {}", d.r));
            let r1 = self.ins(format!("add i64 {}, 1", r));
            let wrap = self.ins(format!("icmp eq i64 {}, {}", r1, d.c));
            let r2 = self.ins(format!("select i1 {}, i64 0, i64 {}", wrap, r1));
            self.emit(format!("store i64 {}, ptr {}", r2, d.r));
            let q = self.ins(format!("load i64, ptr {}", d.q));
            let k = self.ins(format!("zext i1 {} to i64", wrap));
            let q1 = self.ins(format!("add i64 {}, {}", q, k));
            self.emit(format!("store i64 {}, ptr {}", q1, d.q));
        }
    }

    /// The slot of `a / b` or `a % b` carried by the loop (`divmod_start`).
    fn divmod_slot(&self, op: BinOp, a: &Expr, b: &Expr, w: u32) -> Option<String> {
        let (ExprKind::Var(x, _), ExprKind::Lit(Lit::Int(c))) = (&a.kind, &b.kind) else { return None };
        let d = self.divmod.iter().find(|d| d.var == *x && d.c == *c && d.w == w)?;
        match op {
            BinOp::Div => Some(d.q.clone()),
            BinOp::Rem => Some(d.r.clone()),
            _ => None,
        }
    }

    /// Emits `define internal void @isc_task_N(ptr %ctx, i64 %i)`, whose body
    /// sees the free variables of `scope` (captured in a context struct, like
    /// the body of `bench`) and the `extra` values; returns the function, the
    /// context pointer and the size of the context in bytes, to give to
    /// `isc_parallel_for` (which copies a small context into the line the
    /// workers read the job from).
    fn outline_task(
        &mut self,
        scope: &[&Expr],
        extra: &[V],
        body: &mut dyn FnMut(&mut Self, &[V], &str) -> R<()>,
    ) -> R<(String, String, String)> {
        let mut caps = Vec::new();
        let mut bound = HashSet::new();
        for e in scope {
            free_vars(e, &mut bound, &mut caps);
        }
        let mut vals: Vec<V> = Vec::new();
        for c in &caps {
            vals.push(self.env.get(c).cloned().ok_or_else(|| format!("codegen: task capture %{} unbound", c))?);
        }
        vals.extend(extra.iter().cloned());
        let ctx_ty = if vals.is_empty() {
            "{ i1 }".to_string()
        } else {
            format!("{{ {} }}", vals.iter().map(|c| c.t.clone()).collect::<Vec<_>>().join(", "))
        };
        let ctxp = self.alloca(&ctx_ty);
        let end = self.ins(format!("getelementptr {}, ptr null, i32 1", ctx_ty));
        let ctx_bytes = self.ins(format!("ptrtoint ptr {} to i64", end));
        for (i, c) in vals.iter().enumerate() {
            let g = self.ins(format!("getelementptr inbounds {}, ptr {}, i32 0, i32 {}", ctx_ty, ctxp, i));
            self.emit(format!("store {} {}, ptr {}", c.t, c.r, g));
        }
        self.ntask += 1;
        let name = format!("@isc_task_{}", self.ntask);
        let saved_f = std::mem::take(&mut self.f);
        let saved_env = std::mem::take(&mut self.env);
        let saved_divmod = std::mem::take(&mut self.divmod);
        let saved_vec_fail = self.vec_fail.take();
        let saved_task = std::mem::replace(&mut self.in_task, true);
        // the regions of the caller are not the task's (D200)
        let saved_region = self.region.take();
        let saved_region_vars = std::mem::take(&mut self.region_vars);
        // the recursive types the task may share with other threads (D175)
        let mut shared = HashSet::new();
        for c in &caps {
            let t = self.p.vars[*c as usize].ty.clone();
            self.rec_types_in(&t, &mut shared);
        }
        for e in scope {
            self.rec_types_in(&e.ty, &mut shared);
        }
        let saved_shared = std::mem::replace(&mut self.task_shared, Some(shared));
        let mut loaded = Vec::new();
        for (i, c) in vals.iter().enumerate() {
            let g = self.ins(format!("getelementptr inbounds {}, ptr %ctx, i32 0, i32 {}", ctx_ty, i));
            let x = self.ins(format!("load {}, ptr {}", c.t, g));
            if i < caps.len() {
                self.env.insert(caps[i], v(x, c.t.clone()));
            } else {
                loaded.push(v(x, c.t.clone()));
            }
        }
        let wide = std::mem::take(&mut self.wide_task);
        let r = body(self, &loaded, "%i");
        self.emit("ret void");
        let attrs = if wide && self.opts.isa == Isa::Native && self.opts.isa.vector_registers().0 == 32 && self.opts.isa.vector_registers().1 == 32 {
            // AVX-512 on the host: 512-bit vectors for this task only
            self.wide_used = true;
            " #1"
        } else {
            self.attrs()
        };
        let text = self.finish(&format!("define internal void {}(ptr %ctx, i64 %i){}", name, attrs));
        self.f = saved_f;
        self.env = saved_env;
        self.divmod = saved_divmod;
        self.vec_fail = saved_vec_fail;
        self.in_task = saved_task;
        self.task_shared = saved_shared;
        self.region = saved_region;
        self.region_vars = saved_region_vars;
        r?;
        self.funcs.push(text);
        Ok((name, ctxp, ctx_bytes))
    }

    /// Applies a two-parameter lambda at width `w`.
    fn apply2(&mut self, l: &Lambda, a: V, b: V, w: u32) -> R<V> {
        self.bind(l.params[0], a);
        self.bind(l.params[1], b);
        let r = self.ex(&l.body, w)?;
        Ok(self.widen(&r, w))
    }

    /// `reduce_block` of section 10 over elements `start .. start + m`.
    fn reduce_block(&mut self, src: &Src, start: &str, m: &str, l: &Lambda, z: &V, t: &Type, w: u32) -> R<V> {
        let st = lt(t);
        if w > 1 {
            let vty = format!("<{} x {}>", w, st);
            let acc = self.alloca(&vty);
            let zw = self.widen(z, w);
            self.emit(format!("store {} {}, ptr {}", vty, zw.r, acc));
            let mfull = self.ins(format!("and i64 {}, -{}", m, w));
            let passes = reduce_passes(w, esize(t) as u32, src.loads().max(1), self.vregs);
            if passes == 1 {
                let row = |g: &mut Self, i: &str| -> R<()> {
                    let idx = g.ins(format!("add i64 {}, {}", start, i));
                    let x = g.elem_replayed(src, &idx, w)?;
                    let x = g.widen(&x, w);
                    let a = g.ins(format!("load {}, ptr {}", vty, acc));
                    let r = g.apply2(l, V { r: a, t: vty.clone(), uni: false }, x, w)?;
                    g.emit(format!("store {} {}, ptr {}", vty, r.r, acc));
                    Ok(())
                };
                // An array read from memory (`stream`, see STREAM_BYTES): four
                // rows per iteration, then the rows left one at a time; each
                // accumulator still receives its rows in order. Unrolled, the
                // loop is faster from memory and slower from the caches
                // (DECISIONS D86).
                // One unrolled loop (no iteration when the array is not
                // streamed), then one loop of single rows for the rest: two
                // loops, not three, for LLVM to compile (compilation time).
                let mut rows_done = "0".to_string();
                if let Some(s) = self.stream.clone() {
                    let group = w as i64 * STREAM_UNROLL;
                    let mg = self.ins(format!("and i64 {}, -{}", m, group));
                    let mgroups = self.ins(format!("select i1 {}, i64 {}, i64 0", s, mg));
                    self.for_range("0", &mgroups, group, &mut |g, i| {
                        for k in 0..STREAM_UNROLL {
                            let ik = g.ins(format!("add i64 {}, {}", i, k * w as i64));
                            row(g, &ik)?;
                        }
                        Ok(())
                    })?;
                    rows_done = mgroups;
                }
                self.for_range(&rows_done, &mfull, w as i64, &mut |g, i| row(g, i))?;
            } else {
                // more accumulators than registers: one pass per group of
                // pw accumulators, each over every row of the block
                let pw = w / passes;
                let pty = format!("<{} x {}>", pw, st);
                // The rows are taken in chunks of `chunk` elements that stay
                // in L1; every pass runs over the chunk before the next chunk,
                // so a cache line fetched for one pass (or by the next-line
                // prefetcher) is still there for the others. Between chunks,
                // the accumulators of each pass live in memory, through
                // volatile accesses: kept in registers across the other
                // passes, they would make LLVM spill accumulators in the loop.
                let gbuf = self.alloca(&vty);
                self.emit(format!("store volatile {} {}, ptr {}", vty, zw.r, gbuf));
                let chunk = reduce_chunk(w, esize(t) as u32, src.loads().max(1));
                let nch = self.ins(format!("add i64 {}, {}", mfull, chunk - 1));
                let nch = self.ins(format!("sdiv i64 {}, {}", nch, chunk));
                self.for_range("0", &nch, 1, &mut |g, c| {
                    let lo = g.ins(format!("mul i64 {}, {}", c, chunk));
                    let hi = g.ins(format!("add i64 {}, {}", lo, chunk));
                    let short = g.ins(format!("icmp slt i64 {}, {}", mfull, hi));
                    let hi = g.ins(format!("select i1 {}, i64 {}, i64 {}", short, mfull, hi));
                    for p in 0..passes {
                        let gp = g.ins(format!("getelementptr inbounds {}, ptr {}, i64 {}", st, gbuf, p * pw));
                        let a0 = g.ins(format!("load volatile {}, ptr {}", pty, gp));
                        let pacc = g.alloca(&pty);
                        g.emit(format!("store {} {}, ptr {}", pty, a0, pacc));
                        g.for_range(&lo, &hi, w as i64, &mut |g, i| {
                            let row = g.ins(format!("add i64 {}, {}", start, i));
                            let idx = g.ins(format!("add i64 {}, {}", row, p * pw));
                            let x = g.elem_replayed(src, &idx, pw)?;
                            let x = g.widen(&x, pw);
                            let a = g.ins(format!("load {}, ptr {}", pty, pacc));
                            let r = g.apply2(l, V { r: a, t: pty.clone(), uni: false }, x, pw)?;
                            g.emit(format!("store {} {}, ptr {}", pty, r.r, pacc));
                            Ok(())
                        })?;
                        let res = g.ins(format!("load {}, ptr {}", pty, pacc));
                        g.emit(format!("store volatile {} {}, ptr {}", pty, res, gp));
                    }
                    Ok(())
                })?;
                let all = self.ins(format!("load volatile {}, ptr {}", vty, gbuf));
                self.emit(format!("store {} {}, ptr {}", vty, all, acc));
            }
            // the elements left (fewer than w): each into its lane, through
            // an array of scalars. An `extractelement` / `insertelement` at a
            // variable lane made LLVM store and reload every accumulator
            // register for each element; the upper levels of a reduction of
            // a few blocks are only such elements (DECISIONS D86).
            // (a vector of `i1` is packed in memory, not laid out like the
            // array: lane by lane in the register for it)
            let old_tail = st == "i1";
            if old_tail {
                self.for_range(&mfull, m, 1, &mut |g, i| {
                    let idx = g.ins(format!("add i64 {}, {}", start, i));
                    let x = g.elem(src, &idx, 1)?;
                    let lane = g.ins(format!("sub i64 {}, {}", i, mfull));
                    let av = g.ins(format!("load {}, ptr {}", vty, acc));
                    let a = g.ins(format!("extractelement {} {}, i64 {}", vty, av, lane));
                    let r = g.apply2(l, V { r: a, t: st.clone(), uni: false }, x, 1)?;
                    let av2 = g.ins(format!("insertelement {} {}, {} {}, i64 {}", vty, av, st, r.r, lane));
                    g.emit(format!("store {} {}, ptr {}", vty, av2, acc));
                    Ok(())
                })?;
            }
            let rest = if old_tail { "false".to_string() } else { self.ins(format!("icmp slt i64 {}, {}", mfull, m)) };
            let ltail = self.label();
            let ldone = self.label();
            self.cbr(&rest, &ltail, &ldone);
            self.block(&ltail);
            // a slot of the vector type, so that it has the alignment the
            // store of the vector assumes (an array of scalars is aligned to
            // one scalar only: the aligned store faulted)
            let lanes_buf = self.alloca(&vty);
            let av = self.ins(format!("load {}, ptr {}", vty, acc));
            self.emit(format!("store {} {}, ptr {}", vty, av, lanes_buf));
            self.for_range_scalar(&mfull, m, 1, &mut |g, i| {
                let idx = g.ins(format!("add i64 {}, {}", start, i));
                let x = g.elem(src, &idx, 1)?;
                let lane = g.ins(format!("sub i64 {}, {}", i, mfull));
                let pl = g.ins(format!("getelementptr inbounds {}, ptr {}, i64 {}", st, lanes_buf, lane));
                let a = g.ins(format!("load {}, ptr {}", st, pl));
                let r = g.apply2(l, V { r: a, t: st.clone(), uni: false }, x, 1)?;
                g.emit(format!("store {} {}, ptr {}", st, r.r, pl));
                Ok(())
            })?;
            let av2 = self.ins(format!("load {}, ptr {}", vty, lanes_buf));
            self.emit(format!("store {} {}, ptr {}", vty, av2, acc));
            self.br(&ldone);
            self.block(&ldone);
            // tree: acc[j] = op(acc[j], acc[j + d]) for d = 8, 4, 2, 1
            let mut cur = V { r: self.ins(format!("load {}, ptr {}", vty, acc)), t: vty.clone(), uni: false };
            let mut d = w / 2;
            while d >= 1 {
                let (lo, hi) = if d == 1 {
                    let a = self.ins(format!("extractelement {} {}, i64 0", cur.t, cur.r));
                    let b = self.ins(format!("extractelement {} {}, i64 1", cur.t, cur.r));
                    (V { r: a, t: st.clone(), uni: false }, V { r: b, t: st.clone(), uni: false })
                } else {
                    let lo_idx: Vec<String> = (0..d).map(|i| format!("i32 {}", i)).collect();
                    let hi_idx: Vec<String> = (d..2 * d).map(|i| format!("i32 {}", i)).collect();
                    let ht = format!("<{} x {}>", d, st);
                    let a = self.ins(format!("shufflevector {} {}, {} poison, <{} x i32> <{}>", cur.t, cur.r, cur.t, d, lo_idx.join(", ")));
                    let b = self.ins(format!("shufflevector {} {}, {} poison, <{} x i32> <{}>", cur.t, cur.r, cur.t, d, hi_idx.join(", ")));
                    (V { r: a, t: ht.clone(), uni: false }, V { r: b, t: ht, uni: false })
                };
                cur = self.apply2(l, lo, hi, d)?;
                d /= 2;
            }
            cur.uni = true;
            return Ok(cur);
        }
        // scalar version: V accumulators in memory
        let lanes = self.opts.lanes;
        let aty = format!("[{} x {}]", lanes, st);
        let acc = self.alloca(&aty);
        for j in 0..lanes {
            let g = self.ins(format!("getelementptr inbounds {}, ptr {}, i64 0, i64 {}", aty, acc, j));
            self.emit(format!("store {} {}, ptr {}", st, z.r, g));
        }
        self.for_range("0", m, 1, &mut |g, i| {
            let idx = g.ins(format!("add i64 {}, {}", start, i));
            let x = g.elem(src, &idx, 1)?;
            let lane = g.ins(format!("and i64 {}, {}", i, lanes - 1));
            let pa = g.ins(format!("getelementptr inbounds {}, ptr {}, i64 0, i64 {}", aty, acc, lane));
            let a = g.ins(format!("load {}, ptr {}", st, pa));
            let r = g.apply2(l, V { r: a, t: st.clone(), uni: false }, x, 1)?;
            g.emit(format!("store {} {}, ptr {}", st, r.r, pa));
            Ok(())
        })?;
        let mut d = lanes / 2;
        while d >= 1 {
            for j in 0..d {
                let pj = self.ins(format!("getelementptr inbounds {}, ptr {}, i64 0, i64 {}", aty, acc, j));
                let pk = self.ins(format!("getelementptr inbounds {}, ptr {}, i64 0, i64 {}", aty, acc, j + d));
                let a = self.ins(format!("load {}, ptr {}", st, pj));
                let b = self.ins(format!("load {}, ptr {}", st, pk));
                let r = self.apply2(l, V { r: a, t: st.clone(), uni: false }, V { r: b, t: st.clone(), uni: false }, 1)?;
                self.emit(format!("store {} {}, ptr {}", st, r.r, pj));
            }
            d /= 2;
        }
        let p0 = self.ins(format!("getelementptr inbounds {}, ptr {}, i64 0, i64 0", aty, acc));
        let r = self.ins(format!("load {}, ptr {}", st, p0));
        Ok(v(r, st))
    }

    /// `reduce` of section 10: blocks of B elements, then the block results
    /// are reduced the same way until one value remains.
    fn reduce(&mut self, e: &Expr, x: &Expr, l: &Lambda, z: &Expr) -> R<V> {
        // scalars are reduced with vectors; tuples one element at a time
        let t = x.ty.elem().clone();
        let zv = self.ex(z, 1)?;
        let lanes = self.opts.lanes;
        // read by `reduce_block`, whose vector loops replay a failed check
        // (D185)
        let w = if src_vec_ok_replay(x) && lam_ok(l) { lanes } else { 1 };
        let wm = if lam_ok(l) { lanes } else { 1 };
        let (src, n) = self.prepare_src(x)?;
        // An exact operator whose `z` is its identity, or `min` and `max`
        // (D183): the result depends neither on the order of the elements
        // nor on the lanes starting from `z` (the fixed form adds a `z` that
        // is not 0 in each of them, docs/language.md), so that the fixed
        // form is not needed for the bits. Fewer elements than lanes: one
        // loop instead of the blocks,
        // the lanes and their tree, whose setup cost a hundred instructions
        // per call (`count` over 4 probabilities per byte in fasta). More:
        // the fixed form (within a block of B, LLVM removes checks of
        // bounds that it keeps in one loop over the whole range: k-nucleotide
        // 12 % slower with one loop).
        let exact = isc_ir::passes::order_free(l, z) && t.is_scalar();
        // the elements are loops (calls, reductions): shared by chunks, in
        // a task too (D201)
        if exact && matches!(&x.kind, ExprKind::Map(_, ml) if inner_loop(&ml.body)) {
            return self.reduce_heavy(e, x, l, z, &zv, &n, &t, w);
        }
        let exact_res = if exact {
            let limit = lanes as i64 - 1;
            let slot = self.alloca(&lt(&t));
            let small = self.ins(format!("icmp sle i64 {}, {}", n, limit));
            let lloop = self.label();
            let lfixed = self.label();
            let ljoin = self.label();
            self.cbr(&small, &lloop, &lfixed);
            self.block(&lloop);
            let r = self.reduce_loop(&src, &n, l, &zv, &t)?;
            self.emit(format!("store {} {}, ptr {}", lt(&t), r.r, slot));
            self.br(&ljoin);
            self.block(&lfixed);
            Some((slot, ljoin))
        } else {
            None
        };
        let m = mt(&t);
        let big = self.ins(format!("icmp sgt i64 {}, {}", n, BLOCK));
        let n1 = self.ins(format!("add i64 {}, {}", n, BLOCK - 1));
        let cdiv = self.ins(format!("sdiv i64 {}, {}", n1, BLOCK));
        let c0 = self.ins(format!("select i1 {}, i64 {}, i64 1", big, cdiv));
        // the block results on the stack when they fit: no allocation and
        // no free per call (DECISIONS D86)
        let sp = self.ins("call ptr @llvm.stacksave.p0()".to_string());
        let parts_heap = self.alloca("i1");
        self.emit(format!("store i1 false, ptr {}", parts_heap));
        // one element: a column of 64 bytes per leaf for a tuple
        let slot = if matches!(t, Type::Tuple(_)) { self.alloca(&format!("[{} x i64]", 8 * leaves(&t).len())) } else { self.alloca(&m) };
        let pa = self.alloca("ptr");
        self.emit(format!("store ptr {}, ptr {}", slot, pa));
        let lalloc = self.label();
        let ljoin = self.label();
        self.cbr(&big, &lalloc, &ljoin);
        self.block(&lalloc);
        let hp = self.alloc_scratch(&c0, &t, &parts_heap);
        self.emit(format!("store ptr {}, ptr {}", hp, pa));
        self.br(&ljoin);
        self.block(&ljoin);
        let parts = self.ins(format!("load ptr, ptr {}", pa));
        let bl = BLOCK.to_string();
        if self.in_task {
            let stream = if matches!(src, Src::Mat { .. }) { Some(self.stream_flag(&n, &t)) } else { None };
            self.for_range("0", &c0, 1, &mut |g, j| {
                let start = g.ins(format!("mul i64 {}, {}", j, BLOCK));
                let rem = g.ins(format!("sub i64 {}, {}", n, start));
                let lt_ = g.ins(format!("icmp slt i64 {}, {}", rem, bl));
                let mm = g.ins(format!("select i1 {}, i64 {}, i64 {}", lt_, rem, bl));
                g.stream = stream.clone();
                let r = g.reduce_block(&src, &start, &mm, l, &zv, &t, w);
                g.stream = None;
                let r = r?;
                g.store_elem(&parts, &c0, j, &t, &r, 1);
                Ok(())
            })?;
        } else {
            // the first level in parallel: block j is computed by task j and
            // stored in parts[j], so the result is the same for any number of
            // threads (SPEC section 10)
            // Few task results (at most PAD_BLOCKS): each in its own 128
            // bytes (an aligned pair of cache lines), then copied into
            // `parts`. Side by side they would share a cache line that the
            // cores take from each other when the tasks end, all at once
            // (false sharing): 32 blocks of `dot` with 8 threads, 2596.7 ns
            // side by side, 2486.0 ns apart; beyond, the copy costs more
            // than it saves (64 blocks: 18101.6 against 18269.8 ns;
            // DECISIONS D81).
            let min_leaf = leaves(&t).iter().map(|(_, ty)| esize(ty)).min().unwrap_or(8).max(1);
            let cell = (128 / min_leaf).max(1);
            let few = self.ins(format!("icmp sle i64 {}, {}", c0, PAD_BLOCKS));
            let stride = self.ins(format!("select i1 {}, i64 {}, i64 1", few, cell));
            let cdst = self.ins(format!("mul i64 {}, {}", c0, stride));
            let pd = self.alloca("ptr");
            self.emit(format!("store ptr {}, ptr {}", parts, pd));
            let lpad = self.label();
            let lgo = self.label();
            self.cbr(&few, &lpad, &lgo);
            self.block(&lpad);
            // (on the heap: on the stack they cost more compilation time than
            // they saved, DECISIONS D87)
            let padded = self.alloc_elems(&cdst, &t);
            self.emit(format!("store ptr {}, ptr {}", padded, pd));
            self.br(&lgo);
            self.block(&lgo);
            let dst = self.ins(format!("load ptr, ptr {}", pd));
            let extra = [v(dst.clone(), "ptr"), v(cdst.clone(), "i64"), v(stride.clone(), "i64")];
            let (task, ctx, ctx_bytes) = self.outline_task(&[e], &extra, &mut |g, extra, j| {
                let (src, n) = g.prepare_src(x)?;
                let zv = g.ex(z, 1)?;
                let start = g.ins(format!("mul i64 {}, {}", j, BLOCK));
                let rem = g.ins(format!("sub i64 {}, {}", n, start));
                let lt_ = g.ins(format!("icmp slt i64 {}, {}", rem, bl));
                let mm = g.ins(format!("select i1 {}, i64 {}, i64 {}", lt_, rem, bl));
                // only a materialized array: with an element-wise map in the
                // loop, four rows per iteration were not faster (large `dot`,
                // `std_dev`), and each unrolled loop is code to compile
                g.stream = if matches!(src, Src::Mat { .. }) { Some(g.stream_flag(&n, &t)) } else { None };
                let r = g.reduce_block(&src, &start, &mm, l, &zv, &t, w);
                g.stream = None;
                let r = r?;
                let jj = g.ins(format!("mul i64 {}, {}", j, extra[2].r));
                g.store_elem(&extra[0].r, &extra[1].r, &jj, &t, &r, 1);
                Ok(())
            })?;
            self.emit(format!("call void @isc_parallel_for(i64 {}, ptr {}, ptr {}, i64 {})", c0, task, ctx, ctx_bytes));
            let lcopy = self.label();
            let ldone = self.label();
            self.cbr(&few, &lcopy, &ldone);
            self.block(&lcopy);
            self.for_range("0", &c0, 1, &mut |g, j| {
                let jj = g.ins(format!("mul i64 {}, {}", j, cell));
                let r = g.load_elem(&dst, &cdst, &jj, &t, 1);
                g.store_elem(&parts, &c0, j, &t, &r, 1);
                Ok(())
            })?;
            self.emit(format!("call void @isc_free(ptr {})", dst));
            self.br(&ldone);
            self.block(&ldone);
        }
        // upper levels, in place in `parts`
        let ca = self.alloca("i64");
        self.emit(format!("store i64 {}, ptr {}", c0, ca));
        let head = self.label();
        let body = self.label();
        let done = self.label();
        self.br(&head);
        self.block(&head);
        let c = self.ins(format!("load i64, ptr {}", ca));
        let more = self.ins(format!("icmp sgt i64 {}, 1", c));
        self.cbr(&more, &body, &done);
        self.block(&body);
        let c1 = self.ins(format!("add i64 {}, {}", c, BLOCK - 1));
        let c2 = self.ins(format!("sdiv i64 {}, {}", c1, BLOCK));
        let msrc = Src::Mat { ptr: parts.clone(), len: c0.clone(), elem: t.clone() };
        self.for_range("0", &c2, 1, &mut |g, k| {
            let start = g.ins(format!("mul i64 {}, {}", k, BLOCK));
            let rem = g.ins(format!("sub i64 {}, {}", c, start));
            let lt_ = g.ins(format!("icmp slt i64 {}, {}", rem, bl));
            let mm = g.ins(format!("select i1 {}, i64 {}, i64 {}", lt_, rem, bl));
            let r = g.reduce_block(&msrc, &start, &mm, l, &zv, &t, wm)?;
            g.store_elem(&parts, &c0, k, &t, &r, 1);
            Ok(())
        })?;
        self.emit(format!("store i64 {}, ptr {}", c2, ca));
        self.br(&head);
        self.block(&done);
        let res = self.load_elem(&parts, &c0, "0", &t, 1);
        // `parts_heap` is false unless `parts` came from the heap
        self.free_scratch(&parts, &parts_heap);
        self.emit(format!("call void @llvm.stackrestore.p0(ptr {})", sp));
        if let Some((slot, ljoin)) = exact_res {
            self.emit(format!("store {} {}, ptr {}", lt(&t), res.r, slot));
            self.br(&ljoin);
            self.block(&ljoin);
            let r = self.ins(format!("load {}, ptr {}", lt(&t), slot));
            return Ok(V { r, t: lt(&t), uni: true });
        }
        Ok(V { r: res.r, t: res.t, uni: true })
    }

    /// The `reduce` of an exact operator in one loop, from `z` (D183).
    /// An exact `reduce` (D183) of a `map` whose elements are loops (D201):
    /// any grouping gives the same bits, so HEAVY_CHUNKS chunks per thread
    /// taken in turn (`isc_parallel_for_dynamic`, in a task too), each
    /// chunk's result in its own 128 bytes (D81), then reduced here in order.
    /// The blocks of B elements of the fixed form would compute 32 calls of
    /// binary-trees on one thread.
    #[allow(clippy::too_many_arguments)]
    fn reduce_heavy(&mut self, e: &Expr, x: &Expr, l: &Lambda, z: &Expr, zv: &V, n: &str, t: &Type, w: u32) -> R<V> {
        self.decls.insert("declare i64 @llvm.smax.i64(i64, i64)".into());
        self.decls.insert("declare i64 @llvm.smin.i64(i64, i64)".into());
        let t0 = self.ins("call i64 @isc_thread_count()".to_string());
        let tc = self.ins(format!("mul i64 {}, {}", t0, HEAVY_CHUNKS));
        let k0 = self.ins(format!("call i64 @llvm.smin.i64(i64 {}, i64 {})", n, tc));
        let k = self.ins(format!("call i64 @llvm.smax.i64(i64 {}, i64 1)", k0));
        let b0 = self.ins(format!("add i64 {}, {}", n, k));
        let b1 = self.ins(format!("sub i64 {}, 1", b0));
        let b2 = self.ins(format!("sdiv i64 {}, {}", b1, k));
        let bs = self.ins(format!("call i64 @llvm.smax.i64(i64 {}, i64 1)", b2));
        let c0 = self.ins(format!("add i64 {}, {}", n, bs));
        let c1 = self.ins(format!("sub i64 {}, 1", c0));
        let c2 = self.ins(format!("sdiv i64 {}, {}", c1, bs));
        let c = self.ins(format!("call i64 @llvm.smax.i64(i64 {}, i64 1)", c2));
        let cell = (128 / esize(t).max(1)).max(1);
        let cn = self.ins(format!("mul i64 {}, {}", c, cell));
        let dst = self.alloc_elems(&cn, t);
        let extra = [v(dst.clone(), "ptr"), v(cn.clone(), "i64"), v(bs.clone(), "i64")];
        let et = t.clone();
        let (task, ctx, ctx_bytes) = self.outline_task(&[e], &extra, &mut |g, extra, j| {
            let (src, n) = g.prepare_src(x)?;
            let zv = g.ex(z, 1)?;
            let start = g.ins(format!("mul i64 {}, {}", j, extra[2].r));
            let lim = g.ins(format!("add i64 {}, {}", start, extra[2].r));
            let end = g.ins(format!("call i64 @llvm.smin.i64(i64 {}, i64 {})", n, lim));
            // by blocks of B, in vector code like the fixed form
            let st = lt(&et);
            let acc = g.alloca(&st);
            g.emit(format!("store {} {}, ptr {}", st, zv.r, acc));
            // (`for_range` with a step takes whole steps only: the blocks by
            // their number)
            let len = g.ins(format!("sub i64 {}, {}", end, start));
            let nb0 = g.ins(format!("add i64 {}, {}", len, BLOCK - 1));
            let nb = g.ins(format!("sdiv i64 {}, {}", nb0, BLOCK));
            g.for_range("0", &nb, 1, &mut |g, q| {
                let off = g.ins(format!("mul i64 {}, {}", q, BLOCK));
                let b = &g.ins(format!("add i64 {}, {}", start, off));
                let lim = g.ins(format!("add i64 {}, {}", b, BLOCK));
                let bend = g.ins(format!("call i64 @llvm.smin.i64(i64 {}, i64 {})", end, lim));
                let m = g.ins(format!("sub i64 {}, {}", bend, b));
                let el = g.reduce_block(&src, b, &m, l, &zv, &et, w)?;
                let a = g.ins(format!("load {}, ptr {}", st, acc));
                let r = g.apply2(l, V { r: a, t: st.clone(), uni: false }, el, 1)?;
                g.emit(format!("store {} {}, ptr {}", st, r.r, acc));
                Ok(())
            })?;
            let r = g.ins(format!("load {}, ptr {}", st, acc));
            let jj = g.ins(format!("mul i64 {}, {}", j, cell));
            g.store_elem(&extra[0].r, &extra[1].r, &jj, &et, &V { r, t: st, uni: true }, 1);
            Ok(())
        })?;
        self.emit(format!("call void @isc_parallel_for_dynamic(i64 {}, ptr {}, ptr {}, i64 {})", c, task, ctx, ctx_bytes));
        let st = lt(t);
        let acc = self.alloca(&st);
        self.emit(format!("store {} {}, ptr {}", st, zv.r, acc));
        self.for_range("0", &c, 1, &mut |g, j| {
            let jj = g.ins(format!("mul i64 {}, {}", j, cell));
            let el = g.load_elem(&dst, &cn, &jj, t, 1);
            let a = g.ins(format!("load {}, ptr {}", st, acc));
            let r = g.apply2(l, V { r: a, t: st.clone(), uni: false }, el, 1)?;
            g.emit(format!("store {} {}, ptr {}", st, r.r, acc));
            Ok(())
        })?;
        self.emit(format!("call void @isc_free(ptr {})", dst));
        let r = self.ins(format!("load {}, ptr {}", st, acc));
        Ok(V { r, t: st, uni: true })
    }

    fn reduce_loop(&mut self, src: &Src, n: &str, l: &Lambda, z: &V, t: &Type) -> R<V> {
        let st = lt(t);
        let acc = self.alloca(&st);
        self.emit(format!("store {} {}, ptr {}", st, z.r, acc));
        self.for_range("0", n, 1, &mut |g, i| {
            let el = g.elem(src, i, 1)?;
            let a = g.ins(format!("load {}, ptr {}", st, acc));
            let r = g.apply2(l, V { r: a, t: st.clone(), uni: false }, el, 1)?;
            g.emit(format!("store {} {}, ptr {}", st, r.r, acc));
            Ok(())
        })?;
        let r = self.ins(format!("load {}, ptr {}", st, acc));
        Ok(V { r, t: st, uni: true })
    }

    /// `scan` (SPEC section 6): from left to right into a new array; its
    /// source fused like the source of a `fold` (F1). With a predefined
    /// associative operator on integers or booleans and more than one block,
    /// in parallel (`scan_parallel`); otherwise sequential, as SPEC requires
    /// for floats (section 10).
    /// The array read by a `reduce` or a `scan` of values that own memory
    /// (D134), and whether it is a new one, to free after: a variable is
    /// borrowed; a `filter` (a stream) is evaluated into a new array.
    fn owned_src(&mut self, x: &Expr) -> R<(V, String, String, bool)> {
        let fresh = !matches!(x.kind, ExprKind::Var(..) | ExprKind::Proj(..) | ExprKind::Slice(..) | ExprKind::Index(..));
        let a = if isc_ir::passes::is_stream(x) { self.materialize_stream(x)? } else { self.ex(x, 1)? };
        let p = self.ins(format!("extractvalue {{ ptr, i64 }} {}, 0", a.r));
        let n = self.ins(format!("extractvalue {{ ptr, i64 }} {}, 1", a.r));
        Ok((a, p, n, fresh))
    }

    /// One block of the fixed form of `reduce` (section 10) for values that
    /// own memory (D134): V lanes, each from a copy of `z`, element `i` into
    /// lane `i mod V`, then the tree; every value replaced is freed, the
    /// lanes other than the first at the end.
    #[allow(clippy::too_many_arguments)]
    fn reduce_block_owned(&mut self, p: &str, n: &str, lo: &str, hi: &str, l: &Lambda, zv: &V, t: &Type) -> R<V> {
        let lanes = self.opts.lanes as i64;
        let ty = lt(t);
        let accs = self.alloca(&format!("[{} x {}]", lanes, ty));
        let lane = |g: &mut Self, j: &str| g.ins(format!("getelementptr inbounds [{} x {}], ptr {}, i64 0, i64 {}", lanes, ty, accs, j));
        self.for_range_scalar("0", &lanes.to_string(), 1, &mut |g, j| {
            let c = g.copy_value(zv, t);
            let a = lane(g, j);
            g.emit(format!("store {} {}, ptr {}", ty, c.r, a));
            Ok(())
        })?;
        self.for_range_scalar(lo, hi, 1, &mut |g, i| {
            let off = g.ins(format!("sub i64 {}, {}", i, lo));
            let j = g.ins(format!("urem i64 {}, {}", off, lanes));
            let slot = lane(g, &j);
            let a = v(g.ins(format!("load {}, ptr {}", ty, slot)), ty.clone());
            let x = g.load_elem(p, n, i, t, 1);
            g.bind(l.params[0], a.clone());
            g.bind(l.params[1], x);
            let r = g.ex(&l.body, 1)?;
            g.drop_value(&a, t);
            g.emit(format!("store {} {}, ptr {}", ty, r.r, slot));
            Ok(())
        })?;
        let mut d = lanes / 2;
        while d >= 1 {
            let dd = d;
            self.for_range_scalar("0", &dd.to_string(), 1, &mut |g, j| {
                let sa = lane(g, j);
                let k = g.ins(format!("add i64 {}, {}", j, dd));
                let sb = lane(g, &k);
                let a = v(g.ins(format!("load {}, ptr {}", ty, sa)), ty.clone());
                let b = v(g.ins(format!("load {}, ptr {}", ty, sb)), ty.clone());
                g.bind(l.params[0], a.clone());
                g.bind(l.params[1], b);
                let r = g.ex(&l.body, 1)?;
                g.drop_value(&a, t);
                g.emit(format!("store {} {}, ptr {}", ty, r.r, sa));
                Ok(())
            })?;
            d /= 2;
        }
        self.for_range_scalar("1", &lanes.to_string(), 1, &mut |g, j| {
            let s = lane(g, j);
            let a = v(g.ins(format!("load {}, ptr {}", ty, s)), ty.clone());
            g.drop_value(&a, t);
            Ok(())
        })?;
        let s0 = lane(self, "0");
        Ok(v(self.ins(format!("load {}, ptr {}", ty, s0)), ty))
    }

    /// `reduce` of values that own memory (D134), the fixed form of section
    /// 10 as in the interpreter: one block when there are at most B
    /// elements, otherwise the results of the blocks of B reduced in the
    /// same way, level by level; each level frees the one before.
    fn reduce_owned(&mut self, x: &Expr, l: &Lambda, z: &Expr) -> R<V> {
        let t = l.body.ty.clone();
        let at = Type::Array(Box::new(t.clone()), isc_ir::Fixed::ANY);
        let zv = self.ex(z, 1)?;
        let (av, p, n, fresh) = self.owned_src(x)?;
        let curp = self.alloca("ptr");
        let curn = self.alloca("i64");
        let owned = self.alloca("i1");
        self.emit(format!("store ptr {}, ptr {}", p, curp));
        self.emit(format!("store i64 {}, ptr {}", n, curn));
        self.emit(format!("store i1 false, ptr {}", owned));
        let head = self.label();
        let body = self.label();
        let done = self.label();
        self.br(&head);
        self.block(&head);
        let cp = self.ins(format!("load ptr, ptr {}", curp));
        let c = self.ins(format!("load i64, ptr {}", curn));
        let big = self.ins(format!("icmp sgt i64 {}, {}", c, BLOCK));
        self.cbr(&big, &body, &done);
        self.block(&body);
        let c1 = self.ins(format!("add i64 {}, {}", c, BLOCK - 1));
        let nb = self.ins(format!("sdiv i64 {}, {}", c1, BLOCK));
        let q = self.alloc_elems(&nb, &t);
        self.for_range_scalar("0", &nb, 1, &mut |g, b| {
            let lo = g.ins(format!("mul i64 {}, {}", b, BLOCK));
            let lo2 = g.ins(format!("add i64 {}, {}", lo, BLOCK));
            let lt2 = g.ins(format!("icmp slt i64 {}, {}", lo2, c));
            let hi = g.ins(format!("select i1 {}, i64 {}, i64 {}", lt2, lo2, c));
            let r = g.reduce_block_owned(&cp, &c, &lo, &hi, l, &zv, &t)?;
            g.store_elem(&q, &nb, b, &t, &r, 1);
            Ok(())
        })?;
        self.drop_level(&cp, &c, &owned, &at);
        self.emit(format!("store ptr {}, ptr {}", q, curp));
        self.emit(format!("store i64 {}, ptr {}", nb, curn));
        self.emit(format!("store i1 true, ptr {}", owned));
        self.br(&head);
        self.block(&done);
        let r = self.reduce_block_owned(&cp, &c, "0", &c, l, &zv, &t)?;
        self.drop_level(&cp, &c, &owned, &at);
        self.drop_value(&zv, &t);
        if fresh {
            self.drop_value(&av, &at);
        }
        Ok(r)
    }

    /// Frees a level of `reduce_owned` (its elements and its block) when it
    /// is one of its own.
    fn drop_level(&mut self, p: &str, n: &str, owned: &str, at: &Type) {
        let o = self.ins(format!("load i1, ptr {}", owned));
        let lfree = self.label();
        let lnext = self.label();
        self.cbr(&o, &lfree, &lnext);
        self.block(&lfree);
        let a = self.arr(p, n);
        self.drop_value(&a, at);
        self.br(&lnext);
        self.block(&lnext);
    }

    /// `scan` of values that own memory (D134): from left to right, each
    /// result owned by the new array and read as the next accumulator.
    fn scan_owned(&mut self, x: &Expr, l: &Lambda, z: &Expr) -> R<V> {
        let t = l.body.ty.clone();
        let ty = lt(&t);
        let zv = self.ex(z, 1)?;
        let (av, p, n, fresh) = self.owned_src(x)?;
        let out = self.alloc_elems(&n, &t);
        let prev = self.alloca(&ty);
        self.emit(format!("store {} {}, ptr {}", ty, zv.r, prev));
        self.for_range_scalar("0", &n, 1, &mut |g, i| {
            let a = v(g.ins(format!("load {}, ptr {}", ty, prev)), ty.clone());
            let x = g.load_elem(&p, &n, i, &t, 1);
            g.bind(l.params[0], a);
            g.bind(l.params[1], x);
            let r = g.ex(&l.body, 1)?;
            g.store_elem(&out, &n, i, &t, &r, 1);
            g.emit(format!("store {} {}, ptr {}", ty, r.r, prev));
            Ok(())
        })?;
        self.drop_value(&zv, &t);
        if fresh {
            self.drop_value(&av, &Type::Array(Box::new(t.clone()), isc_ir::Fixed::ANY));
        }
        Ok(self.arr(&out, &n))
    }

    fn scan(&mut self, e: &Expr, x: &Expr, l: &Lambda, z: &Expr) -> R<V> {
        let elem_ty = x.ty.elem().clone();
        let (src, n) = self.prepare_src(x)?;
        let zv = self.ex(z, 1)?;
        let out = self.alloc_elems(&n, &elem_ty);
        if elem_ty.is_float() {
            // the fixed form of a scan of floats (D112): the first block from
            // `z`, the others from their first element plus the carry; in
            // parallel with more than one block and one thread
            let several = self.ins(format!("icmp sgt i64 {}, {}", n, BLOCK));
            let lblk = self.label();
            let lone = self.label();
            let ljoin = self.label();
            self.cbr(&several, &lblk, &lone);
            self.block(&lblk);
            if self.in_task {
                self.scan_blocked_seq(&src, l, &zv, &out, &n)?;
            } else {
                let threads = self.ins("call i64 @isc_thread_count()".to_string());
                let multi = self.ins(format!("icmp sgt i64 {}, 1", threads));
                let lpar = self.label();
                let lseq = self.label();
                self.cbr(&multi, &lpar, &lseq);
                self.block(&lpar);
                self.scan_parallel_float(e, x, l, &zv, &out, &n)?;
                self.br(&ljoin);
                self.block(&lseq);
                self.scan_blocked_seq(&src, l, &zv, &out, &n)?;
            }
            self.br(&ljoin);
            self.block(&lone);
            self.scan_range(&src, l, &zv, &out, &n, "0", &n, &n)?;
            self.br(&ljoin);
            self.block(&ljoin);
        } else if !self.in_task && assoc_scan_op(l, &elem_ty) {
            let vec = src_vec_ok(x);
            // more than one block and more than one thread: with one thread
            // the two passes of the parallel form cost twice the sequential one
            let several = self.ins(format!("icmp sgt i64 {}, {}", n, BLOCK));
            let threads = self.ins("call i64 @isc_thread_count()".to_string());
            let multi = self.ins(format!("icmp sgt i64 {}, 1", threads));
            let big = self.ins(format!("and i1 {}, {}", several, multi));
            let lpar = self.label();
            let lseq = self.label();
            let ljoin = self.label();
            self.cbr(&big, &lpar, &lseq);
            self.block(&lpar);
            self.scan_parallel(e, x, l, &zv, &out, &n)?;
            self.br(&ljoin);
            self.block(&lseq);
            self.scan_range_fast(&src, vec, l, &zv, &out, &n, "0", &n)?;
            self.br(&ljoin);
            self.block(&ljoin);
        } else {
            self.scan_range_fast(&src, src_vec_ok(x), l, &zv, &out, &n, "0", &n)?;
        }
        let r0 = self.ins(format!("insertvalue {{ ptr, i64 }} poison, ptr {}, 0", out));
        let r = self.ins(format!("insertvalue {{ ptr, i64 }} {}, i64 {}, 1", r0, n));
        Ok(V { r, t: "{ ptr, i64 }".into(), uni: true })
    }

    /// `filter` (SPEC section 6): the elements counted, an array of exactly
    /// that size (its columns are laid out for it), then the elements
    /// written; the predicate is pure, so that it can be evaluated twice.
    /// More than one block and more than one thread: by blocks, in
    /// parallel (`filter_parallel`); the same array for any number of
    /// threads (D178).
    fn filter(&mut self, e: &Expr, x: &Expr, l: &Lambda) -> R<V> {
        let elem_ty = x.ty.elem().clone();
        let (src, n) = self.prepare_src(x)?;
        let w = self.filter_width(x, l, &elem_ty);
        if self.in_task {
            return self.filter_range(&src, &n, l, &elem_ty, w);
        }
        let several = self.ins(format!("icmp sgt i64 {}, {}", n, BLOCK));
        let threads = self.ins("call i64 @isc_thread_count()".to_string());
        let multi = self.ins(format!("icmp sgt i64 {}, 1", threads));
        let big = self.ins(format!("and i1 {}, {}", several, multi));
        let res = self.alloca("{ ptr, i64 }");
        let lpar = self.label();
        let lseq = self.label();
        let ljoin = self.label();
        self.cbr(&big, &lpar, &lseq);
        self.block(&lpar);
        let r = self.filter_parallel(e, x, l, &n, w)?;
        self.emit(format!("store {{ ptr, i64 }} {}, ptr {}", r.r, res));
        self.br(&ljoin);
        self.block(&lseq);
        let r = self.filter_range(&src, &n, l, &elem_ty, w)?;
        self.emit(format!("store {{ ptr, i64 }} {}, ptr {}", r.r, res));
        self.br(&ljoin);
        self.block(&ljoin);
        let r = self.ins(format!("load {{ ptr, i64 }}, ptr {}", res));
        Ok(V { r, t: "{ ptr, i64 }".into(), uni: true })
    }

    /// The lanes of a `filter` in vector code (D195): its source and its
    /// predicate vector code, elements of 32 or 64 bits (or bytes with
    /// VBMI2) that the target compresses; 1 otherwise.
    fn filter_width(&self, x: &Expr, l: &Lambda, elem_ty: &Type) -> u32 {
        let (wide, bytes) = COMPRESS.with(|c| c.get());
        let size_ok = match elem_ty {
            Type::Int(_) | Type::F64 | Type::F32 => match esize(elem_ty) {
                4 | 8 => wide,
                1 => bytes,
                _ => false,
            },
            _ => false,
        };
        // a parameter bound to an `iota`: the index of the loop (D185)
        if matches!(x.kind, ExprKind::Iota(_)) {
            INDEX_VARS.with(|s| s.borrow_mut().insert(l.params[0]));
        }
        if size_ok && src_vec_ok(x) && lam_ok(l) {
            // bytes: 32 per vector (a `ymm` register); wider: 16 lanes
            if esize(elem_ty) == 1 { 32 } else { 16 }
        } else {
            1
        }
    }

    /// The elements of `lo .. hi` of `src` that `l` keeps, counted: `w` at
    /// a time (the bits of the mask counted, D195), then one at a time.
    fn count_kept(&mut self, src: &Src, l: &Lambda, lo: &str, hi: &str, w: u32) -> R<String> {
        let cnt = self.alloca("i64");
        self.emit(format!("store i64 0, ptr {}", cnt));
        let mut from = lo.to_string();
        if w > 1 {
            let span = self.ins(format!("sub i64 {}, {}", hi, lo));
            let full = self.ins(format!("and i64 {}, -{}", span, w));
            let vend = self.ins(format!("add i64 {}, {}", lo, full));
            self.decls.insert(format!("declare i{w} @llvm.ctpop.i{w}(i{w})", w = w));
            self.for_range(lo, &vend, w as i64, &mut |g, i| {
                let el = g.elem(src, i, w)?;
                g.bind(l.params[0], el);
                let keep = g.ex(&l.body, w)?;
                let keep = g.widen(&keep, w);
                let bits = g.ins(format!("bitcast <{} x i1> {} to i{}", w, keep.r, w));
                let k = g.ins(format!("call i{w} @llvm.ctpop.i{w}(i{w} {b})", w = w, b = bits));
                let k64 = g.ins(format!("zext i{} {} to i64", w, k));
                let c = g.ins(format!("load i64, ptr {}", cnt));
                let c2 = g.ins(format!("add i64 {}, {}", c, k64));
                g.emit(format!("store i64 {}, ptr {}", c2, cnt));
                Ok(())
            })?;
            from = vend;
        }
        self.for_range(&from, hi, 1, &mut |g, i| {
            let el = g.elem(src, i, 1)?;
            g.bind(l.params[0], el);
            let keep = g.ex(&l.body, 1)?;
            let c = g.ins(format!("load i64, ptr {}", cnt));
            let k = g.ins(format!("zext i1 {} to i64", keep.r));
            let c2 = g.ins(format!("add i64 {}, {}", c, k));
            g.emit(format!("store i64 {}, ptr {}", c2, cnt));
            Ok(())
        })?;
        Ok(self.ins(format!("load i64, ptr {}", cnt)))
    }

    /// The elements of `lo .. hi` of `src` that `l` keeps, written into
    /// `out` (of `k` elements) from the place in the slot `pos`: `w` at a
    /// time, compressed (`llvm.masked.compressstore`: `vpcompress`, D195),
    /// then one at a time.
    #[allow(clippy::too_many_arguments)]
    fn write_kept(&mut self, src: &Src, l: &Lambda, lo: &str, hi: &str, out: &str, k: &str, pos: &str, elem_ty: &Type, w: u32) -> R<()> {
        let mut from = lo.to_string();
        if w > 1 {
            let span = self.ins(format!("sub i64 {}, {}", hi, lo));
            let full = self.ins(format!("and i64 {}, -{}", span, w));
            let vend = self.ins(format!("add i64 {}, {}", lo, full));
            let m = mt(elem_ty);
            let suffix = match m.as_str() {
                "double" => "f64".to_string(),
                "float" => "f32".to_string(),
                t => t.to_string(),
            };
            let name = format!("llvm.masked.compressstore.v{}{}", w, suffix);
            self.decls.insert(format!("declare void @{}(<{} x {}>, ptr, <{} x i1>)", name, w, m, w));
            self.decls.insert(format!("declare i{w} @llvm.ctpop.i{w}(i{w})", w = w));
            self.for_range(lo, &vend, w as i64, &mut |g, i| {
                let el = g.elem(src, i, w)?;
                let el = g.widen(&el, w);
                g.bind(l.params[0], el.clone());
                let keep = g.ex(&l.body, w)?;
                let keep = g.widen(&keep, w);
                // none kept: nothing written; all kept: the vector stored;
                // some: compressed into memory (the instruction is slow:
                // only when it is needed)
                let bits = g.ins(format!("bitcast <{} x i1> {} to i{}", w, keep.r, w));
                let none = g.ins(format!("icmp eq i{} {}, 0", w, bits));
                let lsome = g.label();
                let lall = g.label();
                let lpart = g.label();
                let ldone = g.label();
                g.cbr(&none, &ldone, &lsome);
                g.block(&lsome);
                let c = g.ins(format!("load i64, ptr {}", pos));
                let at = g.ins(format!("getelementptr inbounds {}, ptr {}, i64 {}", m, out, c));
                let all = g.ins(format!("icmp eq i{} {}, -1", w, bits));
                g.cbr(&all, &lall, &lpart);
                g.block(&lall);
                g.emit(format!("store <{} x {}> {}, ptr {}, align {}", w, m, el.r, at, esize(elem_ty)));
                let c_all = g.ins(format!("add i64 {}, {}", c, w));
                g.emit(format!("store i64 {}, ptr {}", c_all, pos));
                g.br(&ldone);
                g.block(&lpart);
                g.emit(format!("call void @{}(<{} x {}> {}, ptr {}, <{} x i1> {})", name, w, m, el.r, at, w, keep.r));
                let kk = g.ins(format!("call i{w} @llvm.ctpop.i{w}(i{w} {b})", w = w, b = bits));
                let k64 = g.ins(format!("zext i{} {} to i64", w, kk));
                let c2 = g.ins(format!("add i64 {}, {}", c, k64));
                g.emit(format!("store i64 {}, ptr {}", c2, pos));
                g.br(&ldone);
                g.block(&ldone);
                Ok(())
            })?;
            from = vend;
        }
        self.for_range(&from, hi, 1, &mut |g, i| {
            let el = g.elem(src, i, 1)?;
            g.bind(l.params[0], el.clone());
            let keep = g.ex(&l.body, 1)?;
            let lyes = g.label();
            let lnext = g.label();
            g.cbr(&keep.r, &lyes, &lnext);
            g.block(&lyes);
            let c = g.ins(format!("load i64, ptr {}", pos));
            // a node kept: one owner more, the new array (D130)
            let el = if holds_owned(elem_ty) { g.copy_value(&el, elem_ty) } else { el };
            g.store_elem(out, k, &c, elem_ty, &el, 1);
            let c2 = g.ins(format!("add i64 {}, 1", c));
            g.emit(format!("store i64 {}, ptr {}", c2, pos));
            g.br(&lnext);
            g.block(&lnext);
            Ok(())
        })
    }

    /// The sequential `filter` of the `n` elements of `src`, `w` at a time
    /// when its lanes are compressed (D195).
    fn filter_range(&mut self, src: &Src, n: &str, l: &Lambda, elem_ty: &Type, w: u32) -> R<V> {
        let k = self.count_kept(src, l, "0", n, w)?;
        let out = self.alloc_elems(&k, elem_ty);
        let pos = self.alloca("i64");
        self.emit(format!("store i64 0, ptr {}", pos));
        self.write_kept(src, l, "0", n, &out, &k, &pos, elem_ty, w)?;
        Ok(self.arr(&out, &k))
    }

    /// `filter` by blocks of B elements (D178): the elements each block
    /// keeps, counted (task j); the place of each block in the result, the
    /// sum of the counts before it (the caller, in order); then the
    /// elements of each block written from its place (task j). The result
    /// is the sequential one, whatever the number of threads.
    fn filter_parallel(&mut self, e: &Expr, x: &Expr, l: &Lambda, n: &str, w: u32) -> R<V> {
        let elem_ty = x.ty.elem().clone();
        let bl = BLOCK.to_string();
        let n1 = self.ins(format!("add i64 {}, {}", n, BLOCK - 1));
        let c0 = self.ins(format!("sdiv i64 {}, {}", n1, BLOCK));
        let bytes = self.ins(format!("mul i64 {}, 8", c0));
        let counts = self.ins(format!("call ptr @isc_alloc(i64 {})", bytes));
        // the elements of block j: start .. end
        let range = |g: &mut Self, n: &str, j: &str| -> (String, String) {
            let start = g.ins(format!("mul i64 {}, {}", j, BLOCK));
            let rem = g.ins(format!("sub i64 {}, {}", n, start));
            let short = g.ins(format!("icmp slt i64 {}, {}", rem, bl));
            let m = g.ins(format!("select i1 {}, i64 {}, i64 {}", short, rem, bl));
            let end = g.ins(format!("add i64 {}, {}", start, m));
            (start, end)
        };
        // 1. the elements each block keeps
        let extra = [v(counts.clone(), "ptr")];
        let (task, ctx, ctx_bytes) = self.outline_task(&[e], &extra, &mut |g, extra, j| {
            let (src, n) = g.prepare_src(x)?;
            let (start, end) = range(g, &n, j);
            let c = g.count_kept(&src, l, &start, &end, w)?;
            let slot = g.ins(format!("getelementptr inbounds i64, ptr {}, i64 {}", extra[0].r, j));
            g.emit(format!("store i64 {}, ptr {}", c, slot));
            Ok(())
        })?;
        self.emit(format!("call void @isc_parallel_for(i64 {}, ptr {}, ptr {}, i64 {})", c0, task, ctx, ctx_bytes));
        // 2. the place of each block: the sum of the counts before it
        let run = self.alloca("i64");
        self.emit(format!("store i64 0, ptr {}", run));
        self.for_range("0", &c0, 1, &mut |g, j| {
            let slot = g.ins(format!("getelementptr inbounds i64, ptr {}, i64 {}", counts, j));
            let cj = g.ins(format!("load i64, ptr {}", slot));
            let before = g.ins(format!("load i64, ptr {}", run));
            g.emit(format!("store i64 {}, ptr {}", before, slot));
            let after = g.ins(format!("add i64 {}, {}", before, cj));
            g.emit(format!("store i64 {}, ptr {}", after, run));
            Ok(())
        })?;
        let k = self.ins(format!("load i64, ptr {}", run));
        let out = self.alloc_elems(&k, &elem_ty);
        // 3. the elements of each block, from its place
        let extra = [v(counts.clone(), "ptr"), v(out.clone(), "ptr"), v(k.clone(), "i64")];
        let (task, ctx, ctx_bytes) = self.outline_task(&[e], &extra, &mut |g, extra, j| {
            let (src, n) = g.prepare_src(x)?;
            let (start, end) = range(g, &n, j);
            let slot = g.ins(format!("getelementptr inbounds i64, ptr {}, i64 {}", extra[0].r, j));
            let first = g.ins(format!("load i64, ptr {}", slot));
            let pos = g.alloca("i64");
            g.emit(format!("store i64 {}, ptr {}", first, pos));
            let (out, k) = (extra[1].r.clone(), extra[2].r.clone());
            g.write_kept(&src, l, &start, &end, &out, &k, &pos, &elem_ty, w)
        })?;
        self.emit(format!("call void @isc_parallel_for(i64 {}, ptr {}, ptr {}, i64 {})", c0, task, ctx, ctx_bytes));
        self.emit(format!("call void @isc_free(ptr {})", counts));
        Ok(self.arr(&out, &k))
    }

    /// out[i] = op(out[i - 1], x[i]) for i in lo .. hi, out[lo - 1] = `z`;
    /// returns the last value. `len` is the length of `out` (its layout).
    #[allow(clippy::too_many_arguments)]
    fn scan_range(&mut self, src: &Src, l: &Lambda, z: &V, out: &str, len: &str, lo: &str, hi: &str, _n: &str) -> R<V> {
        let elem_ty = l.body.ty.clone();
        let alt = lt(&elem_ty);
        let acc = self.alloca(&alt);
        self.emit(format!("store {} {}, ptr {}", alt, z.r, acc));
        self.for_range(lo, hi, 1, &mut |g, i| {
            let a = g.ins(format!("load {}, ptr {}", alt, acc));
            let el = g.elem(src, i, 1)?;
            g.bind(l.params[0], v(a, alt.clone()));
            g.bind(l.params[1], el);
            let r = g.ex(&l.body, 1)?;
            g.emit(format!("store {} {}, ptr {}", alt, r.r, acc));
            g.store_elem(out, len, i, &elem_ty, &r, 1);
            Ok(())
        })?;
        let last = self.ins(format!("load {}, ptr {}", alt, acc));
        Ok(v(last, alt))
    }

    /// The parallel `scan` of an associative operator: the reduction of each
    /// block (task j), the inclusive values before each block from `z`, from
    /// left to right (the caller), then each block again from its value
    /// (task j). The operator is associative, so that the result is the one
    /// of the sequential scan for any number of threads (SPEC section 6).
    fn scan_parallel(&mut self, e: &Expr, x: &Expr, l: &Lambda, z: &V, out: &str, n: &str) -> R<()> {
        let elem_ty = x.ty.elem().clone();
        let alt = lt(&elem_ty);
        let bl = BLOCK.to_string();
        let n1 = self.ins(format!("add i64 {}, {}", n, BLOCK - 1));
        let c0 = self.ins(format!("sdiv i64 {}, {}", n1, BLOCK));
        let parts = self.alloc_elems(&c0, &elem_ty);
        // 1. the reduction of each block, from its first element
        let extra = [v(parts.clone(), "ptr"), v(c0.clone(), "i64")];
        let (task, ctx, ctx_bytes) = self.outline_task(&[e], &extra, &mut |g, extra, j| {
            let (src, n) = g.prepare_src(x)?;
            let start = g.ins(format!("mul i64 {}, {}", j, BLOCK));
            let rem = g.ins(format!("sub i64 {}, {}", n, start));
            let short = g.ins(format!("icmp slt i64 {}, {}", rem, bl));
            let m = g.ins(format!("select i1 {}, i64 {}, i64 {}", short, rem, bl));
            let end = g.ins(format!("add i64 {}, {}", start, m));
            let first = g.elem(&src, &start, 1)?;
            let acc = g.alloca(&alt);
            g.emit(format!("store {} {}, ptr {}", alt, first.r, acc));
            let from = g.ins(format!("add i64 {}, 1", start));
            g.for_range(&from, &end, 1, &mut |g, i| {
                let a = g.ins(format!("load {}, ptr {}", alt, acc));
                let el = g.elem(&src, i, 1)?;
                g.bind(l.params[0], v(a, alt.clone()));
                g.bind(l.params[1], el);
                let r = g.ex(&l.body, 1)?;
                g.emit(format!("store {} {}, ptr {}", alt, r.r, acc));
                Ok(())
            })?;
            let p = g.ins(format!("load {}, ptr {}", alt, acc));
            g.store_elem(&extra[0].r, &extra[1].r, j, &elem_ty, &v(p, alt.clone()), 1);
            Ok(())
        })?;
        self.emit(format!("call void @isc_parallel_for(i64 {}, ptr {}, ptr {}, i64 {})", c0, task, ctx, ctx_bytes));
        // 2. the value before each block: z, then op(before, reduction)
        let run = self.alloca(&alt);
        self.emit(format!("store {} {}, ptr {}", alt, z.r, run));
        self.for_range("0", &c0, 1, &mut |g, j| {
            let pj = g.load_elem(&parts, &c0, j, &elem_ty, 1);
            let before = g.ins(format!("load {}, ptr {}", alt, run));
            g.store_elem(&parts, &c0, j, &elem_ty, &v(before.clone(), alt.clone()), 1);
            g.bind(l.params[0], v(before, alt.clone()));
            g.bind(l.params[1], pj);
            let r = g.ex(&l.body, 1)?;
            g.emit(format!("store {} {}, ptr {}", alt, r.r, run));
            Ok(())
        })?;
        // 3. each block from the value before it
        let extra = [v(parts.clone(), "ptr"), v(c0.clone(), "i64"), v(out.to_string(), "ptr"), v(n.to_string(), "i64")];
        let (task, ctx, ctx_bytes) = self.outline_task(&[e], &extra, &mut |g, extra, j| {
            let (src, n) = g.prepare_src(x)?;
            let start = g.ins(format!("mul i64 {}, {}", j, BLOCK));
            let rem = g.ins(format!("sub i64 {}, {}", n, start));
            let short = g.ins(format!("icmp slt i64 {}, {}", rem, bl));
            let m = g.ins(format!("select i1 {}, i64 {}, i64 {}", short, rem, bl));
            let end = g.ins(format!("add i64 {}, {}", start, m));
            let before = g.load_elem(&extra[0].r, &extra[1].r, j, &elem_ty, 1);
            g.scan_range_fast(&src, src_vec_ok(x), l, &before, &extra[2].r, &extra[3].r, &start, &end)?;
            Ok(())
        })?;
        self.emit(format!("call void @isc_parallel_for(i64 {}, ptr {}, ptr {}, i64 {})", c0, task, ctx, ctx_bytes));
        self.emit(format!("call void @isc_free(ptr {})", parts));
        Ok(())
    }

    /// `scan_range` of an operator of `int_scan_op` on a source read by
    /// vectors (`vec`): W elements at a time (one register of 512 bits),
    /// their inclusive scan in the register by log2(W) shifts (the lanes
    /// shifted in hold the neutral value), then combined with the last
    /// value of the vector before; the exact result of the scalar loop
    /// (integers: the order of an associative, commutative operator does
    /// not change the result). Any other scan: `scan_range`.
    #[allow(clippy::too_many_arguments)]
    fn scan_range_fast(&mut self, src: &Src, vec: bool, l: &Lambda, z: &V, out: &str, len: &str, lo: &str, hi: &str) -> R<V> {
        let et = l.body.ty.clone();
        let Some((_, neutral)) = int_scan_op(l, &et).filter(|_| vec) else {
            return self.scan_range(src, l, z, out, len, lo, hi, hi);
        };
        let st = lt(&et);
        let w = 64 / esize(&et) as u32;
        let vty = format!("<{} x {}>", w, st);
        let span = self.ins(format!("sub i64 {}, {}", hi, lo));
        let full = self.ins(format!("and i64 {}, -{}", span, w));
        let vend = self.ins(format!("add i64 {}, {}", lo, full));
        let carry = self.alloca(&vty);
        let zw = self.widen(&V { uni: true, ..z.clone() }, w);
        self.emit(format!("store {} {}, ptr {}", vty, zw.r, carry));
        let ident = format!("<{}>", (0..w).map(|_| format!("{} {}", st, neutral)).collect::<Vec<_>>().join(", "));
        // an output larger than the caches (STREAM_BYTES) is written with
        // non-temporal stores: no read of each line before its write (its
        // address: 64-byte aligned, `lo` a multiple of the block or 0)
        let nt = !self.opts.isa.is_wasm();
        let bytes = self.ins(format!("mul i64 {}, {}", len, esize(&et)));
        let huge = self.ins(format!("icmp sge i64 {}, {}", bytes, self.opts.stream_bytes));
        let lnt = self.label();
        let lplain = self.label();
        let lafter = self.label();
        if nt {
            self.cbr(&huge, &lnt, &lplain);
        } else {
            self.br(&lplain);
        }
        let modes: &[bool] = if nt { &[true, false] } else { &[false] };
        for &streaming in modes {
        self.block(if streaming { &lnt } else { &lplain });
        self.for_range_scalar(lo, &vend, w as i64, &mut |g, i| {
            let mut x = g.elem(src, i, w)?;
            let mut s = 1;
            while s < w {
                // lane j gets lane j - s of x, the neutral value below s
                let mask: Vec<String> = (0..w).map(|j| format!("i32 {}", if j < s { j } else { w + j - s })).collect();
                let sh = g.ins(format!("shufflevector {} {}, {} {}, <{} x i32> <{}>", vty, ident, vty, x.r, w, mask.join(", ")));
                x = g.apply2(l, x, V { r: sh, t: vty.clone(), uni: false }, w)?;
                s *= 2;
            }
            // the carry of the next vector: op(carry, last lane of x), so
            // that the permutation stays off the chain from one vector to
            // the next (one operation per vector on it)
            let c = g.ins(format!("load {}, ptr {}", vty, carry));
            let cv = V { r: c, t: vty.clone(), uni: false };
            let last = vec![format!("i32 {}", w - 1); w as usize].join(", ");
            let xl = g.ins(format!("shufflevector {} {}, {} poison, <{} x i32> <{}>", vty, x.r, vty, w, last));
            let y = g.apply2(l, cv.clone(), x, w)?;
            if streaming {
                let po = g.ins(format!("getelementptr inbounds {}, ptr {}, i64 {}", st, out, i));
                g.emit(format!("store {} {}, ptr {}, align 64, !nontemporal !{{i32 1}}", vty, y.r, po));
            } else {
                g.store_elem(out, len, i, &et, &y, w);
            }
            let c2 = g.apply2(l, cv, V { r: xl, t: vty.clone(), uni: false }, w)?;
            g.emit(format!("store {} {}, ptr {}", vty, c2.r, carry));
            Ok(())
        })?;
        if streaming {
            // the non-temporal stores ordered before what follows (another
            // thread reads them after the end of the parallel loop)
            self.emit("call void @llvm.x86.sse.sfence()".to_string());
            self.decls.insert("declare void @llvm.x86.sse.sfence()".to_string());
        }
        self.br(&lafter);
        }
        self.block(&lafter);
        let c = self.ins(format!("load {}, ptr {}", vty, carry));
        let c0 = self.ins(format!("extractelement {} {}, i64 0", vty, c));
        self.scan_range(src, l, &v(c0, st.clone()), out, len, &vend, hi, hi)
    }

    /// out[i] = op(carry, local_i) for i in lo .. hi, where local is the
    /// scan of x[lo ..] from x[lo] (the blocks after the first of the fixed
    /// form of a scan of floats, D112).
    #[allow(clippy::too_many_arguments)]
    fn scan_range_carry(&mut self, src: &Src, l: &Lambda, carry: &V, out: &str, len: &str, lo: &str, hi: &str) -> R<()> {
        let elem_ty = l.body.ty.clone();
        let alt = lt(&elem_ty);
        let acc = self.alloca(&alt);
        let first = self.elem(src, lo, 1)?;
        self.emit(format!("store {} {}, ptr {}", alt, first.r, acc));
        let r0 = self.apply2(l, carry.clone(), first, 1)?;
        self.store_elem(out, len, lo, &elem_ty, &r0, 1);
        let from = self.ins(format!("add i64 {}, 1", lo));
        self.for_range(&from, hi, 1, &mut |g, i| {
            let a = g.ins(format!("load {}, ptr {}", alt, acc));
            let el = g.elem(src, i, 1)?;
            let r = g.apply2(l, v(a, alt.clone()), el, 1)?;
            g.emit(format!("store {} {}, ptr {}", alt, r.r, acc));
            let o = g.apply2(l, carry.clone(), r, 1)?;
            g.store_elem(out, len, i, &elem_ty, &o, 1);
            Ok(())
        })
    }

    /// The fixed form of a scan of floats in one pass: the first block from
    /// `z`, each other block with the last value of the block before as its
    /// carry.
    fn scan_blocked_seq(&mut self, src: &Src, l: &Lambda, z: &V, out: &str, n: &str) -> R<()> {
        let elem_ty = l.body.ty.clone();
        let bl = BLOCK.to_string();
        self.scan_range(src, l, z, out, n, "0", &bl, n)?;
        let nb1 = self.ins(format!("add i64 {}, {}", n, BLOCK - 1));
        let nb = self.ins(format!("sdiv i64 {}, {}", nb1, BLOCK));
        self.for_range_scalar("1", &nb, 1, &mut |g, b| {
            let start = g.ins(format!("mul i64 {}, {}", b, BLOCK));
            let rem = g.ins(format!("sub i64 {}, {}", n, start));
            let short = g.ins(format!("icmp slt i64 {}, {}", rem, bl));
            let m = g.ins(format!("select i1 {}, i64 {}, i64 {}", short, rem, bl));
            let end = g.ins(format!("add i64 {}, {}", start, m));
            let prev = g.ins(format!("sub i64 {}, 1", start));
            let carry = g.load_elem(out, n, &prev, &elem_ty, 1);
            g.scan_range_carry(src, l, &carry, out, n, &start, &end)
        })
    }

    /// The fixed form of a scan of floats in parallel: (1) the scan of each
    /// block, from `z` for the first, from its first element for the others,
    /// its last value kept; (2) the carries, from left to right: the last
    /// value of the first block, then op(carry, last value); (3) each block
    /// written. The same values as `scan_blocked_seq` for any number of
    /// threads.
    fn scan_parallel_float(&mut self, e: &Expr, x: &Expr, l: &Lambda, z: &V, out: &str, n: &str) -> R<()> {
        let elem_ty = x.ty.elem().clone();
        let alt = lt(&elem_ty);
        let bl = BLOCK.to_string();
        let n1 = self.ins(format!("add i64 {}, {}", n, BLOCK - 1));
        let c0 = self.ins(format!("sdiv i64 {}, {}", n1, BLOCK));
        let parts = self.alloc_elems(&c0, &elem_ty);
        let extra = [v(parts.clone(), "ptr"), v(c0.clone(), "i64"), z.clone()];
        let (task, ctx, ctx_bytes) = self.outline_task(&[e], &extra, &mut |g, extra, j| {
            let (src, n) = g.prepare_src(x)?;
            let start = g.ins(format!("mul i64 {}, {}", j, BLOCK));
            let rem = g.ins(format!("sub i64 {}, {}", n, start));
            let short = g.ins(format!("icmp slt i64 {}, {}", rem, bl));
            let m = g.ins(format!("select i1 {}, i64 {}, i64 {}", short, rem, bl));
            let end = g.ins(format!("add i64 {}, {}", start, m));
            // block 0: from z; the others: from their first element
            let first = g.elem(&src, &start, 1)?;
            let zero = g.ins(format!("icmp eq i64 {}, 0", j));
            let z0 = g.apply2(l, extra[2].clone(), first.clone(), 1)?;
            let init = g.ins(format!("select i1 {}, {} {}, {} {}", zero, alt, z0.r, alt, first.r));
            let acc = g.alloca(&alt);
            g.emit(format!("store {} {}, ptr {}", alt, init, acc));
            let from = g.ins(format!("add i64 {}, 1", start));
            g.for_range(&from, &end, 1, &mut |g, i| {
                let a = g.ins(format!("load {}, ptr {}", alt, acc));
                let el = g.elem(&src, i, 1)?;
                let r = g.apply2(l, v(a, alt.clone()), el, 1)?;
                g.emit(format!("store {} {}, ptr {}", alt, r.r, acc));
                Ok(())
            })?;
            let p = g.ins(format!("load {}, ptr {}", alt, acc));
            g.store_elem(&extra[0].r, &extra[1].r, j, &elem_ty, &v(p, alt.clone()), 1);
            Ok(())
        })?;
        self.emit(format!("call void @isc_parallel_for(i64 {}, ptr {}, ptr {}, i64 {})", c0, task, ctx, ctx_bytes));
        // 2. the carry of each block after the first, in parts[b]
        let run = self.alloca(&alt);
        let p0 = self.load_elem(&parts, &c0, "0", &elem_ty, 1);
        self.emit(format!("store {} {}, ptr {}", alt, p0.r, run));
        self.for_range_scalar("1", &c0, 1, &mut |g, b| {
            let last = g.load_elem(&parts, &c0, b, &elem_ty, 1);
            let carry = g.ins(format!("load {}, ptr {}", alt, run));
            g.store_elem(&parts, &c0, b, &elem_ty, &v(carry.clone(), alt.clone()), 1);
            let r = g.apply2(l, v(carry, alt.clone()), last, 1)?;
            g.emit(format!("store {} {}, ptr {}", alt, r.r, run));
            Ok(())
        })?;
        // 3. each block written
        let extra = [v(parts.clone(), "ptr"), v(c0.clone(), "i64"), v(out.to_string(), "ptr"), v(n.to_string(), "i64"), z.clone()];
        let (task, ctx, ctx_bytes) = self.outline_task(&[e], &extra, &mut |g, extra, j| {
            let (src, n) = g.prepare_src(x)?;
            let start = g.ins(format!("mul i64 {}, {}", j, BLOCK));
            let rem = g.ins(format!("sub i64 {}, {}", n, start));
            let short = g.ins(format!("icmp slt i64 {}, {}", rem, bl));
            let m = g.ins(format!("select i1 {}, i64 {}, i64 {}", short, rem, bl));
            let end = g.ins(format!("add i64 {}, {}", start, m));
            let zero = g.ins(format!("icmp eq i64 {}, 0", j));
            let lfirst = g.label();
            let lrest = g.label();
            let ldone = g.label();
            g.cbr(&zero, &lfirst, &lrest);
            g.block(&lfirst);
            g.scan_range(&src, l, &extra[4], &extra[2].r, &extra[3].r, &start, &end, &n)?;
            g.br(&ldone);
            g.block(&lrest);
            let carry = g.load_elem(&extra[0].r, &extra[1].r, j, &elem_ty, 1);
            g.scan_range_carry(&src, l, &carry, &extra[2].r, &extra[3].r, &start, &end)?;
            g.br(&ldone);
            g.block(&ldone);
            Ok(())
        })?;
        self.emit(format!("call void @isc_parallel_for(i64 {}, ptr {}, ptr {}, i64 {})", c0, task, ctx, ctx_bytes));
        self.emit(format!("call void @isc_free(ptr {})", parts));
        Ok(())
    }

    /// The width of a `map` whose elements call SPMD functions of `state`
    /// parameters (D140): as many lanes as the vector registers hold with
    /// about 8 bytes per parameter but one, at least 4 (the program can
    /// give vectors of any width: on AVX-512 they are 512 bits wide). The
    /// SPMD loop is bound by the latency of its chain of operations, so
    /// independent lanes pay until the registers are full: mandelbrot
    /// (5 parameters) on one core is fastest with 64 lanes on AVX-512, 16
    /// with AVX or AVX2, 8 with SSE (D140).
    fn spmd_width(&self, state: usize) -> u32 {
        let f = crate::backend::host_features();
        let has = |name: &str| f.split(',').any(|x| x == format!("+{}", name));
        let bytes = match self.opts.isa {
            Isa::Avx512 => 64 * 32,
            // explicit vectors are 512 bits wide there: faster than 256 bits
            // with the masks of AVX-512VL even with 4 cores at the lower
            // frequency of AVX-512 (mandelbrot: 640 ms against 875 ms, D140)
            Isa::Native if has("avx512f") => 64 * 32,
            Isa::Sse | Isa::Wasm32 | Isa::Wasm32Threads => 16 * 16,
            Isa::Native if !has("avx") => 16 * 16,
            _ => 32 * 16,
        };
        let per_lane = 8 * (state.saturating_sub(1)).max(4) as u32;
        let w = (bytes / per_lane).max(4);
        // a power of two, at most 64
        (1u32 << (31 - w.leading_zeros())).min(64)
    }

    /// The masks of the two branches of an `if` in vector code: the
    /// current lanes with the condition, and with its negation.
    fn branch_masks(&mut self, cv: &V, w: u32) -> (String, String) {
        let mt = format!("<{} x i1>", w);
        let c = self.widen(cv, w);
        let all = self.cst("i1", "true", w);
        let base = self.mask.clone().unwrap_or_else(|| all.clone());
        let t = self.ins(format!("and {} {}, {}", mt, base, c.r));
        let nc = self.ins(format!("xor {} {}, {}", mt, c.r, all));
        let f = self.ins(format!("and {} {}, {}", mt, base, nc));
        (t, f)
    }

    /// A `fold` in vector code over elements that are the same in every
    /// lane (D140): a scalar loop, the accumulator a vector.
    fn fold_vec(&mut self, x: &Expr, init: &Expr, l: &Lambda, w: u32) -> R<V> {
        let (src, n) = self.prepare_src(x)?;
        let iv = self.ex(init, w)?;
        let iv = self.widen(&iv, w);
        let at = iv.t.clone();
        let acc = self.alloca(&at);
        self.emit(format!("store {} {}, ptr {}", at, iv.r, acc));
        self.for_range_scalar("0", &n, 1, &mut |g, i| {
            let a = V { r: g.ins(format!("load {}, ptr {}", at, acc)), t: at.clone(), uni: false };
            let el = g.elem(&src, i, 1)?;
            g.bind(l.params[0], a);
            g.bind(l.params[1], el);
            let r = g.ex(&l.body, w)?;
            let r = g.widen(&r, w);
            g.emit(format!("store {} {}, ptr {}", at, r.r, acc));
            Ok(())
        })?;
        Ok(V { r: self.ins(format!("load {}, ptr {}", at, acc)), t: at, uni: false })
    }

    /// A `reduce` in vector code (D185): its operator order free, over an
    /// array of the context, maybe through a `map` whose lambda reads the
    /// lanes: a scalar loop over the elements, the accumulator a vector.
    fn reduce_vec(&mut self, x: &Expr, op: &Lambda, z: &Expr, w: u32) -> R<V> {
        let (arr, lam) = match &x.kind {
            ExprKind::Map(ins, m) => (&ins[0], Some(m)),
            _ => (x, None),
        };
        let (src, n) = self.prepare_src(arr)?;
        let zv = self.ex(z, 1)?;
        let zv = self.widen(&zv, w);
        let at = zv.t.clone();
        let acc = self.alloca(&at);
        self.emit(format!("store {} {}, ptr {}", at, zv.r, acc));
        self.for_range_scalar("0", &n, 1, &mut |g, i| {
            let el = g.elem(&src, i, 1)?;
            let xv = match lam {
                Some(m) => {
                    g.bind(m.params[0], el);
                    g.ex(&m.body, w)?
                }
                None => el,
            };
            let xv = g.widen(&xv, w);
            let a = V { r: g.ins(format!("load {}, ptr {}", at, acc)), t: at.clone(), uni: false };
            let r = g.apply2(op, a, xv, w)?;
            g.emit(format!("store {} {}, ptr {}", at, r.r, acc));
            Ok(())
        })?;
        Ok(V { r: self.ins(format!("load {}, ptr {}", at, acc)), t: at, uni: false })
    }

    /// The target has mask registers (AVX-512).
    fn has_mask_registers(&self) -> bool {
        let f = crate::backend::host_features();
        self.opts.isa == Isa::Avx512 || (self.opts.isa == Isa::Native && f.split(',').any(|x| x == "+avx512f"))
    }

    /// A SPMD call (D140): the state of each lane in vectors; at each
    /// round the body is computed for every lane, a lane whose path ends
    /// in a value keeps it and stops, a lane whose path ends in a tail call
    /// takes its new arguments; the loop ends when no lane is active.
    fn spmd_call(&mut self, f: u32, args: &[Expr], w: u32) -> R<V> {
        let fun = &self.p.fns[f as usize];
        let mt = format!("<{} x i1>", w);
        let mut vals = Vec::new();
        for a in args {
            vals.push(self.ex(a, w)?);
        }
        let uniform = spmd_uniform(fun, f, &vals.iter().map(uni).collect::<Vec<_>>());
        let mut state = Vec::new();
        for ((x, p), &u) in vals.into_iter().zip(fun.params.iter()).zip(uniform.iter()) {
            // a uniform parameter: one scalar for all the lanes
            let x = if u { x } else { self.widen(&x, w) };
            let slot = self.alloca(&x.t);
            self.emit(format!("store {} {}, ptr {}", x.t, x.r, slot));
            state.push((*p, slot, x.t.clone(), u));
        }
        let rt = format!("<{} x {}>", w, lt(&fun.body.ty));
        let res = self.alloca(&rt);
        self.emit(format!("store {} zeroinitializer, ptr {}", rt, res));
        let active = self.alloca(&mt);
        let all = self.cst("i1", "true", w);
        let entry = self.mask.clone().unwrap_or_else(|| all.clone());
        self.emit(format!("store {} {}, ptr {}", mt, entry, active));
        let saved_mask = self.mask.take();
        let head = self.label();
        let done = self.label();
        let lbody = self.label();
        self.br(&head);
        self.block(&head);
        let act = self.ins(format!("load {}, ptr {}", mt, active));
        let any = self.ins(format!("call i1 @llvm.vector.reduce.or.v{}i1({} {})", w, mt, act));
        self.decls.insert(format!("declare i1 @llvm.vector.reduce.or.v{}i1({})", w, mt));
        self.cbr(&any, &lbody, &done);
        self.block(&lbody);
        // the current state, and the next one: the first tail call reached
        // gives it, the others only to their lanes (a lane that stops keeps
        // no state)
        let mut next = Vec::new();
        for (p, slot, t, u) in &state {
            let x = self.ins(format!("load {}, ptr {}", t, slot));
            self.bind(*p, V { r: x.clone(), t: t.clone(), uni: *u });
            next.push((x, false, *u, t.clone()));
        }
        let mut finished = "zeroinitializer".to_string();
        let body = fun.body.clone();
        self.spmd_body(&body, f, &act, w, &res, &rt, &mut next, &mut finished)?;
        for ((_, slot, t, _), (x, _, _, _)) in state.iter().zip(next.iter()) {
            self.emit(format!("store {} {}, ptr {}", t, x, slot));
        }
        let nf = self.ins(format!("xor {} {}, {}", mt, finished, all));
        let act2 = self.ins(format!("and {} {}, {}", mt, act, nf));
        self.emit(format!("store {} {}, ptr {}", mt, act2, active));
        self.br(&head);
        self.block(&done);
        self.mask = saved_mask;
        Ok(V { r: self.ins(format!("load {}, ptr {}", rt, res)), t: rt, uni: false })
    }

    /// One path of the body of a SPMD function under the lanes `m`; `next`:
    /// the next value of each parameter, whether a tail call gave it,
    /// whether it is uniform, and its type.
    #[allow(clippy::too_many_arguments)]
    fn spmd_body(
        &mut self,
        e: &Expr,
        f: u32,
        m: &str,
        w: u32,
        res: &str,
        rt: &str,
        next: &mut Vec<(String, bool, bool, String)>,
        finished: &mut String,
    ) -> R<()> {
        let mt = format!("<{} x i1>", w);
        match &e.kind {
            ExprKind::Let { var, init, body, .. } => {
                let x = self.ex(init, w)?;
                self.bind(*var, x);
                self.spmd_body(body, f, m, w, res, rt, next, finished)
            }
            // a uniform condition: every active lane takes the same branch, a
            // real one, without masks (D146)
            ExprKind::If(c, t, x) if { let cv = self.ex(c, w)?; let u = uni(&cv); self.spmd_cond = Some(cv); u } => {
                let cv = self.spmd_cond.take().unwrap();
                let (lt_, lf, lj) = (self.label(), self.label(), self.label());
                self.cbr(&cv.r, &lt_, &lf);
                self.block(&lt_);
                let (mut next_t, mut fin_t) = (next.clone(), finished.clone());
                self.spmd_body(t, f, m, w, res, rt, &mut next_t, &mut fin_t)?;
                let end_t = self.current_label(&lt_);
                self.br(&lj);
                self.block(&lf);
                let (mut next_f, mut fin_f) = (next.clone(), finished.clone());
                self.spmd_body(x, f, m, w, res, rt, &mut next_f, &mut fin_f)?;
                let end_f = self.current_label(&lf);
                self.br(&lj);
                self.block(&lj);
                for k in 0..next.len() {
                    let (vt, gt, u, ty) = next_t[k].clone();
                    let (vf, gf, _, _) = next_f[k].clone();
                    let v = if vt == vf { vt } else { self.ins(format!("phi {} [ {}, %{} ], [ {}, %{} ]", ty, vt, end_t, vf, end_f)) };
                    next[k] = (v, gt || gf, u, ty);
                }
                *finished = if fin_t == fin_f {
                    fin_t
                } else {
                    self.ins(format!("phi {} [ {}, %{} ], [ {}, %{} ]", mt, fin_t, end_t, fin_f, end_f))
                };
                Ok(())
            }
            ExprKind::If(c, t, x) => {
                let cv = match self.spmd_cond.take() {
                    Some(cv) => cv,
                    None => self.ex(c, w)?,
                };
                let cv = self.widen(&cv, w);
                let all = self.cst("i1", "true", w);
                let mt_ = self.ins(format!("and {} {}, {}", mt, m, cv.r));
                let nc = self.ins(format!("xor {} {}, {}", mt, cv.r, all));
                let mf = self.ins(format!("and {} {}, {}", mt, m, nc));
                self.spmd_body(t, f, &mt_, w, res, rt, next, finished)?;
                self.spmd_body(x, f, &mf, w, res, rt, next, finished)
            }
            ExprKind::Call(g, args) if *g == f => {
                for (k, a) in args.iter().enumerate() {
                    let (cur, given, u, ty) = next[k].clone();
                    if u {
                        // the same value from every tail call
                        if !given {
                            let x = self.ex(a, w)?;
                            if !uni(&x) {
                                return Err("codegen: a uniform parameter of a SPMD call is not uniform".into());
                            }
                            next[k] = (x.r, true, true, ty);
                        }
                        continue;
                    }
                    let x = self.ex(a, w)?;
                    let x = self.widen(&x, w);
                    let r = if given { self.ins(format!("select {} {}, {} {}, {} {}", mt, m, x.t, x.r, x.t, cur)) } else { x.r };
                    next[k] = (r, true, false, ty);
                }
                Ok(())
            }
            // a value: without mask registers, computed only in the rounds
            // where a lane reaches it (each lane does once); with them
            // (AVX-512) the test would cost more than the select (D140)
            _ if self.has_mask_registers() || matches!(e.kind, ExprKind::Lit(_) | ExprKind::Var(..)) => {
                let x = self.ex(e, w)?;
                let x = self.widen(&x, w);
                let r = self.ins(format!("load {}, ptr {}", rt, res));
                let r2 = self.ins(format!("select {} {}, {} {}, {} {}", mt, m, rt, x.r, rt, r));
                self.emit(format!("store {} {}, ptr {}", rt, r2, res));
                *finished = self.ins(format!("or {} {}, {}", mt, finished, m));
                Ok(())
            }
            _ => {
                let any = self.ins(format!("call i1 @llvm.vector.reduce.or.v{}i1({} {})", w, mt, m));
                self.decls.insert(format!("declare i1 @llvm.vector.reduce.or.v{}i1({})", w, mt));
                let ldo = self.label();
                let lnext = self.label();
                self.cbr(&any, &ldo, &lnext);
                self.block(&ldo);
                let x = self.ex(e, w)?;
                let x = self.widen(&x, w);
                let r = self.ins(format!("load {}, ptr {}", rt, res));
                let r2 = self.ins(format!("select {} {}, {} {}, {} {}", mt, m, rt, x.r, rt, r));
                self.emit(format!("store {} {}, ptr {}", rt, r2, res));
                self.br(&lnext);
                self.block(&lnext);
                *finished = self.ins(format!("or {} {}, {}", mt, finished, m));
                Ok(())
            }
        }
    }

    fn fold(&mut self, x: &Expr, init: &Expr, l: &Lambda) -> R<V> {
        let (src, n) = self.prepare_src(x)?;
        let iv = self.ex(init, 1)?;
        let at = init.ty.clone();
        let alt = lt(&at);
        let acc = self.alloca(&alt);
        self.emit(format!("store {} {}, ptr {}", alt, iv.r, acc));
        self.for_range("0", &n, 1, &mut |g, i| {
            let a = g.ins(format!("load {}, ptr {}", alt, acc));
            let av = v(a, alt.clone());
            let el = g.elem(&src, i, 1)?;
            g.bind(l.params[0], av.clone());
            g.bind(l.params[1], el);
            let r = g.ex(&l.body, 1)?;
            if l.drop_params[0] {
                g.drop_value(&av, &at);
            }
            g.emit(format!("store {} {}, ptr {}", alt, r.r, acc));
            Ok(())
        })?;
        let r = self.ins(format!("load {}, ptr {}", alt, acc));
        Ok(v(r, alt))
    }

    // -----------------------------------------------------------------
    // reductions of columns (rule F3, DECISIONS D100)

    /// `reduce(map(xs, (v) -> (f_1(v), ..., f_k(v))), op, (z_1, ..., z_k))`
    /// whose operator works component by component (the form `horizontal`
    /// builds): the k reductions `reduce(map(xs, f_i), op_i, z_i)`, which
    /// give the same results, as columns.
    fn columns_of(x: &Expr, l: &Lambda, z: &Expr) -> Option<Vec<(Expr, Lambda, Expr)>> {
        let Type::Tuple(ts) = &l.body.ty else { return None };
        if ts.iter().any(|t| !t.is_scalar()) {
            return None;
        }
        let (ExprKind::Map(ins, m), ExprKind::Tuple(ops), ExprKind::Tuple(zs)) = (&x.kind, &l.body.kind, &z.kind) else { return None };
        let ExprKind::Tuple(fs) = &m.body.kind else { return None };
        let k = ts.len();
        if fs.len() != k || ops.len() != k || zs.len() != k {
            return None;
        }
        let mut cols = Vec::new();
        for i in 0..k {
            // op_i: let a_i = A.i in let b_i = B.i in body_i
            let ExprKind::Let { var: ai, init: pa, body: rest, .. } = &ops[i].kind else { return None };
            let ExprKind::Let { var: bi, init: pb, body: bi_body, .. } = &rest.kind else { return None };
            let proj_of = |e: &Expr, p: VarId| matches!(&e.kind, ExprKind::Proj(t, j) if *j as usize == i && matches!(t.kind, ExprKind::Var(w, _) if w == p));
            if !proj_of(pa, l.params[0]) || !proj_of(pb, l.params[1]) {
                return None;
            }
            // the column: the input itself when f_i is the identity
            let src = match &fs[i].kind {
                ExprKind::Var(w, _) if ins.len() == 1 && *w == m.params[0] => ins[0].clone(),
                _ => Expr::new(
                    ExprKind::Map(ins.clone(), Lambda { params: m.params.clone(), body: Box::new(fs[i].clone()), drop_params: vec![false; m.params.len()] }),
                    Type::Array(Box::new(ts[i].clone()), isc_ir::Fixed::ANY),
                    x.span,
                ),
            };
            let op = Lambda { params: vec![*ai, *bi], body: bi_body.clone(), drop_params: vec![false, false] };
            cols.push((src, op, zs[i].clone()));
        }
        Some(cols)
    }

    /// The reductions of `cols`, one array read once: each block of B
    /// elements is reduced for every column in turn while it is in the cache
    /// (in parallel, block j by task j), then the block results of each
    /// column are reduced as in `reduce`. Each column has the form of section
    /// 10, so the same result as alone.
    fn reduce_columns(&mut self, e: &Expr, cols: &[(Expr, Lambda, Expr)]) -> R<V> {
        let lanes = self.opts.lanes;
        let mut zs = Vec::new();
        for (_, _, z) in cols {
            zs.push(self.ex(z, 1)?);
        }
        let ts: Vec<Type> = cols.iter().map(|(x, _, _)| x.ty.elem().clone()).collect();
        let ws: Vec<u32> = cols.iter().map(|(x, l, _)| if src_vec_ok(x) && lam_ok(l) { lanes } else { 1 }).collect();
        let wms: Vec<u32> = cols.iter().map(|(_, l, _)| if lam_ok(l) { lanes } else { 1 }).collect();
        let mut srcs = Vec::new();
        let mut n = String::new();
        for (x, _, _) in cols {
            let (s, len) = self.prepare_src(x)?;
            srcs.push(s);
            n = len;
        }
        let k = cols.len();
        let res: Vec<String> = ts.iter().map(|t| self.alloca(&lt(t))).collect();
        let bl = BLOCK.to_string();
        let big = self.ins(format!("icmp sgt i64 {}, {}", n, BLOCK));
        let lsmall = self.label();
        let lbig = self.label();
        let lend = self.label();
        self.cbr(&big, &lbig, &lsmall);
        // one block
        self.block(&lsmall);
        for i in 0..k {
            let r = self.reduce_block(&srcs[i], "0", &n, &cols[i].1, &zs[i], &ts[i], ws[i])?;
            self.emit(format!("store {} {}, ptr {}", lt(&ts[i]), r.r, res[i]));
        }
        self.br(&lend);
        // blocks: their results in `parts` (one array per column)
        self.block(&lbig);
        let n1 = self.ins(format!("add i64 {}, {}", n, BLOCK - 1));
        let c0 = self.ins(format!("sdiv i64 {}, {}", n1, BLOCK));
        let parts: Vec<String> = ts.iter().map(|t| self.alloc_elems(&c0, t)).collect();
        if self.in_task {
            self.for_range("0", &c0, 1, &mut |g, j| {
                let start = g.ins(format!("mul i64 {}, {}", j, BLOCK));
                let rem = g.ins(format!("sub i64 {}, {}", n, start));
                let short = g.ins(format!("icmp slt i64 {}, {}", rem, bl));
                let mm = g.ins(format!("select i1 {}, i64 {}, i64 {}", short, rem, bl));
                for i in 0..k {
                    let r = g.reduce_block(&srcs[i], &start, &mm, &cols[i].1, &zs[i], &ts[i], ws[i])?;
                    g.store_elem(&parts[i], &c0, j, &ts[i], &r, 1);
                }
                Ok(())
            })?;
        } else {
            let mut extra: Vec<V> = parts.iter().map(|p| v(p.clone(), "ptr")).collect();
            extra.push(v(c0.clone(), "i64"));
            let (task, ctx, ctx_bytes) = self.outline_task(&[e], &extra, &mut |g, extra, j| {
                let mut srcs = Vec::new();
                let mut n = String::new();
                for (x, _, _) in cols {
                    let (s, len) = g.prepare_src(x)?;
                    srcs.push(s);
                    n = len;
                }
                let start = g.ins(format!("mul i64 {}, {}", j, BLOCK));
                let rem = g.ins(format!("sub i64 {}, {}", n, start));
                let short = g.ins(format!("icmp slt i64 {}, {}", rem, bl));
                let mm = g.ins(format!("select i1 {}, i64 {}, i64 {}", short, rem, bl));
                for i in 0..k {
                    let zv = g.ex(&cols[i].2, 1)?;
                    let r = g.reduce_block(&srcs[i], &start, &mm, &cols[i].1, &zv, &ts[i], ws[i])?;
                    g.store_elem(&extra[i].r, &extra[k].r, j, &ts[i], &r, 1);
                }
                Ok(())
            })?;
            self.emit(format!("call void @isc_parallel_for(i64 {}, ptr {}, ptr {}, i64 {})", c0, task, ctx, ctx_bytes));
        }
        // upper levels of each column, in place in its `parts`
        for i in 0..k {
            let ca = self.alloca("i64");
            self.emit(format!("store i64 {}, ptr {}", c0, ca));
            let head = self.label();
            let body = self.label();
            let done = self.label();
            self.br(&head);
            self.block(&head);
            let c = self.ins(format!("load i64, ptr {}", ca));
            let more = self.ins(format!("icmp sgt i64 {}, 1", c));
            self.cbr(&more, &body, &done);
            self.block(&body);
            let c1 = self.ins(format!("add i64 {}, {}", c, BLOCK - 1));
            let c2 = self.ins(format!("sdiv i64 {}, {}", c1, BLOCK));
            let msrc = Src::Mat { ptr: parts[i].clone(), len: c0.clone(), elem: ts[i].clone() };
            let (pi, ti, li, zi, wi) = (parts[i].clone(), ts[i].clone(), &cols[i].1, zs[i].clone(), wms[i]);
            self.for_range("0", &c2, 1, &mut |g, q| {
                let start = g.ins(format!("mul i64 {}, {}", q, BLOCK));
                let rem = g.ins(format!("sub i64 {}, {}", c, start));
                let short = g.ins(format!("icmp slt i64 {}, {}", rem, bl));
                let mm = g.ins(format!("select i1 {}, i64 {}, i64 {}", short, rem, bl));
                let r = g.reduce_block(&msrc, &start, &mm, li, &zi, &ti, wi)?;
                g.store_elem(&pi, &c0, q, &ti, &r, 1);
                Ok(())
            })?;
            self.emit(format!("store i64 {}, ptr {}", c2, ca));
            self.br(&head);
            self.block(&done);
            let r = self.load_elem(&parts[i], &c0, "0", &ts[i], 1);
            self.emit(format!("store {} {}, ptr {}", lt(&ts[i]), r.r, res[i]));
            self.emit(format!("call void @isc_free(ptr {})", parts[i]));
        }
        self.br(&lend);
        self.block(&lend);
        let tt = Type::Tuple(ts.clone());
        let mut acc = "poison".to_string();
        for i in 0..k {
            let r = self.ins(format!("load {}, ptr {}", lt(&ts[i]), res[i]));
            acc = self.ins(format!("insertvalue {} {}, {} {}, {}", lt(&tt), acc, lt(&ts[i]), r, i));
        }
        Ok(V { r: acc, t: lt(&tt), uni: true })
    }

    // -----------------------------------------------------------------
    // streams (rule F2, DECISIONS D99): a `filter`, possibly followed by
    // `map`s of one input, read in order by a `reduce`, a `fold` or an
    // evaluated `map`, in one pass over the filter's input

    fn prepare_stream<'e>(&mut self, e: &'e Expr) -> R<Stream<'e>> {
        match &e.kind {
            ExprKind::Filter(x, l) => {
                let (base, n) = self.prepare_src(x)?;
                let vec = src_vec_ok(x) && lam_ok(l) && x.ty.elem().is_scalar();
                Ok(Stream { base, n, pred: l, posts: Vec::new(), vec })
            }
            ExprKind::Map(ins, l) if ins.len() == 1 => {
                let mut s = self.prepare_stream(&ins[0])?;
                s.vec = s.vec && lam_ok(l) && isc_ir::passes::cannot_panic(&l.body) && e.ty.elem().is_scalar();
                s.posts.push(l);
                Ok(s)
            }
            _ => Err("internal error: not a stream".into()),
        }
    }

    /// Calls `f` with each element the stream keeps, in order.
    fn for_each_kept(&mut self, s: &Stream, f: &mut dyn FnMut(&mut Self, V) -> R<()>) -> R<()> {
        let n = s.n.clone();
        self.for_range("0", &n, 1, &mut |g, i| {
            let el = g.elem(&s.base, i, 1)?;
            g.bind(s.pred.params[0], el.clone());
            let keep = g.ex(&s.pred.body, 1)?;
            let lyes = g.label();
            let lnext = g.label();
            g.cbr(&keep.r, &lyes, &lnext);
            g.block(&lyes);
            let mut x = el;
            for l in &s.posts {
                g.bind(l.params[0], x);
                x = g.ex(&l.body, 1)?;
            }
            f(g, x)?;
            g.br(&lnext);
            g.block(&lnext);
            Ok(())
        })
    }

    /// `fold` over a stream: the accumulator changes only for the elements
    /// kept.
    fn fold_stream(&mut self, x: &Expr, init: &Expr, l: &Lambda) -> R<V> {
        let st = self.prepare_stream(x)?;
        let iv = self.ex(init, 1)?;
        let at = init.ty.clone();
        let alt = lt(&at);
        let acc = self.alloca(&alt);
        self.emit(format!("store {} {}, ptr {}", alt, iv.r, acc));
        self.for_each_kept(&st, &mut |g, el| {
            let a = g.ins(format!("load {}, ptr {}", alt, acc));
            let av = v(a, alt.clone());
            g.bind(l.params[0], av.clone());
            g.bind(l.params[1], el);
            let r = g.ex(&l.body, 1)?;
            if l.drop_params[0] {
                g.drop_value(&av, &at);
            }
            g.emit(format!("store {} {}, ptr {}", alt, r.r, acc));
            Ok(())
        })?;
        let r = self.ins(format!("load {}, ptr {}", alt, acc));
        Ok(v(r, alt))
    }

    /// An evaluated `map` of a stream: the elements kept are counted, then
    /// mapped into an array of exactly that size.
    fn materialize_stream(&mut self, e: &Expr) -> R<V> {
        let elem_ty = e.ty.elem().clone();
        let st = self.prepare_stream(e)?;
        let cnt = self.alloca("i64");
        self.emit(format!("store i64 0, ptr {}", cnt));
        let n = st.n.clone();
        self.for_range("0", &n, 1, &mut |g, i| {
            let el = g.elem(&st.base, i, 1)?;
            g.bind(st.pred.params[0], el);
            let keep = g.ex(&st.pred.body, 1)?;
            let c = g.ins(format!("load i64, ptr {}", cnt));
            let k = g.ins(format!("zext i1 {} to i64", keep.r));
            let c2 = g.ins(format!("add i64 {}, {}", c, k));
            g.emit(format!("store i64 {}, ptr {}", c2, cnt));
            Ok(())
        })?;
        let k = self.ins(format!("load i64, ptr {}", cnt));
        let out = self.alloc_elems(&k, &elem_ty);
        self.emit(format!("store i64 0, ptr {}", cnt));
        self.for_each_kept(&st, &mut |g, x| {
            let c = g.ins(format!("load i64, ptr {}", cnt));
            let x = if holds_owned(&elem_ty) { g.copy_value(&x, &elem_ty) } else { x };
            g.store_elem(&out, &k, &c, &elem_ty, &x, 1);
            let c2 = g.ins(format!("add i64 {}, 1", c));
            g.emit(format!("store i64 {}, ptr {}", c2, cnt));
            Ok(())
        })?;
        let r0 = self.ins(format!("insertvalue {{ ptr, i64 }} poison, ptr {}, 0", out));
        let r = self.ins(format!("insertvalue {{ ptr, i64 }} {}, i64 {}, 1", r0, k));
        Ok(V { r, t: "{ ptr, i64 }".into(), uni: true })
    }

    /// `reduce` over a stream, with the result of the form of section 10 on
    /// the array of the kept elements, for any operator and any `e`: the k-th
    /// element kept goes to accumulator k mod V of its block; a block of B
    /// elements is closed when one more element comes, and its result goes,
    /// the same way, to the level above (the reduction of the block results).
    /// At the end, a level that has received no block result is the last
    /// one; the levels below give theirs first. Sequential, in one pass.
    fn reduce_stream(&mut self, x: &Expr, l: &Lambda, z: &Expr) -> R<V> {
        let zv = self.ex(z, 1)?;
        let st = self.prepare_stream(x)?;
        let t = l.body.ty.clone();
        if st.vec && lam_ok(l) && t.is_scalar() && !self.in_task {
            // in parallel with more than one thread and two blocks of inputs
            let threads = self.ins("call i64 @isc_thread_count()".to_string());
            let multi = self.ins(format!("icmp sgt i64 {}, 1", threads));
            let big = self.ins(format!("icmp sgt i64 {}, {}", st.n, 2 * BLOCK));
            let par = self.ins(format!("and i1 {}, {}", multi, big));
            let alt = lt(&t);
            let res = self.alloca(&alt);
            let lpar = self.label();
            let lseq = self.label();
            let ljoin = self.label();
            self.cbr(&par, &lpar, &lseq);
            self.block(&lpar);
            let r = self.reduce_stream_parallel(x, &st, l, &zv, &t, &threads)?;
            self.emit(format!("store {} {}, ptr {}", alt, r.r, res));
            self.br(&ljoin);
            self.block(&lseq);
            let r = self.reduce_stream_seq(&st, l, &zv, &t)?;
            self.emit(format!("store {} {}, ptr {}", alt, r.r, res));
            self.br(&ljoin);
            self.block(&ljoin);
            let out = self.ins(format!("load {}, ptr {}", alt, res));
            return Ok(v(out, alt));
        }
        self.reduce_stream_seq(&st, l, &zv, &t)
    }

    /// `reduce_stream` in one pass, on the current thread.
    fn reduce_stream_seq(&mut self, st: &Stream, l: &Lambda, zv: &V, t: &Type) -> R<V> {
        let zv = zv.clone();
        let t = t.clone();
        let alt = lt(&t);
        let lanes = self.opts.lanes as i64;
        let acc = self.alloca(&format!("[{} x {}]", STREAM_LEVELS * lanes, alt));
        let cnt = self.alloca(&format!("[{} x i64]", STREAM_LEVELS));
        let levels = STREAM_LEVELS.to_string();
        let zc = zv.clone();
        let altc = alt.clone();
        self.for_range_scalar("0", &levels, 1, &mut |g, lv| {
            g.rs_reset(&acc, &cnt, lv, &zc, &altc);
            Ok(())
        })?;
        if st.vec && lam_ok(l) && t.is_scalar() {
            self.stream_vector(st, l, &zv, &acc, &cnt, &t)?;
        } else {
            self.for_each_kept(st, &mut |g, el| g.rs_push(&acc, &cnt, "0", el, l, &zv, &alt))?;
        }
        // the result: the first level that has received no block result
        let res = self.alloca(&alt);
        let lvl = self.alloca("i64");
        self.emit(format!("store i64 0, ptr {}", lvl));
        let head = self.label();
        let check = self.label();
        let up = self.label();
        let flush = self.label();
        let next = self.label();
        let fin = self.label();
        let done = self.label();
        self.br(&head);
        self.block(&head);
        let lv = self.ins(format!("load i64, ptr {}", lvl));
        let last = self.ins(format!("icmp eq i64 {}, {}", lv, STREAM_LEVELS - 1));
        self.cbr(&last, &fin, &check);
        self.block(&check);
        let lv1 = self.ins(format!("add i64 {}, 1", lv));
        let p1 = self.ins(format!("getelementptr inbounds [{} x i64], ptr {}, i64 0, i64 {}", STREAM_LEVELS, cnt, lv1));
        let c1 = self.ins(format!("load i64, ptr {}", p1));
        let has_up = self.ins(format!("icmp sgt i64 {}, 0", c1));
        self.cbr(&has_up, &up, &fin);
        self.block(&up);
        let p0 = self.ins(format!("getelementptr inbounds [{} x i64], ptr {}, i64 0, i64 {}", STREAM_LEVELS, cnt, lv));
        let c0 = self.ins(format!("load i64, ptr {}", p0));
        let partial = self.ins(format!("icmp sgt i64 {}, 0", c0));
        self.cbr(&partial, &flush, &next);
        self.block(&flush);
        let r = self.rs_tree(&acc, &lv, l, &alt)?;
        self.rs_push(&acc, &cnt, &lv1, r, l, &zv, &alt)?;
        self.br(&next);
        self.block(&next);
        self.emit(format!("store i64 {}, ptr {}", lv1, lvl));
        self.br(&head);
        self.block(&fin);
        let lvf = self.ins(format!("load i64, ptr {}", lvl));
        let r = self.rs_tree(&acc, &lvf, l, &alt)?;
        self.emit(format!("store {} {}, ptr {}", alt, r.r, res));
        self.br(&done);
        self.block(&done);
        let out = self.ins(format!("load {}, ptr {}", alt, res));
        Ok(v(out, alt))
    }

    /// `reduce_stream` on several threads, with the same result (D120). By
    /// section 10, the reduction of the kept elements is the reduction of
    /// the results of their blocks of B (one block: its result; none: the
    /// tree of z). The inputs are cut into chunks of at most STREAM_CHUNK,
    /// taken by rounds of one chunk per thread: (1) each chunk of the round
    /// counts its kept elements; (2) from the counts, the index of the first
    /// kept element of each chunk; a chunk owns the blocks that start in
    /// its range of indices, skips the kept elements of a block started
    /// before it and reads past its end to finish its last block
    /// (`stream_blocks`); (3) at the end, the block results reduced in the
    /// fixed form. The inputs of a round (8 MiB of `f64` with 8 threads) are
    /// still in the caches when (2) reads them again.
    fn reduce_stream_parallel(&mut self, x: &Expr, st: &Stream, l: &Lambda, zv: &V, t: &Type, threads: &str) -> R<V> {
        const STREAM_CHUNK: i64 = 32 << 10;
        let alt = lt(t);
        let n = st.n.clone();
        // chunks of a multiple of 64 inputs (lines of the compression)
        let per = self.ins(format!("add i64 {}, {}", n, threads));
        let per1 = self.ins(format!("sub i64 {}, 1", per));
        let per2 = self.ins(format!("sdiv i64 {}, {}", per1, threads));
        let ch0 = self.ins(format!("add i64 {}, 63", per2));
        let ch1 = self.ins(format!("and i64 {}, -64", ch0));
        let small = self.ins(format!("icmp slt i64 {}, {}", ch1, STREAM_CHUNK));
        let ch = self.ins(format!("select i1 {}, i64 {}, i64 {}", small, ch1, STREAM_CHUNK));
        let nc0 = self.ins(format!("add i64 {}, {}", n, ch));
        let nc1 = self.ins(format!("sub i64 {}, 1", nc0));
        let nc = self.ins(format!("sdiv i64 {}, {}", nc1, ch));
        let nc_1 = self.ins(format!("add i64 {}, 1", nc));
        // the kept elements of each chunk, and the index of the first kept
        // element of each chunk (and of the chunk after the last)
        let counts = self.alloc_elems(&nc, &Type::Int(IntKind::I64));
        let starts = self.alloc_elems(&nc_1, &Type::Int(IntKind::I64));
        // at most one block per B inputs, plus a partial one
        let mb0 = self.ins(format!("sdiv i64 {}, {}", n, BLOCK));
        let mb = self.ins(format!("add i64 {}, 2", mb0));
        let parts = self.alloc_elems(&mb, t);
        let run = self.alloca("i64");
        self.emit(format!("store i64 0, ptr {}", run));
        let nr0 = self.ins(format!("add i64 {}, {}", nc, threads));
        let nr1 = self.ins(format!("sub i64 {}, 1", nr0));
        let rounds = self.ins(format!("sdiv i64 {}, {}", nr1, threads));
        // the count of chunk c (if c < nc)
        let count = |g: &mut Self, extra: &[V], c: &str| -> R<()> {
            let st = g.prepare_stream(x)?;
            let inside = g.ins(format!("icmp slt i64 {}, {}", c, extra[2].r));
            let lgo = g.label();
            let lend = g.label();
            g.cbr(&inside, &lgo, &lend);
            g.block(&lgo);
            let lo = g.ins(format!("mul i64 {}, {}", c, extra[1].r));
            let lim = g.ins(format!("add i64 {}, {}", lo, extra[1].r));
            let short = g.ins(format!("icmp slt i64 {}, {}", st.n, lim));
            let hi = g.ins(format!("select i1 {}, i64 {}, i64 {}", short, st.n, lim));
            let k = g.alloca("i64");
            g.emit(format!("store i64 0, ptr {}", k));
            g.for_range(&lo, &hi, 1, &mut |g, i| {
                let el = g.elem(&st.base, i, 1)?;
                g.bind(st.pred.params[0], el);
                let keep = g.ex(&st.pred.body, 1)?;
                let one = g.ins(format!("zext i1 {} to i64", keep.r));
                let a = g.ins(format!("load i64, ptr {}", k));
                let b = g.ins(format!("add i64 {}, {}", a, one));
                g.emit(format!("store i64 {}, ptr {}", b, k));
                Ok(())
            })?;
            let kv = g.ins(format!("load i64, ptr {}", k));
            let pc = g.ins(format!("getelementptr inbounds i64, ptr {}, i64 {}", extra[0].r, c));
            g.emit(format!("store i64 {}, ptr {}", kv, pc));
            g.br(&lend);
            g.block(&lend);
            Ok(())
        };
        let count = std::cell::RefCell::new(count);
        // round 0 counted first
        let extra = [v(counts.clone(), "ptr"), v(ch.clone(), "i64"), v(nc.clone(), "i64")];
        let (task, ctx, ctx_bytes) = self.outline_task(&[x], &extra, &mut |g, extra, j| (count.borrow_mut())(g, extra, j))?;
        let m0 = self.ins(format!("icmp slt i64 {}, {}", nc, threads));
        let m = self.ins(format!("select i1 {}, i64 {}, i64 {}", m0, nc, threads));
        self.emit(format!("call void @isc_parallel_for(i64 {}, ptr {}, ptr {}, i64 {})", m, task, ctx, ctx_bytes));
        self.for_range_scalar("0", &rounds, 1, &mut |g, r| {
            let c_lo = g.ins(format!("mul i64 {}, {}", r, threads));
            let lim = g.ins(format!("add i64 {}, {}", c_lo, threads));
            let short = g.ins(format!("icmp slt i64 {}, {}", nc, lim));
            let c_hi = g.ins(format!("select i1 {}, i64 {}, i64 {}", short, nc, lim));
            let m = g.ins(format!("sub i64 {}, {}", c_hi, c_lo));
            // the starts of the chunks of the round and of the next chunk
            g.for_range_scalar(&c_lo, &c_hi, 1, &mut |g, c| {
                let pc = g.ins(format!("getelementptr inbounds i64, ptr {}, i64 {}", counts, c));
                let k = g.ins(format!("load i64, ptr {}", pc));
                let r = g.ins(format!("load i64, ptr {}", run));
                let ps = g.ins(format!("getelementptr inbounds i64, ptr {}, i64 {}", starts, c));
                g.emit(format!("store i64 {}, ptr {}", r, ps));
                let r2 = g.ins(format!("add i64 {}, {}", r, k));
                g.emit(format!("store i64 {}, ptr {}", r2, run));
                Ok(())
            })?;
            let rr = g.ins(format!("load i64, ptr {}", run));
            let pn = g.ins(format!("getelementptr inbounds i64, ptr {}, i64 {}", starts, c_hi));
            g.emit(format!("store i64 {}, ptr {}", rr, pn));
            // the blocks of the chunks of the round, then the count of the
            // same chunk of the next round (its memory read while other
            // threads compute)
            let extra = [v(counts.clone(), "ptr"), v(ch.clone(), "i64"), v(nc.clone(), "i64"), v(starts.clone(), "ptr"), v(parts.clone(), "ptr"), zv.clone(), v(c_lo.clone(), "i64"), v(threads.to_string(), "i64")];
            let (task, ctx, ctx_bytes) = g.outline_task(&[x], &extra, &mut |g, extra, j| {
                let st = g.prepare_stream(x)?;
                let c = g.ins(format!("add i64 {}, {}", extra[6].r, j));
                let ps = g.ins(format!("getelementptr inbounds i64, ptr {}, i64 {}", extra[3].r, c));
                let s = g.ins(format!("load i64, ptr {}", ps));
                let c1 = g.ins(format!("add i64 {}, 1", c));
                let pe = g.ins(format!("getelementptr inbounds i64, ptr {}, i64 {}", extra[3].r, c1));
                let e = g.ins(format!("load i64, ptr {}", pe));
                // blocks b0 .. b1: those that start in s .. e; the last one
                // ends at its B-th element, or at the end of the inputs
                let s1 = g.ins(format!("add i64 {}, {}", s, BLOCK - 1));
                let b0 = g.ins(format!("sdiv i64 {}, {}", s1, BLOCK));
                let e1 = g.ins(format!("add i64 {}, {}", e, BLOCK - 1));
                let b1 = g.ins(format!("sdiv i64 {}, {}", e1, BLOCK));
                let any = g.ins(format!("icmp slt i64 {}, {}", b0, b1));
                let lgo = g.label();
                let lend = g.label();
                g.cbr(&any, &lgo, &lend);
                g.block(&lgo);
                let first = g.ins(format!("mul i64 {}, {}", b0, BLOCK));
                let skip = g.ins(format!("sub i64 {}, {}", first, s));
                let blocks = g.ins(format!("sub i64 {}, {}", b1, b0));
                let take = g.ins(format!("mul i64 {}, {}", blocks, BLOCK));
                let lo = g.ins(format!("mul i64 {}, {}", c, extra[1].r));
                let zv = extra[5].clone();
                let t2 = l.body.ty.clone();
                g.stream_blocks(&st, l, &zv, &t2, &lo, &skip, &take, &extra[4].r, &b0)?;
                g.br(&lend);
                g.block(&lend);
                let next = g.ins(format!("add i64 {}, {}", c, extra[7].r));
                (count.borrow_mut())(g, extra, &next)
            })?;
            g.emit(format!("call void @isc_parallel_for(i64 {}, ptr {}, ptr {}, i64 {})", m, task, ctx, ctx_bytes));
            Ok(())
        })?;
        let kk = self.ins(format!("load i64, ptr {}", run));
        let nb0 = self.ins(format!("add i64 {}, {}", kk, BLOCK - 1));
        let nb = self.ins(format!("sdiv i64 {}, {}", nb0, BLOCK));
        // (3) the block results: none, one, or their reduction
        let res = self.alloca(&alt);
        let one = self.ins(format!("icmp eq i64 {}, 1", nb));
        let lone = self.label();
        let lmore = self.label();
        let ljoin = self.label();
        self.cbr(&one, &lone, &lmore);
        self.block(&lone);
        let p0 = self.ins(format!("load {}, ptr {}", alt, parts));
        self.emit(format!("store {} {}, ptr {}", alt, p0, res));
        self.br(&ljoin);
        self.block(&lmore);
        let r = self.reduce_mat_seq(&parts, &nb, l, zv, t)?;
        self.emit(format!("store {} {}, ptr {}", alt, r.r, res));
        self.br(&ljoin);
        self.block(&ljoin);
        self.emit(format!("call void @isc_free(ptr {})", parts));
        self.emit(format!("call void @isc_free(ptr {})", counts));
        self.emit(format!("call void @isc_free(ptr {})", starts));
        let out = self.ins(format!("load {}, ptr {}", alt, res));
        Ok(v(out, alt))
    }

    /// The fixed form of section 10 over the `n` scalars at `p`, on the
    /// current thread: while there are more than B, the results of their
    /// blocks replace them (block j is read before p[j] is written: j < j B
    /// for j >= 1); then one `reduce_block`.
    fn reduce_mat_seq(&mut self, p: &str, n: &str, l: &Lambda, z: &V, t: &Type) -> R<V> {
        let w = if lam_ok(l) { self.opts.lanes } else { 1 };
        let len = self.alloca("i64");
        self.emit(format!("store i64 {}, ptr {}", n, len));
        let head = self.label();
        let lbody = self.label();
        let lexit = self.label();
        self.br(&head);
        self.block(&head);
        let m = self.ins(format!("load i64, ptr {}", len));
        let big = self.ins(format!("icmp sgt i64 {}, {}", m, BLOCK));
        self.cbr(&big, &lbody, &lexit);
        self.block(&lbody);
        let c1 = self.ins(format!("add i64 {}, {}", m, BLOCK - 1));
        let c = self.ins(format!("sdiv i64 {}, {}", c1, BLOCK));
        let src = Src::Mat { ptr: p.to_string(), len: m.clone(), elem: t.clone() };
        let bl = BLOCK.to_string();
        self.for_range_scalar("0", &c, 1, &mut |g, j| {
            let start = g.ins(format!("mul i64 {}, {}", j, BLOCK));
            let rem = g.ins(format!("sub i64 {}, {}", m, start));
            let lt_ = g.ins(format!("icmp slt i64 {}, {}", rem, bl));
            let mm = g.ins(format!("select i1 {}, i64 {}, i64 {}", lt_, rem, bl));
            let r = g.reduce_block(&src, &start, &mm, l, z, t, w)?;
            g.store_elem(p, &c, j, t, &r, 1);
            Ok(())
        })?;
        self.emit(format!("store i64 {}, ptr {}", c, len));
        self.br(&head);
        self.block(&lexit);
        let m = self.ins(format!("load i64, ptr {}", len));
        let src = Src::Mat { ptr: p.to_string(), len: m.clone(), elem: t.clone() };
        self.reduce_block(&src, "0", &m, l, z, t, w)
    }

    /// The blocks of kept elements of indices first .. first + take, where
    /// first is the index of the (skip + 1)-th kept element from input `lo`:
    /// their results in parts[b0 ..] (the last one may be partial at the end
    /// of the kept elements). As `stream_vector`: compressed by chunks of
    /// COMPACT inputs, added by groups of V, the last ones one at a time
    /// (D120).
    #[allow(clippy::too_many_arguments)]
    fn stream_blocks(&mut self, st: &Stream, l: &Lambda, z: &V, t: &Type, lo: &str, skip: &str, take: &str, parts: &str, b0: &str) -> R<()> {
        const COMPACT: i64 = 1024;
        let lanes = self.opts.lanes;
        let alt = lt(t);
        let cw = (64 / esize(t)).max(1) as u32;
        let vty = format!("<{} x {}>", lanes, alt);
        let cty = format!("<{} x {}>", cw, alt);
        let mty = format!("<{} x i1>", cw);
        let buf = self.alloca(&format!("[{} x {}]", COMPACT + 2 * lanes as i64 + cw as i64, alt));
        let tree = self.alloca(&format!("[{} x {}]", STREAM_LEVELS * lanes as i64, alt));
        let accv = self.alloca(&vty);
        let zw = self.widen(z, lanes);
        self.emit(format!("store {} {}, ptr {}", vty, zw.r, accv));
        let slot = |g: &mut Self, init: &str| -> String {
            let a = g.alloca("i64");
            g.emit(format!("store i64 {}, ptr {}", init, a));
            a
        };
        let fill = slot(self, "0");
        let blk = slot(self, "0");
        let bi = slot(self, b0);
        let taken = slot(self, "0");
        let left = slot(self, skip);
        let pos = slot(self, lo);
        let cname = format!("llvm.experimental.vector.compress.v{}{}", cw, ty_suffix(t));
        self.decls.insert(format!("declare {} @{}({}, {}, {})", cty, cname, cty, mty, cty));
        let pop = format!("llvm.ctpop.i{}", cw);
        self.decls.insert(format!("declare i{} @{}(i{})", cw, pop, cw));
        let n = st.n.clone();
        let nfull = self.ins(format!("and i64 {}, -{}", n, cw));
        // the block in the accumulators closed: its tree to parts[bi]
        let close = |g: &mut Self| -> R<()> {
            let av = g.ins(format!("load {}, ptr {}", vty, accv));
            g.emit(format!("store {} {}, ptr {}, align {}", vty, av, tree, esize(t)));
            let r = g.rs_tree(&tree, "0", l, &alt)?;
            let b = g.ins(format!("load i64, ptr {}", bi));
            let pb = g.ins(format!("getelementptr inbounds {}, ptr {}, i64 {}", alt, parts, b));
            g.emit(format!("store {} {}, ptr {}", alt, r.r, pb));
            let b2 = g.ins(format!("add i64 {}, 1", b));
            g.emit(format!("store i64 {}, ptr {}", b2, bi));
            let zw = g.widen(z, lanes);
            g.emit(format!("store {} {}, ptr {}", vty, zw.r, accv));
            g.emit(format!("store i64 0, ptr {}", blk));
            Ok(())
        };
        let close = std::cell::RefCell::new(close);
        // up to `count` groups of V from the front of the buffer
        let groups = |g: &mut Self, count: &str| -> R<()> {
            g.for_range_scalar("0", count, 1, &mut |g, j| {
                let b = g.ins(format!("load i64, ptr {}", blk));
                let full = g.ins(format!("icmp eq i64 {}, {}", b, BLOCK));
                let lc = g.label();
                let lg = g.label();
                g.cbr(&full, &lc, &lg);
                g.block(&lc);
                (close.borrow_mut())(g)?;
                g.br(&lg);
                g.block(&lg);
                let off = g.ins(format!("mul i64 {}, {}", j, lanes));
                let pg = g.ins(format!("getelementptr inbounds {}, ptr {}, i64 {}", alt, buf, off));
                let gv = g.ins(format!("load {}, ptr {}, align {}", vty, pg, esize(t)));
                let a = g.ins(format!("load {}, ptr {}", vty, accv));
                let r = g.apply2(l, V { r: a, t: vty.clone(), uni: false }, V { r: gv, t: vty.clone(), uni: false }, lanes)?;
                g.emit(format!("store {} {}, ptr {}", vty, r.r, accv));
                let b = g.ins(format!("load i64, ptr {}", blk));
                let b2 = g.ins(format!("add i64 {}, {}", b, lanes));
                g.emit(format!("store i64 {}, ptr {}", b2, blk));
                Ok(())
            })
        };
        let groups = std::cell::RefCell::new(groups);
        // drops the first min(fill, left) elements of the buffer
        let drop_front = |g: &mut Self| -> R<()> {
            let f = g.ins(format!("load i64, ptr {}", fill));
            let lf = g.ins(format!("load i64, ptr {}", left));
            let less = g.ins(format!("icmp ult i64 {}, {}", f, lf));
            let d = g.ins(format!("select i1 {}, i64 {}, i64 {}", less, f, lf));
            let rest = g.ins(format!("sub i64 {}, {}", f, d));
            let some = g.ins(format!("icmp ugt i64 {}, 0", d));
            let lm = g.label();
            let ln = g.label();
            g.cbr(&some, &lm, &ln);
            g.block(&lm);
            let nv0 = g.ins(format!("add i64 {}, {}", rest, lanes - 1));
            let nv = g.ins(format!("udiv i64 {}, {}", nv0, lanes));
            g.for_range_scalar("0", &nv, 1, &mut |g, j| {
                let off = g.ins(format!("mul i64 {}, {}", j, lanes));
                let from = g.ins(format!("add i64 {}, {}", off, d));
                let pf = g.ins(format!("getelementptr inbounds {}, ptr {}, i64 {}", alt, buf, from));
                let pt = g.ins(format!("getelementptr inbounds {}, ptr {}, i64 {}", alt, buf, off));
                let x = g.ins(format!("load {}, ptr {}, align {}", vty, pf, esize(t)));
                g.emit(format!("store {} {}, ptr {}, align {}", vty, x, pt, esize(t)));
                Ok(())
            })?;
            g.emit(format!("store i64 {}, ptr {}", rest, fill));
            let lf2 = g.ins(format!("sub i64 {}, {}", lf, d));
            g.emit(format!("store i64 {}, ptr {}", lf2, left));
            g.br(&ln);
            g.block(&ln);
            Ok(())
        };
        // by chunks of COMPACT inputs while kept elements are still wanted
        let head = self.label();
        let lbody = self.label();
        let lexit = self.label();
        self.br(&head);
        self.block(&head);
        let p = self.ins(format!("load i64, ptr {}", pos));
        let more = self.ins(format!("icmp slt i64 {}, {}", p, nfull));
        let tk = self.ins(format!("load i64, ptr {}", taken));
        let want = self.ins(format!("icmp slt i64 {}, {}", tk, take));
        let go = self.ins(format!("and i1 {}, {}", more, want));
        self.cbr(&go, &lbody, &lexit);
        self.block(&lbody);
        let lim = self.ins(format!("add i64 {}, {}", p, COMPACT));
        let short = self.ins(format!("icmp slt i64 {}, {}", nfull, lim));
        let c1 = self.ins(format!("select i1 {}, i64 {}, i64 {}", short, nfull, lim));
        self.for_range_scalar(&p, &c1, cw as i64, &mut |g, i| {
            let x = g.elem(&st.base, i, cw)?;
            let x = g.widen(&x, cw);
            g.bind(st.pred.params[0], x.clone());
            let keep = g.ex(&st.pred.body, cw)?;
            let keep = g.widen(&keep, cw);
            let mut y = x;
            for q in &st.posts {
                g.bind(q.params[0], y);
                let r = g.ex(&q.body, cw)?;
                y = g.widen(&r, cw);
            }
            let comp = g.ins(format!("call {} @{}({} {}, {} {}, {} poison)", cty, cname, cty, y.r, mty, keep.r, cty));
            let c = g.ins(format!("load i64, ptr {}", fill));
            let at = g.ins(format!("getelementptr inbounds {}, ptr {}, i64 {}", alt, buf, c));
            g.emit(format!("store {} {}, ptr {}, align {}", cty, comp, at, esize(t)));
            let bits = g.ins(format!("bitcast {} {} to i{}", mty, keep.r, cw));
            let k = g.ins(format!("call i{} @{}(i{} {})", cw, pop, cw, bits));
            let k64 = g.ins(format!("zext i{} {} to i64", cw, k));
            let c2 = g.ins(format!("add i64 {}, {}", c, k64));
            g.emit(format!("store i64 {}, ptr {}", c2, fill));
            Ok(())
        })?;
        self.emit(format!("store i64 {}, ptr {}", c1, pos));
        drop_front(self)?;
        let f = self.ins(format!("load i64, ptr {}", fill));
        let tk = self.ins(format!("load i64, ptr {}", taken));
        let want = self.ins(format!("sub i64 {}, {}", take, tk));
        let ng1 = self.ins(format!("udiv i64 {}, {}", f, lanes));
        let ng2 = self.ins(format!("udiv i64 {}, {}", want, lanes));
        let fewer = self.ins(format!("icmp ult i64 {}, {}", ng1, ng2));
        let ng = self.ins(format!("select i1 {}, i64 {}, i64 {}", fewer, ng1, ng2));
        (groups.borrow_mut())(self, &ng)?;
        let used = self.ins(format!("mul i64 {}, {}", ng, lanes));
        let tk2 = self.ins(format!("add i64 {}, {}", tk, used));
        self.emit(format!("store i64 {}, ptr {}", tk2, taken));
        // the fewer than V left to the front (when more are wanted)
        let pr = self.ins(format!("getelementptr inbounds {}, ptr {}, i64 {}", alt, buf, used));
        let rv = self.ins(format!("load {}, ptr {}, align {}", vty, pr, esize(t)));
        self.emit(format!("store {} {}, ptr {}, align {}", vty, rv, buf, esize(t)));
        let rest = self.ins(format!("sub i64 {}, {}", f, used));
        self.emit(format!("store i64 {}, ptr {}", rest, fill));
        self.br(&head);
        self.block(&lexit);
        // the last inputs (fewer than a line), one at a time, while kept
        // elements are still wanted
        let tk = self.ins(format!("load i64, ptr {}", taken));
        let want = self.ins(format!("icmp slt i64 {}, {}", tk, take));
        let ltail = self.label();
        let lfin = self.label();
        self.cbr(&want, &ltail, &lfin);
        self.block(&ltail);
        let p = self.ins(format!("load i64, ptr {}", pos));
        self.for_range_scalar(&p, &n, 1, &mut |g, i| {
            let x = g.elem(&st.base, i, 1)?;
            g.bind(st.pred.params[0], x.clone());
            let keep = g.ex(&st.pred.body, 1)?;
            let ly = g.label();
            let ln = g.label();
            g.cbr(&keep.r, &ly, &ln);
            g.block(&ly);
            let lf = g.ins(format!("load i64, ptr {}", left));
            let skipping = g.ins(format!("icmp ugt i64 {}, 0", lf));
            let lsk = g.label();
            let ladd = g.label();
            g.cbr(&skipping, &lsk, &ladd);
            g.block(&lsk);
            let lf2 = g.ins(format!("sub i64 {}, 1", lf));
            g.emit(format!("store i64 {}, ptr {}", lf2, left));
            g.br(&ln);
            g.block(&ladd);
            let c = g.ins(format!("load i64, ptr {}", fill));
            let tk = g.ins(format!("load i64, ptr {}", taken));
            let have = g.ins(format!("add i64 {}, {}", tk, c));
            let room = g.ins(format!("icmp slt i64 {}, {}", have, take));
            let lput = g.label();
            g.cbr(&room, &lput, &ln);
            g.block(&lput);
            let mut y = x;
            for q in &st.posts {
                g.bind(q.params[0], y);
                y = g.ex(&q.body, 1)?;
            }
            let at = g.ins(format!("getelementptr inbounds {}, ptr {}, i64 {}", alt, buf, c));
            g.emit(format!("store {} {}, ptr {}", alt, y.r, at));
            let c2 = g.ins(format!("add i64 {}, 1", c));
            g.emit(format!("store i64 {}, ptr {}", c2, fill));
            g.br(&ln);
            g.block(&ln);
            Ok(())
        })?;
        // the buffer (fewer than V + a line): its groups, then one at a time
        let f = self.ins(format!("load i64, ptr {}", fill));
        let ng = self.ins(format!("udiv i64 {}, {}", f, lanes));
        (groups.borrow_mut())(self, &ng)?;
        let used = self.ins(format!("mul i64 {}, {}", ng, lanes));
        let c = self.ins(format!("sub i64 {}, {}", f, used));
        let some = self.ins(format!("icmp ugt i64 {}, 0", c));
        let b = self.ins(format!("load i64, ptr {}", blk));
        let full = self.ins(format!("icmp eq i64 {}, {}", b, BLOCK));
        let both = self.ins(format!("and i1 {}, {}", some, full));
        let lc = self.label();
        let lg = self.label();
        self.cbr(&both, &lc, &lg);
        self.block(&lc);
        (close.borrow_mut())(self)?;
        self.br(&lg);
        self.block(&lg);
        let at = format!("[{} x {}]", lanes, alt);
        self.for_range_scalar("0", &c, 1, &mut |g, j| {
            let pa = g.ins(format!("getelementptr inbounds {}, ptr {}, i64 0, i64 {}", at, accv, j));
            let from = g.ins(format!("add i64 {}, {}", used, j));
            let pb = g.ins(format!("getelementptr inbounds {}, ptr {}, i64 {}", alt, buf, from));
            let a = g.ins(format!("load {}, ptr {}", alt, pa));
            let bv = g.ins(format!("load {}, ptr {}", alt, pb));
            let r = g.apply2(l, v(a, alt.clone()), v(bv, alt.clone()), 1)?;
            g.emit(format!("store {} {}, ptr {}", alt, r.r, pa));
            Ok(())
        })?;
        let b = self.ins(format!("load i64, ptr {}", blk));
        let b2 = self.ins(format!("add i64 {}, {}", b, c));
        self.emit(format!("store i64 {}, ptr {}", b2, blk));
        self.br(&lfin);
        self.block(&lfin);
        // the last block of this chunk
        let b = self.ins(format!("load i64, ptr {}", blk));
        let open = self.ins(format!("icmp sgt i64 {}, 0", b));
        let lc = self.label();
        let ld = self.label();
        self.cbr(&open, &lc, &ld);
        self.block(&lc);
        (close.borrow_mut())(self)?;
        self.br(&ld);
        self.block(&ld);
        Ok(())
    }

    /// Level 0 of `reduce_stream` with vectors (D110): each line of `cw`
    /// elements is compared at once and its kept elements compressed (in
    /// order) at the end of a buffer; each group of V kept elements is then
    /// added, as one vector, to the V accumulators, which is the order of
    /// section 10 (the k-th kept element in accumulator k mod V, since a
    /// block holds a whole number of groups). A full block is closed, as in
    /// `rs_push`, when the next group comes; the last elements go one at a
    /// time. The accumulators and the count end in level 0 of `acc`/`cnt`.
    #[allow(clippy::too_many_arguments)]
    fn stream_vector(&mut self, st: &Stream, l: &Lambda, z: &V, accm: &str, cnt: &str, t: &Type) -> R<()> {
        let lanes = self.opts.lanes;
        let alt = lt(t);
        let cw = (64 / esize(t)).max(1) as u32;
        let vty = format!("<{} x {}>", lanes, alt);
        let cty = format!("<{} x {}>", cw, alt);
        let mty = format!("<{} x i1>", cw);
        // room for a chunk of COMPACT inputs after fewer than V kept ones,
        // and for the read of V elements from the last full group
        const COMPACT: i64 = 1024;
        let buf = self.alloca(&format!("[{} x {}]", COMPACT + 2 * lanes as i64 + cw as i64, alt));
        let accv = self.alloca(&vty);
        let zw = self.widen(z, lanes);
        self.emit(format!("store {} {}, ptr {}", vty, zw.r, accv));
        let fill = self.alloca("i64");
        self.emit(format!("store i64 0, ptr {}", fill));
        let blk = self.alloca("i64");
        self.emit(format!("store i64 0, ptr {}", blk));
        let cname = format!("llvm.experimental.vector.compress.v{}{}", cw, ty_suffix(t));
        self.decls.insert(format!("declare {} @{}({}, {}, {})", cty, cname, cty, mty, cty));
        let pop = format!("llvm.ctpop.i{}", cw);
        self.decls.insert(format!("declare i{} @{}(i{})", cw, pop, cw));
        let n = st.n.clone();
        let nfull = self.ins(format!("and i64 {}, -{}", n, cw));
        // a group of V kept elements to the accumulators (the block closed
        // first when it is full)
        let group = |g: &mut Self| -> R<()> {
            let b = g.ins(format!("load i64, ptr {}", blk));
            let full = g.ins(format!("icmp eq i64 {}, {}", b, BLOCK));
            let lclose = g.label();
            let lgo = g.label();
            g.cbr(&full, &lclose, &lgo);
            g.block(&lclose);
            g.stream_close(&accv, accm, cnt, &blk, l, z, t)?;
            g.br(&lgo);
            g.block(&lgo);
            let gv = g.ins(format!("load {}, ptr {}, align {}", vty, buf, esize(t)));
            let a = g.ins(format!("load {}, ptr {}", vty, accv));
            let r = g.apply2(l, V { r: a, t: vty.clone(), uni: false }, V { r: gv, t: vty.clone(), uni: false }, lanes)?;
            g.emit(format!("store {} {}, ptr {}", vty, r.r, accv));
            let b = g.ins(format!("load i64, ptr {}", blk));
            let b2 = g.ins(format!("add i64 {}, {}", b, lanes));
            g.emit(format!("store i64 {}, ptr {}", b2, blk));
            // the kept elements beyond the group to the front (fewer than cw)
            let rest = g.ins(format!("getelementptr inbounds {}, ptr {}, i64 {}", alt, buf, lanes));
            let rv = g.ins(format!("load {}, ptr {}, align {}", cty, rest, esize(t)));
            g.emit(format!("store {} {}, ptr {}, align {}", cty, rv, buf, esize(t)));
            let c = g.ins(format!("load i64, ptr {}", fill));
            let c2 = g.ins(format!("sub i64 {}, {}", c, lanes));
            g.emit(format!("store i64 {}, ptr {}", c2, fill));
            Ok(())
        };
        let group = std::cell::RefCell::new(group);
        // by chunks of COMPACT inputs: their kept elements compressed into
        // the buffer, without a branch, then every full group of V added to
        // the accumulators (a branch only to close a block, every B / V
        // groups), the fewer than V left moved to the front; the kept
        // elements reach the accumulators in the same order as one at a
        // time (measured: 32 768 `f64`, 8 083 ns per call with a test after
        // each compress, D119)
        // (by chunk numbers: a loop of step COMPACT would skip the last,
        // partial chunk)
        let nc1 = self.ins(format!("add i64 {}, {}", nfull, COMPACT - 1));
        let nchunks = self.ins(format!("udiv i64 {}, {}", nc1, COMPACT));
        self.for_range_scalar("0", &nchunks, 1, &mut |g, ci| {
            let c0 = g.ins(format!("mul i64 {}, {}", ci, COMPACT));
            let c0 = c0.as_str();
            let lim = g.ins(format!("add i64 {}, {}", c0, COMPACT));
            let short = g.ins(format!("icmp slt i64 {}, {}", nfull, lim));
            let c1 = g.ins(format!("select i1 {}, i64 {}, i64 {}", short, nfull, lim));
            g.for_range_scalar(c0, &c1, cw as i64, &mut |g, i| {
                let x = g.elem(&st.base, i, cw)?;
                let x = g.widen(&x, cw);
                g.bind(st.pred.params[0], x.clone());
                let keep = g.ex(&st.pred.body, cw)?;
                let keep = g.widen(&keep, cw);
                let mut y = x;
                for p in &st.posts {
                    g.bind(p.params[0], y);
                    let r = g.ex(&p.body, cw)?;
                    y = g.widen(&r, cw);
                }
                let comp = g.ins(format!("call {} @{}({} {}, {} {}, {} poison)", cty, cname, cty, y.r, mty, keep.r, cty));
                let c = g.ins(format!("load i64, ptr {}", fill));
                let at = g.ins(format!("getelementptr inbounds {}, ptr {}, i64 {}", alt, buf, c));
                g.emit(format!("store {} {}, ptr {}, align {}", cty, comp, at, esize(t)));
                let bits = g.ins(format!("bitcast {} {} to i{}", mty, keep.r, cw));
                let k = g.ins(format!("call i{} @{}(i{} {})", cw, pop, cw, bits));
                let k64 = g.ins(format!("zext i{} {} to i64", cw, k));
                let c2 = g.ins(format!("add i64 {}, {}", c, k64));
                g.emit(format!("store i64 {}, ptr {}", c2, fill));
                Ok(())
            })?;
            let f = g.ins(format!("load i64, ptr {}", fill));
            let ng = g.ins(format!("udiv i64 {}, {}", f, lanes));
            g.for_range_scalar("0", &ng, 1, &mut |g, j| {
                let b = g.ins(format!("load i64, ptr {}", blk));
                let full = g.ins(format!("icmp eq i64 {}, {}", b, BLOCK));
                let lclose = g.label();
                let lgo = g.label();
                g.cbr(&full, &lclose, &lgo);
                g.block(&lclose);
                g.stream_close(&accv, accm, cnt, &blk, l, z, t)?;
                g.br(&lgo);
                g.block(&lgo);
                let off = g.ins(format!("mul i64 {}, {}", j, lanes));
                let pg = g.ins(format!("getelementptr inbounds {}, ptr {}, i64 {}", alt, buf, off));
                let gv = g.ins(format!("load {}, ptr {}, align {}", vty, pg, esize(t)));
                let a = g.ins(format!("load {}, ptr {}", vty, accv));
                let r = g.apply2(l, V { r: a, t: vty.clone(), uni: false }, V { r: gv, t: vty.clone(), uni: false }, lanes)?;
                g.emit(format!("store {} {}, ptr {}", vty, r.r, accv));
                let b = g.ins(format!("load i64, ptr {}", blk));
                let b2 = g.ins(format!("add i64 {}, {}", b, lanes));
                g.emit(format!("store i64 {}, ptr {}", b2, blk));
                Ok(())
            })?;
            // the fewer than V left to the front
            let used = g.ins(format!("mul i64 {}, {}", ng, lanes));
            let pr = g.ins(format!("getelementptr inbounds {}, ptr {}, i64 {}", alt, buf, used));
            let rv = g.ins(format!("load {}, ptr {}, align {}", vty, pr, esize(t)));
            g.emit(format!("store {} {}, ptr {}, align {}", vty, rv, buf, esize(t)));
            let left = g.ins(format!("sub i64 {}, {}", f, used));
            g.emit(format!("store i64 {}, ptr {}", left, fill));
            Ok(())
        })?;
        // the last elements, one at a time, into the buffer
        self.for_range_scalar(&nfull, &n, 1, &mut |g, i| {
            let x = g.elem(&st.base, i, 1)?;
            g.bind(st.pred.params[0], x.clone());
            let keep = g.ex(&st.pred.body, 1)?;
            let ly = g.label();
            let ln = g.label();
            g.cbr(&keep.r, &ly, &ln);
            g.block(&ly);
            let mut y = x;
            for p in &st.posts {
                g.bind(p.params[0], y);
                y = g.ex(&p.body, 1)?;
            }
            let c = g.ins(format!("load i64, ptr {}", fill));
            let at = g.ins(format!("getelementptr inbounds {}, ptr {}, i64 {}", alt, buf, c));
            g.emit(format!("store {} {}, ptr {}", alt, y.r, at));
            let c2 = g.ins(format!("add i64 {}, 1", c));
            g.emit(format!("store i64 {}, ptr {}", c2, fill));
            let enough = g.ins(format!("icmp uge i64 {}, {}", c2, lanes));
            let lg = g.label();
            let lc = g.label();
            g.cbr(&enough, &lg, &lc);
            g.block(&lg);
            (group.borrow_mut())(g)?;
            g.br(&lc);
            g.block(&lc);
            g.br(&ln);
            g.block(&ln);
            Ok(())
        })?;
        // fewer than V left: a full block is closed first, then each one to
        // its accumulator
        let c = self.ins(format!("load i64, ptr {}", fill));
        let some = self.ins(format!("icmp ugt i64 {}, 0", c));
        let b = self.ins(format!("load i64, ptr {}", blk));
        let full = self.ins(format!("icmp eq i64 {}, {}", b, BLOCK));
        let both = self.ins(format!("and i1 {}, {}", some, full));
        let lclose = self.label();
        let lgo = self.label();
        self.cbr(&both, &lclose, &lgo);
        self.block(&lclose);
        self.stream_close(&accv, accm, cnt, &blk, l, z, t)?;
        self.br(&lgo);
        self.block(&lgo);
        let at = format!("[{} x {}]", lanes, alt);
        self.for_range_scalar("0", &c, 1, &mut |g, j| {
            let pa = g.ins(format!("getelementptr inbounds {}, ptr {}, i64 0, i64 {}", at, accv, j));
            let pb = g.ins(format!("getelementptr inbounds {}, ptr {}, i64 {}", alt, buf, j));
            let a = g.ins(format!("load {}, ptr {}", alt, pa));
            let bv = g.ins(format!("load {}, ptr {}", alt, pb));
            let r = g.apply2(l, v(a, alt.clone()), v(bv, alt.clone()), 1)?;
            g.emit(format!("store {} {}, ptr {}", alt, r.r, pa));
            Ok(())
        })?;
        let b = self.ins(format!("load i64, ptr {}", blk));
        let b2 = self.ins(format!("add i64 {}, {}", b, c));
        // level 0: the accumulators and the count of the open block
        let av = self.ins(format!("load {}, ptr {}", vty, accv));
        self.emit(format!("store {} {}, ptr {}, align {}", vty, av, accm, esize(t)));
        let pc = self.ins(format!("getelementptr inbounds [{} x i64], ptr {}, i64 0, i64 0", STREAM_LEVELS, cnt));
        self.emit(format!("store i64 {}, ptr {}", b2, pc));
        Ok(())
    }

    /// Closes the full block held in the vector of accumulators `accv`: its
    /// tree (through level 0 of `accm`) goes to level 1, then `accv` starts
    /// again from `z`.
    #[allow(clippy::too_many_arguments)]
    fn stream_close(&mut self, accv: &str, accm: &str, cnt: &str, blk: &str, l: &Lambda, z: &V, t: &Type) -> R<()> {
        let lanes = self.opts.lanes;
        let alt = lt(t);
        let vty = format!("<{} x {}>", lanes, alt);
        let av = self.ins(format!("load {}, ptr {}", vty, accv));
        // `accm` is an array of scalars: aligned to one element only
        self.emit(format!("store {} {}, ptr {}, align {}", vty, av, accm, esize(t)));
        let r = self.rs_tree(accm, "0", l, &alt)?;
        self.rs_push(accm, cnt, "1", r, l, z, &alt)?;
        let zw = self.widen(z, lanes);
        self.emit(format!("store {} {}, ptr {}", vty, zw.r, accv));
        self.emit(format!("store i64 0, ptr {}", blk));
        Ok(())
    }

    /// The accumulators of level `lv` set to `z`, its count to 0.
    fn rs_reset(&mut self, acc: &str, cnt: &str, lv: &str, z: &V, alt: &str) {
        let lanes = self.opts.lanes as i64;
        let base = self.ins(format!("mul i64 {}, {}", lv, lanes));
        let total = STREAM_LEVELS * lanes;
        let at = format!("[{} x {}]", total, alt);
        let _ = self.for_range_scalar("0", &lanes.to_string(), 1, &mut |g, j| {
            let k = g.ins(format!("add i64 {}, {}", base, j));
            let p = g.ins(format!("getelementptr inbounds {}, ptr {}, i64 0, i64 {}", at, acc, k));
            g.emit(format!("store {} {}, ptr {}", alt, z.r, p));
            Ok(())
        });
        let pc = self.ins(format!("getelementptr inbounds [{} x i64], ptr {}, i64 0, i64 {}", STREAM_LEVELS, cnt, lv));
        self.emit(format!("store i64 0, ptr {}", pc));
    }

    /// The tree of section 10 over the V accumulators of level `lv`:
    /// acc[j] = op(acc[j], acc[j + d]) for d = V/2, ..., 1; returns acc[0].
    fn rs_tree(&mut self, acc: &str, lv: &str, l: &Lambda, alt: &str) -> R<V> {
        let lanes = self.opts.lanes as i64;
        let base = self.ins(format!("mul i64 {}, {}", lv, lanes));
        let at = format!("[{} x {}]", STREAM_LEVELS * lanes, alt);
        let steps = (lanes as u64).trailing_zeros().to_string();
        let half = (lanes / 2).to_string();
        let alts = alt.to_string();
        self.for_range_scalar("0", &steps, 1, &mut |g, s| {
            let d = g.ins(format!("lshr i64 {}, {}", half, s));
            g.for_range_scalar("0", &d, 1, &mut |g, j| {
                let k = g.ins(format!("add i64 {}, {}", base, j));
                let kd = g.ins(format!("add i64 {}, {}", k, d));
                let pj = g.ins(format!("getelementptr inbounds {}, ptr {}, i64 0, i64 {}", at, acc, k));
                let pk = g.ins(format!("getelementptr inbounds {}, ptr {}, i64 0, i64 {}", at, acc, kd));
                let a = g.ins(format!("load {}, ptr {}", alts, pj));
                let b = g.ins(format!("load {}, ptr {}", alts, pk));
                let r = g.apply2(l, v(a, alts.clone()), v(b, alts.clone()), 1)?;
                g.emit(format!("store {} {}, ptr {}", alts, r.r, pj));
                Ok(())
            })
        })?;
        let p0 = self.ins(format!("getelementptr inbounds {}, ptr {}, i64 0, i64 {}", at, acc, base));
        let r = self.ins(format!("load {}, ptr {}", alt, p0));
        Ok(v(r, alt.to_string()))
    }

    /// Adds `x` to level `lv` (accumulator count mod V), without closing it.
    fn rs_add(&mut self, acc: &str, cnt: &str, lv: &str, x: V, l: &Lambda, alt: &str) -> R<()> {
        let lanes = self.opts.lanes as i64;
        let at = format!("[{} x {}]", STREAM_LEVELS * lanes, alt);
        let pc = self.ins(format!("getelementptr inbounds [{} x i64], ptr {}, i64 0, i64 {}", STREAM_LEVELS, cnt, lv));
        let c = self.ins(format!("load i64, ptr {}", pc));
        let lane = self.ins(format!("and i64 {}, {}", c, lanes - 1));
        let base = self.ins(format!("mul i64 {}, {}", lv, lanes));
        let k = self.ins(format!("add i64 {}, {}", base, lane));
        let p = self.ins(format!("getelementptr inbounds {}, ptr {}, i64 0, i64 {}", at, acc, k));
        let a = self.ins(format!("load {}, ptr {}", alt, p));
        let r = self.apply2(l, v(a, alt.to_string()), x, 1)?;
        self.emit(format!("store {} {}, ptr {}", alt, r.r, p));
        let c2 = self.ins(format!("add i64 {}, 1", c));
        self.emit(format!("store i64 {}, ptr {}", c2, pc));
        Ok(())
    }

    /// Adds `x` to level `start`; a full level (B elements) is closed first:
    /// its result goes to the level above, the same way.
    #[allow(clippy::too_many_arguments)]
    fn rs_push(&mut self, acc: &str, cnt: &str, start: &str, x: V, l: &Lambda, z: &V, alt: &str) -> R<()> {
        let lvl = self.alloca("i64");
        let cur = self.alloca(alt);
        self.emit(format!("store i64 {}, ptr {}", start, lvl));
        self.emit(format!("store {} {}, ptr {}", alt, x.r, cur));
        let head = self.label();
        let close = self.label();
        let add = self.label();
        let done = self.label();
        self.br(&head);
        self.block(&head);
        let lv = self.ins(format!("load i64, ptr {}", lvl));
        let pc = self.ins(format!("getelementptr inbounds [{} x i64], ptr {}, i64 0, i64 {}", STREAM_LEVELS, cnt, lv));
        let c = self.ins(format!("load i64, ptr {}", pc));
        let full = self.ins(format!("icmp eq i64 {}, {}", c, BLOCK));
        let below_top = self.ins(format!("icmp slt i64 {}, {}", lv, STREAM_LEVELS - 1));
        let both = self.ins(format!("and i1 {}, {}", full, below_top));
        self.cbr(&both, &close, &add);
        self.block(&close);
        let r = self.rs_tree(acc, &lv, l, alt)?;
        self.rs_reset(acc, cnt, &lv, z, alt);
        let xv = self.ins(format!("load {}, ptr {}", alt, cur));
        self.rs_add(acc, cnt, &lv, v(xv, alt.to_string()), l, alt)?;
        self.emit(format!("store {} {}, ptr {}", alt, r.r, cur));
        let lv1 = self.ins(format!("add i64 {}, 1", lv));
        self.emit(format!("store i64 {}, ptr {}", lv1, lvl));
        self.br(&head);
        self.block(&add);
        let xv = self.ins(format!("load {}, ptr {}", alt, cur));
        self.rs_add(acc, cnt, &lv, v(xv, alt.to_string()), l, alt)?;
        self.br(&done);
        self.block(&done);
        Ok(())
    }

    /// A `fold` of rule P (`additive_update`): one contiguous chunk of `x` per
    /// thread, each accumulated into four interleaved private zero copies
    /// (independent dependency chains when values repeat), merged into the
    /// task's copy; the task copies are then added to `init` in place.
    #[allow(clippy::too_many_arguments)]
    fn fold_private(&mut self, e: &Expr, x: &Expr, init: &Expr, l: &Lambda, gi: &Expr, h: &Expr, span: Span) -> R<V> {
        // 8 private copies per task: with a skewed input (the same few
        // counters again and again) each copy's chain of dependent increments
        // is half as long as with 4, and the processor overlaps them; on the
        // skewed histogram 4.4 % faster than with 4 copies, the same speed on
        // a uniform input (DECISIONS D78)
        const K: usize = 8;
        let et = init.ty.elem().clone();
        let it = lt(&et);
        let bytes_per = esize(&et);
        let (_, n) = self.prepare_src(x)?;
        let iv = self.ex(init, 1)?;
        let ip = self.ins(format!("extractvalue {{ ptr, i64 }} {}, 0", iv.r));
        let len = self.ins(format!("extractvalue {{ ptr, i64 }} {}, 1", iv.r));
        // tasks = min(threads, max(1, ceil(n / B)))
        let thr = self.ins("call i64 @isc_thread_count()".into());
        let n1 = self.ins(format!("add i64 {}, {}", n, BLOCK - 1));
        let nb = self.ins(format!("sdiv i64 {}, {}", n1, BLOCK));
        let nb1 = self.ins(format!("call i64 @llvm.smax.i64(i64 {}, i64 1)", nb));
        self.decls.insert("declare i64 @llvm.smax.i64(i64, i64)".into());
        self.decls.insert("declare i64 @llvm.smin.i64(i64, i64)".into());
        let tasks = self.ins(format!("call i64 @llvm.smin.i64(i64 {}, i64 {})", thr, nb1));
        // The K copies of a task in one block, `stride` bytes apart: at least
        // the size of a copy and (4096 / K) times an odd number, so that the
        // same counter of two copies is never a multiple of 4096 bytes apart.
        // Otherwise both have the same 12 low address bits, the processor
        // makes each load of one copy wait for the pending stores to the
        // other ("4K aliasing"), and the independent chains of the copies
        // become one (DECISIONS D64). Computed in the IR, so that LLVM folds
        // it when the length is a constant. The blocks of all the tasks are
        // allocated here at once (D77): a task never allocates, so the
        // threads of the pool never create a memory arena of their own.
        let bytes = self.ins(format!("mul i64 {}, {}", len, bytes_per));
        let unit = 4096 / K as i64;
        let m = self.ins(format!("add i64 {}, {}", bytes, unit - 1));
        let m = self.ins(format!("udiv i64 {}, {}", m, unit));
        let m = self.ins(format!("or i64 {}, 1", m));
        let stride = self.ins(format!("mul i64 {}, {}", m, unit));
        // The block of a task: its K copies, then its failure record
        // { failed (0 or 1), the index out of bounds }, in whole pages of
        // 4096 bytes, aligned on a page (DECISIONS D78). The L2 streamer of a
        // core prefetches ahead up to the end of the page: it must never
        // fetch a line of the block of another thread, whose counters would
        // then move between the cores; and no two tasks write the same line.
        let copies = self.ins(format!("mul i64 {}, {}", stride, K));
        let need = self.ins(format!("add i64 {}, 4111", copies));
        let per_task = self.ins(format!("and i64 {}, -4096", need));
        let all_bytes = self.ins(format!("mul i64 {}, {}", per_task, tasks));
        let all_bytes = self.ins(format!("add i64 {}, 4096", all_bytes));
        let raw = self.ins(format!("call ptr @isc_alloc(i64 {})", all_bytes));
        let addr = self.ins(format!("ptrtoint ptr {} to i64", raw));
        let pad = self.ins(format!("sub i64 0, {}", addr));
        let pad = self.ins(format!("and i64 {}, 4095", pad));
        let all = self.ins(format!("getelementptr inbounds i8, ptr {}, i64 {}", raw, pad));
        let p_v = l.params[1];
        let extra = [
            v(tasks.clone(), "i64"),
            v(len.clone(), "i64"),
            v(all.clone(), "ptr"),
            v(stride.clone(), "i64"),
            v(per_task.clone(), "i64"),
        ];
        // The number of counters, when the initial value says it (`zeros()`
        // of a sized type, an array literal): then the task knows it as a
        // constant, and LLVM addresses the copies from one register with
        // constant offsets and keeps only the loop without checks.
        let const_len: Option<i64> = match &init.kind {
            ExprKind::Zeros(n) => match n.kind {
                ExprKind::Lit(Lit::Int(c)) if c >= 0 => Some(c),
                _ => None,
            },
            ExprKind::ArrayLit(es) => Some(es.len() as i64),
            ExprKind::ConstArray(ls) => Some(ls.len() as i64),
            _ => None,
        };
        // 4 rounds of the K copies per iteration: 32 elements, so that the
        // instructions of the loop itself are a small part of the work
        // (measured against 8, 16 and 64 elements, DECISIONS D78)
        let unroll: usize = 4;
        let (task, ctx, ctx_bytes) = self.outline_task(&[e], &extra, &mut |g, ex, t| {
            let (tasks, all) = (ex[0].r.clone(), ex[2].r.clone());
            let (len, stride, per_task) = match const_len {
                Some(c) => {
                    let unit = 4096 / K as i64;
                    let m = ((c * bytes_per + unit - 1) / unit) | 1;
                    let st = m * unit;
                    (c.to_string(), st.to_string(), ((st * K as i64 + 4111) & -4096).to_string())
                }
                None => (ex[1].r.clone(), ex[3].r.clone(), ex[4].r.clone()),
            };
            let total = g.ins(format!("mul i64 {}, {}", stride, K));
            let off = g.ins(format!("mul i64 {}, {}", per_task, t));
            let block = g.ins(format!("getelementptr inbounds i8, ptr {}, i64 {}", all, off));
            // A task that meets an index out of bounds records it and stops;
            // the caller reports the failure of the first task that failed,
            // which is the first one in the order of the elements (each task
            // takes the next chunk): the panic is the sequential one, for any
            // number of threads (SPEC section 8, rule 2).
            let fslot = g.ins(format!("getelementptr inbounds i8, ptr {}, i64 {}", block, total));
            g.emit(format!("store {{ i64, i64 }} {{ i64 0, i64 0 }}, ptr {}", fslot));
            let lstop = g.label();
            let (src, n) = g.prepare_src(x)?;
            let a = g.ins(format!("mul i64 {}, {}", n, t));
            let lo = g.ins(format!("sdiv i64 {}, {}", a, tasks));
            let t1 = g.ins(format!("add i64 {}, 1", t));
            let b = g.ins(format!("mul i64 {}, {}", n, t1));
            let hi = g.ins(format!("sdiv i64 {}, {}", b, tasks));
            g.decls.insert("declare void @llvm.memset.p0.i64(ptr, i8, i64, i1)".into());
            g.emit(format!("call void @llvm.memset.p0.i64(ptr {}, i8 0, i64 {}, i1 false)", block, total));
            let subs: Vec<String> = (0..K)
                .map(|k| {
                    let off = g.ins(format!("mul i64 {}, {}", stride, k));
                    g.ins(format!("getelementptr inbounds i8, ptr {}, i64 {}", block, off))
                })
                .collect();
            let add_into = |g: &mut Self, sub: &str, i: &str, check: bool| -> R<()> {
                let el = g.elem(&src, i, 1)?;
                g.bind(p_v, el);
                let idx = g.ex(gi, 1)?;
                let val = g.ex(h, 1)?;
                let i64x = g.index64(&idx, &gi.ty);
                if check {
                    let ok = g.ins(format!("icmp ult i64 {}, {}", i64x, len));
                    let lok = g.label();
                    let lbad = g.label();
                    g.cbr(&ok, &lok, &lbad);
                    g.block(&lbad);
                    let rec = g.ins(format!("insertvalue {{ i64, i64 }} {{ i64 1, i64 0 }}, i64 {}, 1", i64x));
                    g.emit(format!("store {{ i64, i64 }} {}, ptr {}", rec, fslot));
                    g.br(&lstop);
                    g.block(&lok);
                }
                let gp = g.ins(format!("getelementptr inbounds {}, ptr {}, i64 {}", it, sub, i64x));
                let old = g.ins(format!("load {}, ptr {}", it, gp));
                let new = g.ins(format!("add {} {}, {}", it, old, val.r));
                g.emit(format!("store {} {}, ptr {}", it, new, gp));
                Ok(())
            };
            // `unroll` rounds of the K copies per iteration (fewer loop
            // instructions per element)
            let step = (K * unroll) as i64;
            let span_len = g.ins(format!("sub i64 {}, {}", hi, lo));
            let full = g.ins(format!("and i64 {}, -{}", span_len, step));
            let q = g.ins(format!("add i64 {}, {}", lo, full));
            let main_loop = |g: &mut Self, check: bool| -> R<()> {
                g.for_range(&lo, &q, step, &mut |g, i| {
                    for j in 0..K * unroll {
                        let ij = g.ins(format!("add i64 {}, {}", i, j));
                        add_into(g, &subs[j % K], &ij, check)?;
                    }
                    Ok(())
                })
            };
            // An unsigned index of b bits is below 2^b: when the counters are
            // at least that many, no index can be out of bounds and the loop
            // runs without checks (SPEC section 5: the compiler removes the
            // checks it proves useless); otherwise every index is checked.
            match &gi.ty {
                Type::Int(k) if !k.signed() && k.bits() < 64 => {
                    let max = (1u64 << k.bits()) - 1;
                    let safe = g.ins(format!("icmp ugt i64 {}, {}", len, max));
                    let lfast = g.label();
                    let lcheck = g.label();
                    let ljoin = g.label();
                    g.cbr(&safe, &lfast, &lcheck);
                    g.block(&lfast);
                    main_loop(g, false)?;
                    g.br(&ljoin);
                    g.block(&lcheck);
                    main_loop(g, true)?;
                    g.br(&ljoin);
                    g.block(&ljoin);
                }
                _ => main_loop(g, true)?,
            }
            g.for_range(&q, &hi, 1, &mut |g, i| add_into(g, &subs[0], i, true))?;
            // merge the interleaved copies into the first one
            g.for_range("0", &len, 1, &mut |g, j| {
                let p0 = g.ins(format!("getelementptr inbounds {}, ptr {}, i64 {}", it, subs[0], j));
                let mut acc = g.ins(format!("load {}, ptr {}", it, p0));
                for sub in &subs[1..] {
                    let pk = g.ins(format!("getelementptr inbounds {}, ptr {}, i64 {}", it, sub, j));
                    let x = g.ins(format!("load {}, ptr {}", it, pk));
                    acc = g.ins(format!("add {} {}, {}", it, acc, x));
                }
                g.emit(format!("store {} {}, ptr {}", it, acc, p0));
                Ok(())
            })?;
            g.br(&lstop);
            g.block(&lstop);
            Ok(())
        })?;
        self.emit(format!("call void @isc_parallel_for(i64 {}, ptr {}, ptr {}, i64 {})", tasks, task, ctx, ctx_bytes));
        // the first failure, in the order of the tasks
        let unsigned = if is_signed(&gi.ty) { 0 } else { 1 };
        self.for_range("0", &tasks, 1, &mut |g, t| {
            let off = g.ins(format!("mul i64 {}, {}", per_task, t));
            let off = g.ins(format!("add i64 {}, {}", off, copies));
            let fslot = g.ins(format!("getelementptr inbounds i8, ptr {}, i64 {}", all, off));
            let f = g.ins(format!("load {{ i64, i64 }}, ptr {}", fslot));
            let failed = g.ins(format!("extractvalue {{ i64, i64 }} {}, 0", f));
            let bad = g.ins(format!("extractvalue {{ i64, i64 }} {}, 1", f));
            let ok = g.ins(format!("icmp eq i64 {}, 0", failed));
            g.guard(&ok, format!("call void @isc_panic_bounds(i64 {}, i64 {}, i32 {}, {})", bad, len, unsigned, Self::pos(span)));
            Ok(())
        })?;
        // add the private copies to the initial value, in place
        self.for_range("0", &tasks, 1, &mut |g, t| {
            // the merged copies of task t are its first copy
            let off = g.ins(format!("mul i64 {}, {}", per_task, t));
            let pt = g.ins(format!("getelementptr inbounds i8, ptr {}, i64 {}", all, off));
            g.for_range("0", &len, 1, &mut |g, j| {
                let pa = g.ins(format!("getelementptr inbounds {}, ptr {}, i64 {}", it, ip, j));
                let pb = g.ins(format!("getelementptr inbounds {}, ptr {}, i64 {}", it, pt, j));
                let a = g.ins(format!("load {}, ptr {}", it, pa));
                let b = g.ins(format!("load {}, ptr {}", it, pb));
                let s = g.ins(format!("add {} {}, {}", it, a, b));
                g.emit(format!("store {} {}, ptr {}", it, s, pa));
                Ok(())
            })?;
            Ok(())
        })?;
        self.emit(format!("call void @isc_free(ptr {})", raw));
        Ok(iv)
    }

    // ----- actions -------------------------------------------------------------

    fn str_parts(&mut self, s: &V) -> (String, String) {
        let p = self.ins(format!("extractvalue {{ ptr, i64 }} {}, 0", s.r));
        let n = self.ins(format!("extractvalue {{ ptr, i64 }} {}, 1", s.r));
        (p, n)
    }

    fn print_lit(&mut self, s: &str) {
        let (g, n) = self.str_const(s);
        self.emit(format!("call void @isc_print_str(ptr {}, i64 {})", g, n));
    }

    /// The operations on strings (D134): each in the runtime; a result
    /// string is new, the operands are only read.
    fn str_op(&mut self, e: &Expr, op: isc_ir::StrOp, es: &[Expr]) -> R<V> {
        use isc_ir::StrOp as S;
        let mut vals = Vec::new();
        for x in es {
            vals.push(self.ex(x, 1)?);
        }
        let new_str = |g: &mut Self, call: String| {
            let out = g.alloca("{ ptr, i64 }");
            g.emit(call.replace("OUT", &out));
            v(g.ins(format!("load {{ ptr, i64 }}, ptr {}", out)), "{ ptr, i64 }")
        };
        Ok(match op {
            S::Concat => {
                let (p, n) = self.str_parts(&vals[0]);
                let (q, m) = self.str_parts(&vals[1]);
                new_str(self, format!("call void @isc_str_concat(ptr OUT, ptr {}, i64 {}, ptr {}, i64 {})", p, n, q, m))
            }
            S::Len => {
                let (_, n) = self.str_parts(&vals[0]);
                v(n, "i64")
            }
            // the bytes: an array of `u8`, the same layout
            S::Bytes => {
                let (p, n) = self.str_parts(&vals[0]);
                new_str(self, format!("call void @isc_str_new(ptr OUT, ptr {}, i64 {})", p, n))
            }
            S::FromBytes => {
                let (p, n) = self.str_parts(&vals[0]);
                let pos = Self::pos(e.span);
                new_str(self, format!("call void @isc_str_from_bytes(ptr OUT, ptr {}, i64 {}, {})", p, n, pos))
            }
            S::ToString => {
                let x = &vals[0];
                let call = match &es[0].ty {
                    Type::Int(k) => {
                        let w = if k.bits() < 64 {
                            let op = if k.signed() { "sext" } else { "zext" };
                            self.ins(format!("{} i{} {} to i64", op, k.bits(), x.r))
                        } else {
                            x.r.clone()
                        };
                        let f = if k.signed() { "isc_to_string_i64" } else { "isc_to_string_u64" };
                        format!("call void @{}(ptr OUT, i64 {})", f, w)
                    }
                    Type::F64 => format!("call void @isc_to_string_f64(ptr OUT, double {})", x.r),
                    Type::F32 => format!("call void @isc_to_string_f32(ptr OUT, float {})", x.r),
                    Type::Bool => {
                        let b = self.ins(format!("zext i1 {} to i32", x.r));
                        format!("call void @isc_to_string_bool(ptr OUT, i32 {})", b)
                    }
                    t => return Err(format!("codegen: toString of {:?}", t)),
                };
                new_str(self, call)
            }
            // text (D135): an array of strings has the layout of a string
            S::Lines => {
                let (p, n) = self.str_parts(&vals[0]);
                new_str(self, format!("call void @isc_lines(ptr OUT, ptr {}, i64 {})", p, n))
            }
            S::Split => {
                let (p, n) = self.str_parts(&vals[0]);
                let (q, m) = self.str_parts(&vals[1]);
                let pos = Self::pos(e.span);
                new_str(self, format!("call void @isc_split(ptr OUT, ptr {}, i64 {}, ptr {}, i64 {}, {})", p, n, q, m, pos))
            }
            S::Trim => {
                let (p, n) = self.str_parts(&vals[0]);
                new_str(self, format!("call void @isc_trim(ptr OUT, ptr {}, i64 {})", p, n))
            }
            // the functions of TypeScript (D155)
            S::IndexOf => {
                let (p, n) = self.str_parts(&vals[0]);
                let (q, m) = self.str_parts(&vals[1]);
                v(self.ins(format!("call i64 @isc_index_of(ptr {}, i64 {}, ptr {}, i64 {})", p, n, q, m)), "i64")
            }
            S::Includes | S::StartsWith | S::EndsWith => {
                let (p, n) = self.str_parts(&vals[0]);
                let (q, m) = self.str_parts(&vals[1]);
                let which = match op {
                    S::Includes => 0,
                    S::StartsWith => 1,
                    _ => 2,
                };
                let r = self.ins(format!("call i32 @isc_str_test(ptr {}, i64 {}, ptr {}, i64 {}, i32 {})", p, n, q, m, which));
                v(self.ins(format!("icmp ne i32 {}, 0", r)), "i1")
            }
            S::ReplaceAll | S::ReplaceFirst => {
                let (p, n) = self.str_parts(&vals[0]);
                let (q, m) = self.str_parts(&vals[1]);
                let (r, k) = self.str_parts(&vals[2]);
                let all = (op == S::ReplaceAll) as i32;
                new_str(self, format!("call void @isc_replace_all(ptr OUT, ptr {}, i64 {}, ptr {}, i64 {}, ptr {}, i64 {}, i32 {})", p, n, q, m, r, k, all))
            }
            // regular expressions (D157)
            S::Rx(x) => {
                use isc_ir::RxOp as X;
                let pos = Self::pos(e.span);
                match x {
                    X::New => {
                        let (p, n) = self.str_parts(&vals[0]);
                        let (q, m) = self.str_parts(&vals[1]);
                        new_str(self, format!("call void @isc_rx_new(ptr OUT, ptr {}, i64 {}, ptr {}, i64 {}, {})", p, n, q, m, pos))
                    }
                    X::Test => {
                        let (p, n) = self.str_parts(&vals[0]);
                        let (q, m) = self.str_parts(&vals[1]);
                        let r = self.ins(format!("call i64 @isc_rx_int(ptr {}, i64 {}, ptr {}, i64 {}, i32 0)", p, n, q, m));
                        v(self.ins(format!("icmp ne i64 {}, 0", r)), "i1")
                    }
                    X::Search | X::Count => {
                        let (p, n) = self.str_parts(&vals[1]);
                        let (q, m) = self.str_parts(&vals[0]);
                        let which = if x == X::Search { 1 } else { 2 };
                        v(self.ins(format!("call i64 @isc_rx_int(ptr {}, i64 {}, ptr {}, i64 {}, i32 {})", p, n, q, m, which)), "i64")
                    }
                    X::Matches => {
                        let (p, n) = self.str_parts(&vals[0]);
                        let (q, m) = self.str_parts(&vals[1]);
                        new_str(self, format!("call void @isc_rx_matches(ptr OUT, ptr {}, i64 {}, ptr {}, i64 {})", p, n, q, m))
                    }
                    X::Replace | X::ReplaceAll => {
                        let (p, n) = self.str_parts(&vals[0]);
                        let (q, m) = self.str_parts(&vals[1]);
                        let (r, k) = self.str_parts(&vals[2]);
                        let all = (x == X::ReplaceAll) as i32;
                        new_str(self, format!("call void @isc_rx_replace(ptr OUT, ptr {}, i64 {}, ptr {}, i64 {}, ptr {}, i64 {}, i32 {}, {})", p, n, q, m, r, k, all, pos))
                    }
                }
            }
            S::ToUpper | S::ToLower => {
                let (p, n) = self.str_parts(&vals[0]);
                let up = (op == S::ToUpper) as i32;
                new_str(self, format!("call void @isc_to_case(ptr OUT, ptr {}, i64 {}, i32 {})", p, n, up))
            }
            S::PadStart | S::PadEnd => {
                let (p, n) = self.str_parts(&vals[0]);
                let (q, m) = self.str_parts(&vals[2]);
                let start = (op == S::PadStart) as i32;
                new_str(self, format!("call void @isc_pad(ptr OUT, ptr {}, i64 {}, i64 {}, ptr {}, i64 {}, i32 {})", p, n, vals[1].r, q, m, start))
            }
            S::Repeat => {
                let (p, n) = self.str_parts(&vals[0]);
                let pos = Self::pos(e.span);
                new_str(self, format!("call void @isc_repeat(ptr OUT, ptr {}, i64 {}, i64 {}, {})", p, n, vals[1].r, pos))
            }
            S::TrimStart | S::TrimEnd => {
                let (p, n) = self.str_parts(&vals[0]);
                let end = (op == S::TrimEnd) as i32;
                new_str(self, format!("call void @isc_trim_side(ptr OUT, ptr {}, i64 {}, i32 {})", p, n, end))
            }
            S::Slice => {
                let (p, n) = self.str_parts(&vals[0]);
                let pos = Self::pos(e.span);
                new_str(self, format!("call void @isc_str_slice(ptr OUT, ptr {}, i64 {}, i64 {}, i64 {}, {})", p, n, vals[1].r, vals[2].r, pos))
            }
            // `bigint` (D156): the code of the runtime
            S::Big(b) => {
                use isc_ir::BigOp as B;
                let pos = Self::pos(e.span);
                match b {
                    B::Add | B::Sub | B::Mul | B::Div | B::Rem | B::Pow | B::Neg => {
                        let code = match b {
                            B::Add => 0,
                            B::Sub => 1,
                            B::Mul => 2,
                            B::Div => 3,
                            B::Rem => 4,
                            B::Pow => 5,
                            _ => 6,
                        };
                        let (p, n) = self.str_parts(&vals[0]);
                        let (q, m) = if vals.len() > 1 { self.str_parts(&vals[1]) } else { ("null".to_string(), "0".to_string()) };
                        new_str(self, format!("call void @isc_big_op(ptr OUT, ptr {}, i64 {}, ptr {}, i64 {}, i32 {}, {})", p, n, q, m, code, pos))
                    }
                    B::Cmp(op) => {
                        let (p, n) = self.str_parts(&vals[0]);
                        let (q, m) = self.str_parts(&vals[1]);
                        let c = self.ins(format!("call i32 @isc_big_cmp(ptr {}, i64 {}, ptr {}, i64 {})", p, n, q, m));
                        let pred = match op {
                            BinOp::Lt => "slt",
                            BinOp::Le => "sle",
                            BinOp::Gt => "sgt",
                            _ => "sge",
                        };
                        v(self.ins(format!("icmp {} i32 {}, 0", pred, c)), "i1")
                    }
                    B::FromI64 => new_str(self, format!("call void @isc_big_from_i64(ptr OUT, i64 {})", vals[0].r)),
                    B::FromU64 => new_str(self, format!("call void @isc_big_from_u64(ptr OUT, i64 {})", vals[0].r)),
                    B::Parse => {
                        let (p, n) = self.str_parts(&vals[0]);
                        new_str(self, format!("call void @isc_big_parse(ptr OUT, ptr {}, i64 {}, {})", p, n, pos))
                    }
                    B::ToI64 => {
                        let (p, n) = self.str_parts(&vals[0]);
                        v(self.ins(format!("call i64 @isc_big_to_i64(ptr {}, i64 {})", p, n)), "i64")
                    }
                    B::ToF64 => {
                        let (p, n) = self.str_parts(&vals[0]);
                        v(self.ins(format!("call double @isc_big_to_f64(ptr {}, i64 {})", p, n)), "double")
                    }
                    B::ToString => {
                        let (p, n) = self.str_parts(&vals[0]);
                        new_str(self, format!("call void @isc_big_to_string(ptr OUT, ptr {}, i64 {})", p, n))
                    }
                    B::ArgSort => {
                        let (p, n) = self.str_parts(&vals[0]);
                        new_str(self, format!("call void @isc_argsort(ptr OUT, ptr {}, i64 {}, i32 12)", p, n))
                    }
                }
            }
            S::JsNumber | S::JsParseFloat | S::JsParseInt => {
                let (p, n) = self.str_parts(&vals[0]);
                let (which, radix) = match op {
                    S::JsNumber => (0, "0".to_string()),
                    S::JsParseFloat => (1, "0".to_string()),
                    _ => (2, vals[1].r.clone()),
                };
                v(self.ins(format!("call double @isc_js_number(ptr {}, i64 {}, i32 {}, i64 {})", p, n, which, radix)), "double")
            }
            S::Group => {
                let (p, n) = self.str_parts(&vals[0]);
                let kind = match es[0].ty.elem() {
                    Type::Int(k) => (k.bits().trailing_zeros() as i32 - 3) + if k.signed() { 0 } else { 4 },
                    Type::Bool => 10,
                    _ => 11,
                };
                new_str(self, format!("call void @isc_group(ptr OUT, ptr {}, i64 {}, i32 {})", p, n, kind))
            }
            S::ArgSort => {
                let (p, n) = self.str_parts(&vals[0]);
                let kind = match es[0].ty.elem() {
                    Type::Int(k) => (k.bits().trailing_zeros() as i32 - 3) + if k.signed() { 0 } else { 4 },
                    Type::F32 => 8,
                    Type::F64 => 9,
                    Type::Bool => 10,
                    _ => 11,
                };
                new_str(self, format!("call void @isc_argsort(ptr OUT, ptr {}, i64 {}, i32 {})", p, n, kind))
            }
            S::ToFixed => {
                let pos = Self::pos(e.span);
                new_str(self, format!("call void @isc_to_fixed(ptr OUT, double {}, i64 {}, {})", vals[0].r, vals[1].r, pos))
            }
            S::Join => {
                let (p, n) = self.str_parts(&vals[0]);
                let (q, m) = self.str_parts(&vals[1]);
                new_str(self, format!("call void @isc_join(ptr OUT, ptr {}, i64 {}, ptr {}, i64 {})", p, n, q, m))
            }
            S::Cmp(b) => {
                let (p, n) = self.str_parts(&vals[0]);
                let (q, m) = self.str_parts(&vals[1]);
                let c = self.ins(format!("call i32 @isc_str_cmp(ptr {}, i64 {}, ptr {}, i64 {})", p, n, q, m));
                let pred = match b {
                    BinOp::Eq => "eq",
                    BinOp::Ne => "ne",
                    BinOp::Lt => "slt",
                    BinOp::Le => "sle",
                    BinOp::Gt => "sgt",
                    BinOp::Ge => "sge",
                    _ => unreachable!(),
                };
                v(self.ins(format!("icmp {} i32 {}, 0", pred, c)), "i1")
            }
        })
    }

    fn print_value(&mut self, x: &V, t: &Type, shape: &Shape) -> R<()> {
        match t {
            Type::Int(k) => {
                let w = if k.bits() < 64 {
                    let op = if k.signed() { "sext" } else { "zext" };
                    self.ins(format!("{} i{} {} to i64", op, k.bits(), x.r))
                } else {
                    x.r.clone()
                };
                let f = if k.signed() { "isc_print_i64" } else { "isc_print_u64" };
                self.emit(format!("call void @{}(i64 {})", f, w));
            }
            Type::F64 => self.emit(format!("call void @isc_print_f64(double {})", x.r)),
            Type::F32 => self.emit(format!("call void @isc_print_f32(float {})", x.r)),
            Type::Bool => {
                let b = self.ins(format!("zext i1 {} to i32", x.r));
                self.emit(format!("call void @isc_print_bool(i32 {})", b));
            }
            Type::Unit => self.print_lit("()"),
            Type::Rec(i) => {
                self.rec_prints.insert(*i);
                self.emit(format!("call void @isc_rec_print_{}(ptr {})", i, x.r));
            }
            Type::Str => {
                let (p, n) = self.str_parts(x);
                self.emit(format!("call void @isc_print_str(ptr {}, i64 {})", p, n));
            }
            // a `bigint`: its decimal text (D156)
            Type::Big => {
                let (p, n) = self.str_parts(x);
                self.emit(format!("call void @isc_print_big(ptr {}, i64 {})", p, n));
            }
            Type::Tuple(ts) if matches!(shape, Shape::Sum(_)) => {
                // the constructor of the tag, then its slots
                let vars = match shape {
                    Shape::Sum(v) => v.clone(),
                    _ => unreachable!(),
                };
                let tag = self.ins(format!("extractvalue {} {}, 0", x.t, x.r));
                let done = self.label();
                for (k, (name, slots, shapes)) in vars.iter().enumerate() {
                    let this = self.label();
                    let next = self.label();
                    let is = self.ins(format!("icmp eq i32 {}, {}", tag, k));
                    self.cbr(&is, &this, &next);
                    self.block(&this);
                    self.print_lit(name);
                    if !slots.is_empty() {
                        self.print_lit("(");
                        for (i, (&slot, sh)) in slots.iter().zip(shapes.iter()).enumerate() {
                            if i > 0 {
                                self.print_lit(", ");
                            }
                            let f = self.ins(format!("extractvalue {} {}, {}", x.t, x.r, slot));
                            let ct = ts[slot as usize].clone();
                            self.print_value(&v(f, lt(&ct)), &ct, sh)?;
                        }
                        self.print_lit(")");
                    }
                    self.br(&done);
                    self.block(&next);
                }
                self.br(&done);
                self.block(&done);
            }
            Type::Tuple(ts) => {
                let (open, close, names, subs) = match shape {
                    Shape::Record(ns, ss) => ("{", "}", Some(ns.clone()), Some(ss.clone())),
                    Shape::Tuple(ss) => ("(", ")", None, Some(ss.clone())),
                    _ => ("(", ")", None, None),
                };
                self.print_lit(open);
                for (i, c) in ts.iter().enumerate() {
                    if i > 0 {
                        self.print_lit(", ");
                    }
                    if let Some(ns) = &names {
                        self.print_lit(&format!("{}: ", ns[i]));
                    }
                    let f = self.ins(format!("extractvalue {} {}, {}", x.t, x.r, i));
                    let sub = subs.as_ref().map(|ss| ss[i].clone()).unwrap_or(Shape::Plain);
                    self.print_value(&v(f, lt(c)), c, &sub)?;
                }
                self.print_lit(close);
            }
            Type::Array(et, _) if et.is_array() => {
                let inner = et.elem().clone();
                let (p, rows, cols, stride) = self.nested_parts(x);
                let sub = match shape {
                    Shape::Array(e) => (**e).clone(),
                    _ => Shape::Plain,
                };
                let et = (**et).clone();
                self.print_lit("[");
                self.for_range("0", &rows, 1, &mut |g, i| {
                    let first = g.ins(format!("icmp eq i64 {}, 0", i));
                    let lsep = g.label();
                    let lel = g.label();
                    g.cbr(&first, &lel, &lsep);
                    g.block(&lsep);
                    g.print_lit(", ");
                    g.br(&lel);
                    g.block(&lel);
                    let row = g.row_view(&p, &cols, &stride, i, &inner);
                    g.print_value(&row, &et, &sub)
                })?;
                self.print_lit("]");
            }
            Type::Array(et, _) => {
                let (p, n) = self.str_parts(x);
                self.print_lit("[");
                let et = (**et).clone();
                let sub = match shape {
                    Shape::Array(e) => (**e).clone(),
                    _ => Shape::Plain,
                };
                self.for_range("0", &n, 1, &mut |g, i| {
                    let first = g.ins(format!("icmp eq i64 {}, 0", i));
                    let lsep = g.label();
                    let lel = g.label();
                    g.cbr(&first, &lel, &lsep);
                    g.block(&lsep);
                    g.print_lit(", ");
                    g.br(&lel);
                    g.block(&lel);
                    let el = g.load_elem(&p, &n, i, &et, 1);
                    g.print_value(&el, &et, &sub)
                })?;
                self.print_lit("]");
            }
        }
        Ok(())
    }

    fn action(&mut self, op: ActionOp, args: &[Expr], e: &Expr) -> R<V> {
        let mut vals = Vec::new();
        for a in args {
            vals.push(self.ex(a, 1)?);
        }
        let pos = Self::pos(e.span);
        Ok(match op {
            // the standard input (D155)
            ActionOp::ReadStdin | ActionOp::ReadStdinText => {
                let f = if op == ActionOp::ReadStdin { "isc_read_stdin" } else { "isc_read_stdin_text" };
                let slot = self.alloca("{ ptr, i64 }");
                self.emit(format!("call void @{}(ptr {}, {})", f, slot, pos));
                let r = self.ins(format!("load {{ ptr, i64 }}, ptr {}", slot));
                v(r, "{ ptr, i64 }")
            }
            ActionOp::ReadF64 | ActionOp::ReadF32 | ActionOp::ReadBytes => {
                let (p, n) = self.str_parts(&vals[0]);
                let f = match op {
                    ActionOp::ReadF64 => "isc_read_f64",
                    ActionOp::ReadF32 => "isc_read_f32",
                    _ => "isc_read_bytes",
                };
                let slot = self.alloca("{ ptr, i64 }");
                self.emit(format!("call void @{}(ptr {}, ptr {}, i64 {}, {})", f, slot, p, n, pos));
                let r = self.ins(format!("load {{ ptr, i64 }}, ptr {}", slot));
                v(r, "{ ptr, i64 }")
            }
            ActionOp::WriteF64 | ActionOp::WriteF32 | ActionOp::WriteBytes => {
                let (p, n) = self.str_parts(&vals[0]);
                let (d, m) = self.str_parts(&vals[1]);
                let f = match op {
                    ActionOp::WriteF64 => "isc_write_f64",
                    ActionOp::WriteF32 => "isc_write_f32",
                    _ => "isc_write_bytes",
                };
                self.emit(format!("call void @{}(ptr {}, i64 {}, ptr {}, i64 {}, {})", f, p, n, d, m, pos));
                v("false", "i1")
            }
            ActionOp::Print(k) => {
                let shape = self.p.shapes[k as usize].clone();
                self.print_value(&vals[0], &args[0].ty, &shape)?;
                self.emit("call void @isc_print_newline()");
                v("false", "i1")
            }
            ActionOp::Clock => v(self.ins("call i64 @isc_clock_ns()".into()), "i64"),
            ActionOp::ParseI64 => {
                let (p, n) = self.str_parts(&vals[0]);
                v(self.ins(format!("call i64 @isc_parse_i64(ptr {}, i64 {}, {})", p, n, pos)), "i64")
            }
            ActionOp::ParseF64 => {
                let (p, n) = self.str_parts(&vals[0]);
                v(self.ins(format!("call double @isc_parse_f64(ptr {}, i64 {}, {})", p, n, pos)), "double")
            }
            ActionOp::PrintBytes => {
                let (p, n) = self.str_parts(&vals[0]);
                self.emit(format!("call void @isc_print_bytes(ptr {}, i64 {})", p, n));
                v("false", "i1")
            }
            // the fallible forms (D138): the value and the message through
            // pointers, then the pair
            ActionOp::OrError(f) => {
                use isc_ir::Fallible as F;
                let (p, n) = self.str_parts(&vals[0]);
                let err = self.alloca("{ ptr, i64 }");
                if matches!(f, F::WriteBytes | F::WriteText) {
                    let (d, m) = self.str_parts(&vals[1]);
                    self.emit(format!("call void @isc_try_write_bytes(ptr {}, ptr {}, i64 {}, ptr {}, i64 {})", err, p, n, d, m));
                    return Ok(v(self.ins(format!("load {{ ptr, i64 }}, ptr {}", err)), "{ ptr, i64 }"));
                }
                let (vt, name) = match f {
                    F::ParseI64 => ("i64".to_string(), "isc_try_parse_i64"),
                    F::ParseF64 => ("double".to_string(), "isc_try_parse_f64"),
                    F::ReadBytes => ("{ ptr, i64 }".to_string(), "isc_try_read_bytes"),
                    F::ReadF64 => ("{ ptr, i64 }".to_string(), "isc_try_read_f64"),
                    F::ReadText => ("{ ptr, i64 }".to_string(), "isc_try_read_text"),
                    F::ReadCsvF64 => (NESTED.to_string(), "isc_try_read_csv_f64"),
                    F::WriteBytes | F::WriteText => unreachable!(),
                };
                let val = self.alloca(&vt);
                let skip = if f == F::ReadCsvF64 { format!(", i64 {}", vals[1].r) } else { String::new() };
                self.emit(format!("call void @{}(ptr {}, ptr {}, ptr {}, i64 {}{})", name, val, err, p, n, skip));
                let x = self.ins(format!("load {}, ptr {}", vt, val));
                let m = self.ins(format!("load {{ ptr, i64 }}, ptr {}", err));
                let tt = lt(&e.ty);
                let t0 = self.ins(format!("insertvalue {} poison, {} {}, 0", tt, vt, x));
                v(self.ins(format!("insertvalue {} {}, {{ ptr, i64 }} {}, 1", tt, t0, m)), tt)
            }
            // text (D135)
            ActionOp::ReadText => {
                let (p, n) = self.str_parts(&vals[0]);
                let slot = self.alloca("{ ptr, i64 }");
                self.emit(format!("call void @isc_read_text(ptr {}, ptr {}, i64 {}, {})", slot, p, n, pos));
                v(self.ins(format!("load {{ ptr, i64 }}, ptr {}", slot)), "{ ptr, i64 }")
            }
            ActionOp::WriteText => {
                let (p, n) = self.str_parts(&vals[0]);
                let (d, m) = self.str_parts(&vals[1]);
                self.emit(format!("call void @isc_write_text(ptr {}, i64 {}, ptr {}, i64 {}, {})", p, n, d, m, pos));
                v("false", "i1")
            }
            ActionOp::ParseCsvF64 | ActionOp::ReadCsvF64 => {
                let (p, n) = self.str_parts(&vals[0]);
                let f = if op == ActionOp::ParseCsvF64 { "isc_parse_csv_f64" } else { "isc_read_csv_f64" };
                let slot = self.alloca(NESTED);
                self.emit(format!("call void @{}(ptr {}, ptr {}, i64 {}, i64 {}, {})", f, slot, p, n, vals[1].r, pos));
                v(self.ins(format!("load {}, ptr {}", NESTED, slot)), NESTED)
            }
            ActionOp::Extern(i) => self.call_extern(i, &vals, e)?,
        })
    }

    /// A call of a C function (System V x86-64, D125): integers with their
    /// C extension (`signext`/`zeroext` below 32 bits), `bool` as `i1
    /// zeroext`, floats as `float`/`double`, a `string` as a NUL-terminated
    /// copy freed after the call, an array of scalars as a pointer to its
    /// elements and its length.
    fn call_extern(&mut self, i: u32, vals: &[V], e: &Expr) -> R<V> {
        // the same code for WebAssembly: its C ABI passes these scalars and
        // pointers directly (WASI's C library for libc and libm)
        let f = self.p.externs[i as usize].clone();
        let int_attr = |t: &Type| -> String {
            match t {
                Type::Int(k) if k.bits() < 32 => format!("i{} {}", k.bits(), if k.signed() { "signext" } else { "zeroext" }),
                Type::Int(k) => format!("i{}", k.bits()),
                Type::Bool => "i1 zeroext".to_string(),
                Type::F64 => "double".to_string(),
                Type::F32 => "float".to_string(),
                _ => "void".to_string(),
            }
        };
        let mut decl_params = Vec::new();
        let mut call_args = Vec::new();
        let mut copies = Vec::new();
        for (x, t) in vals.iter().zip(f.params.iter()) {
            match t {
                Type::Str => {
                    let (p, n) = self.str_parts(x);
                    let (g, gl) = self.str_const(&f.name);
                    let c = self.ins(format!("call ptr @isc_cstr(ptr {}, i64 {}, ptr {}, i64 {}, {})", p, n, g, gl, Self::pos(e.span)));
                    decl_params.push("ptr".to_string());
                    call_args.push(format!("ptr {}", c));
                    copies.push(c);
                }
                Type::Array(_, _) => {
                    let (p, n) = self.str_parts(x);
                    decl_params.push("ptr".to_string());
                    decl_params.push("i64".to_string());
                    call_args.push(format!("ptr {}", p));
                    call_args.push(format!("i64 {}", n));
                }
                t => {
                    let a = int_attr(t);
                    decl_params.push(a.clone());
                    call_args.push(format!("{} {}", a, x.r));
                }
            }
        }
        let ret = match &f.ret {
            Type::Int(k) if k.bits() < 32 => format!("{} i{}", if k.signed() { "signext" } else { "zeroext" }, k.bits()),
            Type::Bool => "zeroext i1".to_string(),
            t => int_attr(t),
        };
        self.decls.insert(format!("declare {} @{}({})", ret, f.name, decl_params.join(", ")));
        // `nobuiltin`: LLVM must not replace the call of a C function it
        // knows (`cbrt` computed at compilation, `strlen` of a constant): the
        // call is an effect of the action, run as written, with the result
        // of the library the program is linked with
        let call = format!("call {} @{}({}) nobuiltin", ret, f.name, call_args.join(", "));
        let r = if f.ret == Type::Unit {
            self.emit(call);
            v("false", "i1")
        } else {
            v(self.ins(call), lt(&f.ret))
        };
        for c in copies {
            self.emit(format!("call void @isc_free(ptr {})", c));
        }
        Ok(r)
    }

    /// `bench(r, f)`: the body becomes a function called through an opaque
    /// pointer, `r` times; all results but the last are freed.
    fn bench(&mut self, reps: &Expr, body: &Expr, e: &Expr) -> R<V> {
        let rv = self.ex(reps, 1)?;
        let ok = self.ins(format!("icmp sge i64 {}, 1", rv.r));
        self.guard(&ok, format!("call void @isc_panic_bench(i64 {}, {})", rv.r, Self::pos(e.span)));
        let mut caps = Vec::new();
        free_vars(body, &mut HashSet::new(), &mut caps);
        let cap_vals: Vec<V> = caps.iter().map(|c| self.env[c].clone()).collect();
        let env_ty = if cap_vals.is_empty() {
            "{ i1 }".to_string()
        } else {
            format!("{{ {} }}", cap_vals.iter().map(|c| c.t.clone()).collect::<Vec<_>>().join(", "))
        };
        let envp = self.alloca(&env_ty);
        for (i, c) in cap_vals.iter().enumerate() {
            let g = self.ins(format!("getelementptr inbounds {}, ptr {}, i32 0, i32 {}", env_ty, envp, i));
            self.emit(format!("store {} {}, ptr {}", c.t, c.r, g));
        }
        // the body function
        self.nbench += 1;
        let name = format!("@isc_bench_{}", self.nbench);
        let rt = lt(&body.ty);
        let saved_f = std::mem::take(&mut self.f);
        let saved_env = std::mem::take(&mut self.env);
        let saved_divmod = std::mem::take(&mut self.divmod);
        let saved_vec_fail = self.vec_fail.take();
        for (i, c) in caps.iter().enumerate() {
            let t = cap_vals[i].t.clone();
            let g = self.ins(format!("getelementptr inbounds {}, ptr %env, i32 0, i32 {}", env_ty, i));
            let x = self.ins(format!("load {}, ptr {}", t, g));
            self.env.insert(*c, v(x, t));
        }
        let r = self.ex(body, 1)?;
        self.emit(format!("ret {} {}", rt, r.r));
        let attrs = self.attrs();
        let text = self.finish(&format!("define internal {} {}(ptr %env) noinline{}", rt, name, attrs));
        self.funcs.push(text);
        self.f = saved_f;
        self.env = saved_env;
        self.divmod = saved_divmod;
        self.vec_fail = saved_vec_fail;
        // the loop
        let t0 = self.ins("call i64 @isc_clock_ns()".into());
        let ia = self.alloca("i64");
        self.emit(format!("store i64 0, ptr {}", ia));
        let lloop = self.label();
        let lnext = self.label();
        let ldone = self.label();
        self.br(&lloop);
        self.block(&lloop);
        let fp = if self.opts.isa.is_wasm() {
            // the function pointer goes through a volatile slot: LLVM cannot
            // see which function is called, so it cannot hoist or merge calls
            let fs = self.alloca("ptr");
            self.emit(format!("store volatile ptr {}, ptr {}", name, fs));
            self.ins(format!("load volatile ptr, ptr {}", fs))
        } else {
            self.ins(format!("call ptr asm sideeffect \"\", \"=r,0\"(ptr {})", name))
        };
        let res = self.ins(format!("call {} {}(ptr {})", rt, fp, envp));
        let i = self.ins(format!("load i64, ptr {}", ia));
        let i1 = self.ins(format!("add i64 {}, 1", i));
        self.emit(format!("store i64 {}, ptr {}", i1, ia));
        let last = self.ins(format!("icmp sge i64 {}, {}", i1, rv.r));
        self.cbr(&last, &ldone, &lnext);
        self.block(&lnext);
        self.drop_value(&v(res.clone(), rt.clone()), &body.ty);
        self.br(&lloop);
        self.block(&ldone);
        let t1 = self.ins("call i64 @isc_clock_ns()".into());
        let ns = self.ins(format!("sub i64 {}, {}", t1, t0));
        let tt = lt(&e.ty);
        let a = self.ins(format!("insertvalue {} poison, {} {}, 0", tt, rt, res));
        let b = self.ins(format!("insertvalue {} {}, i64 {}, 1", tt, a, ns));
        Ok(v(b, tt))
    }
}
