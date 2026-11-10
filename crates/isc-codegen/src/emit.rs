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
use isc_ir::blocks::{free_vars, inner_loop, mentions, print_stream_ok, stream_map_ok, windowed_filter};
use isc_syntax::Span;

/// V, the number of interleaved accumulators of the fixed reduction form
/// (SPEC section 10). Other values are an experiment (`iscc --lanes`), not
/// the language.
pub const LANES: u32 = 32;
pub const BLOCK: i64 = 4096;

/// The bytes a task of a parallel fill writes at least (D321): its elements
/// do not depend on the cut (D121), so its grain is free of B, which fixes
/// the results of the reductions; a task of B bytes (lines of fasta, 4 KiB)
/// paid the start and the end of a task every 4096 bytes. At least B
/// elements, as before for elements of 4 bytes and more.
pub const FILL_BYTES: i64 = 16 << 10;

/// The elements of a task of a parallel fill of elements of `es` bytes
/// (D321).
fn fill_block(es: i64) -> i64 {
    BLOCK.max(FILL_BYTES / es.max(1))
}

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
/// The bytes of the buffer of the stack where `print` has the prelude
/// write the text of a float (`emit_text_print`): the 2 of a separator and
/// the most `$decimalText` writes, 420 (a sign, `0.`, 400 zeros and 17
/// digits), on a multiple of 64.
const TEXT_BUF: usize = 448;

/// The functions lowered for the texts of the prelude (from
/// `Program::text_fns` on) that the program reaches: by `toString` of an
/// integer or of a float, by `toFixed`, by a call, from its body, from its
/// own functions and from those reached. (`print` of a float calls
/// variants of them, and what those call, emitted when they are.)
fn text_fns_reached(p: &Program) -> HashSet<u32> {
    fn scan(p: &Program, e: &Expr, out: &mut HashSet<u32>, todo: &mut Vec<u32>) {
        let mut reach = |f: u32| {
            if f >= p.text_fns && out.insert(f) {
                todo.push(f);
            }
        };
        match &e.kind {
            ExprKind::Call(f, _) => reach(*f),
            ExprKind::StrOp(isc_ir::StrOp::ToString, xs) => match (&xs[0].ty, p.int_text, p.float_text) {
                (Type::Int(_), Some((signed, unsigned)), _) => {
                    reach(signed);
                    reach(unsigned);
                }
                (Type::F64, _, Some((double, _))) => reach(double),
                (Type::F32, _, Some((_, single))) => reach(single),
                _ => {}
            },
            ExprKind::StrOp(isc_ir::StrOp::ToFixed, _) => {
                if let Some(f) = p.fixed_text {
                    reach(f);
                }
            }
            _ => {}
        }
        for c in e.children() {
            scan(p, c, out, todo);
        }
    }
    let (mut out, mut todo) = (HashSet::new(), Vec::new());
    scan(p, &p.body, &mut out, &mut todo);
    for f in p.fns.iter().take(p.text_fns as usize) {
        scan(p, &f.body, &mut out, &mut todo);
    }
    // the functions lowered after those of the texts that the code
    // generator calls itself: the digits of a padded integer (D606, lowered
    // when the program pads one), the cases of a text that is not ASCII
    // (D608, lowered when the program changes a case), the entries of the
    // allocator (D605, in every program)
    let kernel = p.kernel.map(|(a, b, c)| vec![a, b, c]).unwrap_or_default();
    for f in p.digit_words.into_iter().chain(p.case_text).chain(kernel) {
        if f >= p.text_fns && out.insert(f) {
            todo.push(f);
        }
    }
    while let Some(f) = todo.pop() {
        scan(p, &p.fns[f as usize].body, &mut out, &mut todo);
    }
    out
}

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
/// The numbers of the metadata of the kernel's memory (D605): the base of
/// the arena, the words of the arena and those of the zone of the thread
/// are three kinds of memory that are never one another (the arena is not
/// where the words of the module are), which LLVM is told (`!tbaa`): a
/// store to a word of the arena does not make it read the base again, and
/// the tests of the words of a header stay folded after the first store.
/// The accesses of a program to its own blocks carry no such mark: they
/// may be any memory.
const KERNEL_IDS: u64 = 1 << 21;
const TBAA_BASE: u64 = KERNEL_IDS + 4;
const TBAA_ARENA: u64 = KERNEL_IDS + 5;
const TBAA_ZONE: u64 = KERNEL_IDS + 6;
/// Width of the element-wise loops (speed only, never results).
pub const MAP_WIDTH: u32 = 16;

/// At most this many elements of a `map` whose elements are loops of their
/// own are made where the `map` is, without a task (D520).
const HEAVY_FEW: i64 = 4;

/// The indices a search of rule F tests one at a time before its vectors
/// (D512).
const FIRST_SCALAR: i64 = 8;
/// The lanes of the vectors of a search of rule F among bytes after its
/// whole vectors, and of an array too short for one (D595).
const NARROW_SEARCH: u32 = 16;
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
    /// An executable for Windows (`x86_64-pc-windows-gnu`, D246): the same
    /// code for the same instruction set, another object format and ABI.
    pub windows: bool,
    /// For Windows, Microsoft's ABI (`x86_64-pc-windows-msvc`, D542): the
    /// same code, in objects that the static libraries of Visual Studio and
    /// Microsoft's C runtime agree with.
    pub msvc: bool,
}

impl Default for Options {
    fn default() -> Options {
        // native code of an iscc on Windows is for Windows (D247)
        Options { lanes: LANES, isa: Isa::Native, stream_bytes: STREAM_BYTES, stack_scratch: STACK_SCRATCH, windows: cfg!(windows), msvc: false }
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
        // a resource of the system: its number (D339)
        Type::Handle => "i64".into(),
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
        Type::Rec(_) | Type::Handle => 8,
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
    /// The tables read at `x % c` by the vector loop being emitted, copied
    /// before it with their first `w - 1` elements after their `c` (D273):
    /// the copy, and whether it was made (the table has `c` elements).
    period_tables: HashMap<(VarId, i64), (String, String)>,
    /// The remainders `x % c` of the reads of those tables carried from a
    /// row to the next by the whole rows of `fill_rows` (D318): per `Rem`
    /// expression, the slot of the remainder of its first lane and whether
    /// every lane of every row is at least 0.
    carried: HashMap<*const Expr, (String, String)>,
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
    /// Inside a function of the prelude named `$…` (the kernel, D605): the
    /// statements of its actions are those of the runtime, not of a
    /// program. They run in their order, as every effect does, but their
    /// values are not kept behind a barrier (SPEC section 12 is the rule of
    /// the actions of a program): LLVM merges and removes their pure parts
    /// as it does those of a function.
    kernel_fn: bool,
    /// A primitive of the kernel was emitted: its metadata are the module's.
    kernel_used: bool,
    /// The quotients and remainders of the index of the loop being emitted
    /// by constants, carried from one iteration to the next (D180).
    divmod: Vec<DivMod>,
    /// In vector code read by `elem_replayed`: the block that computes the
    /// elements of the vector again one at a time, where a failed check
    /// jumps, so that the panic is the one of the scalar loop (D185).
    vec_fail: Option<String>,
    /// The texts freed now are parts of tuples (D518): tested as literals
    /// of the program before the call of `isc_free`.
    literal_test: bool,
    /// The variables whose block a `concat` may take (D533): read by it
    /// alone, then freed.
    taking: HashSet<VarId>,
    /// The owned arrays whose elements the gather of a permutation takes
    /// (D548): read without a copy, the array freed alone.
    moving: HashSet<VarId>,
    /// The `lines` and `split` whose parts are views of their text (D555):
    /// the values of the variables of `Program::views`, whatever path of
    /// the generated code binds them.
    view_parts: HashSet<*const Expr>,
    /// The texts taken out of tuples (their registers): tested as literals
    /// wherever they are freed (D518).
    part_values: HashSet<String>,
    /// The `let`s of a split `fold` (D260): the values of each element
    /// computed before its loop, in a slot on the stack, its type, and the
    /// variable of the element (the index into the slot).
    split_vars: HashMap<VarId, (String, String, VarId)>,
    /// While the values of a split `fold` are computed (D260): the block of
    /// the `fold` as written, where a failed check of an index jumps.
    split_fail: Option<String>,
    /// The last variable made for the parts of a split `fold` put into
    /// vectors (D260), counted down from the top of the identifiers.
    split_fresh: VarId,
    /// The array literals put on the stack (D191): never freed.
    stack_vars: HashSet<VarId>,
    /// The concatenations that give the value of a text that its scope only
    /// reads, and the buffer on the stack they write it into when it fits
    /// (D404).
    stack_concats: HashMap<*const Expr, String>,
    /// Such a text (its value) and its buffer: freed only when it is not in
    /// its buffer (D404).
    stack_strs: HashMap<String, String>,
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
    /// The arrays of `windowed_filter` (D293) in the body of their `let`: a
    /// copy of the `filter`; the variable holds the places of the blocks and
    /// the length.
    windowed: HashMap<VarId, Expr>,
    /// The arrays of `windowed_let` whose blocks start at an origin: the
    /// counts of a caller's blocks over the array they are a range of
    /// (D426), the origin after the slots of their places.
    windowed_origin: HashSet<VarId>,
    /// The tuples of sum types told apart by their type, with the slots of
    /// each variant: a value frees the fields of its variant only (D448).
    sum_slots: HashMap<Type, Vec<Vec<u32>>>,
    /// The largest byte of each variable whose bytes are known (D450).
    byte_bounds: HashMap<VarId, u8>,
    /// The functions whose windowed `filter` (D293) reads a range of one of
    /// their parameters with a predicate of its elements alone (D426): the
    /// index of the parameter and the lambda.
    count_users: HashMap<u32, (usize, &'p Lambda)>,
    /// The functions whose variant with the counts of a caller is called
    /// (`isc_fn_{f}_sm`, its first parameter).
    count_needed: BTreeSet<u32>,
    /// In such a variant: the lambda whose counts its parameter `%sm` gives.
    count_param: Option<(String, &'p Lambda)>,
    /// The functions of the prelude whose text `print` writes
    /// (`$rtF64Text`, `$rtF32Text`): their functions `isc_fn_{f}_print` and
    /// `isc_fn_{f}_digits` to emit, which call the variant `isc_fn_{f}_buf`.
    text_prints: BTreeSet<u32>,
    /// The functions whose variant `isc_fn_{f}_buf` is called: the texts it
    /// gives as its result written in a buffer of its caller (its first
    /// parameter) and not in a block, or data of the program; its result
    /// is not freed.
    buf_needed: BTreeSet<u32>,
    /// In such a variant: the register of the buffer.
    text_buf: Option<String>,
    /// The functions of the texts of the prelude (`Program::text_fns`)
    /// whose code is called, and those emitted: one called and not reached
    /// before (`text_fns_reached`) is emitted at the end.
    text_called: BTreeSet<u32>,
    text_emitted: HashSet<u32>,
    /// The texts that such a variant writes in its buffer and its calls of
    /// other such variants (`buffered_texts`).
    buf_texts: HashSet<*const Expr>,
    /// The arrays whose counts by blocks are made, while they are in scope:
    /// the block of the counts (`[base, length, count of block 0, ...]`,
    /// `base` null when they were not made).
    counts: HashMap<VarId, String>,
    /// While the `init` of a `let` that makes counts is emitted: the
    /// `filter` that makes them (its expression), the array, the lambda, the
    /// block of the counts and the pointer of the array.
    count_req: Option<(*const Expr, VarId, &'p Lambda, String, String)>,
    /// The indices of vector code checked before the loop of their `fold`
    /// (`fold_vec`, D296): read without a check of their bounds.
    prechecked: HashSet<*const Expr>,
    /// The variables bound to a string literal (borrowed, D301): their text.
    str_lits: HashMap<VarId, String>,
    /// The variables bound to `toString(x)` of an integer whose only use is
    /// the text of a `padStart` or `padEnd` (D348): `x` as an `i64`, signed.
    int_texts: HashMap<VarId, IntText>,
    /// The `padStart` at the end of the value of a `let` whose only use is a
    /// part of a concatenation (D359): described, not made.
    defer_pad: Option<*const Expr>,
    /// The description it gave.
    padded: Option<IntText>,
    /// The variables bound to a string literal (D363): parts of a template
    /// written as constants.
    lit_vars: HashMap<VarId, String>,
    /// The table of the powers of 10 emitted (the lengths of the integers
    /// of a template, D363).
    pow10: bool,
    /// The table of the pairs of digits `00` to `99` is a global (D441).
    digits2: bool,
    /// The functions of the large joins and of the joins never made are
    /// emitted (`join_fns`, D607).
    join_fns: bool,
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
    /// The functions of texts in another case written (`text_case_fn`): in
    /// upper case, with a buffer.
    case_fns: HashSet<(bool, bool)>,
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
/// The form `concat(a, b)` of arrays is lowered to (D136): `map(iota(n),
/// (i) -> if i < l then a[i] else b[i - l])`, `a`, `b`, `l` and `n` simple
/// (variables, projections, lengths, literals, sums) and free of `i`: its
/// parts `(a, b, l, n)`.
fn concat_parts(e: &Expr) -> Option<(&Expr, &Expr, &Expr, &Expr)> {
    use ExprKind::*;
    let Map(ins, lam) = &e.kind else { return None };
    if ins.len() != 1 || lam.params.len() != 1 {
        return None;
    }
    let Iota(n) = &ins[0].kind else { return None };
    let p = lam.params[0];
    let is_p = |x: &Expr| matches!(x.kind, Var(v, _) if v == p);
    let If(c, t, f) = &lam.body.kind else { return None };
    let Binary(BinOp::Lt, i, l) = &c.kind else { return None };
    let Index(a, i1) = &t.kind else { return None };
    let Index(b, j) = &f.kind else { return None };
    let Binary(BinOp::Sub, i2, l2) = &j.kind else { return None };
    let et = e.ty.elem();
    let fits = is_p(i) && is_p(i1) && is_p(i2) && same_simple(l, l2) && [&**a, &**b, &**l, &**n].iter().all(|x| simple_of(x, p));
    (fits && a.ty.is_array() && b.ty.is_array() && a.ty.elem() == et && b.ty.elem() == et).then(|| (&**a, &**b, &**l, &**n))
}

/// `concat(x, b)` of arrays of scalars is the only read of `x` in `e`, on its
/// straight path (`let`s and `free`s, no loop, no branch): its block may be
/// taken by the `concat` and grown (D533).
/// The `lines` and `split` that are the values of the variables of
/// `Program::views` (D555): made as views of their text.
fn view_parts(p: &Program) -> HashSet<*const Expr> {
    fn walk(e: &Expr, views: &std::collections::BTreeSet<VarId>, out: &mut HashSet<*const Expr>) {
        if let ExprKind::Let { var, init, .. } = &e.kind {
            if views.contains(var) && matches!(init.kind, ExprKind::StrOp(isc_ir::StrOp::Lines | isc_ir::StrOp::Split, _)) {
                out.insert(&**init as *const Expr);
            }
        }
        for c in e.children() {
            walk(c, views, out);
        }
    }
    let mut out = HashSet::new();
    walk(&p.body, &p.views, &mut out);
    for f in &p.fns {
        walk(&f.body, &p.views, &mut out);
    }
    out
}

fn concat_alone(e: &Expr, x: VarId) -> bool {
    let only = HashSet::from([x]);
    match &e.kind {
        ExprKind::Let { init, body, .. } => {
            if concat_reads(init, x) {
                return !mentions(body, &only) && !frees(body, x);
            }
            !mentions(init, &only) && concat_alone(body, x)
        }
        ExprKind::Free(y, b) if *y != x => concat_alone(b, x),
        _ => false,
    }
}

/// `m` is `map(iota(n), (i) -> if i < len(x) then x[i] else w[i - len(x)])`
/// (D136), its elements scalars, `x` read nowhere else in it but its
/// lengths.
fn concat_reads(m: &Expr, x: VarId) -> bool {
    let Some((a, _, _, _)) = concat_parts(m) else { return false };
    m.ty.elem().is_scalar() && matches!(a.kind, ExprKind::Var(v, _) if v == x)
}

/// A variable other than `p`, a projection or a length of one, a literal,
/// or a sum or difference of those: evaluated again without a cost
fn simple_of(x: &Expr, p: VarId) -> bool {
    use ExprKind::*;
    match &x.kind {
        Var(v, _) => *v != p,
        Lit(_) => true,
        Proj(y, _) | Len(y) => simple_of(y, p),
        Binary(BinOp::Add | BinOp::Sub, y, z) => simple_of(y, p) && simple_of(z, p),
        _ => false,
    }
}

/// The same simple expression (spans aside)
fn same_simple(x: &Expr, y: &Expr) -> bool {
    use ExprKind::*;
    match (&x.kind, &y.kind) {
        (Var(v, _), Var(w, _)) => v == w,
        (Lit(a), Lit(b)) => a == b,
        (Proj(a, i), Proj(b, j)) => i == j && same_simple(a, b),
        (Len(a), Len(b)) => same_simple(a, b),
        (Binary(o, a, b), Binary(q, c, d)) => o == q && same_simple(a, c) && same_simple(b, d),
        _ => false,
    }
}

/// Elements copied by bytes: scalars, or tuples of scalars (columns)
fn plain_columns(t: &Type) -> bool {
    match t {
        Type::Tuple(ts) => ts.iter().all(plain_columns),
        _ => t.is_scalar(),
    }
}

fn holds_owned(t: &Type) -> bool {
    match t {
        Type::Rec(_) | Type::Str | Type::Big => true,
        // an array in a tuple is a block of its own (an array of records
        // holding arrays: each record its own, freed and copied with it,
        // D336), not a row of the block of an array of arrays
        Type::Tuple(ts) => ts.iter().any(|c| c.is_array() || holds_owned(c)),
        Type::Array(e, _) => holds_owned(e),
        _ => false,
    }
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
    /// The target loads and stores bytes under a mask (AVX-512 BW and VL):
    /// a row of 33 to 64 bytes in one vector (D274).
    static BYTE_MASKS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
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
    "declare void @isc_print_bool(i32)",
    "declare void @isc_print_str(ptr, i64)",
    "declare void @isc_print_newline()",
    "declare void @isc_print_i64_line(i64)",
    "declare void @isc_print_u64_line(i64)",
    "declare void @isc_print_bytes(ptr, i64)",
    "declare void @isc_flush()",
    "declare i32 @isc_main_end(i32)",
    "declare void @isc_try_read_bytes(ptr, ptr, ptr, i64)",
    "declare void @isc_try_read_f64(ptr, ptr, ptr, i64)",
    "declare void @isc_try_read_text(ptr, ptr, ptr, i64)",
    "declare void @isc_try_read_csv_f64(ptr, ptr, ptr, i64, i64)",
    "declare void @isc_try_write_bytes(ptr, ptr, i64, ptr, i64)",
    "declare i64 @isc_clock_ns()",
    "declare void @isc_str_from_bytes(ptr, ptr, i64, i32, i32)",
    "declare void @isc_str_adopt_bytes(ptr, ptr, i64, i32, i32)",
    "declare void @isc_read_text(ptr, ptr, i64, i32, i32)",
    "declare void @isc_write_text(ptr, i64, ptr, i64, i32, i32)",
    "declare void @isc_lines(ptr, ptr, i64)",
    "declare void @isc_split(ptr, ptr, i64, ptr, i64, i32, i32)",
    "declare void @isc_lines_view(ptr, ptr, i64)",
    "declare void @isc_split_view(ptr, ptr, i64, ptr, i64, i32, i32)",
    "declare void @isc_replace_all(ptr, ptr, i64, ptr, i64, ptr, i64, i32)",
    "declare void @isc_rx_new(ptr, ptr, i64, ptr, i64, i32, i32)",
    "declare i64 @isc_rx_int(ptr, i64, ptr, i64, i32)",
    "declare void @isc_rx_matches(ptr, ptr, i64, ptr, i64)",
    "declare void @isc_rx_replace(ptr, ptr, i64, ptr, i64, ptr, i64, i32, i32, i32, i32)",
    "declare ptr @isc_str_reserve(ptr, i64, i64)",
    "declare void @isc_put_int(ptr, i64, i64, i64, i64, i64)",
    "declare void @isc_parallel_for_site(i64, ptr, ptr, i64, ptr, i32)",
    "declare void @isc_str_append(ptr, ptr, i64, ptr, i64)",
    "declare void @isc_str_append_n(ptr, ptr, i64)",
    "declare void @isc_panic_repeat(i64, i32, i32) noreturn",
    "declare void @isc_panic_str_slice(i64, i64, i64, i32, i32) noreturn",
    "declare void @isc_panic_boundary(i64, i32, i32) noreturn",
    "declare void @isc_panic_text_slice(ptr, i64, i64, i64, i32, i32) noreturn",
    "declare i64 @isc_last_index_of(ptr, i64, ptr, i64)",
    "declare void @isc_hash_index(ptr, ptr, i64, i32)",
    "declare i64 @isc_hash_find(ptr, i64, ptr, i64, i32, i64, ptr, i64)",
    "declare void @isc_panic_fixed(i64, i32, i32) noreturn",
    "declare void @isc_read_stdin(ptr, i32, i32)",
    "declare void @isc_argsort(ptr, ptr, i64, i32)",
    "declare void @isc_group(ptr, ptr, i64, i32)",
    "declare i64 @isc_group_parts(i64)",
    "declare ptr @isc_groups_new(i64, i64, i32)",
    "declare void @isc_groups_add(ptr, i64, ptr, i64, i64, i32)",
    "declare void @isc_groups_end(ptr, ptr)",
    "declare double @isc_js_number(ptr, i64, i32, i64)",
    "declare void @isc_print_big(ptr, i64)",
    "declare void @isc_big_op(ptr, ptr, i64, ptr, i64, i32, i32, i32)",
    "declare void @isc_big_muladd_div(ptr, ptr, i64, ptr, i64, ptr, i64, ptr, i64, i32, i32)",
    "declare void @isc_big_muladd(ptr, ptr, i64, ptr, i64, ptr, i64, i32, i32, i32)",
    "declare i32 @isc_big_cmp(ptr, i64, ptr, i64)",
    "declare void @isc_big_from_i64(ptr, i64)",
    "declare void @isc_big_from_u64(ptr, i64)",
    "declare void @isc_big_parse(ptr, ptr, i64, i32, i32)",
    "declare i64 @isc_big_to_i64(ptr, i64)",
    "declare double @isc_big_to_f64(ptr, i64)",
    "declare void @isc_big_to_string(ptr, ptr, i64)",
    "declare void @isc_read_stdin_text(ptr, i32, i32)",
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
    "declare void @isc_panic_word(i64, i32, i32) noreturn",
    "declare void @isc_panic_text(ptr, i64, ptr, i64, ptr, i64, i32, i32) noreturn",
    "declare void @isc_panic_tail(i64, i32, i32) noreturn",
    "declare void @llvm.memcpy.p0.p0.i64(ptr, ptr, i64, i1)",
    "declare ptr @llvm.stacksave.p0()",
    "declare void @llvm.stackrestore.p0(ptr)",
    "declare void @isc_parallel_for(i64, ptr, ptr, i64)",
    "declare void @isc_parallel_for_dynamic(i64, ptr, ptr, i64)",
    "declare void @isc_parallel_actions(i64, ptr, ptr, i64)",
    "declare void @isc_parallel_for_fill(i64, ptr, ptr, i64)",
    "declare i64 @isc_thread_count()",
    "declare ptr @isc_cstr(ptr, i64, ptr, i64, i32, i32)",
];

/// Emits the LLVM module of a program.
pub fn emit(p: &Program) -> R<String> {
    emit_with(p, &Options::default())
}

/// Emits the LLVM module of a program with explicit options.
pub fn emit_with(p: &Program, opts: &Options) -> R<String> {
    emit_mode(p, opts, Pre::Inline).map(|(text, _, _)| text)
}

/// The functions of the prelude that the code generator calls itself (the
/// allocator, the texts of numbers, the cases), compiled once for a
/// compiler and a target and not in every program (D611). Their names in
/// that module (`isc.pre.<name of the prelude><variant>`), each with a
/// word for all it was made from (its text and the texts of what it names
/// in the module) and with its declaration: a program declares one, and
/// leaves its body out, when it would make it from the same texts.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PreManifest {
    /// name → (what it was made from, its declaration)
    pub fns: std::collections::BTreeMap<String, (String, String)>,
    /// The module holds the allocator: its words, its rare paths.
    pub kernel: bool,
}

impl PreManifest {
    /// The manifest as the text kept beside the module.
    pub fn to_text(&self) -> String {
        let mut out = format!("kernel\t{}\n", self.kernel);
        for (name, (made, declaration)) in &self.fns {
            out.push_str(&format!("fn\t{}\t{}\t{}\n", name, made, declaration));
        }
        out
    }

    /// The manifest of such a text (none: not one of this compiler's).
    pub fn from_text(text: &str) -> Option<PreManifest> {
        let mut m = PreManifest::default();
        let mut lines = text.lines();
        m.kernel = lines.next()?.strip_prefix("kernel\t")?.parse().ok()?;
        for line in lines {
            let mut parts = line.strip_prefix("fn\t")?.splitn(3, '\t');
            let (name, made, declaration) = (parts.next()?, parts.next()?, parts.next()?);
            m.fns.insert(name.to_string(), (made.to_string(), declaration.to_string()));
        }
        Some(m)
    }
}

/// The declarations of the manifest, each made what LLVM wrote of the
/// function once the module was optimized (`optimized`, its text then): its
/// attributes and those of its parameters and of its result. A program
/// that calls a function of the module knows of it what the module itself
/// knew (it never returns, it writes nothing its caller reads, ...).
pub fn pre_declarations(optimized: &str, m: &mut PreManifest) {
    let mut groups: HashMap<&str, &str> = HashMap::new();
    for line in optimized.lines() {
        if let Some((n, body)) = line.strip_prefix("attributes #").and_then(|r| r.split_once(" = { ")) {
            groups.insert(n, body.trim_end().trim_end_matches('}').trim_end());
        }
    }
    for line in optimized.lines() {
        let Some(rest) = line.strip_prefix("define ") else { continue };
        let Some(name) = ir_defined(rest) else { continue };
        let Some(entry) = m.fns.get_mut(name) else { continue };
        let Some(open) = rest.find(&format!("@{}(", name)) else { continue };
        let close = ir_close(rest, open + name.len() + 1);
        let attrs: Vec<&str> = rest[close..].split_whitespace().filter_map(|t| t.strip_prefix('#')).filter_map(|n| groups.get(n).copied()).collect();
        entry.1 = format!("declare {} {}", &rest[..close], attrs.join(" ")).trim_end().to_string();
    }
}

/// A word for a text: equal texts, equal words (FNV-1a from two bases).
fn text_word(text: &str) -> String {
    let hash = |seed: u64| text.bytes().fold(seed, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3));
    format!("{:016x}{:016x}", hash(0xcbf29ce484222325), hash(0x84222325cbf29ce4))
}

/// What the functions of a module are made from (D611): the text of each
/// under the names every program gives (`canon`), the constants and the
/// words it names written as words for their values, and the functions of
/// the module it names.
struct PreMade<'a> {
    /// The functions of the module, by those names.
    texts: &'a HashMap<String, &'a str>,
    /// The constants and the words of the module: their values.
    values: &'a HashMap<&'a str, &'a str>,
    canon: &'a dyn Fn(&str) -> String,
    /// name → (a word for its text so written, the functions of the module
    /// it names)
    seen: HashMap<String, (String, Vec<String>)>,
}

impl PreMade<'_> {
    /// `text` so written, and the functions of the module it names.
    fn written(&self, text: &str, depth: usize) -> (String, Vec<String>) {
        let mut calls: Vec<String> = Vec::new();
        let out = ir_names(text, |n, out| {
            if let Some(value) = self.values.get(n).filter(|_| depth < 8) {
                let (v, more) = self.written(value, depth + 1);
                out.push_str("K.");
                out.push_str(&text_word(&v));
                calls.extend(more);
            } else {
                let c = (self.canon)(n);
                if self.texts.contains_key(&c) {
                    calls.push(c.clone());
                }
                out.push_str(&c);
            }
        });
        (out, calls)
    }

    /// The word for all the function `name` is made from: its text and the
    /// texts of the functions of the module it reaches.
    fn hash(&mut self, name: &str) -> String {
        let mut reached: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        let mut todo = vec![name.to_string()];
        while let Some(n) = todo.pop() {
            if !reached.insert(n.clone()) {
                continue;
            }
            if !self.seen.contains_key(&n) {
                // (each text read once, whatever the number of functions
                // that reach it)
                let (text, calls) = self.texts.get(&n).map(|t| self.written(t, 0)).unwrap_or_default();
                self.seen.insert(n.clone(), (text_word(&text), calls));
            }
            todo.extend(self.seen[&n].1.iter().cloned());
        }
        let mut all = String::new();
        for n in &reached {
            all.push_str(n);
            all.push(' ');
            all.push_str(&self.seen[n].0);
            all.push('\n');
        }
        text_word(&all)
    }
}

/// How a module stands to those functions.
#[derive(Clone, Copy)]
enum Pre<'a> {
    /// Each one the program reaches is emitted in its module (no module
    /// compiled once at hand).
    Inline,
    /// The module compiled once itself, from a program that reaches them
    /// all: they are its exported functions; it has no `main`.
    Make,
    /// A program linked with that module.
    Use(&'a PreManifest),
}

/// The module compiled once (D611), from the program that uses every
/// function of the prelude the code generator calls, and its manifest.
pub fn emit_pre(p: &Program, opts: &Options) -> R<(String, PreManifest)> {
    emit_mode(p, opts, Pre::Make).map(|(text, manifest, _)| (text, manifest))
}

/// The module of a program to be linked with the module compiled once, and
/// whether it takes anything from it (no: the program is whole without it).
pub fn emit_using(p: &Program, opts: &Options, pre: &PreManifest) -> R<(String, bool)> {
    emit_mode(p, opts, Pre::Use(pre)).map(|(text, _, using)| (text, using))
}

/// The definitions of a text of LLVM IR, one after the other: each function
/// (from its line `define` to its line `}`) alone, the lines between them
/// kept together.
fn ir_parts(text: &str) -> Vec<(bool, String)> {
    let mut out: Vec<(bool, String)> = Vec::new();
    let mut inside = false;
    for line in text.split_inclusive('\n') {
        if !inside && line.starts_with("define ") {
            inside = true;
            out.push((true, String::new()));
        } else if !inside && !matches!(out.last(), Some((false, _))) {
            out.push((false, String::new()));
        }
        out.last_mut().unwrap().1.push_str(line);
        if inside && line.trim_end() == "}" {
            inside = false;
        }
    }
    out
}

/// A name of LLVM IR after its sigil: quoted when it is not a plain one.
fn ir_quoted(name: &str) -> String {
    let plain = name.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '$' || c == '-') && !name.starts_with(|c: char| c.is_ascii_digit());
    if plain || name.starts_with('"') { name.to_string() } else { format!("\"{}\"", name) }
}

/// The end of the parenthesis opened at `open` of a line (after it).
fn ir_close(line: &str, open: usize) -> usize {
    let mut depth = 0usize;
    for (i, b) in line.bytes().enumerate().skip(open) {
        match b {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
            _ => {}
        }
    }
    line.len()
}

/// `text` with each name it refers to (`@name`) written by `name`, which is
/// given the name and writes it or another. The bytes of a constant text
/// (`c"…"`) are no names: a text of a program may hold an `@`.
fn ir_names(text: &str, mut name: impl FnMut(&str, &mut String)) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len() + 256);
    // (one pass: `done` what is written, `i` what is read)
    let (mut done, mut i) = (0, 0);
    while i < bytes.len() {
        match bytes[i] {
            b'@' => {
                let start = i + 1;
                let mut end = start;
                while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || matches!(bytes[end], b'_' | b'.' | b'$')) {
                    end += 1;
                }
                out.push_str(&text[done..start]);
                name(&text[start..end], &mut out);
                done = end;
                i = end;
            }
            // (to the quote that closes it: one inside is written `\22`)
            b'"' if i >= 2 && bytes[i - 1] == b'c' && bytes[i - 2] == b' ' => {
                i = text[i + 1..].find('"').map(|e| i + 1 + e + 1).unwrap_or(bytes.len());
            }
            _ => i += 1,
        }
    }
    out.push_str(&text[done..]);
    out
}

/// `text` with each `@name` of `names` written as its other name.
fn ir_named(text: &str, names: &HashMap<String, String>) -> String {
    if names.is_empty() {
        return text.to_string();
    }
    ir_names(text, |n, out| out.push_str(names.get(n).map(|x| x.as_str()).unwrap_or(n)))
}

/// The name of the function a line `define` defines.
fn ir_defined(header: &str) -> Option<&str> {
    let at = header.find('@')? + 1;
    let rest = &header[at..];
    Some(&rest[..rest.find('(')?])
}

/// `isc_fn_<i><variant>`: the function `i` of the program and its variant
/// (`_buf`, `_print`, ...), or nothing for another name.
fn lowered_name(name: &str) -> Option<(usize, &str)> {
    let rest = name.strip_prefix("isc_fn_")?;
    let digits = rest.bytes().take_while(|b| b.is_ascii_digit()).count();
    if digits == 0 {
        return None;
    }
    Some((rest[..digits].parse().ok()?, &rest[digits..]))
}

/// `text` with each `@isc_fn_<i><variant>` of `names` written as its name
/// in the module compiled once.
fn ir_renamed(text: &str, names: &HashMap<(usize, String), String>) -> String {
    if names.is_empty() {
        return text.to_string();
    }
    ir_names(text, |n, out| match lowered_name(n).and_then(|(i, variant)| names.get(&(i, variant.to_string()))) {
        Some(stable) => out.push_str(stable),
        None => out.push_str(n),
    })
}

fn emit_mode(p: &Program, opts: &Options, mode: Pre) -> R<(String, PreManifest, bool)> {
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
    BYTE_MASKS.with(|b| {
        let f = crate::backend::host_features();
        b.set(matches!(opts.isa, Isa::Native | Isa::Avx512) && ["+avx512bw", "+avx512vl"].iter().all(|x| f.split(',').any(|y| y == *x)))
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
        kernel_fn: false,
        kernel_used: false,
        task_shared: None,
        stream: None,
        scalar_loops: 0,
        windowed: HashMap::new(),
        windowed_origin: HashSet::new(),
        sum_slots: p.sum_slots.iter().cloned().collect(),
        byte_bounds: isc_ir::passes::byte_bounds(p),
        count_users: count_users(p),
        count_needed: BTreeSet::new(),
        count_param: None,
        text_prints: BTreeSet::new(),
        buf_needed: BTreeSet::new(),
        text_buf: None,
        buf_texts: HashSet::new(),
        text_called: BTreeSet::new(),
        text_emitted: HashSet::new(),
        counts: HashMap::new(),
        count_req: None,
        prechecked: HashSet::new(),
        str_lits: HashMap::new(),
        int_texts: HashMap::new(),
        defer_pad: None,
        padded: None,
        lit_vars: HashMap::new(),
        pow10: false,
        digits2: false,
        join_fns: false,
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
        literal_test: false,
        taking: HashSet::new(),
        moving: HashSet::new(),
        view_parts: view_parts(p),
        part_values: HashSet::new(),
        split_vars: HashMap::new(),
        split_fail: None,
        split_fresh: VarId::MAX,
        stack_vars: HashSet::new(),
        stack_concats: HashMap::new(),
        stack_strs: HashMap::new(),
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
        case_fns: HashSet::new(),
        fast_math: false,
        other_tails: HashMap::new(),
        pre: HashMap::new(),
        wide_task: false,
        wide_used: false,
    };
    // the recursive functions (D102); of those of the texts of the prelude,
    // the ones the program reaches (a program that writes the text of a
    // `f64` has no use of the function of a `f32`: 3700 lines of LLVM IR
    // read for nothing)
    let reached = text_fns_reached(p);
    for i in 0..p.fns.len() {
        if (i as u32) < p.text_fns || reached.contains(&(i as u32)) {
            g.text_emitted.insert(i as u32);
            g.emit_fn(i, false, false, false)?;
        }
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
    // its output written, the allocator checked at its end (D546)
    let end = g.ins(format!("call i32 @isc_main_end(i32 {})", r.r));
    g.emit(format!("ret i32 {}", end));
    let attrs = g.attrs();
    // the C library of WASI calls `__main_argc_argv`, the name clang gives to
    // `main(argc, argv)` on WebAssembly
    let main_name = if opts.isa.is_wasm() { "__main_argc_argv" } else { "main" };
    // with recursive functions, the body of `main` runs on a thread with a
    // large stack (`isc_run_big_stack`, D102)
    // (the functions of the allocator written in Isocron are in every
    // program, D605, and none nests: they do not make a program one with
    // recursive functions)
    let big_stack = p.fns.len() > p.kernel_fns().len() && !opts.isa.is_wasm();
    let body_name = if big_stack { "isc_main_body" } else { main_name };
    let mut main = g.finish(&format!("define i32 @{}(i32 %argc, ptr %argv){}", body_name, attrs));
    // the variants of the builders whose trees are made in a region (D200),
    // each of which may ask for others
    // and those that take the counts of a caller (D426)
    let mut done: HashSet<u32> = HashSet::new();
    let mut done_counted: HashSet<u32> = HashSet::new();
    loop {
        if let Some(f) = g.rg_needed.iter().copied().find(|f| !done.contains(f)) {
            done.insert(f);
            g.emit_fn(f as usize, true, false, false)?;
        } else if let Some(f) = g.count_needed.iter().copied().find(|f| !done_counted.contains(f)) {
            done_counted.insert(f);
            g.emit_fn(f as usize, false, true, false)?;
        } else {
            break;
        }
    }
    g.emit_rec_helpers()?;
    // `print` of a float: its functions, and the variants of the functions
    // of the prelude that write its text in their buffer, each of which
    // may ask for others
    let mut done_prints: HashSet<u32> = HashSet::new();
    let mut done_buffered: HashSet<u32> = HashSet::new();
    loop {
        if let Some(f) = g.text_prints.iter().copied().find(|f| !done_prints.contains(f)) {
            done_prints.insert(f);
            g.emit_text_print(f)?;
        } else if let Some(f) = g.buf_needed.iter().copied().find(|f| !done_buffered.contains(f)) {
            done_buffered.insert(f);
            g.emit_fn(f as usize, false, false, true)?;
        } else if let Some(f) = g.text_called.iter().copied().find(|f| !g.text_emitted.contains(f)) {
            // called by a function of `print` or by a variant `_buf` alone
            g.text_emitted.insert(f);
            g.emit_fn(f as usize, false, false, false)?;
        } else {
            break;
        }
    }
    if big_stack {
        g.decls.insert("declare i32 @isc_run_big_stack(ptr, i32, ptr)".into());
        main.push_str(&format!(
            "\ndefine i32 @{}(i32 %argc, ptr %argv) {{\nentry:\n  %r = call i32 @isc_run_big_stack(ptr @isc_main_body, i32 %argc, ptr %argv)\n  ret i32 %r\n}}\n",
            main_name
        ));
    }
    // The allocator written in Isocron (D605): every allocation and every
    // free of the module goes through an entry of the module itself. A
    // block of 1 to 1024 bytes is the kernel's (`$kAlloc`, which asks the
    // runtime when the arena is not reserved), a larger one the runtime's;
    // a block given back is the kernel's when it is in the arena (`$kFree`
    // says so), the runtime's otherwise. The entries are declared as the
    // runtime's were (`noalias`) and not inlined while the module is
    // optimized; the backend inlines them after (D600, `LATE_INLINE`).
    // `main` starts the arena when the allocator of Isocron is the one in
    // use (`isc_heap_mode` 2): the runtime reads `ISC_ALLOC` at its first
    // allocation (D81), so a first block is taken from it and given back
    // before the mode is read.
    let mut kernel_text = String::new();
    // (on Linux: the arena is not reserved under Windows nor in WebAssembly
    // yet, where the generated code calls the runtime as before)
    if let (Some((start, alloc, free)), false) = (p.kernel, opts.isa.is_wasm() || opts.windows) {
        let late = |text: &str| -> String {
            text.replace("call noalias ptr @isc_alloc(", "call noalias ptr @isc.late.alloc(")
                .replace("call ptr @isc_alloc(", "call noalias ptr @isc.late.alloc(")
                .replace("call noalias ptr @isc_alloc_zeroed(", "call noalias ptr @isc.late.alloc_zeroed(")
                .replace("call ptr @isc_alloc_zeroed(", "call noalias ptr @isc.late.alloc_zeroed(")
                .replace("call void @isc_free(ptr ", "call void @isc.late.free(ptr ")
                .replace("call void @isc_free_strs(ptr ", "call void @isc.late.free_strs(ptr ")
        };
        for f in g.funcs.iter_mut() {
            *f = late(f);
        }
        // (after `isc_args`: a program without the C library reads its
        // environment, `ISC_ALLOC`, through what that call recorded)
        main = late(&main);
        let args_call = main.find("call void @isc_args(").expect("the arguments of `main` recorded");
        let after = args_call + main[args_call..].find('\n').expect("a line") + 1;
        main.insert_str(after, "  call void @isc.kernel.start()\n");
        g.decls.insert("@isc_heap_mode = external global i8".into());
        g.decls.insert("declare void @llvm.memset.p0.i64(ptr, i8, i64, i1)".into());
        g.decls.insert("declare void @isc_free_strs(ptr, i64)".into());
        let small = format!(
            "entry:\n  %m = add i64 %n, -1\n  %small = icmp ult i64 %m, {}\n  br i1 %small, label %fast, label %slow\nfast:\n  %a = call tailcc i64 @isc_fn_{}(i64 %n)\n  %p = inttoptr i64 %a to ptr\n",
            KERNEL_SMALL, alloc
        );
        kernel_text = format!(
            "\ndefine internal void @isc.kernel.start() noinline nounwind {{\nentry:\n  %first = call ptr @isc_alloc(i64 1)\n  call void @isc_free(ptr %first)\n  %mode = load i8, ptr @isc_heap_mode\n  %on = icmp eq i8 %mode, 2\n  br i1 %on, label %go, label %done\ngo:\n  %b = call tailcc i64 @isc_fn_{start}()\n  br label %done\ndone:\n  ret void\n}}\n\
             \ndefine internal noalias ptr @isc.late.alloc(i64 %n) noinline nounwind {{\n{small}  ret ptr %p\nslow:\n  %q = call noalias ptr @isc_alloc(i64 %n)\n  ret ptr %q\n}}\n\
             \ndefine internal noalias ptr @isc.late.alloc_zeroed(i64 %n) noinline nounwind {{\n{small}  call void @llvm.memset.p0.i64(ptr %p, i8 0, i64 %n, i1 false)\n  ret ptr %p\nslow:\n  %q = call noalias ptr @isc_alloc_zeroed(i64 %n)\n  ret ptr %q\n}}\n\
             \ndefine internal void @isc.late.free(ptr %p) noinline nounwind {{\nentry:\n  %a = ptrtoint ptr %p to i64\n  %c = call tailcc i64 @isc_fn_{free}(i64 %a)\n  %ours = icmp ne i64 %c, 0\n  br i1 %ours, label %done, label %slow\ndone:\n  ret void\nslow:\n  call void @isc_free(ptr %p)\n  ret void\n}}\n\
             \ndefine internal void @isc.kernel.free(ptr %p) nounwind {{\nentry:\n  %a = ptrtoint ptr %p to i64\n  %c = call tailcc i64 @isc_fn_{free}(i64 %a)\n  ret void\n}}\n\
             {strs}",
            start = start,
            small = small,
            free = free,
            strs = free_strs_text(free)
        );
    }
    // The functions compiled once (D611). In the module that holds them,
    // each function lowered for the code generator alone (`text_fns` on,
    // a name of the prelude that no other of them has, not one inlined
    // everywhere) is exported under that name; in a program linked with
    // that module, the same function is declared and its body left out.
    // "The same": what the program would emit for it and for all it is
    // made of (the functions it calls in the module, the constants and the
    // words it names) is, text for text, what the module was made from;
    // a function that differs in anything stays in the program. The
    // allocator the same way, all of it or none: its words and its
    // functions that are not inlined are the module's, and a program whose
    // allocator would differ takes nothing from the module.
    let mut manifest = PreManifest::default();
    let mut using = false;
    if !matches!(mode, Pre::Inline) {
        let wired = !(opts.isa.is_wasm() || opts.windows) && p.kernel.is_some();
        let kernel = p.kernel_fns();
        // (and each function that only the prelude can name, lowered for a
        // call of the program: the rare paths of the reading of a float,
        // `@cold`, are the same in every program that reads one)
        let mut named: HashMap<&str, usize> = HashMap::new();
        for f in p.fns.iter() {
            *named.entry(f.name.as_str()).or_default() += 1;
        }
        // the name of the function `i` that every program gives it
        let stable = |i: usize| -> Option<String> {
            let f = p.fns.get(i)?;
            let ours = f.name.starts_with('$') && named.get(f.name.as_str()) == Some(&1);
            ours.then(|| format!("isc.pre.{}", &f.name[1..]))
        };
        let canon = |name: &str| -> String { lowered_name(name).and_then(|(i, variant)| stable(i).map(|n| format!("{}{}", n, variant))).unwrap_or_else(|| name.to_string()) };
        let entries = ["isc.kernel.start", "isc.kernel.free", "isc.late.free_strs"];
        let fn_parts = ir_parts(&(g.funcs.join("\n") + "\n"));
        let kernel_parts = ir_parts(&kernel_text);
        let mut texts: HashMap<String, &str> = HashMap::new();
        for (is_fn, text) in fn_parts.iter().chain(kernel_parts.iter()) {
            if let Some(name) = text.lines().next().filter(|_| *is_fn).and_then(ir_defined) {
                texts.insert(canon(name), text.as_str());
            }
        }
        let mut values: HashMap<&str, &str> = HashMap::new();
        for d in g.globals.iter().chain(g.decls.iter()) {
            if let Some((name, value)) = d.strip_prefix('@').and_then(|r| r.split_once(" = ")) {
                values.insert(name, value);
            }
        }
        // the functions that may be the module's: (part, function, variant, name)
        let candidate = |is_fn: bool, text: &str| -> Option<(usize, String, String)> {
            let header = text.lines().next()?;
            // one that is never inlined: called the same way from another
            // module (one that LLVM may inline into its callers stays with
            // them, in each program)
            if !is_fn || !header.starts_with("define internal ") || !header.contains(" noinline") {
                return None;
            }
            let (i, variant) = lowered_name(ir_defined(header)?)?;
            let name = stable(i).filter(|_| wired || !kernel.contains(&(i as u32)))?;
            Some((i, variant.to_string(), format!("{}{}", name, variant)))
        };
        let mut made = PreMade { texts: &texts, values: &values, canon: &canon, seen: HashMap::new() };
        let mut hashes: HashMap<String, String> = HashMap::new();
        let mut group: Vec<String> = Vec::new();
        for (is_fn, text) in &fn_parts {
            if let Some((i, _, symbol)) = candidate(*is_fn, text) {
                hashes.insert(symbol.clone(), made.hash(&symbol));
                if kernel.contains(&(i as u32)) {
                    group.push(symbol);
                }
            }
        }
        if wired {
            for e in entries.iter().filter(|e| texts.contains_key(**e)) {
                hashes.insert(e.to_string(), made.hash(e));
                group.push(e.to_string());
            }
        }
        // what this module takes from the one compiled once
        let same = |symbol: &str| -> bool {
            match mode {
                Pre::Make => true,
                Pre::Use(m) => m.fns.get(symbol).map(|e| &e.0) == hashes.get(symbol),
                Pre::Inline => false,
            }
        };
        let whole = match mode {
            Pre::Use(m) => !wired || (m.kernel && group.len() >= entries.len() && group.iter().all(|s| same(s))),
            _ => true,
        };
        let declaration = |symbol: &str| -> Option<String> {
            match mode {
                Pre::Use(m) => m.fns.get(symbol).map(|e| e.1.clone()),
                _ => None,
            }
        };
        if whole {
            let mut names: HashMap<(usize, String), String> = HashMap::new();
            let mut declared: Vec<String> = Vec::new();
            let mut kept = String::new();
            for (is_fn, text) in &fn_parts {
                let Some((i, variant, symbol)) = candidate(*is_fn, text).filter(|c| same(&c.2)) else {
                    kept.push_str(text);
                    continue;
                };
                names.insert((i, variant.clone()), symbol.clone());
                if matches!(mode, Pre::Make) {
                    // (declared so until LLVM has said more of it:
                    // `pre_declarations`)
                    let header = text.lines().next().unwrap_or("");
                    let signature = header["define internal ".len()..].trim_end_matches(" {").replacen(&format!("@isc_fn_{}{}(", i, variant), &format!("@{}(", symbol), 1);
                    manifest.fns.insert(symbol.clone(), (hashes[&symbol].clone(), format!("declare hidden {}", signature)));
                    kept.push_str(&text.replacen("define internal ", "define hidden ", 1));
                } else {
                    declared.extend(declaration(&symbol));
                }
            }
            if wired {
                let mut text = String::new();
                for (is_fn, part) in &kernel_parts {
                    let name = part.lines().next().and_then(ir_defined).unwrap_or("");
                    if !(*is_fn && entries.contains(&name)) {
                        text.push_str(part);
                    } else if matches!(mode, Pre::Make) {
                        let header = part.lines().next().unwrap_or("");
                        let signature = header["define internal ".len()..].trim_end_matches(" {");
                        manifest.fns.insert(name.to_string(), (hashes[name].clone(), format!("declare hidden {}", signature)));
                        text.push_str(&part.replacen("define internal ", "define hidden ", 1));
                    } else {
                        declared.extend(declaration(name));
                    }
                }
                kernel_text = text;
                let words: Vec<String> = g.decls.iter().filter(|d| d.starts_with("@isc_arena_base = ") || d.starts_with("@isc_kernel_zone = ")).cloned().collect();
                for w in words {
                    g.decls.remove(&w);
                    g.decls.insert(match mode {
                        Pre::Make => w.replacen("= internal ", "= hidden ", 1),
                        // (declared: its value is the other module's)
                        _ if w.starts_with("@isc_arena_base") => "@isc_arena_base = external hidden global i64".to_string(),
                        _ => w.replacen("= internal ", "= external hidden ", 1).replacen(" zeroinitializer", "", 1),
                    });
                }
                manifest.kernel = true;
            }
            // The large constants that are data alone (tables: no name in
            // them), under a name made from their value, in the module and
            // in the program alike, each in a group of its own: the linker
            // keeps one of the two. (Defined in both, not declared in the
            // program: LLVM reads the values of a table it can see.)
            let mut data_names: HashMap<String, String> = HashMap::new();
            let mut held: HashSet<String> = HashSet::new();
            let mut data_groups: Vec<String> = Vec::new();
            for gl in g.globals.iter_mut() {
                let Some((name, value)) = gl.strip_prefix('@').and_then(|r| r.split_once(" = ")) else { continue };
                let Some(data) = value.strip_prefix("private unnamed_addr constant ").filter(|d| d.len() >= 256 && !d.contains('@')) else { continue };
                let symbol = format!("isc.pre.k.{}", text_word(value));
                let line = format!("@{} = linkonce_odr hidden unnamed_addr constant {}, comdat", symbol, data);
                data_names.insert(name.to_string(), symbol.clone());
                // (two of one value: one)
                *gl = if held.insert(symbol.clone()) {
                    data_groups.push(format!("${} = comdat any", symbol));
                    line
                } else {
                    String::new()
                };
            }
            for c in data_groups {
                g.decls.insert(c);
            }
            using = !names.is_empty() || wired;
            g.funcs = vec![ir_named(&ir_renamed(&kept, &names), &data_names)];
            main = ir_named(&ir_renamed(&main, &names), &data_names);
            kernel_text = ir_named(&ir_renamed(&kernel_text, &names), &data_names);
            for gl in g.globals.iter_mut() {
                *gl = ir_named(gl, &data_names);
            }
            for d in declared {
                g.decls.insert(d);
            }
            if matches!(mode, Pre::Use(_)) {
                // The functions that only those of the module called are no
                // longer reached: left out here, not handed to LLVM to be
                // read, simplified, then dropped.
                let (fns, entries) = (ir_parts(&g.funcs.concat()), ir_parts(&kernel_text));
                let mut bodies: HashMap<&str, &str> = HashMap::new();
                let mut todo: Vec<&str> = vec![main.as_str()];
                todo.extend(g.globals.iter().chain(g.decls.iter()).map(|d| d.as_str()));
                for (is_fn, text) in fns.iter().chain(entries.iter()) {
                    let header = text.lines().next().unwrap_or("");
                    match ir_defined(header).filter(|_| *is_fn && header.starts_with("define internal ")) {
                        Some(name) => {
                            bodies.insert(name, text.as_str());
                        }
                        None => todo.push(text.as_str()),
                    }
                }
                let mut reached: HashSet<&str> = HashSet::new();
                // (inlined after the passes, by their names: D600)
                for name in crate::backend::LATE_INLINE {
                    if let Some((n, body)) = bodies.get_key_value(name) {
                        reached.insert(*n);
                        todo.push(*body);
                    }
                }
                while let Some(text) = todo.pop() {
                    let mut rest = text;
                    while let Some(at) = rest.find('@') {
                        let name = &rest[at + 1..];
                        let end = name.bytes().position(|b| !(b.is_ascii_alphanumeric() || b == b'_' || b == b'.' || b == b'$')).unwrap_or(name.len());
                        if let Some((n, body)) = bodies.get_key_value(&name[..end]) {
                            if reached.insert(*n) {
                                todo.push(*body);
                            }
                        }
                        rest = &name[end..];
                    }
                }
                let live = |parts: &[(bool, String)]| -> String {
                    let mut out = String::new();
                    for (is_fn, text) in parts {
                        let header = text.lines().next().unwrap_or("");
                        let dead = *is_fn && header.starts_with("define internal ") && ir_defined(header).is_some_and(|n| !reached.contains(n));
                        if !dead {
                            out.push_str(text);
                        }
                    }
                    out
                };
                let (kept, kept_entries) = (live(&fns), live(&entries));
                g.funcs = vec![kept];
                kernel_text = kept_entries;
            }
        }
        if matches!(mode, Pre::Make) {
            // (no program in it: no `main`, no start of the code)
            main = String::new();
            // Each function and each constant in a section of its own: the
            // linker keeps what a program reaches and nothing else of the
            // module. In a Windows object, a group of its own, the only
            // sections a linker leaves out there; a group is named by a
            // symbol, which a `private` constant has not: `internal` there.
            let coff = opts.windows && !opts.isa.is_wasm();
            let mut groups: Vec<String> = Vec::new();
            let mut place = |kind: &str, name: &str, sep: &str| -> String {
                if coff {
                    groups.push(format!("${} = comdat nodeduplicate", ir_quoted(name)));
                    format!("{}comdat", sep)
                } else {
                    let plain: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '_' { c } else { '_' }).collect();
                    format!("{}section \"{}.{}\"", sep, kind, plain.trim_start_matches('.'))
                }
            };
            let mut placed = |text: &str| -> String {
                let mut out = String::with_capacity(text.len() + 4096);
                for line in text.split_inclusive('\n') {
                    let name = line.starts_with("define ").then(|| ir_defined(line)).flatten();
                    let Some(name) = name.filter(|_| !line.contains(" alwaysinline")) else {
                        out.push_str(line);
                        continue;
                    };
                    // after the attributes of the function, before its alignment
                    let end = line.rfind(" {").unwrap_or(line.len());
                    let params = line.find('(').map(|open| ir_close(line, open)).unwrap_or(end);
                    let at = line[params..end].rfind(" align ").map(|a| params + a).unwrap_or(end);
                    let kind = if line[params..end].contains(" cold") { ".text.unlikely" } else { ".text" };
                    out.push_str(&line[..at]);
                    out.push_str(&place(kind, name, " "));
                    out.push_str(&line[at..]);
                }
                out
            };
            g.funcs = vec![placed(&g.funcs.concat())];
            kernel_text = placed(&kernel_text);
            for gl in g.globals.iter_mut() {
                let name = gl.strip_prefix('@').and_then(|r| r.split_once(" = ")).map(|(n, _)| n.to_string());
                if let Some(name) = name.filter(|_| gl.contains(" constant ") && !gl.ends_with(", comdat")) {
                    let at = place(".rodata", &name, ", ");
                    if coff {
                        *gl = gl.replacen(" = private ", " = internal ", 1);
                    }
                    gl.push_str(&at);
                }
            }
            for c in groups {
                g.decls.insert(c);
            }
        }
    }
    let making = matches!(mode, Pre::Make);
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
    if !opts.isa.is_wasm() && !making {
        out.push_str("define hidden void @isc_code_start() align 4096 {\nentry:\n  ret void\n}\n\n");
    }
    for f in &g.funcs {
        out.push_str(f);
        out.push('\n');
    }
    out.push_str(&main);
    out.push_str(&kernel_text);
    if opts.isa == Isa::Wasm32Threads && !making {
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
    if g.kernel_used {
        out.push_str(&format!(
            "\n!{0} = !{{!\"isc kernel\"}}\n!{1} = !{{!\"arena base\", !{0}, i64 0}}\n!{2} = !{{!\"arena word\", !{0}, i64 0}}\n!{3} = !{{!\"zone word\", !{0}, i64 0}}\n!{4} = !{{!{1}, !{1}, i64 0}}\n!{5} = !{{!{2}, !{2}, i64 0}}\n!{6} = !{{!{3}, !{3}, i64 0}}\n",
            KERNEL_IDS,
            KERNEL_IDS + 1,
            KERNEL_IDS + 2,
            KERNEL_IDS + 3,
            TBAA_BASE,
            TBAA_ARENA,
            TBAA_ZONE
        ));
    }
    if g.wide_used {
        out.push_str("\nattributes #1 = { \"min-legal-vector-width\"=\"512\" \"prefer-vector-width\"=\"512\" }\n");
    }

    Ok((out, manifest, using))
}

/// The texts of an array freed by the module itself (D605): each a block of
/// the arena given back to the kernel where it is (`$kFree`, inlined), not
/// one by one through the runtime, which would give each back by a call.
/// What is not such a block is the runtime's: a line of a block of lines
/// (8 bytes past a multiple of 16, D298), and the rest of the array with it
/// (the runtime counts the lines of one block down at once, D305); any
/// other text alone (a literal, the empty text, a block of the runtime).
/// An array of 4096 texts or more whose places are out of order and whose
/// blocks take 12 MiB or more (more than one text in 16 below the one
/// before, the blocks counted with their headers, one text in 16 looked at:
/// `scattered` of the runtime, D557) is the runtime's whole: it frees such
/// texts from the highest place down, and each block of the arena comes
/// back by its entry.
fn free_strs_text(free: u32) -> String {
    format!(
        "\ndefine internal void @isc.late.free_strs(ptr %xs, i64 %n) noinline nounwind {{\n\
entry:\n  %big = icmp sge i64 %n, 4096\n  br i1 %big, label %sh, label %head\n\
sh:\n  %j = phi i64 [ 0, %entry ], [ %j1, %sb ]\n  %below = phi i64 [ 0, %entry ], [ %below1, %sb ]\n  %prev = phi i64 [ 0, %entry ], [ %sa, %sb ]\n  %seen = phi i64 [ 0, %entry ], [ %seen1, %sb ]\n  %bytes = phi i64 [ 0, %entry ], [ %bytes1, %sb ]\n  %sgo = icmp slt i64 %j, %n\n  br i1 %sgo, label %sb, label %sd\n\
sb:\n  %sp = getelementptr {{ ptr, i64 }}, ptr %xs, i64 %j, i32 0\n  %spp = load ptr, ptr %sp\n  %sa = ptrtoint ptr %spp to i64\n  %slt = icmp ult i64 %sa, %prev\n  %slz = zext i1 %slt to i64\n  %below1 = add i64 %below, %slz\n  %seen1 = add i64 %seen, 1\n  %slp = getelementptr {{ ptr, i64 }}, ptr %xs, i64 %j, i32 1\n  %slen = load i64, ptr %slp\n  %sl63 = add i64 %slen, 63\n  %slr = and i64 %sl63, -64\n  %sls = icmp ult i64 %slr, 64\n  %slm = select i1 %sls, i64 64, i64 %slr\n  %sblk = add i64 %slm, 64\n  %sal = and i64 %sa, 63\n  %s64 = icmp eq i64 %sal, 0\n  %sadd = select i1 %s64, i64 %sblk, i64 0\n  %bytes1 = add i64 %bytes, %sadd\n  %j1 = add i64 %j, 16\n  br label %sh\n\
sd:\n  %b16 = shl i64 %below, 4\n  %out = icmp ugt i64 %b16, %seen\n  %span = shl i64 %bytes, 4\n  %wide = icmp uge i64 %span, 12582912\n  %both = and i1 %out, %wide\n  br i1 %both, label %whole, label %head\n\
whole:\n  call void @isc_free_strs(ptr %xs, i64 %n)\n  ret void\n\
head:\n  %i = phi i64 [ 0, %entry ], [ 0, %sd ], [ %i1, %next ]\n  %more = icmp slt i64 %i, %n\n  br i1 %more, label %body, label %done\n\
body:\n  %at = getelementptr {{ ptr, i64 }}, ptr %xs, i64 %i, i32 0\n  %p = load ptr, ptr %at\n  %a = ptrtoint ptr %p to i64\n  %c = call tailcc i64 @isc_fn_{free}(i64 %a)\n  %ours = icmp ne i64 %c, 0\n  br i1 %ours, label %next, label %other\n\
other:\n  %low = and i64 %a, 15\n  %line = icmp eq i64 %low, 8\n  br i1 %line, label %rest, label %one\n\
one:\n  call void @isc_free(ptr %p)\n  br label %next\n\
next:\n  %i1 = add i64 %i, 1\n  br label %head\n\
rest:\n  %xr = getelementptr {{ ptr, i64 }}, ptr %xs, i64 %i\n  %nr = sub i64 %n, %i\n  call void @isc_free_strs(ptr %xr, i64 %nr)\n  ret void\n\
done:\n  ret void\n}}\n",
        free = free
    )
}

/// The base of the arena before it is reserved (D605): an address where a
/// program has no memory (it is not one the processor accepts, with 48 or
/// with 57 bits of addresses), so that no address of the program is at an
/// offset below the size of the arena from it: every test of the arena
/// fails. (A word of the `KERNEL_ARENA` addresses from it would pass the
/// test and be refused by the processor: a fault, no memory read or
/// written.)
const NO_ARENA: u64 = 1 << 62;
/// The bytes of the largest block of the kernel written in Isocron (16
/// classes of 64 bytes); a larger one is the runtime's (D605).
const KERNEL_SMALL: u64 = 1024;

/// The text of an integer not made (D348, D359): the integer (an `i64`), its
/// sign, and its padding (the length, the byte of the fill, `padStart`).
#[derive(Clone)]
struct IntText {
    x: String,
    signed: bool,
    pad: Option<(String, u8, bool)>,
}

/// The fill of a padded text (D606): one byte known to the compiler, or a
/// text (its pointer and its length).
enum Fill {
    Byte(u8),
    Text(String, String),
}

/// `v` is read once in `e`, as a part of a concatenation (a chain, a
/// template), outside the functions of a `map` and the others (D359).
fn sole_concat_part(e: &Expr, v: VarId) -> bool {
    fn uses(e: &Expr, v: VarId) -> usize {
        matches!(e.kind, ExprKind::Var(w, _) if w == v) as usize + e.children().into_iter().map(|c| uses(c, v)).sum::<usize>()
    }
    fn at(e: &Expr, v: VarId) -> bool {
        match &e.kind {
            ExprKind::StrOp(isc_ir::StrOp::Concat, es) if es.iter().any(|x| matches!(x.kind, ExprKind::Var(w, _) if w == v)) => true,
            ExprKind::Map(..) | ExprKind::Reduce(..) | ExprKind::Fold(..) | ExprKind::Scan(..) | ExprKind::Filter(..) | ExprKind::Tasks(..) => false,
            _ => e.children().into_iter().any(|c| at(c, v)),
        }
    }
    uses(e, v) == 1 && at(e, v)
}

/// The `padStart` or `padEnd` at the end of `e` (after its `let`s) of the
/// text of an integer bound by one of those `let`s (`pad(x, n)` of `time`,
/// inlined), and no other in `e` (D359).
fn tail_pad(e: &Expr) -> Option<&Expr> {
    fn pads(e: &Expr) -> usize {
        matches!(e.kind, ExprKind::StrOp(isc_ir::StrOp::PadStart | isc_ir::StrOp::PadEnd, _)) as usize + e.children().into_iter().map(pads).sum::<usize>()
    }
    let mut t = e;
    let mut texts = Vec::new();
    while let ExprKind::Let { var, init, body, .. } = &t.kind {
        if let ExprKind::StrOp(isc_ir::StrOp::ToString, xs) = &init.kind {
            if matches!(xs[0].ty, Type::Int(_)) {
                texts.push(*var);
            }
        }
        t = body;
    }
    match &t.kind {
        ExprKind::StrOp(isc_ir::StrOp::PadStart | isc_ir::StrOp::PadEnd, es) if pads(e) == 1 && matches!(es[0].kind, ExprKind::Var(w, _) if texts.contains(&w)) => Some(t),
        _ => None,
    }
}

/// `v` is read once in `e`, as the text of a `padStart` or `padEnd`, outside
/// the functions of a `map`, `reduce`, `fold`, `scan` or `filter` (D348).
fn sole_pad_text(e: &Expr, v: VarId) -> bool {
    fn uses(e: &Expr, v: VarId) -> usize {
        matches!(e.kind, ExprKind::Var(w, _) if w == v) as usize + e.children().into_iter().map(|c| uses(c, v)).sum::<usize>()
    }
    fn at_pad(e: &Expr, v: VarId) -> bool {
        match &e.kind {
            ExprKind::StrOp(isc_ir::StrOp::PadStart | isc_ir::StrOp::PadEnd, es) if matches!(es[0].kind, ExprKind::Var(w, _) if w == v) => true,
            ExprKind::Map(..) | ExprKind::Reduce(..) | ExprKind::Fold(..) | ExprKind::Scan(..) | ExprKind::Filter(..) | ExprKind::Tasks(..) => false,
            _ => e.children().into_iter().any(|c| at_pad(c, v)),
        }
    }
    uses(e, v) == 1 && at_pad(e, v)
}

/// `let t = (let p = x * y in p + z) in t / c` of bigints, `p` and `t`
/// temporaries read once there, no operand moved (D389): `x`, `y`, `z`,
/// `c` and the place of the division.
fn muladd_div<'a>(t: VarId, init: &'a Expr, body: &'a Expr) -> Option<(&'a Expr, &'a Expr, &'a Expr, &'a Expr, Span)> {
    use isc_ir::BigOp as B;
    let is = |e: &Expr, v: VarId| matches!(e.kind, ExprKind::Var(w, _) if w == v);
    let moved = |e: &Expr| matches!(e.kind, ExprKind::Var(_, Use::Move));
    let ExprKind::StrOp(isc_ir::StrOp::Big(B::Div), d) = &body.kind else { return None };
    if d.len() != 2 || !is(&d[0], t) || mentions(&d[1], &HashSet::from([t])) {
        return None;
    }
    let ExprKind::Let { var: p, init: m, body: a, drop: true, stmt: false } = &init.kind else { return None };
    let ExprKind::StrOp(isc_ir::StrOp::Big(B::Mul), xy) = &m.kind else { return None };
    let ExprKind::StrOp(isc_ir::StrOp::Big(B::Add), zs) = &a.kind else { return None };
    if xy.len() != 2 || zs.len() != 2 {
        return None;
    }
    let z = if is(&zs[0], *p) { &zs[1] } else if is(&zs[1], *p) { &zs[0] } else { return None };
    let all = [&xy[0], &xy[1], z, &d[1]];
    if mentions(z, &HashSet::from([*p])) || all.iter().any(|o| moved(o) || mentions(o, &HashSet::from([t]))) {
        return None;
    }
    Some((&xy[0], &xy[1], z, &d[1], body.span))
}

/// `z + x * y`, `x * y + z` or `z - x * y` of bigints, the product bound to
/// a temporary that only the sum reads (D406): the `let`s around the
/// product (a literal bound for it), `x`, `y`, `z`, and whether it is a
/// difference.
#[allow(clippy::type_complexity)]
fn muladd_parts<'a>(t: VarId, init: &'a Expr, body: &'a Expr) -> Option<(Vec<(VarId, &'a Expr, bool)>, &'a Expr, &'a Expr, &'a Expr, bool, Span)> {
    use isc_ir::BigOp as B;
    let is = |e: &Expr| matches!(e.kind, ExprKind::Var(w, _) if w == t);
    let moved = |e: &Expr| matches!(e.kind, ExprKind::Var(_, Use::Move));
    let (z, sub) = match &body.kind {
        ExprKind::StrOp(isc_ir::StrOp::Big(B::Add), s) if s.len() == 2 && is(&s[1]) => (&s[0], false),
        ExprKind::StrOp(isc_ir::StrOp::Big(B::Add), s) if s.len() == 2 && is(&s[0]) => (&s[1], false),
        ExprKind::StrOp(isc_ir::StrOp::Big(B::Sub), s) if s.len() == 2 && is(&s[1]) => (&s[0], true),
        _ => return None,
    };
    let mut lets = Vec::new();
    let mut m = init;
    while let ExprKind::Let { var, init: i, body: b, drop, stmt: false } = &m.kind {
        lets.push((*var, &**i, *drop));
        m = b;
    }
    let ExprKind::StrOp(isc_ir::StrOp::Big(B::Mul), xy) = &m.kind else { return None };
    if xy.len() != 2 || moved(z) || mentions(z, &HashSet::from([t])) {
        return None;
    }
    let bound: HashSet<VarId> = lets.iter().map(|l| l.0).collect();
    if mentions(z, &bound) || xy.iter().any(|o| moved(o) || mentions(o, &HashSet::from([t]))) || lets.iter().any(|l| mentions(l.1, &HashSet::from([t]))) {
        return None;
    }
    Some((lets, &xy[0], &xy[1], z, sub, body.span))
}

/// A count (D396): the sum, from 0, of a `map` of one input whose value is
/// 1 or 0 by a condition (`count` of the prelude): that input, the
/// condition as a lambda of the parameter of the `map`, and the lanes of
/// its masks, counted by the bits of its masks as a `filter` counts them
/// (`count_kept`), where the sum widened each element to 64 bits
/// (`http.text`: the test of its bytes 69 % of its cycles).
fn count_parts<'a, 'e>(src: &'a Src<'e>, l: &Lambda, z: &V, t: &Type) -> Option<(&'a Src<'e>, Lambda, u32)> {
    let Src::Map(ins, ml) = src else { return None };
    let lit = |e: &Expr, k: i64| matches!(e.kind, ExprKind::Lit(Lit::Int(c)) if c == k);
    let add = l.params.len() == 2
        && matches!(&l.body.kind, ExprKind::Binary(BinOp::Add, a, b) if matches!((&a.kind, &b.kind), (ExprKind::Var(x, _), ExprKind::Var(y, _)) if (*x == l.params[0] && *y == l.params[1]) || (*x == l.params[1] && *y == l.params[0])));
    if ins.len() != 1 || *t != Type::Int(IntKind::I64) || z.r != "0" || !add || ml.params.len() != 1 {
        return None;
    }
    let ExprKind::If(c, a, b) = &ml.body.kind else { return None };
    if !lit(a, 1) || !lit(b, 0) {
        return None;
    }
    // bytes only (32 a vector, as `filter_width`): the sum widened each to
    // 64 bits, 8 times its width; wider elements (an index, `i64`) kept by
    // the sum (their counts on short texts, `json.quote`, +8.7 % by the
    // masks, whose elements left go one at a time)
    if src_elem_size(&ins[0]) != 1 {
        return None;
    }
    // 64 bytes a vector where the target has masks of 64 bytes (AVX-512 BW,
    // D452: half the turns of 32, each a load, a compare, a count of bits)
    let w = if BYTE_MASKS.with(|b| b.get()) { 64 } else { 32 };
    Some((&ins[0], Lambda { params: ml.params.clone(), body: c.clone(), drop_params: ml.drop_params.clone() }, w))
}

/// The bytes of an element of `src` (8 for an index or a value made).
fn src_elem_size(src: &Src) -> i64 {
    match src {
        Src::Mat { elem, .. } => esize(elem),
        Src::Offset { inner, .. } => src_elem_size(inner),
        _ => 8,
    }
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
            Filter(x, l) | Tasks(x, l) => src(x, v) && ok(&l.body, v),
            _ => e.children().into_iter().all(|c| ok(c, v)),
        }
    }
    ok(e, v)
}

/// The bytes of the buffer on the stack of a text that its scope only
/// reads (D404): the texts of messages, of dates, of keys; a longer one
/// takes a block of the heap as before.
const STACK_STR: i64 = 128;

/// From this many bytes on, a join is made by ranges of its strings in
/// parallel (D427).
const JOIN_PARALLEL_MIN: i64 = 1 << 20;

/// `v` is moved somewhere in `e` (given to an owner: a consumer, a call, a
/// result), lambdas included.
fn moves_var(e: &Expr, v: VarId) -> bool {
    matches!(e.kind, ExprKind::Var(w, Use::Move) if w == v) || e.children().into_iter().any(|c| moves_var(c, v))
}

/// The concatenations, padded texts and texts of another case whose text is
/// the value of `e` (D404, D458, D501): `e` itself, the branches of an `if`,
/// the body of a `let` or of a `Free`; not a concatenation given its left
/// text (D351), which writes after it in its block.
fn result_concats(e: &Expr, out: &mut Vec<*const Expr>) {
    use ExprKind::*;
    match &e.kind {
        StrOp(isc_ir::StrOp::Concat, es) if e.ty == Type::Str && !es.is_empty() && !matches!(es[0].kind, Var(_, Use::Move)) => out.push(e as *const Expr),
        // a padded text (D458)
        StrOp(isc_ir::StrOp::PadStart | isc_ir::StrOp::PadEnd, _) if e.ty == Type::Str => out.push(e as *const Expr),
        // a text in upper or lower case (D501)
        StrOp(isc_ir::StrOp::ToUpper | isc_ir::StrOp::ToLower, _) if e.ty == Type::Str => out.push(e as *const Expr),
        If(_, t, f) => {
            result_concats(t, out);
            result_concats(f, out);
        }
        Let { body, .. } | Free(_, body) => result_concats(body, out),
        _ => {}
    }
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
        StrOp(op, _) if op.may_panic() => 1,
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
        StrOp(op, _) if op.may_panic() => false,
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

/// `$group(map(...))` of integer keys (D244): the keys made by blocks that
/// stay in the caches and grouped as they are made, never stored all.
fn group_stream_ok(init: &Expr) -> bool {
    matches!(&init.kind, ExprKind::Map(..))
        && init.ty.is_array()
        && matches!(init.ty.elem(), Type::Int(_))
        && !isc_ir::passes::is_stream(init)
        && !matches!(&init.kind, ExprKind::Map(ins, _) if ins.iter().any(|i| matches!(i.kind, ExprKind::Var(_, Use::Move))))
}

/// What the whole rows of `fill_rows` take out of their loop (D318): an
/// equality `q == K` of the index with a value of before the loop (true in
/// one row at most: the rows apart) and the value of K; the reads of a
/// table at `x % c` (c at least the lanes) whose first lane is `step * row
/// + K`, each with the
/// slot of its remainder, its step, K, c, and whether every lane of every
/// row is at least 0.
#[derive(Default)]
struct RowHoist {
    eq: Option<(*const Expr, String)>,
    rems: Vec<(*const Expr, String, i64, String, i64, String)>,
}

/// An index of the body of a `map` by rows of `c` (D318): `row * row +
/// place * place + c0 + sum of k * v` for variables `v` bound before the
/// loop.
#[derive(Clone, Default)]
struct RowLin {
    row: i64,
    place: i64,
    c0: i64,
    vars: Vec<(i64, VarId)>,
}

impl RowLin {
    fn scaled(&self, k: i64) -> Option<RowLin> {
        Some(RowLin {
            row: self.row.checked_mul(k)?,
            place: self.place.checked_mul(k)?,
            c0: self.c0.checked_mul(k)?,
            vars: self.vars.iter().map(|&(a, v)| a.checked_mul(k).map(|b| (b, v))).collect::<Option<Vec<_>>>()?,
        })
    }

    fn plus(&self, o: &RowLin) -> Option<RowLin> {
        let mut vars = self.vars.clone();
        vars.extend(o.vars.iter().copied());
        Some(RowLin { row: self.row.checked_add(o.row)?, place: self.place.checked_add(o.place)?, c0: self.c0.checked_add(o.c0)?, vars })
    }
}

/// `e` as a `RowLin` (D318): integer literals, `q`, `q / c` (the row),
/// `q % c` (the place), sums, differences, products by a literal, the
/// variables of the `let`s of the body by their values, those of `bound`
/// (bound before the loop) as they are; all of type `i64`.
fn row_lin(e: &Expr, q: VarId, c: i64, lets: &HashMap<VarId, &Expr>, bound: &dyn Fn(VarId) -> bool) -> Option<RowLin> {
    let i64t = Type::Int(IntKind::I64);
    if e.ty != i64t {
        return None;
    }
    let lit = |x: &Expr| match x.kind {
        ExprKind::Lit(Lit::Int(k)) if x.ty == i64t => Some(k),
        _ => None,
    };
    let is_q = |x: &Expr| matches!(x.kind, ExprKind::Var(v, _) if v == q);
    match &e.kind {
        ExprKind::Lit(Lit::Int(k)) => Some(RowLin { c0: *k, ..RowLin::default() }),
        ExprKind::Var(v, _) if *v == q => Some(RowLin { row: c, place: 1, ..RowLin::default() }),
        ExprKind::Var(v, _) => match lets.get(v) {
            Some(init) => row_lin(init, q, c, lets, bound),
            None if bound(*v) => Some(RowLin { vars: vec![(1, *v)], ..RowLin::default() }),
            None => None,
        },
        ExprKind::Binary(BinOp::Div, x, d) if is_q(x) && lit(d) == Some(c) => Some(RowLin { row: 1, ..RowLin::default() }),
        ExprKind::Binary(BinOp::Rem, x, d) if is_q(x) && lit(d) == Some(c) => Some(RowLin { place: 1, ..RowLin::default() }),
        ExprKind::Binary(BinOp::Add, a, b) => row_lin(a, q, c, lets, bound)?.plus(&row_lin(b, q, c, lets, bound)?),
        ExprKind::Binary(BinOp::Sub, a, b) => row_lin(a, q, c, lets, bound)?.plus(&row_lin(b, q, c, lets, bound)?.scaled(-1)?),
        ExprKind::Binary(BinOp::Mul, a, b) => match (lit(a), lit(b)) {
            (Some(k), _) => row_lin(b, q, c, lets, bound)?.scaled(k),
            (_, Some(k)) => row_lin(a, q, c, lets, bound)?.scaled(k),
            _ => None,
        },
        _ => None,
    }
}

/// The function of the runtime of a primitive of the system (D322):
/// `sysGetenv` is `isc_sys_getenv`.
fn sys_symbol(o: isc_ir::SysOp) -> String {
    format!("isc_sys_{}", o.name().trim_start_matches("sys").to_ascii_lowercase())
}

/// The flag of the kind of the keys of `isc_group` for `$groupCounts`
/// (D317), as in the runtime.
const GROUP_COUNTS: i32 = 64;

/// `let ks = map(...) in $group(ks)` (or `$groupCounts`): see
/// `group_stream_ok`; whether it counts only.
fn group_let_ok(var: VarId, init: &Expr, body: &Expr) -> Option<bool> {
    match &body.kind {
        ExprKind::StrOp(op @ (StrOp::Group | StrOp::GroupCounts), args) if group_stream_ok(init) && args.len() == 1 && matches!(args[0].kind, ExprKind::Var(x, _) if x == var) => Some(*op == StrOp::GroupCounts),
        _ => None,
    }
}

/// The kind of the integer keys of `$group` for the runtime (as
/// `isc_argsort`).
fn group_kind(t: &Type) -> i32 {
    match t {
        Type::Int(k) => (k.bits().trailing_zeros() as i32 - 3) + if k.signed() { 0 } else { 4 },
        Type::Bool => 10,
        _ => 11,
    }
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

/// The blocks of the source of a `filter` made by windows (D293): its
/// elements counted by blocks of this many, each window the blocks of its
/// part filtered again (a few blocks more than the elements of a task of its
/// fill).
const WINDOW_BLOCK: i64 = 1024;
/// The bytes of a window on the stack of a task; more on the heap.
const WINDOW_STACK: i64 = 32 << 10;
/// The indices a task may read past each of its ends (lanes of vectors)
/// counted in its window.
const WINDOW_MARGIN: i64 = 64;
/// The widest window of the first task that keeps the windows, in tasks of
/// its fill: 4 times the elements of a task.
const WINDOW_WIDE: i64 = 4;

/// A join of strings read once in order by a `map` of its bytes (D428):
/// `let s = join(parts, sep) in let t = bytes(s) in let c = map(t, l) in
/// rest` (the `let`s of views and scalars of `join` first), `s` and `t`
/// read nowhere else, `c` an array of scalars. Returns those `let`s, the
/// parts, the separator, `c`, the `map`, its lambda, `rest` and the
/// flags of the `let` of `c` (`drop`, `stmt`).
#[allow(clippy::type_complexity)]
fn join_map<'a>(s: VarId, init: &'a Expr, body: &'a Expr) -> Option<(Vec<(VarId, &'a Expr)>, &'a Expr, &'a Expr, VarId, &'a Expr, &'a Lambda, &'a Expr, bool, bool)> {
    let mut lets = Vec::new();
    let mut e = init;
    while let ExprKind::Let { var, init: i, body: b, drop: false, stmt: false } = &e.kind {
        if !(i.ty.is_scalar() || i.ty == Type::Str || (i.ty.is_array() && i.ty.slice_is_view() && matches!(i.kind, ExprKind::Slice(..)))) {
            return None;
        }
        lets.push((*var, &**i));
        e = b;
    }
    let ExprKind::StrOp(StrOp::Join, js) = &e.kind else { return None };
    let [parts, sep] = js.as_slice() else { return None };
    // borrowed: neither made here nor to free
    if !matches!(parts.kind, ExprKind::Var(..)) || !matches!(sep.kind, ExprKind::Var(..) | ExprKind::StrLit(_)) {
        return None;
    }
    let ExprKind::Let { var: t, init: bi, body: b2, drop: false, .. } = &body.kind else { return None };
    let ExprKind::StrOp(StrOp::Bytes, bs) = &bi.kind else { return None };
    if !matches!(bs.as_slice(), [x] if matches!(x.kind, ExprKind::Var(v, _) if v == s)) {
        return None;
    }
    let ExprKind::Let { var: c, init: m, body: rest, drop, stmt } = &b2.kind else { return None };
    let ExprKind::Map(ins, l) = &m.kind else { return None };
    if !matches!(ins.as_slice(), [x] if matches!(x.kind, ExprKind::Var(v, _) if v == *t)) || l.params.len() != 1 || !m.ty.elem().is_scalar() {
        return None;
    }
    let both = HashSet::from([s, *t]);
    if mentions(rest, &both) || mentions(&l.body, &both) {
        return None;
    }
    Some((lets, parts, sep, *c, m, l, rest, *drop, *stmt))
}

/// D426: the functions whose windowed `filter` (D293) reads a range of one
/// of their parameters (`slice` of it, or itself) with a predicate of its
/// elements alone: the counts of that predicate by blocks may come from the
/// pass of a caller over the whole array. For each, the index of the
/// parameter and the lambda (the first such `filter`).
fn count_users(p: &Program) -> HashMap<u32, (usize, &Lambda)> {
    fn walk<'a>(e: &'a Expr, params: &[VarId], out: &mut Option<(usize, &'a Lambda)>) {
        if out.is_some() {
            return;
        }
        if let ExprKind::Let { var, init, body, .. } = &e.kind {
            if let Some((lets, f)) = windowed_filter(*var, init, body) {
                if let ExprKind::Filter(src, l) = &f.kind {
                    let mut s: &Expr = src;
                    if let ExprKind::Var(t, _) = &s.kind {
                        if let Some((_, i)) = lets.iter().find(|(v, _)| v == t) {
                            s = i;
                        }
                    }
                    if let ExprKind::Slice(a, _, _) = &s.kind {
                        s = a;
                    }
                    if let ExprKind::Var(q, _) = &s.kind {
                        let mut bound = HashSet::from([l.params[0]]);
                        let mut free = Vec::new();
                        free_vars(&l.body, &mut bound, &mut free);
                        if let (Some(k), true) = (params.iter().position(|x| x == q), free.is_empty()) {
                            *out = Some((k, l));
                            return;
                        }
                    }
                }
            }
        }
        for c in e.children() {
            walk(c, params, out);
        }
    }
    let mut users = HashMap::new();
    for (i, f) in p.fns.iter().enumerate() {
        let mut found = None;
        walk(&f.body, &f.params, &mut found);
        if let Some(u) = found {
            users.insert(i as u32, u);
        }
    }
    users
}

/// A predicate that no lane can make fail (D437): only its parameter and
/// other variables, literals, comparisons, logic, arithmetic but division
/// and remainder, conversions, choices and `let`s; no read of an array, no
/// call. Evaluated on the masked lanes of a vector too.
fn masked_safe(e: &Expr) -> bool {
    let here = match &e.kind {
        ExprKind::Var(..) | ExprKind::Lit(_) | ExprKind::Unary(..) | ExprKind::Convert(_) | ExprKind::If(..) | ExprKind::Let { .. } => true,
        ExprKind::Binary(op, _, _) => !matches!(op, BinOp::Div | BinOp::Rem),
        _ => false,
    };
    here && e.children().into_iter().all(masked_safe)
}

/// At most this many elements in a table copied by `period_tables`: the
/// copy made before each vector loop, on the stack.
const PERIOD_MAX: i64 = 4096;

/// The reads `a[x % c]` of the body of `l` (D273): `a` an array of scalars
/// bound outside it, `c` an integer literal of at least `w` and at most
/// PERIOD_MAX. With `x` consecutive in the lanes, their remainders cross a
/// multiple of `c` in about `w / c` of the vectors (D229: then a gather);
/// `a[0 .. c]` followed by `a[0 .. w - 1]` reads them all by one load.
fn period_tables(l: &Lambda, w: u32) -> Vec<(VarId, i64, Type)> {
    fn walk(e: &Expr, params: &[VarId], w: u32, out: &mut Vec<(VarId, i64, Type)>) {
        if let ExprKind::Index(x, i) = &e.kind {
            if let (ExprKind::Var(v, _), ExprKind::Binary(BinOp::Rem, _, c)) = (&x.kind, &i.kind) {
                if let ExprKind::Lit(Lit::Int(c)) = c.kind {
                    let scalar = matches!(x.ty.elem(), Type::Int(_) | Type::F64 | Type::F32 | Type::Bool);
                    if !params.contains(v) && scalar && i.ty == Type::Int(IntKind::I64) && c >= w as i64 && c <= PERIOD_MAX && !out.iter().any(|(u, d, _)| u == v && *d == c) {
                        out.push((*v, c, x.ty.elem().clone()));
                    }
                }
            }
        }
        for ch in e.children() {
            walk(ch, params, w, out);
        }
    }
    let mut out = Vec::new();
    walk(&l.body, &l.params, w, &mut out);
    out
}

/// Whether the elements of the source `x` of a `scan` are read by vectors,
/// and whether by `elem_replayed` (D507): a source with more than one
/// place that can fail (`b[i]` and `q[i]`), not vector code otherwise
/// (D185), read by vectors whose failed checks are computed again one at a
/// time; the sources read by vectors before as they were.
fn scan_vec(x: &Expr) -> (bool, bool) {
    if src_vec_ok(x) {
        return (true, false);
    }
    let replay = src_vec_ok_replay(x);
    (replay, replay)
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
        // `fma` and the roundings are instructions (D613); the other
        // functions are calls of the runtime, one element at a time
        ExprKind::Math(f, xs) if f.instruction() => xs.iter().all(|x| vec_ok_given(x, varying, given)),
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

/// `lam_ok` for vector code whose failed checks are replayed one element
/// at a time (D185): the predicate of a `filter` (D362).
fn lam_ok_replay(l: &Lambda) -> bool {
    let before = REPLAY.with(|r| r.replace(true));
    let ok = lam_ok(l);
    REPLAY.with(|r| r.set(before));
    ok
}

/// What a split `fold` may compute before its loop (D260): arithmetic on
/// scalars (no integer division), roots, `fma`, elements of arrays of
/// variables; no call, no loop, nothing that allocates. Its only checks are
/// those of the indices, which jump to the `fold` as written when they fail.
fn splittable(e: &Expr) -> bool {
    use ExprKind::*;
    match &e.kind {
        Lit(_) | Var(..) => true,
        Unary(_, a) | Sqrt(a) => splittable(a),
        Binary(op, a, b) => {
            !(matches!(op, BinOp::Div | BinOp::Rem) && matches!(a.ty, Type::Int(_))) && splittable(a) && splittable(b)
        }
        Index(a, i) => matches!(a.kind, Var(..)) && splittable(i),
        Math(MathFn::Fma, xs) => xs.iter().all(splittable),
        _ => false,
    }
}

/// A root or a division of floats: what the divider of the machine does,
/// one operation for a scalar or a vector of 2 lanes (D260).
fn divides(e: &Expr) -> bool {
    matches!(&e.kind, ExprKind::Sqrt(_))
        || matches!(&e.kind, ExprKind::Binary(BinOp::Div, a, _) if matches!(a.ty, Type::F64 | Type::F32))
        || e.children().into_iter().any(divides)
}

/// `e`, a value of a split `fold` computed in vectors (D260), its largest
/// parts that neither use the divider nor read a value in vectors replaced
/// by new variables (`leaves`: each computed for every lane as a scalar,
/// then put into a vector): only the roots and divisions, and what follows
/// them, are vector code.
fn pack_leaves(e: &Expr, vec_vars: &HashSet<VarId>, leaves: &mut Vec<(VarId, Expr)>, fresh: &mut VarId) -> Expr {
    use ExprKind::*;
    if matches!(e.kind, Lit(_)) {
        return e.clone();
    }
    if !divides(e) && !mentions(e, vec_vars) {
        *fresh -= 1;
        leaves.push((*fresh, e.clone()));
        return Expr::new(Var(*fresh, isc_ir::Use::Borrow), e.ty.clone(), e.span);
    }
    let mut r = e.clone();
    match &mut r.kind {
        Unary(_, a) | Sqrt(a) => {
            let na = pack_leaves(a, vec_vars, leaves, fresh);
            **a = na;
        }
        Binary(_, a, b) => {
            let na = pack_leaves(a, vec_vars, leaves, fresh);
            let nb = pack_leaves(b, vec_vars, leaves, fresh);
            **a = na;
            **b = nb;
        }
        Math(_, xs) => {
            for x in xs.iter_mut() {
                let nx = pack_leaves(x, vec_vars, leaves, fresh);
                *x = nx;
            }
        }
        _ => {}
    }
    r
}

/// A `fold` over `iota(n)`, n an even constant of at most `UNROLL_FULL`,
/// whose body begins with `let`s that read neither the accumulator nor what
/// depends on it, a root or a division among them (D260): n and the
/// variables of the `let`s computed before the loop.
fn split_plan(x: &Expr, l: &Lambda) -> Option<(i64, Vec<VarId>)> {
    let ExprKind::Iota(n) = &x.kind else { return None };
    let ExprKind::Lit(isc_ir::Lit::Int(n)) = n.kind else { return None };
    if *x.ty.elem() != Type::Int(IntKind::I64) || !(2..=UNROLL_FULL).contains(&n) || n % 2 != 0 {
        return None;
    }
    let before = REPLAY.with(|r| r.replace(true));
    let mut varying: HashSet<VarId> = HashSet::from([l.params[1]]);
    let mut avoid: HashSet<VarId> = HashSet::from([l.params[0]]);
    let mut vars = Vec::new();
    let mut costly = false;
    let mut e = &*l.body;
    while let ExprKind::Let { var, init, body, stmt, drop } = &e.kind {
        let ok = !*stmt
            && !*drop
            && matches!(init.ty, Type::F64 | Type::F32 | Type::Int(_))
            && !mentions(init, &avoid)
            && splittable(init)
            && vec_ok_given(init, &mut varying.clone(), std::ptr::null());
        if ok {
            if mentions(init, &varying) {
                varying.insert(*var);
            }
            costly |= divides(init);
            vars.push(*var);
        } else {
            avoid.insert(*var);
        }
        e = body;
    }
    REPLAY.with(|r| r.set(before));
    costly.then_some((n, vars))
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

/// The step `y -> (a * y + c) % m` of an affine recurrence modulo a constant
/// (rule R, D292).
#[derive(Clone, Copy, Debug)]
struct Affine {
    a: i64,
    c: i64,
    m: i64,
}

/// `e` as the step `(y * a + c) % m` of the value `y` (`is_y`) for rule R
/// (D292): the product and the sum in either order, the product (`a` = 1)
/// or the sum (`c` = 0) possibly absent, integer literals with `a >= 0`,
/// `c >= 0` and `m >= 2`, of type `i64`. From `0 <= y < m` the step then
/// never wraps (`(m - 1) * a + c` fits) and its result is again in `0 .. m`;
/// a power of the step (`affine_power`) never wraps either (`m * (m - 1)`
/// fits).
fn affine_step(e: &Expr, is_y: &dyn Fn(&Expr) -> bool) -> Option<Affine> {
    let i64t = Type::Int(IntKind::I64);
    if e.ty != i64t {
        return None;
    }
    let ExprKind::Binary(BinOp::Rem, p, mm) = &e.kind else { return None };
    let ExprKind::Lit(Lit::Int(m)) = mm.kind else { return None };
    let lit = |x: &Expr| match x.kind {
        ExprKind::Lit(Lit::Int(k)) if x.ty == i64t => Some(k),
        _ => None,
    };
    let product = |x: &Expr| -> Option<i64> {
        if is_y(x) {
            return Some(1);
        }
        match &x.kind {
            ExprKind::Binary(BinOp::Mul, u, w) if is_y(u) => lit(w),
            ExprKind::Binary(BinOp::Mul, u, w) if is_y(w) => lit(u),
            _ => None,
        }
    };
    let (a, c) = match &p.kind {
        ExprKind::Binary(BinOp::Add, u, w) => match (product(u), lit(w)) {
            (Some(a), Some(c)) => (a, c),
            _ => (product(w)?, lit(u)?),
        },
        _ => (product(p)?, 0),
    };
    let fits = |x: i128| x <= i64::MAX as i128;
    let (a1, c1, m1) = (a as i128, c as i128, m as i128);
    (a >= 0 && c >= 0 && m >= 2 && fits((m1 - 1) * a1 + c1) && fits(m1 * (m1 - 1))).then_some(Affine { a, c, m })
}

/// Rule L (D346): `fold(iota(n), init, (a, i) => c ? i : a)`, `c` a
/// condition that does not read the accumulator `a` and cannot stop the
/// program (`safe_cond`): the result is the last `i` where `c` holds, or
/// `init`. Returns `c`.
fn last_where<'a>(x: &Expr, l: &'a Lambda) -> Option<&'a Expr> {
    if !matches!(x.kind, ExprKind::Iota(_)) || l.params.len() != 2 || l.body.ty != Type::Int(IntKind::I64) {
        return None;
    }
    let (a, i) = (l.params[0], l.params[1]);
    let ExprKind::If(c, t, f) = &l.body.kind else { return None };
    if !matches!(t.kind, ExprKind::Var(v, _) if v == i) || !matches!(f.kind, ExprKind::Var(v, _) if v == a) || c.ty != Type::Bool {
        return None;
    }
    let (mut bound, mut free) = (HashSet::new(), Vec::new());
    free_vars(c, &mut bound, &mut free);
    (!free.contains(&a) && safe_cond(c)).then_some(&**c)
}

/// Rule F (D505): `fold(iota(n), init, (a, i) => a < 0 && c ? i : a)`
/// (`findIndex` of the prelude), `c` a condition that does not read the
/// accumulator `a`: the first `i` where `c` holds when `init` is negative,
/// `init` when it is not (the guard never holds) or when none does; `c` is
/// evaluated only up to that `i`, as written (the `&&`). Returns `c`.
fn first_where<'a>(x: &Expr, l: &'a Lambda) -> Option<&'a Expr> {
    if !matches!(x.kind, ExprKind::Iota(_)) || l.params.len() != 2 || l.body.ty != Type::Int(IntKind::I64) {
        return None;
    }
    let (a, i) = (l.params[0], l.params[1]);
    let ExprKind::If(g, t, f) = &l.body.kind else { return None };
    if !matches!(t.kind, ExprKind::Var(v, _) if v == i) || !matches!(f.kind, ExprKind::Var(v, _) if v == a) {
        return None;
    }
    // the guard `a < 0 && c`: `if (a < 0) c else false`
    let ExprKind::If(lt, c, no) = &g.kind else { return None };
    let guard = matches!(&lt.kind, ExprKind::Binary(BinOp::Lt, u, z)
        if matches!(u.kind, ExprKind::Var(v, _) if v == a) && matches!(z.kind, ExprKind::Lit(Lit::Int(0))));
    if !guard || !matches!(no.kind, ExprKind::Lit(Lit::Bool(false))) || c.ty != Type::Bool {
        return None;
    }
    let (mut bound, mut free) = (HashSet::new(), Vec::new());
    free_vars(c, &mut bound, &mut free);
    (!free.contains(&a)).then_some(&**c)
}

/// `e` is computed from scalars without anything that can stop the
/// program but reads `x[k]` (checked apart, D346): literals, variables,
/// the operators of scalars but division and remainder (integers wrap,
/// D329; the amount of a shift is taken modulo its bits), lengths,
/// projections, conditions, conversions of an integer to a wider one.
fn safe_cond(e: &Expr) -> bool {
    safe_cond_in(e, false)
}

/// `safe_cond`, with `lets` the lets of scalars too (rule F, D505: the
/// parameter of the predicate of `findIndex` bound to its element).
fn safe_cond_in(e: &Expr, lets: bool) -> bool {
    use ExprKind::*;
    let scalar = |t: &Type| matches!(t, Type::Int(_) | Type::Bool | Type::F64 | Type::F32);
    let safe = |x: &Expr| safe_cond_in(x, lets);
    match &e.kind {
        Lit(_) | Var(..) => true,
        Unary(_, x) => scalar(&e.ty) && safe(x),
        Binary(op, x, y) => !matches!(op, BinOp::Div | BinOp::Rem) && scalar(&e.ty) && scalar(&x.ty) && scalar(&y.ty) && safe(x) && safe(y),
        If(c, t, f) => scalar(&e.ty) && safe(c) && safe(t) && safe(f),
        Len(x) | Proj(x, _) => safe(x),
        Index(a, k) => scalar(&e.ty) && matches!(a.kind, Var(..)) && safe(k),
        Convert(x) => match (&x.ty, &e.ty) {
            (Type::Int(f), Type::Int(t)) => t.bits() >= f.bits() && (f.signed() == t.signed() || (!f.signed() && t.bits() > f.bits())) && safe(x),
            _ => false,
        },
        Let { init, body, .. } if lets => scalar(&init.ty) && safe(init) && safe(body),
        _ => false,
    }
}

/// Rule R (D292): `fold(iota(n), init, (v, t) => v.with(t + 1, f(v[t])))`
/// (or `1 + t`), `f` an affine step modulo a constant of `v[t]` alone
/// (`affine_step`): `v[t + 1] = f(v[t])`, so `v[s] = f^(s - 1)(v[1])`, and a
/// power of `f` is again an affine step. Returns the step and the value of
/// the update (`f(v[t])` as written).
fn affine_recurrence<'a>(x: &Expr, l: &'a Lambda) -> Option<(Affine, &'a Expr)> {
    if !matches!(x.kind, ExprKind::Iota(_)) || l.params.len() != 2 || l.drop_params.first() != Some(&false) {
        return None;
    }
    let (v, t) = (l.params[0], l.params[1]);
    let ExprKind::Update { target, index, value, .. } = &l.body.kind else { return None };
    if !matches!(target.kind, ExprKind::Var(u, _) if u == v) {
        return None;
    }
    let is_t = |e: &Expr| matches!(e.kind, ExprKind::Var(u, _) if u == t);
    let one = |e: &Expr| matches!(e.kind, ExprKind::Lit(Lit::Int(1)));
    if !matches!(&index.kind, ExprKind::Binary(BinOp::Add, a, b) if (is_t(a) && one(b)) || (one(a) && is_t(b))) {
        return None;
    }
    let is_y = |e: &Expr| matches!(&e.kind, ExprKind::Index(a, i) if matches!(a.kind, ExprKind::Var(u, _) if u == v) && is_t(i));
    Some((affine_step(value, &is_y)?, value))
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
    ///
    /// `buffered`: its variant `isc_fn_<i>_buf`, for a function of the
    /// prelude that makes a text: the buffer of its caller its first
    /// parameter, the texts of its result written there (`buffered_texts`).
    fn emit_fn(&mut self, i: usize, in_region: bool, counted: bool, buffered: bool) -> R<()> {
        let f = &self.p.fns[i];
        let saved_text_buf = std::mem::replace(&mut self.text_buf, if buffered { Some("%tb".to_string()) } else { None });
        if buffered {
            self.buffered_texts(&f.body)?;
        }
        // the variant of a builder (D200): its nodes cut from the region of
        // its first parameter
        let saved_region = std::mem::replace(&mut self.region, if in_region { Some("%rg".to_string()) } else { None });
        let saved_f = std::mem::take(&mut self.f);
        let saved_env = std::mem::take(&mut self.env);
        let saved_divmod = std::mem::take(&mut self.divmod);
        // the names of registers of this function alone (D518)
        let saved_parts = std::mem::take(&mut self.part_values);
        let saved_vec_fail = self.vec_fail.take();
        let saved_task = std::mem::replace(&mut self.in_task, false);
        let saved_kernel = std::mem::replace(&mut self.kernel_fn, f.name.starts_with('$'));
        let saved_counts = std::mem::take(&mut self.counts);
        let saved_count_param = self.count_param.take();
        // the texts on the stack, by the names of the registers of a
        // function (D442)
        let saved_stack_strs = std::mem::take(&mut self.stack_strs);
        let mut ps = Vec::new();
        if in_region {
            ps.push("ptr %rg".to_string());
        }
        if buffered {
            ps.push("ptr %tb".to_string());
        }
        // the variant given the counts of a caller (D426): the block of
        // the counts of its parameter, or null
        if counted {
            let (k, l) = self.count_users[&(i as u32)];
            ps.push("ptr %sm".to_string());
            self.counts.insert(f.params[k], "%sm".into());
            self.count_param = Some(("%sm".into(), l));
        }
        for (k, var) in f.params.iter().enumerate() {
            let t = lt(&self.p.vars[*var as usize].ty);
            ps.push(format!("{} %a{}", t, k));
            self.env.insert(*var, v(format!("%a{}", k), t));
        }
        // the text of an integer (D588): a leaf of a frame of 24 bytes, no
        // test of the stack, copied into its callers
        // (and the digits of a padded one, D606; the name of a float that
        // is no number too: two choices of data)
        let int_text = self.p.int_text.is_some_and(|(a, b)| i as u32 == a || i as u32 == b) || self.p.digit_words == Some(i as u32) || self.p.float_name == Some(i as u32);
        // the text of a float and `toFixed`: a small frame too, their rare
        // paths `@cold` functions that test the stack themselves
        let float_text = self.p.float_text.is_some_and(|(a, b)| i as u32 == a || i as u32 == b) || self.p.fixed_text == Some(i as u32);
        // the fast paths of the allocator (D605): leaves, copied into the
        // entries of the generated code; their rare paths are `@cold`
        // functions that test the stack themselves
        let kernel_entry = self.p.kernel.is_some_and(|(_, a, b)| i as u32 == a || i as u32 == b);
        // a function that only the prelude can name and that calls no
        // other but in tail position, its frame then given to the callee
        // (the reading of a float hands its text from one function to the
        // next): never below another frame of its kind, so one small frame
        // at most above the last function that tested the stack, which the
        // margin of the floor holds as it holds the frames of the runtime
        let hands_over = f.name.starts_with('$') && !self.opts.isa.is_wasm() && {
            fn calls_in(e: &Expr) -> usize {
                matches!(e.kind, ExprKind::Call(..)) as usize + e.children().into_iter().map(calls_in).sum::<usize>()
            }
            let own = isc_ir::passes::tail_calls(i as u32, f);
            let others = if own.values().any(|c| !c.moved.is_empty()) { 0 } else { isc_ir::passes::other_tail_calls(i as u32, f).len() };
            calls_in(&f.body) == own.len() + others
        };
        if !self.opts.isa.is_wasm() && !int_text && !float_text && !kernel_entry && !hands_over {
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
        // (nor in a variant `_buf`, whose calls are those of such variants)
        let others =
            if moves || buffered || self.opts.isa.is_wasm() { HashMap::new() } else { isc_ir::passes::other_tail_calls(i as u32, f) };
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
        let name = if in_region {
            format!("{}_rg", i)
        } else if counted {
            format!("{}_sm", i)
        } else if buffered {
            format!("{}_buf", i)
        } else {
            i.to_string()
        };
        // on 64 bytes (D213): a small function called at each node of a
        // tree stays whole in the cache of decoded instructions
        // `@cold` (D584): never inlined back by LLVM, its calls unlikely
        let cold = if self.p.fns[i].cold {
            " cold noinline"
        } else if int_text || kernel_entry {
            " alwaysinline"
        } else {
            ""
        };
        let text = self.finish(&format!("define internal {}{} @isc_fn_{}({}){}{} align 64", cc, ret, name, ps.join(", "), attrs, cold));
        self.region = saved_region;
        self.other_tails = saved_others;
        self.f = saved_f;
        self.env = saved_env;
        self.divmod = saved_divmod;
        self.part_values = saved_parts;
        self.vec_fail = saved_vec_fail;
        self.in_task = saved_task;
        self.kernel_fn = saved_kernel;
        self.counts = saved_counts;
        self.count_param = saved_count_param;
        self.stack_strs = saved_stack_strs;
        self.text_buf = saved_text_buf;
        r?;
        self.funcs.push(text);
        Ok(())
    }

    /// The texts that the variant `_buf` of a function gives as its result,
    /// noted in `buf_texts`: every text its body ends with must be one the
    /// buffer of the caller can hold (`$textTail`, 25 bytes; `$decimalText`,
    /// at most `TEXT_BUF`), a literal (data of the program, read where it
    /// is), or the result of another function, called by its own variant.
    /// The caller does not free the result: any other text, which a block
    /// would hold, is refused here, when the compiler is built and tested.
    fn buffered_texts(&mut self, e: &Expr) -> R<()> {
        use isc_ir::StrOp as S;
        match &e.kind {
            ExprKind::If(_, a, b) => {
                self.buffered_texts(a)?;
                self.buffered_texts(b)
            }
            ExprKind::Let { body, .. } | ExprKind::Free(_, body) | ExprKind::CheckSize(_, _, body) | ExprKind::CheckNonNegative(_, body) => self.buffered_texts(body),
            ExprKind::StrOp(S::TextTail | S::DecimalText, _) | ExprKind::Call(..) if e.ty == Type::Str => {
                self.buf_texts.insert(e as *const Expr);
                Ok(())
            }
            ExprKind::Copy(x) if matches!(x.kind, ExprKind::StrLit(_)) && x.ty == Type::Str => Ok(()),
            ExprKind::StrLit(_) if e.ty == Type::Str => Ok(()),
            ExprKind::StrOp(S::Panic, _) => Ok(()),
            _ => Err("codegen: a text of the prelude that `print` cannot write in its buffer".into()),
        }
    }

    /// The buffer of the variant `_buf` being emitted, for a text that it
    /// writes there (`buffered_texts`).
    fn text_buffer(&self, e: &Expr) -> Option<String> {
        self.text_buf.clone().filter(|_| self.buf_texts.contains(&(e as *const Expr)))
    }

    /// `void @isc_fn_<f>_print(x, pre)`: `print` of a float, without its
    /// newline, after `pre` bytes of the separator `, ` (0, or 2 after an
    /// element of an array, of a tuple or of a value of a sum type: one
    /// copy into the buffer of the standard output for both, where the
    /// separator was a call of its own, 46 instructions). What is not a
    /// number first, by
    /// two tests (a NaN, then an infinity, whose difference with itself is
    /// a NaN): its name, from `$rtFloatName` of the prelude called with
    /// constants, with no digit made and no frame of the digits (the
    /// runtime's `isc_print_f32` wrote a NaN in 15 instructions before the
    /// buffer: through the function of the digits, their frame and their
    /// rare path, it was 27 to 55 % more instructions than before); after
    /// a separator, both in 16 bytes of the stack. A number:
    /// `isc_fn_<f>_digits`, never inlined here (its frame would be made
    /// before the tests): its text written by the variant `_buf` of the
    /// function `f` of the prelude in a buffer of its frame (no block of
    /// the heap), after the separator, then copied into the buffer of the
    /// standard output. The buffer on 64 bytes: the stores of a short text
    /// never cross a line of the cache.
    fn emit_text_print(&mut self, f: u32) -> R<()> {
        let Some(name) = self.p.float_name else {
            return Err("codegen: print of a float, the function of the prelude of its names not lowered".into());
        };
        self.buf_needed.insert(f);
        self.text_called.insert(name);
        self.decls.insert("declare i1 @llvm.expect.i1(i1, i1)".to_string());
        let t = lt(&self.p.vars[self.p.fns[f as usize].params[0] as usize].ty);
        let (cc, attrs) = (self.fn_cc(), self.attrs());
        // the name of a float that is no number, written: alone; after the
        // separator, both from `%nb` (a name of at most 14 bytes: its
        // length is a constant here); else one after the other
        let named = |l: &str, nan: bool, neg: bool| {
            format!(
                "{l}:\n  %r.{l} = call {cc}{{ ptr, i64 }} @isc_fn_{name}(i1 {nan}, i1 {neg})\n  %p.{l} = extractvalue {{ ptr, i64 }} %r.{l}, 0\n  %n.{l} = extractvalue {{ ptr, i64 }} %r.{l}, 1\n  %alone.{l} = icmp eq i64 %pre, 0\n  br i1 %alone.{l}, label %{l}.alone, label %{l}.after\n{l}.alone:\n  call void @isc_print_str(ptr %p.{l}, i64 %n.{l})\n  ret void\n{l}.after:\n  store i16 8236, ptr %nb, align 16\n  %short.{l} = icmp ule i64 %n.{l}, 14\n  br i1 %short.{l}, label %{l}.one, label %{l}.two\n{l}.one:\n  %to.{l} = getelementptr inbounds i8, ptr %nb, i64 2\n  call void @llvm.memcpy.p0.p0.i64(ptr %to.{l}, ptr %p.{l}, i64 %n.{l}, i1 false)\n  %len.{l} = add i64 %n.{l}, 2\n  call void @isc_print_str(ptr %nb, i64 %len.{l})\n  ret void\n{l}.two:\n  call void @isc_print_str(ptr %nb, i64 2)\n  call void @isc_print_str(ptr %p.{l}, i64 %n.{l})\n  ret void\n"
            )
        };
        let (nan, minus, plus) = (named("Lnan", true, false), named("Lminus", false, true), named("Lplus", false, false));
        self.funcs.push(format!(
            "define internal {cc}void @isc_fn_{f}_print({t} %x, i64 %pre){attrs} {{\nentry:\n  %nb = alloca [16 x i8], align 16\n  %nan0 = fcmp uno {t} %x, 0.0\n  %nan = call i1 @llvm.expect.i1(i1 %nan0, i1 false)\n  br i1 %nan, label %Lnan, label %Lnum\n{nan}Lnum:\n  %d = fsub {t} %x, %x\n  %inf0 = fcmp uno {t} %d, 0.0\n  %inf = call i1 @llvm.expect.i1(i1 %inf0, i1 false)\n  br i1 %inf, label %Linf, label %Ldigits\nLinf:\n  %neg = fcmp olt {t} %x, 0.0\n  br i1 %neg, label %Lminus, label %Lplus\n{minus}{plus}Ldigits:\n  call {cc}void @isc_fn_{f}_digits({t} %x, i64 %pre)\n  ret void\n}}\n"
        ));
        // the separator at the start of the buffer, the text after it: one
        // copy from where the bytes asked start; a text that is not in the
        // buffer (data of the program: none for a number today) after the
        // separator
        self.funcs.push(format!(
            "define internal {cc}void @isc_fn_{f}_digits({t} %x, i64 %pre) noinline{attrs} {{\nentry:\n  %tb = alloca [{TEXT_BUF} x i8], align 64\n  store i16 8236, ptr %tb, align 64\n  %at = getelementptr inbounds i8, ptr %tb, i64 2\n  %r = call {cc}{{ ptr, i64 }} @isc_fn_{f}_buf(ptr %at, {t} %x)\n  %p = extractvalue {{ ptr, i64 }} %r, 0\n  %n = extractvalue {{ ptr, i64 }} %r, 1\n  %here = icmp eq ptr %p, %at\n  br i1 %here, label %Lone, label %Ltwo\nLone:\n  %skip = sub i64 2, %pre\n  %from = getelementptr inbounds i8, ptr %tb, i64 %skip\n  %len = add i64 %n, %pre\n  call void @isc_print_str(ptr %from, i64 %len)\n  ret void\nLtwo:\n  call void @isc_print_str(ptr %tb, i64 %pre)\n  call void @isc_print_str(ptr %p, i64 %n)\n  ret void\n}}\n"
        ));
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
            let x = self.env[var].clone();
            self.drop_var(*var, &x);
        }
        for (k, x) in vals.iter().enumerate() {
            let (s, owned, ty) = &tl.slots[k];
            let t = lt(ty);
            // a string literal for an owned parameter (a slot without a
            // flag, D351): the loop owns a copy of it
            if tc.lent.contains(&k) && owned.is_none() && ty.has_heap() {
                let c = self.copy_value(x, ty);
                self.emit(format!("store {} {}, ptr {}", t, c.r, s));
                continue;
            }
            // a string literal (D301): kept by the loop without owning it,
            // the old value freed if the loop owned it
            if let (true, Some(o)) = (tc.lent.contains(&k), owned.as_ref()) {
                let own = self.ins(format!("load i1, ptr {}", o));
                let lf = self.label();
                let ld = self.label();
                self.cbr(&own, &lf, &ld);
                self.block(&lf);
                let old = self.ins(format!("load {}, ptr {}", t, s));
                self.drop_value(&v(old, t.clone()), ty);
                self.br(&ld);
                self.block(&ld);
                self.emit(format!("store i1 false, ptr {}", o));
            }
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
        // once per thread: cold, so that the registers of the function are
        // not kept across it on every entry (D583)
        self.decls.insert("declare i64 @isc_stack_floor() cold".into());
        self.decls.insert("declare void @isc_panic_stack(ptr, i64, i32, i32) noreturn".into());
        // the model of a program (local-exec, as `isc_node_thread`): a load
        // relative to `%fs`, not a call of `__tls_get_addr` at each entry
        // (D583); Windows reaches its variables by `_tls_index` whatever the
        // model
        let model = if self.opts.windows { "" } else { "(localexec)" };
        let tls = format!("@isc_stack_floor_tls = internal thread_local{} global i64 0", model);
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
        let x = self.ins("call i64 @isc_stack_floor() cold".to_string());
        // unknown (0): 1, so that it is not asked again and never stops
        let z = self.ins(format!("icmp eq i64 {}, 0", x));
        let x1 = self.ins(format!("select i1 {}, i64 1, i64 {}", z, x));
        self.emit(format!("store i64 {}, ptr @isc_stack_floor_tls", x1));
        self.emit(format!("store i64 {}, ptr {}", x1, slot));
        self.br(&ljoin);
        self.block(&ljoin);
        let floor = self.ins(format!("load i64, ptr {}", slot));
        // the stack pointer, not the frame address: no frame pointer set up
        // at the entry, so that the registers are saved only on the paths
        // that need them (D583)
        let fa = self.ins("call ptr @llvm.stacksave.p0()".to_string());
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

    /// A literal kept as a string (D361): its bytes 4 bytes past a multiple
    /// of 16 (`is_literal` of the runtime), the pointer to them.
    fn owned_literal(&mut self, s: &str) -> (String, usize) {
        let key = format!("\u{0}owned:{}", s);
        if let Some(x) = self.strs.get(&key) {
            return x.clone();
        }
        let name = format!("@.olit.{}", self.strs.len());
        let n = s.len();
        let bytes = if n == 0 { "zeroinitializer".to_string() } else { format!("c\"{}\"", llvm_string(s)) };
        self.globals.push(format!(
            "{} = private unnamed_addr constant <{{ [4 x i8], [{} x i8] }}> <{{ [4 x i8] zeroinitializer, [{} x i8] {} }}>, align 16",
            name, n, n, bytes
        ));
        let p = format!("getelementptr inbounds (i8, ptr {}, i64 4)", name);
        self.strs.insert(key, (p.clone(), n));
        (p, n)
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
            // a text in its buffer on the stack, or not (D404)
            Type::Str if self.stack_strs.contains_key(&x.r) => {
                let buf = self.stack_strs[&x.r].clone();
                let p = self.ins(format!("extractvalue {{ ptr, i64 }} {}, 0", x.r));
                let on = self.ins(format!("icmp eq ptr {}, {}", p, buf));
                let (lfree, ljoin) = (self.label(), self.label());
                self.cbr(&on, &ljoin, &lfree);
                self.block(&lfree);
                self.emit(format!("call void @isc_free(ptr {})", p));
                self.br(&ljoin);
                self.block(&ljoin);
            }
            // a string: its bytes (D134); a part of a tuple (a message of
            // the faults of a reader, often a literal): a literal of the
            // program kept as a string (D361, 4 bytes past a multiple of 16,
            // where no block starts) not given to the runtime, which would
            // only find it out (D518; tested for every text, the code around
            // other drops laid out otherwise by LLVM: `time.formatDateTime`
            // +8.2 %, `strings.template` +3.8 %)
            Type::Str if !self.literal_test && !self.part_values.contains(&x.r) => {
                let p = self.ins(format!("extractvalue {{ ptr, i64 }} {}, 0", x.r));
                self.emit(format!("call void @isc_free(ptr {})", p));
            }
            Type::Str => {
                let p = self.ins(format!("extractvalue {{ ptr, i64 }} {}, 0", x.r));
                let pi = self.ins(format!("ptrtoint ptr {} to i64", p));
                let low = self.ins(format!("and i64 {}, 15", pi));
                let lit0 = self.ins(format!("icmp eq i64 {}, 4", low));
                self.decls.insert("declare i1 @llvm.expect.i1(i1, i1)".to_string());
                let lit = self.ins(format!("call i1 @llvm.expect.i1(i1 {}, i1 false)", lit0));
                let (lfree, ljoin) = (self.label(), self.label());
                self.cbr(&lit, &ljoin, &lfree);
                self.block(&lfree);
                self.emit(format!("call void @isc_free(ptr {})", p));
                self.br(&ljoin);
                self.block(&ljoin);
            }
            Type::Big => {
                let p = self.ins(format!("extractvalue {{ ptr, i64 }} {}, 0", x.r));
                self.emit(format!("call void @isc_free(ptr {})", p));
            }
            Type::Array(e, _) => {
                let p = self.ins(format!("extractvalue {{ ptr, i64 }} {}, 0", x.r));
                // each node of the elements: one owner less (D130); the
                // strings by the runtime, those of one block at once (D305)
                if **e == Type::Str {
                    let n = self.ins(format!("extractvalue {{ ptr, i64 }} {}, 1", x.r));
                    self.decls.insert("declare void @isc_free_strs(ptr, i64)".into());
                    self.emit(format!("call void @isc_free_strs(ptr {}, i64 {})", p, n));
                } else if holds_owned(e) {
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
            // a value of a sum type: the fields of its variant only, by its
            // tag; those of the others are zeros (D448: their frees, of null,
            // a call each)
            Type::Tuple(ts) if self.sum_slots.contains_key(t) => {
                let slots = self.sum_slots[t].clone();
                let tag = self.ins(format!("extractvalue {} {}, 0", x.t, x.r));
                let lj = self.label();
                let mut cases = Vec::new();
                let mut bodies = Vec::new();
                for (k, mine) in slots.iter().enumerate() {
                    if mine.iter().any(|&j| ts[j as usize].has_heap()) {
                        let l = self.label();
                        cases.push(format!("i32 {}, label %{}", k, l));
                        bodies.push((l, mine.clone()));
                    }
                }
                self.emit(format!("switch i32 {}, label %{} [ {} ]", tag, lj, cases.join(" ")));
                // (a field of a variant: a part of a tuple, D518)
                let saved = std::mem::replace(&mut self.literal_test, true);
                for (l, mine) in bodies {
                    self.block(&l);
                    for j in mine {
                        let c = &ts[j as usize];
                        if c.has_heap() {
                            let f = self.ins(format!("extractvalue {} {}, {}", x.t, x.r, j));
                            self.drop_value(&v(f, lt(c)), c);
                        }
                    }
                    self.br(&lj);
                }
                self.literal_test = saved;
                self.block(&lj);
            }
            Type::Tuple(ts) => {
                let saved = std::mem::replace(&mut self.literal_test, true);
                for (i, c) in ts.iter().enumerate() {
                    if c.has_heap() {
                        let f = self.ins(format!("extractvalue {} {}, {}", x.t, x.r, i));
                        self.drop_value(&v(f, lt(c)), c);
                    }
                }
                self.literal_test = saved;
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
                self.text_copy(&p, &n)
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

    /// The block of the text given `p` (`n` bytes) for `total` bytes: `p`
    /// itself when its block has room, read here as the runtime's `room`
    /// reads it (D511: the call of `isc_str_reserve` 13.6 % of `json.quote`
    /// of escaped texts): a text not empty (the empty one points to no
    /// block), on 64 bytes (a block of the heap: not a literal, not a line
    /// of `lines`), the allocator of Isocron (`isc_heap_mode` 2), the
    /// header of the block 64 bytes before it in use (`IN_USE`) and of a
    /// small class (below 16: `(class + 1) * 64` bytes, `alloc_class_size`)
    /// holding `total`; `isc_str_reserve` otherwise (another block, a
    /// larger class, glibc's allocator). The layout read is that of
    /// `Header` in `heap.rs` (its class a `u32` at 0, its state a byte at 4),
    /// asserted there when the runtime is compiled.
    fn reserve_inline(&mut self, p: &str, n: &str, total: &str) -> String {
        if self.opts.isa.is_wasm() {
            return self.ins(format!("call ptr @isc_str_reserve(ptr {}, i64 {}, i64 {})", p, n, total));
        }
        self.node_decls();
        let (lhead, lfast, lslow, lj) = (self.label(), self.label(), self.label(), self.label());
        let pi = self.ins(format!("ptrtoint ptr {} to i64", p));
        let low = self.ins(format!("and i64 {}, 63", pi));
        let aligned = self.ins(format!("icmp eq i64 {}, 0", low));
        let some = self.ins(format!("icmp sgt i64 {}, 0", n));
        let mode = self.ins("load i8, ptr @isc_heap_mode".to_string());
        let ours = self.ins(format!("icmp eq i8 {}, 2", mode));
        let a1 = self.ins(format!("and i1 {}, {}", aligned, some));
        let a2 = self.ins(format!("and i1 {}, {}", a1, ours));
        self.cbr(&a2, &lhead, &lslow);
        self.block(&lhead);
        let hdr = self.ins(format!("getelementptr inbounds i8, ptr {}, i64 -64", p));
        let class = self.ins(format!("load i32, ptr {}", hdr));
        let sp = self.ins(format!("getelementptr inbounds i8, ptr {}, i64 4", hdr));
        let state = self.ins(format!("load i8, ptr {}", sp));
        let small = self.ins(format!("icmp ult i32 {}, 16", class));
        let inuse = self.ins(format!("icmp eq i8 {}, -91", state));
        let c64 = self.ins(format!("zext i32 {} to i64", class));
        let c1 = self.ins(format!("add i64 {}, 1", c64));
        let size = self.ins(format!("shl i64 {}, 6", c1));
        let fits = self.ins(format!("icmp ule i64 {}, {}", total, size));
        let b1 = self.ins(format!("and i1 {}, {}", small, inuse));
        let b2 = self.ins(format!("and i1 {}, {}", b1, fits));
        self.cbr(&b2, &lfast, &lslow);
        self.block(&lfast);
        self.br(&lj);
        self.block(&lslow);
        let r = self.ins(format!("call ptr @isc_str_reserve(ptr {}, i64 {}, i64 {})", p, n, total));
        let lslow_end = self.current_label(&lslow);
        self.br(&lj);
        self.block(&lj);
        self.ins(format!("phi ptr [ {}, %{} ], [ {}, %{} ]", p, lfast, r, lslow_end))
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
        // the names of registers of this function alone (D518)
        let saved_parts = std::mem::take(&mut self.part_values);
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
        self.part_values = saved_parts;
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
        // the names of registers of this function alone (D518)
        let saved_parts = std::mem::take(&mut self.part_values);
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
        self.part_values = saved_parts;
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
                // the same length and the same bytes (D592)
                let len = self.ins(format!("icmp eq i64 {}, {}", n, m));
                let k = self.ins(format!("select i1 {}, i64 {}, i64 0", len, n));
                let same = self.text_same(&p, &q, &k);
                self.ins(format!("and i1 {}, {}", len, same))
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
        // the names of registers of this function alone (D518)
        let saved_parts = std::mem::take(&mut self.part_values);
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
        self.part_values = saved_parts;
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

    /// Frees `x`, the value of `var`; an array whose texts are views of the
    /// texts of another value (D555) gives back its block alone.
    fn drop_var(&mut self, var: VarId, x: &V) {
        if self.p.views.contains(&var) {
            let p = self.ins(format!("extractvalue {{ ptr, i64 }} {}, 0", x.r));
            self.emit(format!("call void @isc_free(ptr {})", p));
            return;
        }
        let t = self.var_ty(var).clone();
        self.drop_value(x, &t);
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
            // a string literal: borrowed data of the program, read where it
            // is (D301); a consumer that keeps it gets a copy (the ownership
            // pass)
            StrLit(s) if e.ty == Type::Str => {
                let (g, n) = self.str_const(s);
                let r0 = self.ins(format!("insertvalue {{ ptr, i64 }} poison, ptr {}, 0", g));
                Ok(v(self.ins(format!("insertvalue {{ ptr, i64 }} {}, i64 {}, 1", r0, n)), "{ ptr, i64 }"))
            }
            StrLit(s) => {
                // a `bigint`: its bytes in the form of the compiled code
                // (base 256, D168), from the form of the IR (base 128)
                let (g, n) = if e.ty == Type::Big { self.bytes_const(&isc_runtime_core::big_to_256(s.as_bytes())) } else { self.str_const(s) };
                Ok(self.text_copy(&g, &n.to_string()))
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
            // a `let` of a split `fold` (D260): its value for this element,
            // computed before the loop
            Let { var, body, .. } if w == 1 && self.split_vars.contains_key(var) => {
                let (slot, t, kvar) = self.split_vars[var].clone();
                let k = self.env.get(&kvar).cloned().ok_or_else(|| format!("codegen: unbound variable %{}", kvar))?;
                let p = self.ins(format!("getelementptr inbounds {}, ptr {}, i64 {}", t, slot, k.r));
                let x = self.ins(format!("load {}, ptr {}", t, p));
                self.bind(*var, v(x, t));
                self.ex(body, w)
            }
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
                self.print_stream_any(init)?;
                Ok(v("false", "i1"))
            }
            // `(a * k + b) / c` of bigints, the product and the sum bound to
            // temporaries read once: one call, the quotient from the leading
            // words when they decide it, neither made (D389)
            Let { var, init, body, drop: true, stmt: false } if muladd_div(*var, init, body).is_some() => {
                let (x, y, z, c, span) = muladd_div(*var, init, body).unwrap();
                let mut parts = Vec::new();
                for o in [x, y, z, c] {
                    let val = self.ex(o, 1)?;
                    parts.push(self.str_parts(&val));
                }
                let out = self.alloca("{ ptr, i64 }");
                let args = parts.iter().map(|(p, n)| format!("ptr {}, i64 {}", p, n)).collect::<Vec<_>>().join(", ");
                self.emit(format!("call void @isc_big_muladd_div(ptr {}, {}, {})", out, args, Self::pos(span)));
                Ok(v(self.ins(format!("load {{ ptr, i64 }}, ptr {}", out)), "{ ptr, i64 }"))
            }
            // `z + x * y` or `z - x * y` of bigints: one call, one pass when
            // one factor is of one word, the product not made (D406)
            Let { var, init, body, drop: true, stmt: false } if w == 1 && !self.opts.isa.is_wasm() && muladd_parts(*var, init, body).is_some() => {
                let (lets, x, y, z, sub, span) = muladd_parts(*var, init, body).unwrap();
                let mut bound = Vec::new();
                for (q, i, drop) in &lets {
                    let iv = self.ex(i, 1)?;
                    self.bind(*q, iv.clone());
                    bound.push((*q, iv, *drop));
                }
                let mut parts = Vec::new();
                for o in [z, x, y] {
                    let val = self.ex(o, 1)?;
                    parts.push(self.str_parts(&val));
                }
                let out = self.alloca("{ ptr, i64 }");
                let args = parts.iter().map(|(p, n)| format!("ptr {}, i64 {}", p, n)).collect::<Vec<_>>().join(", ");
                self.emit(format!("call void @isc_big_muladd(ptr {}, {}, i32 {}, {})", out, args, sub as i32, Self::pos(span)));
                let r = v(self.ins(format!("load {{ ptr, i64 }}, ptr {}", out)), "{ ptr, i64 }");
                for (q, iv, drop) in bound.iter().rev() {
                    if *drop {
                        self.drop_var(*q, iv);
                    }
                }
                Ok(r)
            }
            // keys made only to be grouped: grouped as they are made (D244)
            Let { var, init, body, .. } if w == 1 && group_let_ok(*var, init, body).is_some() => self.group_stream(init, group_let_ok(*var, init, body) == Some(true)),
            // a join read once in order by a `map` of its bytes: never made,
            // its bytes mapped as they are gathered (D428)
            Let { var, init, body, .. } if w == 1 && !self.in_task && join_map(*var, init, body).is_some() => {
                let (lets, parts, sep, c, m, l, rest, drop, stmt) = join_map(*var, init, body).unwrap();
                self.join_map_let(&lets, parts, sep, c, m, l, rest, drop, stmt)
            }
            // a `filter` read only by the blocks of an array written by
            // blocks: each block makes the part it reads (D293)
            Let { var, init, body, drop, .. } if w == 1 && !self.in_task && windowed_filter(*var, init, body).is_some() => {
                let (lets, filter) = windowed_filter(*var, init, body).unwrap();
                self.windowed_let(*var, &lets, filter, body, *drop)
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
                if let (StrLit(t), Type::Str) = (&init.kind, &init.ty) {
                    self.lit_vars.insert(*var, t.clone());
                }
                if let (StrLit(t), Type::Str) = (&init.kind, &init.ty) {
                    self.str_lits.insert(*var, t.clone());
                }
                // `toString(x)` of an integer read only as the text of one
                // `padStart` or `padEnd`: `x` now, its text written padded by
                // that operation, no string between (D348)
                // (and, D359, one read only as a part of a concatenation, the
                // left one given to it too: its text written by the
                // concatenation in its block)
                if let (StrOp(isc_ir::StrOp::ToString, xs), 1) = (&init.kind, w) {
                    if let Type::Int(k) = xs[0].ty {
                        if !*stmt && ((*drop && sole_pad_text(body, *var)) || sole_concat_part(body, *var)) {
                            let xv = self.ex(&xs[0], 1)?;
                            let x = if k.bits() < 64 {
                                let op = if k.signed() { "sext" } else { "zext" };
                                self.ins(format!("{} i{} {} to i64", op, k.bits(), xv.r))
                            } else {
                                xv.r.clone()
                            };
                            self.int_texts.insert(*var, IntText { x, signed: k.signed(), pad: None });
                            let r = self.ex(body, w);
                            self.int_texts.remove(var);
                            return r;
                        }
                    }
                }
                // a padded text of an integer read only as a part of a
                // concatenation: its `let`s made, its padding described, the
                // concatenation writing it (D359)
                if w == 1 && !*stmt && sole_concat_part(body, *var) {
                    if let Some(t) = tail_pad(init) {
                        let saved = self.defer_pad.replace(t as *const Expr);
                        let iv = self.ex(init, 1);
                        self.defer_pad = saved;
                        let iv = iv?;
                        if let Some(it) = self.padded.take() {
                            self.int_texts.insert(*var, it);
                            let r = self.ex(body, w);
                            self.int_texts.remove(var);
                            return r;
                        }
                        self.bind(*var, iv.clone());
                        let r = self.ex(body, w)?;
                        if *drop {
                            self.drop_var(*var, &iv);
                        }
                        return Ok(r);
                    }
                }
                // a text that its scope only reads (never moved): the
                // concatenations that give it write it into a buffer on the
                // stack when it fits (D404)
                let mut leaves = Vec::new();
                if w == 1 && *drop && !*stmt && init.ty == Type::Str && !moves_var(body, *var) {
                    result_concats(init, &mut leaves);
                }
                let buf = if leaves.is_empty() {
                    None
                } else {
                    // (aligned on its size, D592: never across two pages.
                    // Aligned on 16, it was 16 bytes before the end of a
                    // page in one program measured, and each text copied
                    // into it by a store over two pages: 11 cycles more)
                    let b = self.alloca(&format!("[{} x i8], align {}", STACK_STR, STACK_STR));
                    for l in &leaves {
                        self.stack_concats.insert(*l, b.clone());
                    }
                    Some(b)
                };
                // the counts of the predicate of a later windowed `filter`
                // over the ranges of an array, made by blocks in the pass of
                // this `filter` over it (D426)
                let made = if w == 1 && !self.in_task && !self.count_users.is_empty() { self.count_block(init, body) } else { None };
                let iv = self.ex(init, w);
                if made.is_some() {
                    self.count_req = None;
                }
                for l in &leaves {
                    self.stack_concats.remove(l);
                }
                let iv = iv?;
                if let Some(b) = &buf {
                    self.stack_strs.insert(iv.r.clone(), b.clone());
                }
                // a text taken out of a tuple: tested as a literal wherever it
                // is freed (at the end of its scope, by a `free`, before the
                // jump of a tail call; D518). (A name of a register: one met
                // again in a task only adds the test, always right.)
                if matches!(init.kind, ExprKind::Take(..)) && init.ty == Type::Str {
                    self.part_values.insert(iv.r.clone());
                }
                self.bind(*var, iv.clone());
                if *stmt && w == 1 && !self.kernel_fn {
                    self.black_box(&iv);
                }
                // a value the rest never reads is freed at once, not at the
                // end of its scope: its memory is given again to the next
                // allocation (`let (_, _) = bench(1, f)` before the timed
                // call, DECISIONS D117)
                let early = *drop && w == 1 && !mentions(body, &HashSet::from([*var])) && !frees(body, *var);
                if early {
                    self.drop_var(*var, &iv);
                }
                if let Some((a, block)) = &made {
                    self.counts.insert(*a, block.clone());
                }
                // its block to the `concat` that reads it alone, which grows
                // it (D533): what is left to free, its binding after it
                let takes = *drop && !early && w == 1 && init.ty.is_array() && concat_alone(body, *var);
                if takes {
                    self.taking.insert(*var);
                }
                // its elements to the gather of a permutation of it, which
                // moves them (D548): its block alone freed after
                let moves = *drop
                    && !early
                    && w == 1
                    && !self.p.views.contains(var)
                    && matches!(&init.ty, Type::Array(e, _) if !e.is_array() && holds_owned(e))
                    && isc_ir::passes::permutation_gather(body, *var).is_some();
                if moves {
                    self.moving.insert(*var);
                }
                let r = self.ex(body, w);
                self.moving.remove(var);
                let iv = if takes {
                    self.taking.remove(var);
                    self.env.get(var).cloned().unwrap_or(iv)
                } else {
                    iv
                };
                if let Some((a, block)) = &made {
                    self.counts.remove(a);
                    self.emit(format!("call void @isc_free(ptr {})", block));
                }
                if *drop && !early && moves {
                    if let Ok(_) = &r {
                        let p = self.ins(format!("extractvalue {{ ptr, i64 }} {}, 0", iv.r));
                        self.emit(format!("call void @isc_free(ptr {})", p));
                    }
                } else if *drop && !early {
                    if let Ok(_) = &r {
                        // a part taken out of a tuple (D518)
                        let saved = std::mem::replace(&mut self.literal_test, matches!(init.kind, ExprKind::Take(..)));
                        self.drop_var(*var, &iv);
                        self.literal_test = saved;
                    }
                }
                if buf.is_some() {
                    self.stack_strs.remove(&iv.r);
                }
                r
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
            // no rounding of their own: the instruction, for a scalar and
            // for a vector (D613); the same bits everywhere, a NaN quiet
            Math(f @ (MathFn::Floor | MathFn::Ceil | MathFn::Trunc | MathFn::Abs | MathFn::Round), xs) => {
                let xv = self.ex(&xs[0], w)?;
                let ww = if uni(&xv) { 1 } else { w };
                let (s, k) = if e.ty == Type::F32 { ("float", "f32") } else { ("double", "f64") };
                let (t, kind, bit) = if ww == 1 {
                    (s.to_string(), k.to_string(), "i1".to_string())
                } else {
                    (format!("<{} x {}>", ww, s), format!("v{}{}", ww, k), format!("<{} x i1>", ww))
                };
                let call = |g: &mut Self, name: &str, args: &[&str]| -> String {
                    let full = format!("llvm.{}.{}", name, kind);
                    g.decls.insert(format!("declare {} @{}({})", t, full, vec![t.as_str(); args.len()].join(", ")));
                    let args: Vec<String> = args.iter().map(|a| format!("{} {}", t, a)).collect();
                    g.ins(format!("call {} @{}({})", t, full, args.join(", ")))
                };
                let r = match f {
                    MathFn::Floor => call(self, "floor", &[&xv.r]),
                    MathFn::Ceil => call(self, "ceil", &[&xv.r]),
                    MathFn::Trunc => call(self, "trunc", &[&xv.r]),
                    MathFn::Abs => call(self, "fabs", &[&xv.r]),
                    _ => {
                        // `round` of ECMAScript (D155): the floor, and one
                        // more when what is left is a half or more (exact:
                        // a float and its floor are within 1); a zero
                        // takes the sign of `x` (-0.25 gives -0). No
                        // `x + 0.5`, which rounds; no relaxed arithmetic
                        // here, whatever the function allows.
                        let floor = call(self, "floor", &[&xv.r]);
                        let left = self.ins(format!("fsub {} {}, {}", t, xv.r, floor));
                        let up = self.ins(format!("fcmp oge {} {}, {}", t, left, self.cst(s, &fconst(0.5), ww)));
                        let more = self.ins(format!("fadd {} {}, {}", t, floor, self.cst(s, &fconst(1.0), ww)));
                        let y = self.ins(format!("select {} {}, {} {}, {} {}", bit, up, t, more, t, floor));
                        let zero = self.ins(format!("fcmp oeq {} {}, zeroinitializer", t, y));
                        let signed = call(self, "copysign", &[&y, &xv.r]);
                        self.ins(format!("select {} {}, {} {}, {} {}", bit, zero, t, signed, t, y))
                    }
                };
                Ok(V { r, t, uni: uni(&xv) })
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
            // the strings of the program and the array of their places,
            // aligned as an array (D401)
            ConstStrs(ss) => {
                let vals: Vec<String> = ss
                    .iter()
                    .map(|s| {
                        let (g, n) = self.str_const(s);
                        format!("{{ ptr, i64 }} {{ ptr {}, i64 {} }}", g, n)
                    })
                    .collect();
                let name = format!("@.arr.{}", self.globals.len());
                self.globals.push(format!("{} = private unnamed_addr constant [{} x {{ ptr, i64 }}] [{}], align 64", name, ss.len(), vals.join(", ")));
                Ok(self.arr(&name, &ss.len().to_string()))
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
                match self.split_fail.clone() {
                    // before the loop of a split `fold` (D260)
                    Some(lf) => {
                        let ok = self.ins(format!("icmp ult i64 {}, {}", idx, n));
                        let lok = self.label();
                        self.cbr(&ok, &lok, &lf);
                        self.block(&lok);
                    }
                    None => self.bounds(&idx, &n, &i.ty, span),
                }
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
                // the elements that own memory (strings, arrays of a tuple,
                // nodes): each a copy of its own, as `copy_value` does, not
                // the source's shared (D342: a slice of records that hold
                // arrays freed them twice)
                if holds_owned(&elem_ty) {
                    self.for_range_scalar("0", &m, 1, &mut |g, i| {
                        let el = g.load_elem(&dst, &m, i, &elem_ty, 1);
                        let c = g.copy_value(&el, &elem_ty);
                        g.store_elem(&dst, &m, i, &elem_ty, &c, 1);
                        Ok(())
                    })?;
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
            Tasks(x, l) => self.tasks(e, x, l),
            Fold(x, init, l) if w > 1 => self.fold_vec(x, init, l, w),
            Fold(x, init, l) if last_where(x, l).is_some() => {
                let c = last_where(x, l).unwrap();
                self.fold_last(x, init, l, c)
            }
            Fold(x, init, l) if first_where(x, l).is_some() => {
                let c = first_where(x, l).unwrap();
                self.fold_first(x, init, l, c)
            }
            Fold(x, init, l) if isc_ir::passes::is_stream(x) => self.fold_stream(x, init, l),
            Fold(x, init, l) if affine_recurrence(x, l).is_some() => {
                let (f, value) = affine_recurrence(x, l).unwrap();
                self.fold_affine(x, init, l, f, value)
            }
            Fold(x, init, l) => match additive_update(l) {
                Some((gi, h, read)) if !self.in_task => self.fold_private(e, x, init, l, gi, h, read),
                _ => match split_plan(x, l) {
                    Some((n, vars)) => self.fold_split(x, init, l, n, &vars),
                    None => self.fold(x, init, l),
                },
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
            // a literal kept: its bytes where they are in the program, 4 bytes
            // past a multiple of 16, which the runtime never frees nor writes
            // (D361), not a copy in a block
            Copy(x) if matches!(&x.kind, StrLit(_)) && x.ty == Type::Str => {
                let StrLit(t) = &x.kind else { unreachable!() };
                let (g, n) = self.owned_literal(t);
                let r0 = self.ins(format!("insertvalue {{ ptr, i64 }} poison, ptr {}, 0", g));
                Ok(v(self.ins(format!("insertvalue {{ ptr, i64 }} {}, i64 {}, 1", r0, n)), "{ ptr, i64 }"))
            }
            // an element of an array whose elements a permutation takes
            // (D548): moved
            Copy(x) if matches!(&x.kind, Index(a, _) if matches!(a.kind, Var(v, _) if self.moving.contains(&v))) => self.ex(x, 1),
            Copy(x) => {
                let xv = self.ex(x, 1)?;
                Ok(self.copy_value(&xv, &x.ty))
            }
            Action(op, args) => self.action(*op, args, e),
            Bench(r, body) => self.bench(r, body, e),
            // the body, then a variable freed (D105)
            Free(var, body) => {
                // its block to the `concat` that reads it alone (D533)
                let takes = w == 1 && self.p.vars[*var as usize].ty.is_array() && concat_alone(body, *var);
                if takes {
                    self.taking.insert(*var);
                }
                let r = self.ex(body, w);
                self.taking.remove(var);
                let r = r?;
                let x = self.env[var].clone();
                // (a text freed on the paths that do not move it: often a
                // message of a fault, D518)
                let saved = std::mem::replace(&mut self.literal_test, true);
                self.drop_var(*var, &x);
                self.literal_test = saved;
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
                        let x = self.env[var].clone();
                        self.drop_var(*var, &x);
                    }
                    self.text_called.insert(*f);
                    let r = self.ins(format!("tail call tailcc {} @isc_fn_{}({})", ret, f, vals.join(", ")));
                    self.emit(format!("ret {} {}", ret, r));
                    let dead = self.label();
                    self.block(&dead);
                    return Ok(V { r: "poison".into(), t: ret, uni: true });
                }
                // the result written in the buffer of the variant `_buf`
                // being emitted: the variant of the callee
                let buffer = self.text_buffer(e);
                // the base case of the callee here, the call only for the
                // other branch (D202)
                let inline_base = if buffer.is_some() { None } else { self.base_cases.get(f).copied() };
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
                let r = if let Some(tb) = buffer {
                    self.buf_needed.insert(*f);
                    self.ins(format!("call {}{} @isc_fn_{}_buf(ptr {}, {})", cc, ret, f, tb, vals.join(", ")))
                } else if let (Some(rg), true) = (self.region.clone(), self.builders.contains(f)) {
                    self.rg_needed.insert(*f);
                    self.ins(format!("call {}{} @isc_fn_{}_rg(ptr {}, {})", cc, ret, f, rg, vals.join(", ")))
                } else if let Some(sm) = self.count_arg(*f, args) {
                    // the counts made of the array it is given (D426)
                    self.count_needed.insert(*f);
                    self.ins(format!("call {}{} @isc_fn_{}_sm(ptr {}, {})", cc, ret, f, sm, vals.join(", ")))
                } else {
                    self.text_called.insert(*f);
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
        // `x % c` of consecutive `x` and a constant c of at least w: the
        // lanes are consecutive unless they cross a multiple of c, known from
        // the first lane alone: then one load from the first remainder, the
        // general code (a gather) otherwise (D229)
        if let (ExprKind::Binary(BinOp::Rem, x, c), Type::Int(IntKind::I64), Src::Mat { .. }) = (&i.kind, &i.ty, &src) {
            if let ExprKind::Lit(isc_ir::Lit::Int(c)) = c.kind {
                if w > 1 && c >= w as i64 && x.ty == Type::Int(IntKind::I64) {
                    let xv = self.ex(x, w)?;
                    if let (false, Some((first, 1))) = (uni(&xv), self.f.contig.get(&xv.r).cloned()) {
                        // the remainder of the first lane carried from the
                        // row before (D318), or computed
                        let (r0, nonneg) = match self.f.carried.get(&(i as *const Expr)).cloned() {
                            Some((slot, ok)) => (self.ins(format!("load i64, ptr {}", slot)), ok),
                            None => (self.ins(format!("srem i64 {}, {}", first, c)), self.ins(format!("icmp sge i64 {}, 0", first))),
                        };
                        // the table copied with its period continued (D273):
                        // every remainder of a lane in it, whatever the first
                        let period = match &a.kind {
                            ExprKind::Var(t, _) => self.f.period_tables.get(&(*t, c)).cloned(),
                            _ => None,
                        };
                        if let (Some((buf, made)), Src::Mat { elem, .. }) = (period, &src) {
                            let elem = elem.clone();
                            let ok = self.ins(format!("and i1 {}, {}", nonneg, made));
                            let (lfast, lslow, ljoin) = (self.label(), self.label(), self.label());
                            self.cbr(&ok, &lfast, &lslow);
                            self.block(&lfast);
                            let x1 = self.load_elem(&buf, &n, &r0, &elem, w);
                            let l1 = self.current_label(&lfast);
                            self.br(&ljoin);
                            self.block(&lslow);
                            let iv = self.ex(i, w)?;
                            let x2 = self.index_vec_with(a, i, iv, &src, &n, w, span)?;
                            let l2 = self.current_label(&lslow);
                            self.br(&ljoin);
                            self.block(&ljoin);
                            let r = self.ins(format!("phi {} [ {}, %{} ], [ {}, %{} ]", x1.t, x1.r, l1, x2.r, l2));
                            return Ok(V { r, t: x1.t, uni: false });
                        }
                        let room = self.ins(format!("icmp sle i64 {}, {}", r0, c - w as i64));
                        let ok = self.ins(format!("and i1 {}, {}", nonneg, room));
                        let lfast = self.label();
                        let lslow = self.label();
                        let ljoin = self.label();
                        self.cbr(&ok, &lfast, &lslow);
                        self.block(&lfast);
                        let base = self.widen(&v(r0.clone(), "i64"), w);
                        let steps: Vec<String> = (0..w).map(|k| format!("i64 {}", k)).collect();
                        let fv = self.ins(format!("add <{} x i64> {}, <{}>", w, base.r, steps.join(", ")));
                        self.f.contig.insert(fv.clone(), (r0.clone(), 1));
                        let x1 = self.index_vec_with(a, i, V { r: fv, t: format!("<{} x i64>", w), uni: false }, &src, &n, w, span)?;
                        let l1 = self.current_label(&lfast);
                        self.br(&ljoin);
                        self.block(&lslow);
                        let iv = self.ex(i, w)?;
                        let x2 = self.index_vec_with(a, i, iv, &src, &n, w, span)?;
                        let l2 = self.current_label(&lslow);
                        self.br(&ljoin);
                        self.block(&ljoin);
                        let r = self.ins(format!("phi {} [ {}, %{} ], [ {}, %{} ]", x1.t, x1.r, l1, x2.r, l2));
                        return Ok(V { r, t: x1.t, uni: false });
                    }
                }
            }
        }
        let iv = self.ex(i, w)?;
        self.index_vec_with(a, i, iv, &src, &n, w, span)
    }

    /// `index_vec` once the index `iv` (the lanes of `i`) is computed.
    #[allow(clippy::too_many_arguments)]
    fn index_vec_with(&mut self, a: &Expr, i: &Expr, iv: V, src: &Src, n: &str, w: u32, span: Span) -> R<V> {
        let n = n.to_string();
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
        // checked before the loop of its `fold` (D296): no check here
        if self.mask.is_none() && self.prechecked.contains(&(i as *const Expr)) {
            self.br(&lok);
        } else {
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
        // 64 lanes: only a table in registers (D388; any array of bytes
        // read on 64 lanes took the test and the permutations too: fasta +4
        // % instructions)
        let in_registers = matches!(a.kind, ExprKind::Var(v, _) if self.f.table_parts.contains_key(&v));
        if esize(&elem) == 1 && matches!(elem, Type::Int(_)) && (w <= 32 || (w == 64 && in_registers)) && BYTE_LOOKUP.with(|b| b.get()) {
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
        // a window of 64 bytes holds the lanes of 32 indices at most
        if w > 32 {
            return self.gather(ptr, idx, elem, w);
        }
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
        if w == 64 {
            // 64 lanes (D388): the table in 4 parts of 64 bytes (two of 32
            // each, joined once out of the loop), two permutations of 128 of
            // them (bits 0 to 6 of each index), the one of bit 7
            self.decls.insert("declare <64 x i8> @llvm.x86.avx512.vpermi2var.qi.512(<64 x i8>, <64 x i8>, <64 x i8>)".into());
            let all = (0..64).map(|j| format!("i32 {}", j)).collect::<Vec<_>>().join(", ");
            let t: Vec<String> = (0..4).map(|j| self.ins(format!("shufflevector <32 x i8> {}, <32 x i8> {}, <64 x i32> <{}>", parts[2 * j], parts[2 * j + 1], all))).collect();
            let q0 = self.ins(format!("call <64 x i8> @llvm.x86.avx512.vpermi2var.qi.512(<64 x i8> {}, <64 x i8> {}, <64 x i8> {})", t[0], i8v, t[1]));
            let q1 = self.ins(format!("call <64 x i8> @llvm.x86.avx512.vpermi2var.qi.512(<64 x i8> {}, <64 x i8> {}, <64 x i8> {})", t[2], i8v, t[3]));
            let s7 = self.ins(format!("icmp slt <64 x i8> {}, zeroinitializer", i8v));
            return self.ins(format!("select <64 x i1> {}, <64 x i8> {}, <64 x i8> {}", s7, q1, q0));
        }
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
        if let Some(parts) = concat_parts(e) {
            if plain_columns(e.ty.elem()) {
                return self.concat_copy(e, parts);
            }
        }
        self.materialize_general(e)
    }

    /// `concat(a, b)` of arrays of scalars, or of tuples of scalars,
    /// materialized: a new block and each side copied (`memcpy`, column by
    /// column), not a loop with a test per element (D340). The `map` it is
    /// lowered to stays the other branch, taken when the lengths do not fit
    /// its form (a `map` written so by hand): its checks panic as before.
    fn concat_copy(&mut self, e: &Expr, (a, b, l, n): (&Expr, &Expr, &Expr, &Expr)) -> R<V> {
        let et = e.ty.elem().clone();
        let nv = self.ex(n, 1)?;
        let av = self.ex(a, 1)?;
        let bv = self.ex(b, 1)?;
        let lv = self.ex(l, 1)?;
        let (pa, la) = self.str_parts(&av);
        let (pb, lb) = self.str_parts(&bv);
        let rest = self.ins(format!("sub i64 {}, {}", nv.r, lv.r));
        let c1 = self.ins(format!("icmp sge i64 {}, 0", lv.r));
        let c2 = self.ins(format!("icmp sle i64 {}, {}", lv.r, la));
        let c3 = self.ins(format!("icmp sge i64 {}, 0", rest));
        let c4 = self.ins(format!("icmp sle i64 {}, {}", rest, lb));
        let c12 = self.ins(format!("and i1 {}, {}", c1, c2));
        let c34 = self.ins(format!("and i1 {}, {}", c3, c4));
        let ok = self.ins(format!("and i1 {}, {}", c12, c34));
        let (lfast, lslow, ljoin) = (self.label(), self.label(), self.label());
        self.cbr(&ok, &lfast, &lslow);
        self.block(&lfast);
        // the left array's block taken (its last read, D533): grown in place
        // when it has room, else a block of twice its size (as a text
        // continued, D351), the right one copied after it
        let took = match &a.kind {
            ExprKind::Var(va, _) if self.taking.contains(va) && !matches!(et, Type::Tuple(_)) => Some(*va),
            _ => None,
        };
        if let Some(va) = took {
            let sz = esize(&et);
            let used = self.ins(format!("mul i64 {}, {}", la, sz));
            let need = self.ins(format!("mul i64 {}, {}", nv.r, sz));
            let q = self.ins(format!("call ptr @isc_str_reserve(ptr {}, i64 {}, i64 {})", pa, used, need));
            let at = self.ins(format!("getelementptr inbounds i8, ptr {}, i64 {}", q, used));
            let second = self.ins(format!("mul i64 {}, {}", rest, sz));
            self.emit(format!("call void @llvm.memcpy.p0.p0.i64(ptr {}, ptr {}, i64 {}, i1 false)", at, pb, second));
            let fast = self.arr(&q, &nv.r);
            let lfast_end = self.current_label(&lfast);
            self.br(&ljoin);
            self.block(&lslow);
            let slow = self.materialize_general(e)?;
            let lslow_end = self.current_label(&lslow);
            self.br(&ljoin);
            self.block(&ljoin);
            let r = self.ins(format!("phi {{ ptr, i64 }} [ {}, %{} ], [ {}, %{} ]", fast.r, lfast_end, slow.r, lslow_end));
            // nothing left to free of it when taken: its binding from here
            // (no read of it follows, `concat_alone`)
            let left = self.ins(format!("phi {{ ptr, i64 }} [ zeroinitializer, %{} ], [ {}, %{} ]", lfast_end, av.r, lslow_end));
            self.taking.remove(&va);
            self.env.insert(va, v(left, "{ ptr, i64 }"));
            return Ok(V { r, t: "{ ptr, i64 }".into(), uni: fast.uni && slow.uni });
        }
        let dst = self.alloc_elems(&nv.r, &et);
        let cols = |g: &mut Self, p: &str, len: &str| -> Vec<(Type, String)> {
            if matches!(et, Type::Tuple(_)) {
                g.columns(p, len, &et).into_iter().map(|(_, t, c)| (t, c)).collect()
            } else {
                vec![(et.clone(), p.to_string())]
            }
        };
        let (ca, cb, cd) = (cols(self, &pa, &la), cols(self, &pb, &lb), cols(self, &dst, &nv.r));
        for (((leaf, sa), (_, sb)), (_, d)) in ca.iter().zip(&cb).zip(&cd) {
            let sz = esize(leaf);
            let first = self.ins(format!("mul i64 {}, {}", lv.r, sz));
            self.emit(format!("call void @llvm.memcpy.p0.p0.i64(ptr {}, ptr {}, i64 {}, i1 false)", d, sa, first));
            let at = self.ins(format!("getelementptr inbounds i8, ptr {}, i64 {}", d, first));
            let second = self.ins(format!("mul i64 {}, {}", rest, sz));
            self.emit(format!("call void @llvm.memcpy.p0.p0.i64(ptr {}, ptr {}, i64 {}, i1 false)", at, sb, second));
        }
        let fast = self.arr(&dst, &nv.r);
        let lfast_end = self.current_label(&lfast);
        self.br(&ljoin);
        self.block(&lslow);
        let slow = self.materialize_general(e)?;
        let lslow_end = self.current_label(&lslow);
        self.br(&ljoin);
        self.block(&ljoin);
        let r = self.ins(format!("phi {{ ptr, i64 }} [ {}, %{} ], [ {}, %{} ]", fast.r, lfast_end, slow.r, lslow_end));
        Ok(V { r, t: "{ ptr, i64 }".into(), uni: fast.uni && slow.uni })
    }

    fn materialize_general(&mut self, e: &Expr) -> R<V> {
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
            // a few elements, each a loop of its own, where the loops run one
            // task after the other: filled here, without the task and its
            // dispatch (D520: the members of each object of `json.parse`,
            // two maps of 4 elements, a context and an indirect call each)
            let few = self.ins(format!("icmp sle i64 {}, {}", n, HEAVY_FEW));
            let (lask, lfew, lpar, ldone) = (self.label(), self.label(), self.label(), self.label());
            self.cbr(&few, &lask, &lpar);
            self.block(&lask);
            // (only when the loops would run one task after the other: one
            // thread, or inside a parallel loop; else their parallelism kept)
            self.decls.insert("declare zeroext i1 @isc_loops_alone()".to_string());
            let alone = self.ins("call zeroext i1 @isc_loops_alone()".to_string());
            self.cbr(&alone, &lfew, &lpar);
            self.block(&lfew);
            self.fill_range(&src, &p, &n, &et, "0", &n, w)?;
            self.br(&ldone);
            self.block(&lpar);
            Some(ldone)
        } else {
            let small = self.ins(format!("icmp sle i64 {}, {}", n, fill_block(esize(&et))));
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
            fill_block(esize(&et)).to_string()
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
        // the blocks of the array: each thread its range of them (D254)
        self.par_for(if heavy { 1 } else { 2 }, &c, &task, &ctx, &ctx_bytes);
        if let Some(l) = ldone {
            self.br(&l);
            self.block(&l);
        }
        Ok(self.arr(&p, &n))
    }

    /// A loop of `n` tasks given to the pool (D357): through a word of its
    /// own for this place of the program, which keeps how its runs on the
    /// pool and on the calling thread went (`isc_parallel_for_site`); `mode`
    /// 0 every `threads`-th task to a thread, 1 chunks taken in turn
    /// (`isc_parallel_for_dynamic`), 2 a range for each thread
    /// (`isc_parallel_for_fill`).
    fn par_for(&mut self, mode: i32, n: &str, task: &str, ctx: &str, ctx_bytes: &str) {
        let k = self.globals.iter().filter(|g| g.starts_with("@.parsite.")).count();
        let site = format!("@.parsite.{}", k);
        self.globals.push(format!("{} = private global [4 x i64] zeroinitializer, align 8", site));
        self.emit(format!("call void @isc_parallel_for_site(i64 {}, ptr {}, ptr {}, i64 {}, ptr {}, i32 {})", n, task, ctx, ctx_bytes, site, mode));
    }

    /// `printBytes` of a `map` of bytes made only for it (D225): by blocks of
    /// STREAM bytes, each made in parallel as `materialize` makes the whole
    /// array (tasks of BLOCK elements, the same `fill_range`), into one
    /// buffer of STREAM bytes, then written; the buffer stays in the caches
    /// (a quarter of a MiB per thread), the bytes never go to memory nor to
    /// new pages. The element `i` of the block from `lo` is stored at
    /// `buffer - lo + i`: the code of the elements does not change. The
    /// same bytes, written in the same order.
    /// `print_stream` through the choice of a table and the `let`s before
    /// the `map` (`streamable`): each branch its `map` by blocks, each `let`
    /// bound, then freed as the `let` frees it (D234).
    fn print_stream_any(&mut self, e: &Expr) -> R<()> {
        match &e.kind {
            ExprKind::If(c, a, b) => {
                let cv = self.ex(c, 1)?;
                let la = self.label();
                let lb = self.label();
                let lj = self.label();
                self.cbr(&cv.r, &la, &lb);
                self.block(&la);
                self.print_stream_any(a)?;
                self.br(&lj);
                self.block(&lb);
                self.print_stream_any(b)?;
                self.br(&lj);
                self.block(&lj);
                Ok(())
            }
            // the test of a size, then the rest
            ExprKind::CheckNonNegative(k, body) => {
                let kv = self.ex(k, 1)?;
                let ok = self.ins(format!("icmp sge i64 {}, 0", kv.r));
                self.uniform_check(&ok, format!("call void @isc_panic_negsize(i64 {}, {})", kv.r, Self::pos(e.span)), 1);
                self.print_stream_any(body)
            }
            ExprKind::Let { var, init, body, drop, .. } => {
                let iv = self.ex(init, 1)?;
                self.bind(*var, iv.clone());
                self.print_stream_any(body)?;
                if *drop {
                    self.drop_var(*var, &iv);
                }
                Ok(())
            }
            _ if stream_map_ok(e) => self.print_stream(e),
            // the other branch: made whole, written, freed
            _ => {
                let x = self.ex(e, 1)?;
                let (p, n) = self.str_parts(&x);
                self.emit(format!("call void @isc_print_bytes(ptr {}, i64 {})", p, n));
                self.drop_value(&x, &e.ty);
                Ok(())
            }
        }
    }

    /// `$group(e)`, `e` a `map` of integer keys (D244): the parts of
    /// consecutive indices that the runtime chooses (`isc_group_parts`), in
    /// parallel, each with a table of its own; a part makes its keys by
    /// blocks of GROUP_BLOCK in a buffer on its stack, each block added to
    /// its table (`isc_groups_add`), then the tables merged in their order
    /// (`isc_groups_end`): the columns of `isc_group` for the same keys.
    /// From 2^31 keys on, the array of keys made whole and `isc_group`.
    /// `counts`: `$groupCounts`, tables of counts only (D317).
    fn group_stream(&mut self, e: &Expr, counts: bool) -> R<V> {
        const GROUP_BLOCK: i64 = 1024;
        let et = e.ty.elem().clone();
        let kind = group_kind(&et);
        let whole_kind = kind | if counts { GROUP_COUNTS } else { 0 };
        let spmd = match &e.kind {
            ExprKind::Map(_, l) => spmd_state(self.p, &l.body),
            _ => None,
        };
        let w = match (src_vec_ok_lookup(e), spmd) {
            (false, _) => 1,
            (true, Some(state)) => self.spmd_width(state),
            (true, None) if esize(&et) == 1 => 2 * MAP_WIDTH,
            (true, None) => MAP_WIDTH,
        };
        let (_, n) = self.prepare_src(e)?;
        let out = self.alloca("{ ptr, i64 }");
        let big = self.ins(format!("icmp sge i64 {}, {}", n, 1i64 << 31));
        let lbig = self.label();
        let lstream = self.label();
        let ldone = self.label();
        self.cbr(&big, &lbig, &lstream);
        self.block(&lbig);
        let all = self.materialize(e)?;
        let (p, m) = self.str_parts(&all);
        self.emit(format!("call void @isc_group(ptr {}, ptr {}, i64 {}, i32 {})", out, p, m, whole_kind));
        self.emit(format!("call void @isc_free(ptr {})", p));
        self.br(&ldone);
        self.block(&lstream);
        let parts = self.ins(format!("call i64 @isc_group_parts(i64 {})", n));
        let state = self.ins(format!("call ptr @isc_groups_new(i64 {}, i64 {}, i32 {})", n, parts, counts as i32));
        let extra = [v(state.clone(), "ptr"), v(parts.clone(), "i64")];
        let et2 = et.clone();
        let (task, ctx, ctx_bytes) = self.outline_task(&[e], &extra, &mut |g, x, i| {
            let (src, n) = g.prepare_src(e)?;
            let buf = g.alloca(&format!("[{} x {}]", GROUP_BLOCK, mt(&et2)));
            // the part i: n * i / parts .. n * (i + 1) / parts
            let a0 = g.ins(format!("mul i64 {}, {}", n, i));
            let lo = g.ins(format!("sdiv i64 {}, {}", a0, x[1].r));
            let i1 = g.ins(format!("add i64 {}, 1", i));
            let b0 = g.ins(format!("mul i64 {}, {}", n, i1));
            let hi = g.ins(format!("sdiv i64 {}, {}", b0, x[1].r));
            let len = g.ins(format!("sub i64 {}, {}", hi, lo));
            let nb0 = g.ins(format!("add i64 {}, {}", len, GROUP_BLOCK - 1));
            let nb = g.ins(format!("sdiv i64 {}, {}", nb0, GROUP_BLOCK));
            let state = x[0].r.clone();
            g.for_range_scalar("0", &nb, 1, &mut |g, k| {
                let off = g.ins(format!("mul i64 {}, {}", k, GROUP_BLOCK));
                let start = g.ins(format!("add i64 {}, {}", lo, off));
                let lim = g.ins(format!("add i64 {}, {}", start, GROUP_BLOCK));
                let short = g.ins(format!("icmp slt i64 {}, {}", hi, lim));
                let end = g.ins(format!("select i1 {}, i64 {}, i64 {}", short, hi, lim));
                let neg = g.ins(format!("sub i64 0, {}", start));
                let dst = g.ins(format!("getelementptr {}, ptr {}, i64 {}", mt(&et2), buf, neg));
                g.fill_range(&src, &dst, &n, &et2, &start, &end, w)?;
                let cnt = g.ins(format!("sub i64 {}, {}", end, start));
                g.emit(format!("call void @isc_groups_add(ptr {}, i64 {}, ptr {}, i64 {}, i64 {}, i32 {})", state, i, buf, cnt, start, kind));
                Ok(())
            })
        })?;
        self.par_for(0, &parts, &task, &ctx, &ctx_bytes);
        self.emit(format!("call void @isc_groups_end(ptr {}, ptr {})", out, state));
        self.br(&ldone);
        self.block(&ldone);
        Ok(v(self.ins(format!("load {{ ptr, i64 }}, ptr {}", out)), "{ ptr, i64 }"))
    }

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
        let in_task = self.in_task;
        // the arrays of `windowed_let` it reads: by windows, or made whole
        // here for this array (`windowed_stream_start`)
        let reads = self.windowed_reads(e);
        // a task of the fill writes at least 16 KiB (D321), also where it
        // reads by windows (D490: tasks of 4096 bytes there, as B,
        // reverse-complement 14 % slower)
        let tb = fill_block(esize(&et));
        let (ok, wins) = self.windowed_stream_start(e, &reads, &n, tb)?;
        let small = self.ins(format!("icmp slt i64 {}, {}", n, STREAM));
        let size = self.ins(format!("select i1 {}, i64 {}, i64 {}", small, n, STREAM));
        let buf = self.ins(format!("call ptr @isc_alloc(i64 {})", size));
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
                let c0 = g.ins(format!("add i64 {}, {}", len, tb - 1));
                let c = g.ins(format!("sdiv i64 {}, {}", c0, tb));
                let mut extra = vec![v(dst.clone(), "ptr"), v(lo.to_string(), "i64"), v(hi.clone(), "i64")];
                if !wins.is_empty() {
                    extra.push(v(ok.clone(), "i1"));
                    extra.extend(wins.iter().map(|(_, xw, _)| xw.clone()));
                }
                let et2 = et.clone();
                let filters: Vec<Expr> = reads.iter().map(|(x, _)| g.windowed[x].clone()).collect();
                let mut scope: Vec<&Expr> = vec![e];
                scope.extend(filters.iter());
                let (task, ctx, ctx_bytes) = g.outline_task(&scope, &extra, &mut |t, x, i| {
                    let (src, n) = t.prepare_src(e)?;
                    let off = t.ins(format!("mul i64 {}, {}", i, tb));
                    let start = t.ins(format!("add i64 {}, {}", x[1].r, off));
                    let lim = t.ins(format!("add i64 {}, {}", start, tb));
                    let short = t.ins(format!("icmp slt i64 {}, {}", x[2].r, lim));
                    let end = t.ins(format!("select i1 {}, i64 {}, i64 {}", short, x[2].r, lim));
                    let heaps = match x.get(3) {
                        Some(ok) => t.windowed_task(e, &reads, &filters, &ok.r, &x[4..], &n, &start, &end)?,
                        None => Vec::new(),
                    };
                    t.fill_range(&src, &x[0].r, &n, &et2, &start, &end, w)?;
                    for hp in heaps {
                        t.free_if(&hp);
                    }
                    Ok(())
                })?;
                // each thread its range of the blocks of the chunk (D254)
                g.par_for(2, &c, &task, &ctx, &ctx_bytes);
            }
            let len = g.ins(format!("sub i64 {}, {}", hi, lo));
            g.emit(format!("call void @isc_print_bytes(ptr {}, i64 {})", buf, len));
            Ok(())
        })?;
        self.emit(format!("call void @isc_free(ptr {})", buf));
        // the arrays made whole for it freed; their variables again the
        // places and the length
        for (x, xw, whole) in wins {
            self.free_if(&whole);
            self.bind(x, xw);
        }
        Ok(())
    }

    /// `free(p)` unless `p` is null.
    fn free_if(&mut self, p: &str) {
        let null = self.ins(format!("icmp eq ptr {}, null", p));
        let (lfree, ldone) = (self.label(), self.label());
        self.cbr(&null, &ldone, &lfree);
        self.block(&lfree);
        self.emit(format!("call void @isc_free(ptr {})", p));
        self.br(&ldone);
        self.block(&ldone);
    }

    /// Before the blocks of `print_stream` of the `map` `e` (`n` elements)
    /// that reads the arrays of `windowed_let` (`reads`): whether its blocks
    /// read them by windows (`ok`, an `i1`): the bounds of the indices over
    /// the whole `map` without an overflow, and those of its first task (of
    /// `tb` elements) at most WINDOW_WIDE tasks apart; never in a task.
    /// Otherwise each array is made whole here (`windowed_whole`) and its
    /// variable holds it. Returns `ok` and, for each array, its variable, its
    /// places and length, and the array made whole (or null).
    #[allow(clippy::type_complexity)]
    fn windowed_stream_start(&mut self, e: &Expr, reads: &[(VarId, Vec<(&Expr, Vec<(VarId, &Expr)>)>)], n: &str, tb: i64) -> R<(String, Vec<(VarId, V, String)>)> {
        if reads.is_empty() {
            return Ok(("false".into(), Vec::new()));
        }
        let bad = self.alloca("i1");
        self.emit(format!("store i1 {}, ptr {}", self.in_task, bad));
        let last = self.ins(format!("sub i64 {}, 1", n));
        let first = self.ins(format!("call i64 @llvm.smin.i64(i64 {}, i64 {})", n, tb + WINDOW_MARGIN));
        let first = self.ins(format!("sub i64 {}, 1", first));
        for (_, rs) in reads {
            self.windowed_bounds(e, rs, "0", &last, &bad)?;
            let (a, b) = self.windowed_bounds(e, rs, "0", &first, &bad)?;
            let width = self.checked("sub", &b, &a, &bad);
            let wide = self.ins(format!("icmp sgt i64 {}, {}", width, WINDOW_WIDE * tb));
            let was = self.ins(format!("load i1, ptr {}", bad));
            let now = self.ins(format!("or i1 {}, {}", was, wide));
            self.emit(format!("store i1 {}, ptr {}", now, bad));
        }
        let bad_v = self.ins(format!("load i1, ptr {}", bad));
        let ok = self.ins(format!("xor i1 {}, true", bad_v));
        let mut wins = Vec::new();
        for (x, _) in reads {
            let xw = self.env[x].clone();
            let f = self.windowed[x].clone();
            let origin = self.windowed_origin.contains(x);
            let (lwhole, lskip, ljoin) = (self.label(), self.label(), self.label());
            self.cbr(&ok, &lskip, &lwhole);
            self.block(&lwhole);
            let whole = self.windowed_whole(&f, &xw, origin)?;
            let lfrom = self.current_label(&lwhole);
            self.br(&ljoin);
            self.block(&lskip);
            self.br(&ljoin);
            self.block(&ljoin);
            let wp = self.ins(format!("phi ptr [ {}, %{} ], [ null, %{} ]", whole, lfrom, lskip));
            let k = self.ins(format!("extractvalue {{ ptr, i64 }} {}, 1", xw.r));
            let wv = self.arr(&wp, &k);
            let xs = self.ins(format!("select i1 {}, {{ ptr, i64 }} {}, {{ ptr, i64 }} {}", ok, xw.r, wv.r));
            self.bind(*x, V { r: xs, t: "{ ptr, i64 }".into(), uni: true });
            wins.push((*x, xw, wp));
        }
        Ok((ok, wins))
    }

    /// In a task of `print_stream` (elements `start .. end` of the `map`
    /// `e` of `n` elements), when `ok`: each array of `windowed_let` read by
    /// `e` (its places and length in `xws`) filled for the bounds of its
    /// indices over `start - WINDOW_MARGIN ..= end - 1 + WINDOW_MARGIN` (in
    /// `0 .. n`: what the vectors of the loop may read past its ends) and
    /// its variable bound to it. Returns the blocks of the heap to free.
    #[allow(clippy::too_many_arguments, clippy::type_complexity)]
    fn windowed_task(&mut self, e: &Expr, reads: &[(VarId, Vec<(&Expr, Vec<(VarId, &Expr)>)>)], filters: &[Expr], ok: &str, xws: &[V], n: &str, start: &str, end: &str) -> R<Vec<String>> {
        let mut heaps = Vec::new();
        if reads.is_empty() {
            return Ok(heaps);
        }
        let (lwin, lgo) = (self.label(), self.label());
        let lentry = self.current_label("entry");
        let mut bound = Vec::new();
        for (x, _) in reads {
            bound.push(self.env[x].clone());
        }
        self.cbr(ok, &lwin, &lgo);
        self.block(&lwin);
        let qa = self.ins(format!("sub i64 {}, {}", start, WINDOW_MARGIN));
        let qa = self.ins(format!("call i64 @llvm.smax.i64(i64 {}, i64 0)", qa));
        let qb = self.ins(format!("add i64 {}, {}", end, WINDOW_MARGIN - 1));
        let nm1 = self.ins(format!("sub i64 {}, 1", n));
        let qb = self.ins(format!("call i64 @llvm.smin.i64(i64 {}, i64 {})", qb, nm1));
        let bad = self.alloca("i1");
        self.emit(format!("store i1 false, ptr {}", bad));
        let mut filled = Vec::new();
        for (((x, rs), xw), f) in reads.iter().zip(xws).zip(filters) {
            let (lo, hi) = self.windowed_bounds(e, rs, &qa, &qb, &bad)?;
            let buf = self.alloca(&format!("[{} x i8], align 64", WINDOW_STACK));
            let origin = self.windowed_origin.contains(x);
            let (xv, hp) = self.windowed_fill(f, xw, &lo, &hi, &buf, origin)?;
            filled.push((xv, hp));
        }
        let lwin_end = self.current_label(&lwin);
        self.br(&lgo);
        self.block(&lgo);
        for (((x, _), b), (xv, hp)) in reads.iter().zip(bound).zip(filled) {
            let r = self.ins(format!("phi {{ ptr, i64 }} [ {}, %{} ], [ {}, %{} ]", b.r, lentry, xv.r, lwin_end));
            self.bind(*x, V { r, t: "{ ptr, i64 }".into(), uni: true });
            heaps.push(self.ins(format!("phi ptr [ null, %{} ], [ {}, %{} ]", lentry, hp, lwin_end)));
        }
        Ok(heaps)
    }

    /// `let x = filter(src, p) in body` by windows (`windowed_filter`,
    /// D293): the elements of `src` that `p` keeps counted by blocks of
    /// WINDOW_BLOCK in parallel (tasks of WINDOW_GROUP blocks), the place of
    /// each block in the result (the sum of the counts before it, `places[c]`
    /// the length `k` after the last one) into one block of memory; `x` holds
    /// it and `k` (its length is `k`; freed with `x`), and each block of an
    /// array written by blocks makes the part of `x` it reads
    /// (`windowed_fill`), never the whole array.
    fn windowed_let(&mut self, x: VarId, lets: &[(VarId, &Expr)], filter: &Expr, body: &Expr, drop: bool) -> R<V> {
        const WINDOW_GROUP: i64 = 16;
        for (t, i) in lets {
            let iv = self.ex(i, 1)?;
            self.bind(*t, iv);
        }
        let ExprKind::Filter(input, l) = &filter.kind else { unreachable!() };
        let (src0, n) = self.prepare_src(input)?;
        // the counts of `l` by blocks made by a caller over an array that
        // `input` is a range of (D426)
        let given = match (&self.count_param, &src0) {
            (Some((sm, pl)), Src::Mat { ptr, .. }) if std::ptr::eq(l, *pl) => Some((sm.clone(), ptr.clone())),
            _ => None,
        };
        if let Some((sm, sptr)) = given {
            return self.windowed_given(x, filter, body, drop, &sm, &sptr, &n);
        }
        let c0 = self.ins(format!("add i64 {}, {}", n, WINDOW_BLOCK - 1));
        let c = self.ins(format!("sdiv i64 {}, {}", c0, WINDOW_BLOCK));
        let c1 = self.ins(format!("add i64 {}, 1", c));
        let bytes = self.ins(format!("mul i64 {}, 8", c1));
        let places = self.ins(format!("call ptr @isc_alloc(i64 {})", bytes));
        // 1. the elements each block keeps
        let g0 = self.ins(format!("add i64 {}, {}", c, WINDOW_GROUP - 1));
        let groups = self.ins(format!("sdiv i64 {}, {}", g0, WINDOW_GROUP));
        let extra = [v(places.clone(), "ptr"), v(c.clone(), "i64")];
        self.decls.insert("declare i64 @llvm.smin.i64(i64, i64)".into());
        let (task, ctx, ctx_bytes) = self.outline_task(&[filter], &extra, &mut |g, xs, j| {
            let (src, n) = g.prepare_src(input)?;
            let w = g.filter_width(input, l, input.ty.elem());
            let b0 = g.ins(format!("mul i64 {}, {}", j, WINDOW_GROUP));
            let b1 = g.ins(format!("add i64 {}, {}", b0, WINDOW_GROUP));
            let b1 = g.ins(format!("call i64 @llvm.smin.i64(i64 {}, i64 {})", b1, xs[1].r));
            g.for_range_scalar(&b0, &b1, 1, &mut |g, b| {
                let start = g.ins(format!("mul i64 {}, {}", b, WINDOW_BLOCK));
                let end = g.ins(format!("add i64 {}, {}", start, WINDOW_BLOCK));
                let end = g.ins(format!("call i64 @llvm.smin.i64(i64 {}, i64 {})", end, n));
                let k = g.count_kept(&src, l, &start, &end, w)?;
                let slot = g.ins(format!("getelementptr inbounds i64, ptr {}, i64 {}", xs[0].r, b));
                g.emit(format!("store i64 {}, ptr {}", k, slot));
                Ok(())
            })
        })?;
        self.par_for(0, &groups, &task, &ctx, &ctx_bytes);
        // 2. the place of each block: the sum of the counts before it
        let run = self.alloca("i64");
        self.emit(format!("store i64 0, ptr {}", run));
        self.for_range_scalar("0", &c, 1, &mut |g, b| {
            let slot = g.ins(format!("getelementptr inbounds i64, ptr {}, i64 {}", places, b));
            let cb = g.ins(format!("load i64, ptr {}", slot));
            let before = g.ins(format!("load i64, ptr {}", run));
            g.emit(format!("store i64 {}, ptr {}", before, slot));
            let after = g.ins(format!("add i64 {}, {}", before, cb));
            g.emit(format!("store i64 {}, ptr {}", after, run));
            Ok(())
        })?;
        let k = self.ins(format!("load i64, ptr {}", run));
        let last = self.ins(format!("getelementptr inbounds i64, ptr {}, i64 {}", places, c));
        self.emit(format!("store i64 {}, ptr {}", k, last));
        let xv = self.arr(&places, &k);
        self.bind(x, xv.clone());
        self.windowed.insert(x, filter.clone());
        let r = self.ex(body, 1);
        self.windowed.remove(&x);
        let r = r?;
        if drop {
            self.emit(format!("call void @isc_free(ptr {})", places));
        }
        Ok(r)
    }

    /// `windowed_let` given the block `sm` of the counts of its predicate
    /// made by a caller (D426: `[base, length, count of block 0, ...]`, by
    /// blocks of WINDOW_BLOCK of the array from `base`): when the source
    /// (`sptr`, `n` elements) lies in that array, its blocks are those of
    /// the array, from an origin (its offset in the first one, kept in the
    /// slot after those of the most blocks); the count of each whole block read from
    /// `sm`, those of the two ends counted here. Otherwise the origin is 0
    /// and the counts are made as by `windowed_let`.
    #[allow(clippy::too_many_arguments)]
    fn windowed_given(&mut self, x: VarId, filter: &Expr, body: &Expr, drop: bool, sm: &str, sptr: &str, n: &str) -> R<V> {
        const WINDOW_GROUP: i64 = 16;
        let ExprKind::Filter(input, l) = &filter.kind else { unreachable!() };
        let es = esize(input.ty.elem());
        self.decls.insert("declare i64 @llvm.smin.i64(i64, i64)".into());
        self.decls.insert("declare i64 @llvm.smax.i64(i64, i64)".into());
        // the source within the array of the counts, which were made
        let (lentry, lload, lj) = (self.current_label("entry"), self.label(), self.label());
        let none = self.ins(format!("icmp eq ptr {}, null", sm));
        self.cbr(&none, &lj, &lload);
        self.block(&lload);
        let base = self.ins(format!("load ptr, ptr {}", sm));
        let ls = self.ins(format!("getelementptr inbounds i64, ptr {}, i64 1", sm));
        let alen = self.ins(format!("load i64, ptr {}", ls));
        let bi = self.ins(format!("ptrtoint ptr {} to i64", base));
        let si = self.ins(format!("ptrtoint ptr {} to i64", sptr));
        let offb = self.ins(format!("sub i64 {}, {}", si, bi));
        let made = self.ins(format!("icmp ne ptr {}, null", base));
        let after = self.ins(format!("icmp sge i64 {}, 0", offb));
        let nb = self.ins(format!("mul i64 {}, {}", n, es));
        let endb = self.ins(format!("add i64 {}, {}", offb, nb));
        let lim = self.ins(format!("mul i64 {}, {}", alen, es));
        let within = self.ins(format!("icmp sle i64 {}, {}", endb, lim));
        let ok0 = self.ins(format!("and i1 {}, {}", made, after));
        let ok1 = self.ins(format!("and i1 {}, {}", ok0, within));
        let off1 = self.ins(format!("sdiv i64 {}, {}", offb, es));
        let lload_end = self.current_label(&lload);
        self.br(&lj);
        self.block(&lj);
        let ok = self.ins(format!("phi i1 [ false, %{} ], [ {}, %{} ]", lentry, ok1, lload_end));
        let off = self.ins(format!("phi i64 [ 0, %{} ], [ {}, %{} ]", lentry, off1, lload_end));
        let off = self.ins(format!("select i1 {}, i64 {}, i64 0", ok, off));
        let o = self.ins(format!("and i64 {}, {}", off, WINDOW_BLOCK - 1));
        let a0 = self.ins(format!("sdiv i64 {}, {}", off, WINDOW_BLOCK));
        let c0 = self.ins(format!("add i64 {}, {}", n, WINDOW_BLOCK - 1));
        let c0 = self.ins(format!("add i64 {}, {}", c0, o));
        let c = self.ins(format!("sdiv i64 {}, {}", c0, WINDOW_BLOCK));
        // the places, then the origin in the slot after the most blocks a
        // source of `n` elements can have (the places freed as those of
        // `windowed_let`, with the array)
        let m0 = self.ins(format!("add i64 {}, {}", n, 2 * WINDOW_BLOCK - 2));
        let most = self.ins(format!("sdiv i64 {}, {}", m0, WINDOW_BLOCK));
        let m2 = self.ins(format!("add i64 {}, 2", most));
        let bytes = self.ins(format!("mul i64 {}, 8", m2));
        let places = self.ins(format!("call ptr @isc_alloc(i64 {})", bytes));
        let m1 = self.ins(format!("add i64 {}, 1", most));
        let os = self.ins(format!("getelementptr inbounds i64, ptr {}, i64 {}", places, m1));
        self.emit(format!("store i64 {}, ptr {}", o, os));
        let (lseq, lpar, ldone) = (self.label(), self.label(), self.label());
        self.cbr(&ok, &lseq, &lpar);
        // 1a. the counts of the whole blocks from `sm`, the ends counted
        self.block(&lseq);
        let (src, _) = self.prepare_src(input)?;
        let w = self.filter_width(input, l, input.ty.elem());
        self.for_range_scalar("0", &c, 1, &mut |g, j| {
            let at = g.ins(format!("mul i64 {}, {}", j, WINDOW_BLOCK));
            let s0 = g.ins(format!("sub i64 {}, {}", at, o));
            let start = g.ins(format!("call i64 @llvm.smax.i64(i64 {}, i64 0)", s0));
            let e0 = g.ins(format!("add i64 {}, {}", s0, WINDOW_BLOCK));
            let end = g.ins(format!("call i64 @llvm.smin.i64(i64 {}, i64 {})", e0, n));
            let span = g.ins(format!("sub i64 {}, {}", end, start));
            let whole = g.ins(format!("icmp eq i64 {}, {}", span, WINDOW_BLOCK));
            let (lw, lp, lk) = (g.label(), g.label(), g.label());
            g.cbr(&whole, &lw, &lp);
            g.block(&lw);
            let ab = g.ins(format!("add i64 {}, {}", a0, j));
            let ab2 = g.ins(format!("add i64 {}, 2", ab));
            let from = g.ins(format!("getelementptr inbounds i64, ptr {}, i64 {}", sm, ab2));
            let kw = g.ins(format!("load i64, ptr {}", from));
            g.br(&lk);
            g.block(&lp);
            let kp = g.count_kept(&src, l, &start, &end, w)?;
            let lp_end = g.current_label(&lp);
            g.br(&lk);
            g.block(&lk);
            let k = g.ins(format!("phi i64 [ {}, %{} ], [ {}, %{} ]", kw, lw, kp, lp_end));
            let slot = g.ins(format!("getelementptr inbounds i64, ptr {}, i64 {}", places, j));
            g.emit(format!("store i64 {}, ptr {}", k, slot));
            Ok(())
        })?;
        self.br(&ldone);
        // 1b. otherwise (origin 0) the elements each block keeps, in parallel
        self.block(&lpar);
        let g0 = self.ins(format!("add i64 {}, {}", c, WINDOW_GROUP - 1));
        let groups = self.ins(format!("sdiv i64 {}, {}", g0, WINDOW_GROUP));
        let extra = [v(places.clone(), "ptr"), v(c.clone(), "i64")];
        let (task, ctx, ctx_bytes) = self.outline_task(&[filter], &extra, &mut |g, xs, j| {
            let (src, n) = g.prepare_src(input)?;
            let w = g.filter_width(input, l, input.ty.elem());
            let b0 = g.ins(format!("mul i64 {}, {}", j, WINDOW_GROUP));
            let b1 = g.ins(format!("add i64 {}, {}", b0, WINDOW_GROUP));
            let b1 = g.ins(format!("call i64 @llvm.smin.i64(i64 {}, i64 {})", b1, xs[1].r));
            g.for_range_scalar(&b0, &b1, 1, &mut |g, b| {
                let start = g.ins(format!("mul i64 {}, {}", b, WINDOW_BLOCK));
                let end = g.ins(format!("add i64 {}, {}", start, WINDOW_BLOCK));
                let end = g.ins(format!("call i64 @llvm.smin.i64(i64 {}, i64 {})", end, n));
                let k = g.count_kept(&src, l, &start, &end, w)?;
                let slot = g.ins(format!("getelementptr inbounds i64, ptr {}, i64 {}", xs[0].r, b));
                g.emit(format!("store i64 {}, ptr {}", k, slot));
                Ok(())
            })
        })?;
        self.par_for(0, &groups, &task, &ctx, &ctx_bytes);
        self.br(&ldone);
        self.block(&ldone);
        // 2. the place of each block: the sum of the counts before it
        let run = self.alloca("i64");
        self.emit(format!("store i64 0, ptr {}", run));
        self.for_range_scalar("0", &c, 1, &mut |g, b| {
            let slot = g.ins(format!("getelementptr inbounds i64, ptr {}, i64 {}", places, b));
            let cb = g.ins(format!("load i64, ptr {}", slot));
            let before = g.ins(format!("load i64, ptr {}", run));
            g.emit(format!("store i64 {}, ptr {}", before, slot));
            let after = g.ins(format!("add i64 {}, {}", before, cb));
            g.emit(format!("store i64 {}, ptr {}", after, run));
            Ok(())
        })?;
        let k = self.ins(format!("load i64, ptr {}", run));
        let last = self.ins(format!("getelementptr inbounds i64, ptr {}, i64 {}", places, c));
        self.emit(format!("store i64 {}, ptr {}", k, last));
        let xv = self.arr(&places, &k);
        self.bind(x, xv.clone());
        self.windowed.insert(x, filter.clone());
        self.windowed_origin.insert(x);
        let r = self.ex(body, 1);
        self.windowed.remove(&x);
        self.windowed_origin.remove(&x);
        let r = r?;
        if drop {
            self.emit(format!("call void @isc_free(ptr {})", places));
        }
        Ok(r)
    }

    /// `join_map` (D428): the join's ranges made in parallel (`isc_join_ranges`,
    /// as `join`, D427), each by a task that gathers its bytes 16 KiB at a
    /// time into a buffer on its stack (`isc_join_next`) and maps them from
    /// there into `c`, in vector code (`fill_range`): the join never made,
    /// its bytes read in the caches. Both functions are emitted with the
    /// program (`join_fns`, D607).
    #[allow(clippy::too_many_arguments)]
    fn join_map_let(&mut self, lets: &[(VarId, &Expr)], parts: &Expr, sep: &Expr, c: VarId, m: &Expr, l: &Lambda, rest: &Expr, drop: bool, stmt: bool) -> R<V> {
        const GATHER: i64 = 16 << 10;
        for (t, i) in lets {
            let iv = self.ex(i, 1)?;
            self.bind(*t, iv);
        }
        let pv = self.ex(parts, 1)?;
        let sv = self.ex(sep, 1)?;
        let (pp, pn) = self.str_parts(&pv);
        let (sp, sn) = self.str_parts(&sv);
        let et = m.ty.elem().clone();
        let es = esize(&et);
        let w = match (src_vec_ok_lookup(m), spmd_state(self.p, &l.body)) {
            (false, _) => 1,
            (true, Some(state)) => self.spmd_width(state),
            (true, None) if es == 1 && et.is_scalar() => 2 * MAP_WIDTH,
            (true, None) => MAP_WIDTH,
        };
        // (a mark for the tests)
        self.emit("; join read by a map, never made".to_string());
        self.join_fns()?;
        let first = self.alloca("[257 x i64]");
        let place = self.alloca("[257 x i64]");
        let total = self.join_total(&pp, &pn, &sn)?;
        let ranges = self.ins(format!("call i64 @isc_join_ranges(ptr {}, i64 {}, i64 {}, i64 {}, ptr {}, ptr {})", pp, pn, sn, total, first, place));
        let out = self.alloc_elems(&total, &et);
        // the lambda's captures: a `map` of it over nothing
        let scope = Expr { kind: ExprKind::Map(vec![Expr { kind: ExprKind::Lit(Lit::Unit), ty: Type::Bool, span: m.span, reassoc: false }], l.clone()), ty: m.ty.clone(), span: m.span, reassoc: false };
        let extra = [v(pp.clone(), "ptr"), v(sp.clone(), "ptr"), v(sn.clone(), "i64"), v(first.clone(), "ptr"), v(place.clone(), "ptr"), v(out.clone(), "ptr")];
        let (task, ctx, ctx_bytes) = self.outline_task(&[&scope], &extra, &mut |g, xs, k| {
            let buf = g.alloca(&format!("[{} x i8], align 64", GATHER));
            let cur = g.alloca("{ i64, i64, i64 }");
            let f0 = g.ins(format!("getelementptr inbounds i64, ptr {}, i64 {}", xs[3].r, k));
            let from = g.ins(format!("load i64, ptr {}", f0));
            let k1 = g.ins(format!("add i64 {}, 1", k));
            let f1 = g.ins(format!("getelementptr inbounds i64, ptr {}, i64 {}", xs[3].r, k1));
            let to = g.ins(format!("load i64, ptr {}", f1));
            let p0 = g.ins(format!("getelementptr inbounds i64, ptr {}, i64 {}", xs[4].r, k));
            let start = g.ins(format!("load i64, ptr {}", p0));
            for (field, x) in [from.as_str(), "0", to.as_str()].into_iter().enumerate() {
                let slot = g.ins(format!("getelementptr inbounds {{ i64, i64, i64 }}, ptr {}, i32 0, i32 {}", cur, field));
                g.emit(format!("store i64 {}, ptr {}", x, slot));
            }
            let at = g.alloca("i64");
            g.emit(format!("store i64 {}, ptr {}", start, at));
            let (head, lbody, done) = (g.label(), g.label(), g.label());
            g.br(&head);
            g.block(&head);
            let got = g.ins(format!("call i64 @isc_join_next(ptr {}, ptr {}, ptr {}, i64 {}, ptr {}, i64 {})", cur, xs[0].r, xs[1].r, xs[2].r, buf, GATHER));
            let none = g.ins(format!("icmp eq i64 {}, 0", got));
            g.cbr(&none, &done, &lbody);
            g.block(&lbody);
            let a = g.ins(format!("load i64, ptr {}", at));
            let off = g.ins(format!("mul i64 {}, {}", a, es));
            let dst = g.ins(format!("getelementptr inbounds i8, ptr {}, i64 {}", xs[5].r, off));
            let src = Src::Map(vec![Src::Mat { ptr: buf.clone(), len: got.clone(), elem: Type::Int(IntKind::U8) }], l);
            g.fill_range(&src, &dst, &got, &et, "0", &got, w)?;
            let a2 = g.ins(format!("add i64 {}, {}", a, got));
            g.emit(format!("store i64 {}, ptr {}", a2, at));
            g.br(&head);
            g.block(&done);
            Ok(())
        })?;
        self.par_for(2, &ranges, &task, &ctx, &ctx_bytes);
        let cv = self.arr(&out, &total);
        self.bind(c, cv.clone());
        if stmt {
            self.black_box(&cv);
        }
        let r = self.ex(rest, 1)?;
        if drop {
            self.drop_var(c, &cv);
        }
        Ok(r)
    }

    /// D426: when `init` makes a `filter` over `iota(a.length)` whose
    /// predicate reads `a[i]` and `body` gives `a` to a function of
    /// `count_users`, the block of the counts of that function's predicate
    /// over `a` (by blocks of WINDOW_BLOCK, made by the pass of that
    /// `filter` while each block is in the caches), asked for in
    /// `count_req`: `a` and the block, its `base` null until they are made.
    fn count_block(&mut self, init: &Expr, body: &Expr) -> Option<(VarId, String)> {
        fn reads_at(e: &Expr, a: VarId, i: VarId) -> bool {
            let read = match &e.kind {
                ExprKind::Index(x, j) => matches!(x.kind, ExprKind::Var(v, _) if v == a) && matches!(j.kind, ExprKind::Var(k, _) if k == i),
                _ => false,
            };
            read || e.children().into_iter().any(|c| reads_at(c, a, i))
        }
        fn scan<'e>(e: &'e Expr, lens: &mut Vec<(VarId, VarId)>) -> Option<(&'e Expr, VarId)> {
            match &e.kind {
                ExprKind::Let { var, init, body, .. } => {
                    if let ExprKind::Len(x) = &init.kind {
                        if let ExprKind::Var(a, _) = x.kind {
                            lens.push((*var, a));
                        }
                    }
                    scan(body, lens)
                }
                ExprKind::CheckNonNegative(_, b) => scan(b, lens),
                ExprKind::Filter(src, l) => {
                    let ExprKind::Iota(n) = &src.kind else { return None };
                    let a = match &n.kind {
                        ExprKind::Len(x) => match x.kind {
                            ExprKind::Var(a, _) => a,
                            _ => return None,
                        },
                        ExprKind::Var(v, _) => lens.iter().find(|(w, _)| w == v)?.1,
                        _ => return None,
                    };
                    reads_at(&l.body, a, l.params[0]).then_some((e, a))
                }
                _ => None,
            }
        }
        fn call<'l>(e: &Expr, a: VarId, users: &HashMap<u32, (usize, &'l Lambda)>) -> Option<&'l Lambda> {
            if let ExprKind::Call(g, args) = &e.kind {
                if let Some((k, l)) = users.get(g) {
                    if matches!(args.get(*k).map(|x| &x.kind), Some(ExprKind::Var(v, _)) if *v == a) {
                        return Some(*l);
                    }
                }
            }
            e.children().into_iter().find_map(|c| call(c, a, users))
        }
        let (f, a) = scan(init, &mut Vec::new())?;
        let pl = call(body, a, &self.count_users)?;
        let at = self.var_ty(a).clone();
        if !at.is_array() || self.var_ty(pl.params[0]) != at.elem() {
            return None;
        }
        let av = self.env.get(&a)?.clone();
        if av.t != "{ ptr, i64 }" {
            return None;
        }
        let base = self.ins(format!("extractvalue {{ ptr, i64 }} {}, 0", av.r));
        let n = self.ins(format!("extractvalue {{ ptr, i64 }} {}, 1", av.r));
        let b0 = self.ins(format!("add i64 {}, {}", n, WINDOW_BLOCK - 1));
        let b = self.ins(format!("sdiv i64 {}, {}", b0, WINDOW_BLOCK));
        let b2 = self.ins(format!("add i64 {}, 2", b));
        let bytes = self.ins(format!("mul i64 {}, 8", b2));
        let block = self.ins(format!("call ptr @isc_alloc(i64 {})", bytes));
        self.emit(format!("store ptr null, ptr {}", block));
        let ls = self.ins(format!("getelementptr inbounds i64, ptr {}, i64 1", block));
        self.emit(format!("store i64 {}, ptr {}", n, ls));
        self.count_req = Some((f as *const Expr, a, pl, block.clone(), base));
        Some((a, block))
    }

    /// The block of the counts to give to the function `f` called with
    /// `args` (D426): the array of its parameter of `count_users` is one of
    /// `counts`.
    fn count_arg(&self, f: u32, args: &[Expr]) -> Option<String> {
        if self.in_task {
            return None;
        }
        let (k, _) = self.count_users.get(&f)?;
        match &args.get(*k)?.kind {
            ExprKind::Var(a, _) => self.counts.get(a).cloned(),
            _ => None,
        }
    }

    /// The origin and the count of the blocks of the source of `n`
    /// elements of an array of `windowed_let` whose places are `places`:
    /// with an origin (D426), the one after the slots of the most blocks
    /// (`windowed_given`).
    fn window_blocks(&mut self, places: &str, n: &str, origin: bool) -> (String, String) {
        if !origin {
            let c0 = self.ins(format!("add i64 {}, {}", n, WINDOW_BLOCK - 1));
            return ("0".into(), self.ins(format!("sdiv i64 {}, {}", c0, WINDOW_BLOCK)));
        }
        let m0 = self.ins(format!("add i64 {}, {}", n, 2 * WINDOW_BLOCK - 2));
        let most = self.ins(format!("sdiv i64 {}, {}", m0, WINDOW_BLOCK));
        let m1 = self.ins(format!("add i64 {}, 1", most));
        let os = self.ins(format!("getelementptr inbounds i64, ptr {}, i64 {}", places, m1));
        let o = self.ins(format!("load i64, ptr {}", os));
        let c0 = self.ins(format!("add i64 {}, {}", n, WINDOW_BLOCK - 1));
        let c0 = self.ins(format!("add i64 {}, {}", c0, o));
        (o, self.ins(format!("sdiv i64 {}, {}", c0, WINDOW_BLOCK)))
    }

    /// The elements `start .. end` of the source of `n` elements in its
    /// block `j`, from the origin `o` (D426).
    fn window_range(&mut self, j: &str, o: &str, n: &str) -> (String, String) {
        let at = self.ins(format!("mul i64 {}, {}", j, WINDOW_BLOCK));
        if o == "0" {
            let end = self.ins(format!("add i64 {}, {}", at, WINDOW_BLOCK));
            let end = self.ins(format!("call i64 @llvm.smin.i64(i64 {}, i64 {})", end, n));
            return (at, end);
        }
        let s0 = self.ins(format!("sub i64 {}, {}", at, o));
        let start = self.ins(format!("call i64 @llvm.smax.i64(i64 {}, i64 0)", s0));
        let e0 = self.ins(format!("add i64 {}, {}", s0, WINDOW_BLOCK));
        let end = self.ins(format!("call i64 @llvm.smin.i64(i64 {}, i64 {})", e0, n));
        (start, end)
    }

    /// The reads of the arrays of `windowed_let` in the function of the
    /// `map` `e`: for each array, its indices and the `let`s around each.
    fn windowed_reads<'a>(&self, e: &'a Expr) -> Vec<(VarId, Vec<(&'a Expr, Vec<(VarId, &'a Expr)>)>)> {
        fn walk<'a>(e: &'a Expr, ws: &HashMap<VarId, Expr>, scope: &mut Vec<(VarId, &'a Expr)>, out: &mut Vec<(VarId, Vec<(&'a Expr, Vec<(VarId, &'a Expr)>)>)>) {
            match &e.kind {
                ExprKind::Index(a, i) if matches!(a.kind, ExprKind::Var(x, _) if ws.contains_key(&x)) => {
                    let ExprKind::Var(x, _) = a.kind else { unreachable!() };
                    match out.iter_mut().find(|(y, _)| *y == x) {
                        Some((_, is)) => is.push((&**i, scope.clone())),
                        None => out.push((x, vec![(&**i, scope.clone())])),
                    }
                    walk(i, ws, scope, out);
                }
                ExprKind::Let { var, init, body, .. } => {
                    walk(init, ws, scope, out);
                    scope.push((*var, &**init));
                    walk(body, ws, scope, out);
                    scope.pop();
                }
                _ => {
                    for c in e.children() {
                        walk(c, ws, scope, out);
                    }
                }
            }
        }
        let mut out = Vec::new();
        if let ExprKind::Map(_, l) = &e.kind {
            if !self.windowed.is_empty() {
                walk(&l.body, &self.windowed, &mut Vec::new(), &mut out);
            }
        }
        out
    }

    /// `a op b` (`add`, `sub`, `mul` of `i64`), its overflow added to the
    /// `i1` in the slot `bad`.
    fn checked(&mut self, op: &str, a: &str, b: &str, bad: &str) -> String {
        self.decls.insert(format!("declare {{ i64, i1 }} @llvm.s{}.with.overflow.i64(i64, i64)", op));
        let r = self.ins(format!("call {{ i64, i1 }} @llvm.s{}.with.overflow.i64(i64 {}, i64 {})", op, a, b));
        let o = self.ins(format!("extractvalue {{ i64, i1 }} {}, 1", r));
        let was = self.ins(format!("load i1, ptr {}", bad));
        let now = self.ins(format!("or i1 {}, {}", was, o));
        self.emit(format!("store i1 {}, ptr {}", now, bad));
        self.ins(format!("extractvalue {{ i64, i1 }} {}, 0", r))
    }

    /// The bounds `lo <= e <= hi` of the integer `e` for `q` in `qlo ..=
    /// qhi` (`interval_ok`: the variables bound outside the `map` read as
    /// they are, those of `scope` by their value), as `i64`; when a bound
    /// overflows, `bad` is set. From bounds of the operands without an
    /// overflow, each operation gives bounds of its results.
    #[allow(clippy::too_many_arguments)]
    fn index_interval(&mut self, e: &Expr, q: VarId, qlo: &str, qhi: &str, scope: &[(VarId, &Expr)], bad: &str) -> R<(String, String)> {
        let Type::Int(k) = e.ty else { return Err("codegen: an interval of a non-integer".into()) };
        if k.bits() < 64 {
            let bits = k.bits();
            let (lo, hi) = if k.signed() { (-(1i64 << (bits - 1)), (1i64 << (bits - 1)) - 1) } else { (0, (1i64 << bits) - 1) };
            return Ok((lo.to_string(), hi.to_string()));
        }
        self.decls.insert("declare i64 @llvm.smax.i64(i64, i64)".into());
        self.decls.insert("declare i64 @llvm.smin.i64(i64, i64)".into());
        let lit = |a: &Expr| match a.kind {
            ExprKind::Lit(Lit::Int(c)) => Some(c),
            _ => None,
        };
        Ok(match &e.kind {
            ExprKind::Var(v, _) if *v == q => (qlo.to_string(), qhi.to_string()),
            ExprKind::Var(v, _) => match scope.iter().rposition(|(w, _)| w == v) {
                Some(at) => self.index_interval(scope[at].1, q, qlo, qhi, &scope[..at], bad)?,
                None => {
                    let x = self.ex(e, 1)?;
                    (x.r.clone(), x.r)
                }
            },
            ExprKind::Lit(Lit::Int(c)) => (c.to_string(), c.to_string()),
            ExprKind::Binary(BinOp::Add, a, b) => {
                let (al, ah) = self.index_interval(a, q, qlo, qhi, scope, bad)?;
                let (bl, bh) = self.index_interval(b, q, qlo, qhi, scope, bad)?;
                (self.checked("add", &al, &bl, bad), self.checked("add", &ah, &bh, bad))
            }
            ExprKind::Binary(BinOp::Sub, a, b) => {
                let (al, ah) = self.index_interval(a, q, qlo, qhi, scope, bad)?;
                let (bl, bh) = self.index_interval(b, q, qlo, qhi, scope, bad)?;
                (self.checked("sub", &al, &bh, bad), self.checked("sub", &ah, &bl, bad))
            }
            ExprKind::Binary(BinOp::Mul, a, b) if lit(a).is_some() || lit(b).is_some() => {
                let (x, c) = if let Some(c) = lit(b) { (a, c) } else { (b, lit(a).unwrap()) };
                let (xl, xh) = self.index_interval(x, q, qlo, qhi, scope, bad)?;
                let (l, h) = if c >= 0 { (xl, xh) } else { (xh, xl) };
                (self.checked("mul", &l, &c.to_string(), bad), self.checked("mul", &h, &c.to_string(), bad))
            }
            ExprKind::Binary(BinOp::Div, a, b) if lit(b).is_some_and(|c| c > 0) => {
                let c = lit(b).unwrap();
                let (al, ah) = self.index_interval(a, q, qlo, qhi, scope, bad)?;
                (self.ins(format!("sdiv i64 {}, {}", al, c)), self.ins(format!("sdiv i64 {}, {}", ah, c)))
            }
            // the remainder has the sign of `a` and is smaller than both in size
            ExprKind::Binary(BinOp::Rem, a, b) if lit(b).is_some_and(|c| c > 0) => {
                let c = lit(b).unwrap();
                let (al, ah) = self.index_interval(a, q, qlo, qhi, scope, bad)?;
                let lpos = self.ins(format!("icmp sge i64 {}, 0", al));
                let lneg = self.ins(format!("call i64 @llvm.smax.i64(i64 {}, i64 {})", al, -(c - 1)));
                let lo = self.ins(format!("select i1 {}, i64 0, i64 {}", lpos, lneg));
                let hpos = self.ins(format!("icmp sge i64 {}, 0", ah));
                let hmin = self.ins(format!("call i64 @llvm.smin.i64(i64 {}, i64 {})", ah, c - 1));
                let hi = self.ins(format!("select i1 {}, i64 {}, i64 0", hpos, hmin));
                (lo, hi)
            }
            ExprKind::Unary(UnOp::Neg, a) => {
                let (al, ah) = self.index_interval(a, q, qlo, qhi, scope, bad)?;
                (self.checked("sub", "0", &ah, bad), self.checked("sub", "0", &al, bad))
            }
            ExprKind::If(_, a, b) => {
                let (al, ah) = self.index_interval(a, q, qlo, qhi, scope, bad)?;
                let (bl, bh) = self.index_interval(b, q, qlo, qhi, scope, bad)?;
                (
                    self.ins(format!("call i64 @llvm.smin.i64(i64 {}, i64 {})", al, bl)),
                    self.ins(format!("call i64 @llvm.smax.i64(i64 {}, i64 {})", ah, bh)),
                )
            }
            ExprKind::Convert(a) => self.index_interval(a, q, qlo, qhi, scope, bad)?,
            _ => return Err("codegen: an interval not checked by interval_ok".into()),
        })
    }

    /// The bounds of the indices of `x` read by the `map` `e` (`reads`, of
    /// `windowed_reads`) for its index in `qlo ..= qhi`: the smallest and
    /// the largest of the bounds of each read.
    #[allow(clippy::type_complexity)]
    fn windowed_bounds(&mut self, e: &Expr, reads: &[(&Expr, Vec<(VarId, &Expr)>)], qlo: &str, qhi: &str, bad: &str) -> R<(String, String)> {
        let ExprKind::Map(_, l) = &e.kind else { unreachable!() };
        let q = l.params[0];
        let (mut lo, mut hi) = (String::new(), String::new());
        for (i, scope) in reads {
            let (a, b) = self.index_interval(i, q, qlo, qhi, scope, bad)?;
            if lo.is_empty() {
                (lo, hi) = (a, b);
            } else {
                lo = self.ins(format!("call i64 @llvm.smin.i64(i64 {}, i64 {})", lo, a));
                hi = self.ins(format!("call i64 @llvm.smax.i64(i64 {}, i64 {})", hi, b));
            }
        }
        Ok((lo, hi))
    }

    /// The last block `j` of `places` (`c` blocks, `places[c]` after the
    /// last) with `places[j] <= t`, from `j >= from` (`places[from] <= t <
    /// places[c]`), by halves.
    fn place_search(&mut self, places: &str, from: &str, c: &str, t: &str) -> String {
        let (lo, hi) = (self.alloca("i64"), self.alloca("i64"));
        self.emit(format!("store i64 {}, ptr {}", from, lo));
        self.emit(format!("store i64 {}, ptr {}", c, hi));
        let (head, body, done) = (self.label(), self.label(), self.label());
        self.br(&head);
        self.block(&head);
        let a = self.ins(format!("load i64, ptr {}", lo));
        let b = self.ins(format!("load i64, ptr {}", hi));
        let gap = self.ins(format!("sub i64 {}, {}", b, a));
        let more = self.ins(format!("icmp sgt i64 {}, 1", gap));
        self.cbr(&more, &body, &done);
        self.block(&body);
        let half = self.ins(format!("lshr i64 {}, 1", gap));
        let mid = self.ins(format!("add i64 {}, {}", a, half));
        let slot = self.ins(format!("getelementptr inbounds i64, ptr {}, i64 {}", places, mid));
        let pm = self.ins(format!("load i64, ptr {}", slot));
        let below = self.ins(format!("icmp sle i64 {}, {}", pm, t));
        let a2 = self.ins(format!("select i1 {}, i64 {}, i64 {}", below, mid, a));
        let b2 = self.ins(format!("select i1 {}, i64 {}, i64 {}", below, b, mid));
        self.emit(format!("store i64 {}, ptr {}", a2, lo));
        self.emit(format!("store i64 {}, ptr {}", b2, hi));
        self.scalar_loops += 1;
        self.emit(format!("br label %{}, !llvm.loop !{}", head, self.scalar_loops + 1));
        self.block(&done);
        self.ins(format!("load i64, ptr {}", lo))
    }

    /// The part of the array `xw` of `windowed_let` (of the `filter` `f`)
    /// that holds its elements `lo ..= hi` (bounds of the reads, cut to the
    /// array): the blocks of `src` that hold them, from the block of `lo` to
    /// the one of `hi`, filtered again into `buf` (WINDOW_STACK bytes on the
    /// stack) or into a block of the heap when they need more; the array
    /// read through the result at the same indices, of the same length.
    /// Returns it and the block of the heap to free (or null).
    fn windowed_fill(&mut self, f: &Expr, xw: &V, lo: &str, hi: &str, buf: &str, origin: bool) -> R<(V, String)> {
        let ExprKind::Filter(input, l) = &f.kind else { unreachable!() };
        let et = input.ty.elem().clone();
        let size = esize(&et);
        let (src, n) = self.prepare_src(input)?;
        let w = self.filter_width(input, l, &et);
        let places = self.ins(format!("extractvalue {{ ptr, i64 }} {}, 0", xw.r));
        let k = self.ins(format!("extractvalue {{ ptr, i64 }} {}, 1", xw.r));
        let (o, c) = self.window_blocks(&places, &n, origin);
        let a = self.ins(format!("call i64 @llvm.smax.i64(i64 {}, i64 0)", lo));
        let km1 = self.ins(format!("sub i64 {}, 1", k));
        let b = self.ins(format!("call i64 @llvm.smin.i64(i64 {}, i64 {})", hi, km1));
        let out = self.alloca("{ ptr, i64 }");
        let heap = self.alloca("ptr");
        self.emit(format!("store ptr null, ptr {}", heap));
        // no index of the array: every read is out of bounds and fails first
        self.emit(format!("store {{ ptr, i64 }} {}, ptr {}", xw.r, out));
        let some = self.ins(format!("icmp sle i64 {}, {}", a, b));
        let (lfill, ldone) = (self.label(), self.label());
        self.cbr(&some, &lfill, &ldone);
        self.block(&lfill);
        let j0 = self.place_search(&places, "0", &c, &a);
        let jb = self.place_search(&places, &j0, &c, &b);
        let j1 = self.ins(format!("add i64 {}, 1", jb));
        let s0 = self.ins(format!("getelementptr inbounds i64, ptr {}, i64 {}", places, j0));
        let base = self.ins(format!("load i64, ptr {}", s0));
        let s1 = self.ins(format!("getelementptr inbounds i64, ptr {}, i64 {}", places, j1));
        let top = self.ins(format!("load i64, ptr {}", s1));
        let need = self.ins(format!("sub i64 {}, {}", top, base));
        let bytes = self.ins(format!("mul i64 {}, {}", need, size));
        let big = self.ins(format!("icmp sgt i64 {}, {}", bytes, WINDOW_STACK));
        let (lheap, lstack, lgo) = (self.label(), self.label(), self.label());
        self.cbr(&big, &lheap, &lstack);
        self.block(&lheap);
        let hp = self.ins(format!("call ptr @isc_alloc(i64 {})", bytes));
        self.emit(format!("store ptr {}, ptr {}", hp, heap));
        self.br(&lgo);
        self.block(&lstack);
        self.br(&lgo);
        self.block(&lgo);
        let dst = self.ins(format!("phi ptr [ {}, %{} ], [ {}, %{} ]", hp, lheap, buf, lstack));
        let pos = self.alloca("i64");
        self.for_range_scalar(&j0, &j1, 1, &mut |g, j| {
            let (start, end) = g.window_range(j, &o, &n);
            let sj = g.ins(format!("getelementptr inbounds i64, ptr {}, i64 {}", places, j));
            let pj = g.ins(format!("load i64, ptr {}", sj));
            let at = g.ins(format!("sub i64 {}, {}", pj, base));
            g.emit(format!("store i64 {}, ptr {}", at, pos));
            g.write_kept(&src, l, &start, &end, &dst, &need, &pos, &et, w)
        })?;
        // the element `i` of the array at `dst + (i - base)`
        let off = self.ins(format!("mul i64 {}, -{}", base, size));
        let p = self.ins(format!("getelementptr i8, ptr {}, i64 {}", dst, off));
        let r0 = self.ins(format!("insertvalue {{ ptr, i64 }} poison, ptr {}, 0", p));
        let r = self.ins(format!("insertvalue {{ ptr, i64 }} {}, i64 {}, 1", r0, k));
        self.emit(format!("store {{ ptr, i64 }} {}, ptr {}", r, out));
        self.br(&ldone);
        self.block(&ldone);
        let xv = self.ins(format!("load {{ ptr, i64 }}, ptr {}", out));
        let hp = self.ins(format!("load ptr, ptr {}", heap));
        Ok((V { r: xv, t: "{ ptr, i64 }".into(), uni: true }, hp))
    }

    /// The whole array `xw` of `windowed_let` (of the `filter` `f`): each
    /// block of its source written from its place, in parallel (here in a
    /// task); for the arrays written by blocks whose reads are not in
    /// windows (`print_stream`).
    fn windowed_whole(&mut self, f: &Expr, xw: &V, origin: bool) -> R<String> {
        let ExprKind::Filter(input, l) = &f.kind else { unreachable!() };
        let et = input.ty.elem().clone();
        let places = self.ins(format!("extractvalue {{ ptr, i64 }} {}, 0", xw.r));
        let k = self.ins(format!("extractvalue {{ ptr, i64 }} {}, 1", xw.r));
        let out = self.alloc_elems(&k, &et);
        let (_, n) = self.prepare_src(input)?;
        let (_, c) = self.window_blocks(&places, &n, origin);
        let block = |g: &mut Self, j: &str, places: &str, out: &str, k: &str| -> R<()> {
            let (src, n) = g.prepare_src(input)?;
            let w = g.filter_width(input, l, &et);
            let (o, _) = g.window_blocks(places, &n, origin);
            let (start, end) = g.window_range(j, &o, &n);
            let sj = g.ins(format!("getelementptr inbounds i64, ptr {}, i64 {}", places, j));
            let pj = g.ins(format!("load i64, ptr {}", sj));
            let pos = g.alloca("i64");
            g.emit(format!("store i64 {}, ptr {}", pj, pos));
            g.write_kept(&src, l, &start, &end, out, k, &pos, &et, w)
        };
        if self.in_task {
            self.for_range_scalar("0", &c, 1, &mut |g, j| block(g, j, &places, &out, &k))?;
        } else {
            let extra = [v(places.clone(), "ptr"), v(out.clone(), "ptr"), v(k.clone(), "i64")];
            let (task, ctx, ctx_bytes) = self.outline_task(&[f], &extra, &mut |g, xs, j| block(g, j, &xs[0].r, &xs[1].r, &xs[2].r))?;
            self.par_for(2, &c, &task, &ctx, &ctx_bytes);
        }
        Ok(out)
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
            self.par_for(0, &nb, &task, &ctx, &ctx_bytes);
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
            self.par_for(0, &chunks, &task, &ctx, &ctx_bytes);
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
        self.row_into_at(body, dest, et, expect, body.span)
    }

    /// `row_into` whose panic of a row of another length is at `span`: the
    /// place of the whole row, the same through its `let`s and the branches
    /// of its `if`s (D593: the place the interpreter gives).
    fn row_into_at(&mut self, body: &Expr, dest: Option<&str>, et: &Type, expect: Option<&str>, span: Span) -> R<String> {
        use ExprKind::*;
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
                let len = self.row_into_at(b, dest, et, expect, span)?;
                if *drop {
                    self.drop_var(*var, &iv);
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
            // a row chosen by a test: each branch written at its place
            // (D593)
            If(c, t, f) => {
                let cv = self.ex(c, 1)?;
                let lthen = self.label();
                let lelse = self.label();
                let ljoin = self.label();
                self.cbr(&cv.r, &lthen, &lelse);
                self.block(&lthen);
                let tn = self.row_into_at(t, dest, et, expect, span)?;
                let tend = self.current_label(&lthen);
                self.br(&ljoin);
                self.block(&lelse);
                let en = self.row_into_at(f, dest, et, expect, span)?;
                let eend = self.current_label(&lelse);
                self.br(&ljoin);
                self.block(&ljoin);
                Ok(self.ins(format!("phi i64 [ {}, %{} ], [ {}, %{} ]", tn, tend, en, eend)))
            }
            // (an array of constants, data of the program, borrowed: copied,
            // never freed, D173, D193; the bytes of a text read where it is,
            // D303; the copy `ownership` asked for of a borrowed row made
            // here, into the rows, D593)
            Var(..) | Index(..) | Slice(..) | ConstArray(_) | ConstStrs(_) | Copy(_) | StrOp(..)
                if !matches!(&body.kind, StrOp(..)) || isc_ir::passes::row_view(body) =>
            {
                let moved = matches!(&body.kind, Var(_, Use::Move));
                let read = if let Copy(x) = &body.kind { &**x } else { body };
                let xv = self.ex(read, 1)?;
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
        self.elem_replayed_lanes(src, idx, w, w)
    }

    /// `elem_replayed` whose replay computes only the first `lanes` elements
    /// (a row narrower than the vector, D274: the lanes after it are not
    /// elements of the row, the last row may end the array).
    fn elem_replayed_lanes(&mut self, src: &Src, idx: &str, w: u32, lanes: u32) -> R<V> {
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
        self.for_range_scalar("0", &lanes.to_string(), 1, &mut |g, j| {
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
        // the whole rows of 33 to 64 bytes each in one vector of 64 lanes,
        // the lanes after the row masked (D274): the work of a row that does
        // not depend on its places (a remainder, the checks) once, not twice
        let rows = if w > 1 { row_split(src, w) } else { None };
        // a table of bytes read in the row: permuted in registers for 64
        // lanes too (D388)
        let wide = matches!(&rows, Some((c, _, _)) if *c > w as i64 && *c <= 2 * w as i64)
            && w == 2 * MAP_WIDTH
            && matches!(et, Type::Int(k) if k.bits() == 8)
            && BYTE_MASKS.with(|b| b.get());
        let pw = if wide { 2 * w } else { w };
        // the tables read at `x % c`, copied with their period continued
        // (D273), once before the vector loops
        let periods = match src {
            Src::Map(_, l) if w > 1 => period_tables(l, pw),
            _ => Vec::new(),
        };
        for (t, c, elem) in &periods {
            let Some(tv) = self.env.get(t).cloned() else { continue };
            if tv.t != "{ ptr, i64 }" {
                continue;
            }
            let es = esize(elem);
            let tp = self.ins(format!("extractvalue {{ ptr, i64 }} {}, 0", tv.r));
            let tn = self.ins(format!("extractvalue {{ ptr, i64 }} {}, 1", tv.r));
            let buf = self.alloca(&format!("[{} x i8], align 64", (c + pw as i64 - 1) * es));
            let enough = self.ins(format!("icmp sge i64 {}, {}", tn, c));
            let (lcopy, lj) = (self.label(), self.label());
            self.cbr(&enough, &lcopy, &lj);
            self.block(&lcopy);
            self.emit(format!("call void @llvm.memcpy.p0.p0.i64(ptr {}, ptr {}, i64 {}, i1 false)", buf, tp, c * es));
            let after = self.ins(format!("getelementptr inbounds i8, ptr {}, i64 {}", buf, c * es));
            self.emit(format!("call void @llvm.memcpy.p0.p0.i64(ptr {}, ptr {}, i64 {}, i1 false)", after, tp, (pw as i64 - 1) * es));
            self.br(&lj);
            self.block(&lj);
            self.f.period_tables.insert((*t, *c), (buf, enough));
        }
        if let Some((c, divs, rems)) = rows {
            // by rows of the constant that divides the index (D214)
            self.fill_rows(src, p, n, et, start, end, w, c, &divs, &rems, wide)?;
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
        for (t, c, _) in &periods {
            self.f.period_tables.remove(&(*t, *c));
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
    /// The `RowHoist` of the whole rows `ff .. lf` of `fill_rows` (D318),
    /// its values computed here, before their loop.
    fn row_hoist(&mut self, src: &Src, c: i64, lanes: u32, ff: &str, lf: &str) -> RowHoist {
        let mut h = RowHoist::default();
        let Src::Map(srcs, l) = src else { return h };
        let [Src::Iota] = srcs.as_slice() else { return h };
        let q = l.params[0];
        fn lets_of<'e>(e: &'e Expr, out: &mut HashMap<VarId, &'e Expr>) {
            if let ExprKind::Let { var, init, .. } = &e.kind {
                out.insert(*var, init);
            }
            for ch in e.children() {
                lets_of(ch, out);
            }
        }
        let mut lets = HashMap::new();
        lets_of(&l.body, &mut lets);
        let env: HashSet<VarId> = self.env.keys().copied().filter(|v| !lets.contains_key(v) && *v != q).collect();
        let bound = |v: VarId| env.contains(&v);
        // the value of `r` before the loop: the literal and the variables
        let value = |g: &mut Self, r: &RowLin| -> Option<String> {
            let mut acc = r.c0.to_string();
            for &(k, v) in &r.vars {
                let x = g.env.get(&v)?.clone();
                if x.t != "i64" {
                    return None;
                }
                let m = g.ins(format!("mul i64 {}, {}", x.r, k));
                acc = g.ins(format!("add i64 {}, {}", acc, m));
            }
            Some(acc)
        };
        fn find<'e>(e: &'e Expr, out: &mut Vec<&'e Expr>) {
            if matches!(e.kind, ExprKind::Binary(BinOp::Eq, ..) | ExprKind::Index(..)) {
                out.push(e);
            }
            for ch in e.children() {
                find(ch, out);
            }
        }
        let mut sites = Vec::new();
        find(&l.body, &mut sites);
        let is_q = |x: &Expr| matches!(x.kind, ExprKind::Var(v, _) if v == q);
        for e in sites {
            match &e.kind {
                ExprKind::Binary(BinOp::Eq, a, b) if h.eq.is_none() && a.ty == Type::Int(IntKind::I64) => {
                    let other = if is_q(a) { b } else if is_q(b) { a } else { continue };
                    let Some(k) = row_lin(other, q, c, &lets, &bound) else { continue };
                    if k.row != 0 || k.place != 0 {
                        continue;
                    }
                    if let Some(kv) = value(self, &k) {
                        h.eq = Some((e as *const Expr, kv));
                    }
                }
                ExprKind::Index(t, i) => {
                    let (ExprKind::Var(t, _), ExprKind::Binary(BinOp::Rem, x, cc)) = (&t.kind, &i.kind) else { continue };
                    let ExprKind::Lit(Lit::Int(cc)) = cc.kind else { continue };
                    // read by one load from the remainder of its first lane
                    // (`index_vec`: a table copied with its period, D273, or
                    // read where it is when the lanes do not cross its end)
                    let _ = t;
                    if cc < lanes as i64 || h.rems.iter().any(|r| std::ptr::eq(r.0, &**i)) {
                        continue;
                    }
                    let Some(x) = row_lin(x, q, c, &lets, &bound) else { continue };
                    // consecutive lanes, rows that do not go back, a step
                    // whose products by the rows cannot wrap
                    if x.place != 1 || x.row < 0 || x.row >= 1 << 31 {
                        continue;
                    }
                    let k0 = RowLin { row: 0, place: 0, ..x.clone() };
                    let Some(kv) = value(self, &k0) else { continue };
                    let slot = self.alloca("i64");
                    // every lane at least 0 in every row from `ff`: the
                    // first lane of the row `ff` (the others after it, the
                    // rows below 2^31: no product wraps)
                    let x0a = self.ins(format!("mul i64 {}, {}", ff, x.row));
                    let x0 = self.ins(format!("add i64 {}, {}", x0a, kv));
                    let pos = self.ins(format!("icmp sge i64 {}, 0", x0));
                    let small = self.ins(format!("icmp slt i64 {}, {}", lf, 1i64 << 31));
                    let kpos = self.ins(format!("icmp slt i64 {}, {}", kv, 1i64 << 61));
                    let ok0 = self.ins(format!("and i1 {}, {}", pos, small));
                    let ok = self.ins(format!("and i1 {}, {}", ok0, kpos));
                    h.rems.push((&**i as *const Expr, slot, x.row, kv, cc, ok));
                }
                _ => {}
            }
        }
        h
    }

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
        wide: bool,
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
        // the rows wholly inside `start .. end` (all but the first and the
        // last, D272): their bounds the constants 0 and c, their vectors at
        // constant places (0, w, ..., then c - w), so that what depends on
        // the place in the row (`q % c == k`) folds; the rows cut by `start`
        // or `end` as before
        let sc = self.ins(format!("add i64 {}, {}", start, c - 1));
        let ff = self.ins(format!("sdiv i64 {}, {}", sc, c));
        let lf0 = self.ins(format!("sdiv i64 {}, {}", end, c));
        let lf = self.ins(format!("call i64 @llvm.smax.i64(i64 {}, i64 {})", lf0, ff));
        let mut places: Vec<i64> = (0..c / w as i64).map(|k| k * w as i64).collect();
        if c % w as i64 != 0 {
            places.push(c - w as i64);
        }
        // the whole rows in one vector each (D274): the remainders of the
        // tables read at `x % c` carried from a row to the next, and
        // `q == K` false outside the row of K, those rows apart (D318)
        let hoist = if wide { self.row_hoist(src, c, 2 * w, &ff, &lf) } else { RowHoist::default() };
        let (a, b) = match &hoist.eq {
            Some((_, kv)) => {
                let k_ok = self.ins(format!("icmp sge i64 {}, 0", kv));
                let rk0 = self.ins(format!("sdiv i64 {}, {}", kv, c));
                let rk = self.ins(format!("select i1 {}, i64 {}, i64 -1", k_ok, rk0));
                let ge = self.ins(format!("icmp sge i64 {}, {}", rk, ff));
                let lt = self.ins(format!("icmp slt i64 {}, {}", rk, lf));
                let inside = self.ins(format!("and i1 {}, {}", ge, lt));
                let rk1 = self.ins(format!("add i64 {}, 1", rk));
                (self.ins(format!("select i1 {}, i64 {}, i64 {}", inside, rk, lf)), self.ins(format!("select i1 {}, i64 {}, i64 {}", inside, rk1, lf)))
            }
            None => (lf.clone(), lf.clone()),
        };
        for (i, (from, to)) in [(ff.clone(), a.clone()), (a.clone(), b.clone()), (b.clone(), lf.clone())].into_iter().enumerate() {
            if hoist.eq.is_none() && i > 0 {
                break;
            }
            let lane_false = i != 1;
            if let (true, Some((e, _))) = (lane_false, &hoist.eq) {
                self.pre.insert(*e, V { r: "false".into(), t: "i1".into(), uni: true });
            }
            // the remainders of the first row of this range
            for (e, slot, step0, kv, cc, _) in &hoist.rems {
                let x0a = self.ins(format!("mul i64 {}, {}", from, step0));
                let x0 = self.ins(format!("add i64 {}, {}", x0a, kv));
                let r0 = self.ins(format!("srem i64 {}, {}", x0, cc));
                self.emit(format!("store i64 {}, ptr {}", r0, slot));
                let _ = e;
            }
            for (e, slot, _, _, _, ok) in &hoist.rems {
                self.f.carried.insert(*e, (slot.clone(), ok.clone()));
            }
            let rems_c = hoist.rems.clone();
            self.for_range(&from, &to, 1, &mut |g, row| {
                let bq = g.ins(format!("mul i64 {}, {}", row, c));
                let rowv = V { r: row.to_string(), t: "i64".into(), uni: true };
                if wide {
                    g.row_vector_wide(src, p, 2 * w, c, &rowv, &bq, divs, rems)?;
                    // the remainders of the next row: one step, then less
                    // the divisor where it reaches it
                    for (_, slot, step0, _, cc, _) in &rems_c {
                        let r = g.ins(format!("load i64, ptr {}", slot));
                        let r1 = g.ins(format!("add i64 {}, {}", r, step0.rem_euclid(*cc)));
                        let wrap = g.ins(format!("icmp sge i64 {}, {}", r1, cc));
                        let r2 = g.ins(format!("sub i64 {}, {}", r1, cc));
                        let r3 = g.ins(format!("select i1 {}, i64 {}, i64 {}", wrap, r2, r1));
                        g.emit(format!("store i64 {}, ptr {}", r3, slot));
                    }
                    return Ok(());
                }
                for k in &places {
                    g.row_vector(src, p, n, et, w, &rowv, &bq, &k.to_string(), divs, rems)?;
                }
                Ok(())
            })?;
            for (e, ..) in &hoist.rems {
                self.f.carried.remove(e);
            }
            if let Some((e, _)) = &hoist.eq {
                self.pre.remove(e);
            }
        }
        self.row_range(src, p, n, et, start, end, w, c, divs, rems, &rlo, &ff)?;
        let lf1 = self.ins(format!("call i64 @llvm.smax.i64(i64 {}, i64 {})", lf, rlo));
        self.row_range(src, p, n, et, start, end, w, c, divs, rems, &lf1, &rend)?;
        self.br(&ldone);
        self.block(&ldone);
        Ok(())
    }

    /// The rows `from .. to` of `fill_rows`, each cut to `start .. end`.
    #[allow(clippy::too_many_arguments)]
    fn row_range(
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
        from: &str,
        to: &str,
    ) -> R<()> {
        self.for_range(from, to, 1, &mut |g, row| {
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
        })
    }

    /// A whole row of `c` bytes (`w / 2 < c <= w`) of `fill_rows` in one
    /// vector of `w` lanes (D274): the lanes from `c` on masked (their
    /// checks and their gathers skipped, the replay of a failed check
    /// computing the `c` elements only), the row stored under the mask.
    #[allow(clippy::too_many_arguments)]
    fn row_vector_wide(&mut self, src: &Src, p: &str, w: u32, c: i64, row: &V, bq: &str, divs: &[*const Expr], rems: &[*const Expr]) -> R<()> {
        let places = self.elem(&Src::Iota, "0", w)?;
        for d in divs {
            self.pre.insert(*d, row.clone());
        }
        for m in rems {
            self.pre.insert(*m, places.clone());
        }
        let lanes: Vec<String> = (0..w as i64).map(|j| format!("i1 {}", if j < c { "true" } else { "false" })).collect();
        let mask = format!("<{}>", lanes.join(", "));
        let saved = self.mask.replace(mask.clone());
        let x = self.elem_replayed_lanes(src, bq, w, c as u32);
        self.mask = saved;
        for e in divs.iter().chain(rems) {
            self.pre.remove(e);
        }
        let x = self.widen(&x?, w);
        let name = format!("llvm.masked.store.v{}i8.p0", w);
        self.decls.insert(format!("declare void @{}(<{} x i8>, ptr, i32 immarg, <{} x i1>)", name, w, w));
        let g = self.ins(format!("getelementptr inbounds i8, ptr {}, i64 {}", p, bq));
        self.emit(format!("call void @{}(<{} x i8> {}, ptr {}, i32 1, <{} x i1> {})", name, w, x.r, g, w, mask));
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
        // the names of registers of this function alone (D518)
        let saved_parts = std::mem::take(&mut self.part_values);
        let saved_vec_fail = self.vec_fail.take();
        let saved_task = std::mem::replace(&mut self.in_task, true);
        // the texts on the stack of the caller, known by the names of its
        // registers, which the task numbers again from 0: not the task's
        // (D442: a text of the task under the same name took the caller's
        // out, its free without the test of its buffer)
        let saved_stack_strs = std::mem::take(&mut self.stack_strs);
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
        self.part_values = saved_parts;
        self.vec_fail = saved_vec_fail;
        self.in_task = saved_task;
        self.stack_strs = saved_stack_strs;
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
        // a count of bytes by the masks of its condition (D396)
        if w > 1 {
            if let Some((ins, pred, cw)) = count_parts(src, l, z, t) {
                let end = self.ins(format!("add i64 {}, {}", start, m));
                let k = self.count_kept(ins, &pred, start, &end, cw)?;
                return Ok(V { r: k, t: "i64".into(), uni: true });
            }
        }
        self.reduce_block_sum(src, start, m, l, z, t, w)
    }

    #[allow(clippy::too_many_arguments)]
    fn reduce_block_sum(&mut self, src: &Src, start: &str, m: &str, l: &Lambda, z: &V, t: &Type, w: u32) -> R<V> {
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
        // scalar version of an associative and commutative operator on
        // integers (`+`, `*`, `min`, `max`, modular) from its neutral value:
        // every order of the elements gives the value of the fixed form, so
        // one accumulator, a register (D304: the V accumulators in memory
        // cost a load and a store for each element: `sum` of the lengths of
        // slices 2.17 ns an element against 1.32 in C)
        if int_scan_op(l, t).is_some_and(|(_, neutral)| z.r.parse::<i128>().ok() == Some(neutral)) {
            let acc = self.alloca(&st);
            self.emit(format!("store {} {}, ptr {}", st, z.r, acc));
            // not vectorized by LLVM: an element of this loop is not one of
            // a vector (its strings behind pointers: `count` of a
            // `startsWith` gathered 4.09 ns an element against 1.95)
            self.for_range_scalar("0", m, 1, &mut |g, i| {
                let idx = g.ins(format!("add i64 {}, {}", start, i));
                let x = g.elem(src, &idx, 1)?;
                let a = g.ins(format!("load {}, ptr {}", st, acc));
                let r = g.apply2(l, V { r: a, t: st.clone(), uni: false }, x, 1)?;
                g.emit(format!("store {} {}, ptr {}", st, r.r, acc));
                Ok(())
            })?;
            return Ok(v(self.ins(format!("load {}, ptr {}", st, acc)), st));
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
            // a count of fewer elements than a vector: in one masked vector
            // when the target has them (D437), not one element at a time
            // (D443: the count of `json.quote` over a name of 16 bytes)
            let counted = match count_parts(&src, l, &zv, &t) {
                // fewer than 32 elements: a vector of 32 lanes (D452: one of
                // 64 cost `json.message` 5 %)
                Some((ins, pred, cw)) if w > 1 && cw > 1 && BYTE_MASKS.with(|b| b.get()) => Some(self.count_kept(ins, &pred, "0", &n, cw.min(32))?),
                _ => None,
            };
            let r = match counted {
                Some(k) => V { r: k, t: "i64".into(), uni: true },
                None => self.reduce_loop(&src, &n, l, &zv, &t)?,
            };
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
            self.par_for(0, &c0, &task, &ctx, &ctx_bytes);
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
        self.par_for(1, &c, &task, &ctx, &ctx_bytes);
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
        // (the interpreter runs it; no code for its rows yet, D593)
        if elem_ty.is_array() {
            return Err(format!("{}:{}: a `scan` whose elements are arrays is not supported in the POC (SPEC section 15)", e.span.line, e.span.col));
        }
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
            let (vec, replay) = scan_vec(x);
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
            self.scan_range_fast(&src, vec, replay, l, &zv, &out, &n, "0", &n)?;
            self.br(&ljoin);
            self.block(&ljoin);
        } else {
            let (vec, replay) = scan_vec(x);
            self.scan_range_fast(&src, vec, replay, l, &zv, &out, &n, "0", &n)?;
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
    /// `mapParallel(x, f)` (D373): a task of the pool for each element of
    /// `x`, the action of the lambda applied to it and its result written at
    /// its place; `isc_parallel_actions` runs them at the same time on the
    /// threads of the pool (never on the calling thread alone). Over an
    /// array of arrays, each element is a row, a view of its block; results
    /// that are arrays (D562) are kept apart by their tasks (each its own
    /// block), then, all of one length (else the panic of a size, as a `map`
    /// whose rows differ), moved into the one block of an array of arrays,
    /// row after row, their blocks freed.
    fn tasks(&mut self, e: &Expr, x: &Expr, l: &Lambda) -> R<V> {
        let et = e.ty.elem().clone();
        let xt = x.ty.elem().clone();
        let leaf = |t: &Type| t.is_scalar() || matches!(t, Type::Str | Type::Big);
        if (et.is_array() && !leaf(et.elem())) || (xt.is_array() && !xt.elem().is_scalar() && !matches!(xt.elem(), Type::Str | Type::Big)) {
            return Err(format!("{}:{}: `mapParallel` over arrays of arrays of arrays, or of them, is not supported", e.span.line, e.span.col));
        }
        let rows_in = xt.is_array();
        let rows_out = et.is_array();
        let xv = self.ex(x, 1)?;
        let n = if rows_in { self.nested_parts(&xv).1 } else { self.ins(format!("extractvalue {{ ptr, i64 }} {}, 1", xv.r)) };
        // the results, or the arrays the tasks make (16 bytes each)
        let p = if rows_out {
            let bytes = self.ins(format!("mul i64 {}, 16", n));
            self.ins(format!("call ptr @isc_alloc(i64 {})", bytes))
        } else {
            self.alloc_elems(&n, &et)
        };
        let (task, ctx, ctx_bytes) = self.outline_task(&[e], &[v(p.clone(), "ptr")], &mut |g, extra, i| {
            let xv = g.ex(x, 1)?;
            let el = if rows_in {
                let (xp, rows, cols, stride) = g.nested_parts(&xv);
                let _ = rows;
                g.row_view(&xp, &cols, &stride, i, xt.elem())
            } else {
                let xp = g.ins(format!("extractvalue {{ ptr, i64 }} {}, 0", xv.r));
                let xn = g.ins(format!("extractvalue {{ ptr, i64 }} {}, 1", xv.r));
                g.load_elem(&xp, &xn, i, &xt, 1)
            };
            g.bind(l.params[0], el);
            let r = g.ex(&l.body, 1)?;
            if rows_out {
                let at = g.ins(format!("getelementptr inbounds {{ ptr, i64 }}, ptr {}, i64 {}", extra[0].r, i));
                g.emit(format!("store {{ ptr, i64 }} {}, ptr {}", r.r, at));
            } else {
                let xn = if rows_in { g.nested_parts(&xv).1 } else { g.ins(format!("extractvalue {{ ptr, i64 }} {}, 1", xv.r)) };
                g.store_elem(&extra[0].r, &xn, i, &et, &r, 1);
            }
            Ok(())
        })?;
        self.emit(format!("call void @isc_parallel_actions(i64 {}, ptr {}, ptr {}, i64 {})", n, task, ctx, ctx_bytes));
        if !rows_out {
            return Ok(self.arr(&p, &n));
        }
        // the rows: their length that of the first (none: 0), each checked,
        // their elements moved into one block, their blocks freed
        let inner = et.elem().clone();
        let mslot = self.alloca("i64");
        self.emit(format!("store i64 0, ptr {}", mslot));
        let some = self.ins(format!("icmp sgt i64 {}, 0", n));
        let (lfirst, ljoin) = (self.label(), self.label());
        self.cbr(&some, &lfirst, &ljoin);
        self.block(&lfirst);
        let r0 = self.ins(format!("load {{ ptr, i64 }}, ptr {}", p));
        let m0 = self.ins(format!("extractvalue {{ ptr, i64 }} {}, 1", r0));
        self.emit(format!("store i64 {}, ptr {}", m0, mslot));
        self.br(&ljoin);
        self.block(&ljoin);
        let m = self.ins(format!("load i64, ptr {}", mslot));
        let total = self.ins(format!("mul i64 {}, {}", n, m));
        let block = self.alloc_elems(&total, &inner);
        let esz = esize(&inner);
        let span = l.body.span;
        let (pp, bb, mm) = (p.clone(), block.clone(), m.clone());
        self.for_range_scalar("0", &n, 1, &mut |g, i| {
            let at = g.ins(format!("getelementptr inbounds {{ ptr, i64 }}, ptr {}, i64 {}", pp, i));
            let r = g.ins(format!("load {{ ptr, i64 }}, ptr {}", at));
            let rp = g.ins(format!("extractvalue {{ ptr, i64 }} {}, 0", r));
            let rn = g.ins(format!("extractvalue {{ ptr, i64 }} {}, 1", r));
            let same = g.ins(format!("icmp eq i64 {}, {}", rn, mm));
            g.guard(&same, format!("call void @isc_panic_size(i64 {}, i64 {}, {})", rn, mm, Self::pos(span)));
            let off = g.ins(format!("mul i64 {}, {}", i, mm));
            let off = g.ins(format!("mul i64 {}, {}", off, esz));
            let dest = g.ins(format!("getelementptr inbounds i8, ptr {}, i64 {}", bb, off));
            let bytes = g.ins(format!("mul i64 {}, {}", mm, esz));
            g.emit(format!("call void @llvm.memcpy.p0.p0.i64(ptr {}, ptr {}, i64 {}, i1 false)", dest, rp, bytes));
            g.emit(format!("call void @isc_free(ptr {})", rp));
            Ok(())
        })?;
        self.emit(format!("call void @isc_free(ptr {})", p));
        Ok(self.nested(&block, &n, &m, &m))
    }

    fn filter(&mut self, e: &Expr, x: &Expr, l: &Lambda) -> R<V> {
        let elem_ty = x.ty.elem().clone();
        // (counted or read in order by its consumer, a `filter` of rows
        // makes no array: only the one kept as an array is refused, D593;
        // the interpreter runs it)
        if elem_ty.is_array() {
            return Err(format!("{}:{}: a `filter` of rows kept as an array is not supported in the POC (SPEC section 15)", e.span.line, e.span.col));
        }
        let (src, n) = self.prepare_src(x)?;
        let w = self.filter_width(x, l, &elem_ty);
        if self.in_task {
            return self.filter_range(&src, &n, l, &elem_ty, w);
        }
        let several = self.ins(format!("icmp sgt i64 {}, {}", n, BLOCK));
        // the pass by blocks also when it makes counts (D426): on one thread
        let counting = self.count_req.as_ref().is_some_and(|r| std::ptr::eq(r.0, e));
        let big = if counting {
            several
        } else {
            let threads = self.ins("call i64 @isc_thread_count()".to_string());
            let multi = self.ins(format!("icmp sgt i64 {}, 1", threads));
            self.ins(format!("and i1 {}, {}", several, multi))
        };
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
        // a predicate that can panic in its lanes (`a && b[i + 1] == c`, the
        // index of a branch) too: its mask computed again one element at a
        // time when a check fails (`kept_mask`, D362)
        if size_ok && src_vec_ok(x) && (lam_ok(l) || lam_ok_replay(l)) {
            // bytes: 32 per vector (a `ymm` register); wider: 16 lanes
            if esize(elem_ty) == 1 { 32 } else { 16 }
        } else {
            1
        }
    }

    /// The mask of the elements `i .. i + w` of `src` (their vector `el`)
    /// that `l` keeps, in vector code; a predicate that can panic in its
    /// lanes (D362) jumps on a failed check to the replay of the vector, the
    /// predicate of each element computed again in order, so that the panic
    /// is the one of the scalar loop (its element, its site) and a check of
    /// a lane the scalar code would not have made panics not (D185).
    fn kept_mask(&mut self, src: &Src, l: &Lambda, i: &str, w: u32, el: &V) -> R<V> {
        self.bind(l.params[0], el.clone());
        if lam_ok(l) {
            let keep = self.ex(&l.body, w)?;
            return Ok(self.widen(&keep, w));
        }
        let lfail = self.label();
        let lcont = self.label();
        let saved = std::mem::replace(&mut self.vec_fail, Some(lfail.clone()));
        let keep = self.ex(&l.body, w);
        self.vec_fail = saved;
        let keep = self.widen(&keep?, w);
        let lvec = self.current_label(&lfail);
        self.br(&lcont);
        self.block(&lfail);
        let kt = keep.t.clone();
        let slot = self.alloca(&kt);
        self.emit(format!("store {} zeroinitializer, ptr {}", kt, slot));
        self.for_range_scalar("0", &w.to_string(), 1, &mut |g, j| {
            let k = g.ins(format!("add i64 {}, {}", i, j));
            let x = g.elem(src, &k, 1)?;
            g.bind(l.params[0], x);
            let kv = g.ex(&l.body, 1)?;
            let cur = g.ins(format!("load {}, ptr {}", kt, slot));
            let nv = g.ins(format!("insertelement {} {}, i1 {}, i64 {}", kt, cur, kv.r, j));
            g.emit(format!("store {} {}, ptr {}", kt, nv, slot));
            Ok(())
        })?;
        let rv = self.ins(format!("load {}, ptr {}", kt, slot));
        let lrep = self.current_label(&lfail);
        self.br(&lcont);
        self.block(&lcont);
        let r = self.ins(format!("phi {} [ {}, %{} ], [ {}, %{} ]", kt, keep.r, lvec, rv, lrep));
        self.bind(l.params[0], el.clone());
        Ok(V { r, t: kt, uni: false })
    }

    /// The elements of `lo .. hi` of `src` that `l` keeps, counted: `w` at
    /// a time (the bits of the mask counted, D195), then one at a time.
    fn count_kept(&mut self, src: &Src, l: &Lambda, lo: &str, hi: &str, w: u32) -> R<String> {
        let cnt = self.alloca("i64");
        self.emit(format!("store i64 0, ptr {}", cnt));
        let mut lo = lo.to_string();
        // vectors of 64 bytes read from a boundary of 64 bytes, the bytes
        // before it in one masked vector, over 1024 bytes or more (D452: an
        // unaligned load of 64 bytes crosses two lines of cache, the count
        // of the bases of revcomp +1.3 % of its cycles; on shorter texts in
        // the cache the test costs more than it saves, `json.quote` +2.4 %
        // from 128 bytes)
        if let Src::Mat { ptr, elem, .. } = src {
            if w == 64 && matches!(elem, Type::Int(_)) && esize(elem) == 1 && masked_safe(&l.body) && BYTE_MASKS.with(|b| b.get()) {
                let (ptr, elem) = (ptr.clone(), elem.clone());
                let span = self.ins(format!("sub i64 {}, {}", hi, lo));
                let long = self.ins(format!("icmp sge i64 {}, 1024", span));
                let start = self.alloca("i64");
                self.emit(format!("store i64 {}, ptr {}", lo, start));
                let (llong, lhead, ldone) = (self.label(), self.label(), self.label());
                self.cbr(&long, &llong, &ldone);
                self.block(&llong);
                let at = self.ins(format!("getelementptr inbounds i8, ptr {}, i64 {}", ptr, lo));
                let addr = self.ins(format!("ptrtoint ptr {} to i64", at));
                let neg = self.ins(format!("sub i64 0, {}", addr));
                let head = self.ins(format!("and i64 {}, 63", neg));
                let some = self.ins(format!("icmp sgt i64 {}, 0", head));
                self.cbr(&some, &lhead, &ldone);
                self.block(&lhead);
                self.count_masked(src, l, &ptr, &elem, &lo, &head, w, &cnt)?;
                let moved = self.ins(format!("add i64 {}, {}", lo, head));
                self.emit(format!("store i64 {}, ptr {}", moved, start));
                self.br(&ldone);
                self.block(&ldone);
                lo = self.ins(format!("load i64, ptr {}", start));
            }
        }
        let lo = lo.as_str();
        let mut from = lo.to_string();
        if w > 1 {
            let span = self.ins(format!("sub i64 {}, {}", hi, lo));
            let full = self.ins(format!("and i64 {}, -{}", span, w));
            let vend = self.ins(format!("add i64 {}, {}", lo, full));
            self.decls.insert(format!("declare i{w} @llvm.ctpop.i{w}(i{w})", w = w));
            self.for_range(lo, &vend, w as i64, &mut |g, i| {
                let el = g.elem(src, i, w)?;
                let keep = g.kept_mask(src, l, i, w, &el)?;
                let bits = g.ins(format!("bitcast <{} x i1> {} to i{}", w, keep.r, w));
                let k = g.ins(format!("call i{w} @llvm.ctpop.i{w}(i{w} {b})", w = w, b = bits));
                let k64 = if w == 64 { k } else { g.ins(format!("zext i{} {} to i64", w, k)) };
                let c = g.ins(format!("load i64, ptr {}", cnt));
                let c2 = g.ins(format!("add i64 {}, {}", c, k64));
                g.emit(format!("store i64 {}, ptr {}", c2, cnt));
                Ok(())
            })?;
            from = vend;
            // the rest in one vector, its lanes past `hi` masked, when the
            // elements are read from memory (a masked load reads nothing
            // there) and the predicate cannot fail in a lane (D437: a text
            // shorter than a vector was counted one byte at a time)
            // (AVX-512 BW and VL: a masked load of bytes is one instruction,
            // AVX2 has none for bytes)
            if let Src::Mat { ptr, elem, .. } = src {
                if matches!(elem, Type::Int(_)) && masked_safe(&l.body) && BYTE_MASKS.with(|b| b.get()) {
                    let (ptr, elem) = (ptr.clone(), elem.clone());
                    let rest = self.ins(format!("sub i64 {}, {}", hi, from));
                    let some = self.ins(format!("icmp sgt i64 {}, 0", rest));
                    let (lrest, ldone) = (self.label(), self.label());
                    self.cbr(&some, &lrest, &ldone);
                    self.block(&lrest);
                    self.count_masked(src, l, &ptr, &elem, &from, &rest, w, &cnt)?;
                    self.br(&ldone);
                    self.block(&ldone);
                    return Ok(self.ins(format!("load i64, ptr {}", cnt)));
                }
            }
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

    /// The elements `from .. from + len` (fewer than `w`) of `src`, in
    /// memory at `ptr`, that `l` keeps, counted in one vector whose lanes
    /// from `len` are masked (a masked load reads nothing there, D437) and
    /// added to the count in `cnt`.
    #[allow(clippy::too_many_arguments)]
    fn count_masked(&mut self, src: &Src, l: &Lambda, ptr: &str, elem: &Type, from: &str, len: &str, w: u32, cnt: &str) -> R<()> {
        let m = mt(elem);
        let vm = format!("<{} x {}>", w, m);
        // the lanes below `len`: the low bits of a word, not a compare of
        // `w` indices (D452)
        let one = self.ins(format!("shl i64 1, {}", len));
        let low = self.ins(format!("sub i64 {}, 1", one));
        let low = if w < 64 { self.ins(format!("trunc i64 {} to i{}", low, w)) } else { low };
        let lanes = self.ins(format!("bitcast i{} {} to <{} x i1>", w, low, w));
        let at = self.ins(format!("getelementptr inbounds {}, ptr {}, i64 {}", m, ptr, from));
        let name = format!("llvm.masked.load.v{}{}.p0", w, m);
        self.decls.insert(format!("declare {} @{}(ptr, i32, <{} x i1>, {})", vm, name, w, vm));
        let x = self.ins(format!("call {} @{}(ptr {}, i32 {}, <{} x i1> {}, {} zeroinitializer)", vm, name, at, esize(elem), w, lanes, vm));
        let keep = self.kept_mask(src, l, from, w, &V { r: x, t: vm, uni: false })?;
        let kept = self.ins(format!("and <{} x i1> {}, {}", w, keep.r, lanes));
        let bits = self.ins(format!("bitcast <{} x i1> {} to i{}", w, kept, w));
        self.decls.insert(format!("declare i{w} @llvm.ctpop.i{w}(i{w})", w = w));
        let k = self.ins(format!("call i{w} @llvm.ctpop.i{w}(i{w} {b})", w = w, b = bits));
        let k64 = if w == 64 { k } else { self.ins(format!("zext i{} {} to i64", w, k)) };
        let c = self.ins(format!("load i64, ptr {}", cnt));
        let c2 = self.ins(format!("add i64 {}, {}", c, k64));
        self.emit(format!("store i64 {}, ptr {}", c2, cnt));
        Ok(())
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
                let keep = g.kept_mask(src, l, i, w, &el)?;
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
        // 1. the elements each block keeps; and the counts of a later
        // `filter` over the array this one reads (D426): by blocks of
        // WINDOW_BLOCK, while the block is in the caches
        let req = match &self.count_req {
            Some((f, a, pl, block, base)) if std::ptr::eq(*f, e) && !self.in_task => Some((*a, *pl, block.clone(), base.clone())),
            _ => None,
        };
        let mut extra = vec![v(counts.clone(), "ptr")];
        if let Some((_, _, block, _)) = &req {
            extra.push(v(block.clone(), "ptr"));
        }
        let (task, ctx, ctx_bytes) = self.outline_task(&[e], &extra, &mut |g, extra, j| {
            let (src, n) = g.prepare_src(x)?;
            let (start, end) = range(g, &n, j);
            let c = g.count_kept(&src, l, &start, &end, w)?;
            let slot = g.ins(format!("getelementptr inbounds i64, ptr {}, i64 {}", extra[0].r, j));
            g.emit(format!("store i64 {}, ptr {}", c, slot));
            if let Some((a, pl, _, _)) = &req {
                let ae = Expr { kind: ExprKind::Var(*a, Use::Borrow), ty: g.var_ty(*a).clone(), span: e.span, reassoc: false };
                let (asrc, _) = g.prepare_src(&ae)?;
                let et = ae.ty.elem().clone();
                // bytes counted 64 at a time: one vector of AVX-512
                let wp = match g.filter_width(&ae, pl, &et) {
                    32 => 64,
                    w => w,
                };
                let b0 = g.ins(format!("sdiv i64 {}, {}", start, WINDOW_BLOCK));
                let e0 = g.ins(format!("add i64 {}, {}", end, WINDOW_BLOCK - 1));
                let b1 = g.ins(format!("sdiv i64 {}, {}", e0, WINDOW_BLOCK));
                let to = extra[1].r.clone();
                g.for_range_scalar(&b0, &b1, 1, &mut |g, b| {
                    let lo = g.ins(format!("mul i64 {}, {}", b, WINDOW_BLOCK));
                    let hi = g.ins(format!("add i64 {}, {}", lo, WINDOW_BLOCK));
                    let hi = g.ins(format!("call i64 @llvm.smin.i64(i64 {}, i64 {})", hi, end));
                    let k = g.count_kept(&asrc, pl, &lo, &hi, wp)?;
                    let at = g.ins(format!("add i64 {}, 2", b));
                    let slot = g.ins(format!("getelementptr inbounds i64, ptr {}, i64 {}", to, at));
                    g.emit(format!("store i64 {}, ptr {}", k, slot));
                    Ok(())
                })?;
            }
            Ok(())
        })?;
        self.par_for(0, &c0, &task, &ctx, &ctx_bytes);
        // the counts made: the array they are of
        if let Some((_, _, block, base)) = &req {
            self.decls.insert("declare i64 @llvm.smin.i64(i64, i64)".into());
            self.emit(format!("store ptr {}, ptr {}", base, block));
        }
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
            // the place of the next block (`k` after the last one): a block
            // that keeps nothing is not read again (D255: the headers of a
            // file, a few blocks out of thousands)
            let last = g.ins(format!("icmp sge i64 {}, {}", end, n));
            let j1 = g.ins(format!("add i64 {}, 1", j));
            let jn = g.ins(format!("select i1 {}, i64 {}, i64 {}", last, j, j1));
            let nslot = g.ins(format!("getelementptr inbounds i64, ptr {}, i64 {}", extra[0].r, jn));
            let nx = g.ins(format!("load i64, ptr {}", nslot));
            let next = g.ins(format!("select i1 {}, i64 {}, i64 {}", last, extra[2].r, nx));
            let none = g.ins(format!("icmp eq i64 {}, {}", next, first));
            let lwork = g.label();
            let ldone = g.label();
            g.cbr(&none, &ldone, &lwork);
            g.block(&lwork);
            let pos = g.alloca("i64");
            g.emit(format!("store i64 {}, ptr {}", first, pos));
            let (out, k) = (extra[1].r.clone(), extra[2].r.clone());
            g.write_kept(&src, l, &start, &end, &out, &k, &pos, &elem_ty, w)?;
            g.br(&ldone);
            g.block(&ldone);
            Ok(())
        })?;
        // each thread its range of the blocks (D254)
        self.par_for(2, &c0, &task, &ctx, &ctx_bytes);
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
        self.par_for(0, &c0, &task, &ctx, &ctx_bytes);
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
            let (vec, replay) = scan_vec(x);
            g.scan_range_fast(&src, vec, replay, l, &before, &extra[2].r, &extra[3].r, &start, &end)?;
            Ok(())
        })?;
        self.par_for(0, &c0, &task, &ctx, &ctx_bytes);
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
    #[allow(clippy::too_many_arguments)]
    fn scan_range_fast(&mut self, src: &Src, vec: bool, replay: bool, l: &Lambda, z: &V, out: &str, len: &str, lo: &str, hi: &str) -> R<V> {
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
            let mut x = if replay { g.elem_replayed(src, i, w)? } else { g.elem(src, i, w)? };
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
        self.par_for(0, &c0, &task, &ctx, &ctx_bytes);
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
        self.par_for(0, &c0, &task, &ctx, &ctx_bytes);
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
        // the reads `a[p + j]` of its index `j` over an `iota` checked at
        // `j = 0` and `j = n - 1` before the loop (D296): `p + j` grows with
        // `j`, so that every index of the loop is between those in bounds,
        // and the loop reads them without a check; when one is not, the loop
        // as written (its checks, its panic)
        let reads = if matches!(x.kind, ExprKind::Iota(_)) && self.mask.is_none() { self.fold_index_reads(l) } else { Vec::new() };
        if reads.is_empty() {
            self.fold_vec_loop(&src, &n, &acc, &at, l, w)?;
        } else {
            let ok = self.fold_precheck(&reads, l.params[1], &n, w)?;
            let (lfast, lslow, ljoin) = (self.label(), self.label(), self.label());
            self.cbr(&ok, &lfast, &lslow);
            self.block(&lfast);
            for (_, i) in &reads {
                self.prechecked.insert(*i as *const Expr);
            }
            let fast = self.fold_vec_loop(&src, &n, &acc, &at, l, w);
            for (_, i) in &reads {
                self.prechecked.remove(&(*i as *const Expr));
            }
            fast?;
            self.br(&ljoin);
            self.block(&lslow);
            self.fold_vec_loop(&src, &n, &acc, &at, l, w)?;
            self.br(&ljoin);
            self.block(&ljoin);
        }
        Ok(V { r: self.ins(format!("load {}, ptr {}", at, acc)), t: at, uni: false })
    }

    /// The loop of `fold_vec`.
    fn fold_vec_loop(&mut self, src: &Src, n: &str, acc: &str, at: &str, l: &Lambda, w: u32) -> R<()> {
        self.for_range_scalar("0", n, 1, &mut |g, i| {
            let a = V { r: g.ins(format!("load {}, ptr {}", at, acc)), t: at.to_string(), uni: false };
            let el = g.elem(src, i, 1)?;
            g.bind(l.params[0], a);
            g.bind(l.params[1], el);
            let r = g.ex(&l.body, w)?;
            let r = g.widen(&r, w);
            g.emit(format!("store {} {}, ptr {}", at, r.r, acc));
            Ok(())
        })
    }

    /// The reads `a[p + j]`, `a[j + p]` or `a[j]` of the function `l` of a
    /// `fold` over an `iota` (its index `j`), as (`a`, the index): `a` a
    /// variable bound before the `fold`, `p` an `i64` of variables bound
    /// before it (neither `j` nor the accumulator nor a variable of `l`).
    fn fold_index_reads<'a>(&self, l: &'a Lambda) -> Vec<(&'a Expr, &'a Expr)> {
        fn binders(e: &Expr, out: &mut HashSet<VarId>) {
            match &e.kind {
                ExprKind::Let { var, .. } => {
                    out.insert(*var);
                }
                ExprKind::Map(_, f) | ExprKind::Reduce(_, f, _) | ExprKind::Fold(_, _, f) | ExprKind::Scan(_, f, _) | ExprKind::Filter(_, f) | ExprKind::Tasks(_, f) => out.extend(f.params.iter().copied()),
                _ => {}
            }
            for c in e.children() {
                binders(c, out);
            }
        }
        let (c, j) = (l.params[0], l.params[1]);
        let mut inner = HashSet::from([c, j]);
        binders(&l.body, &mut inner);
        let outer = |e: &Expr| -> bool {
            let (mut bound, mut free) = (HashSet::new(), Vec::new());
            free_vars(e, &mut bound, &mut free);
            free.iter().all(|v| !inner.contains(v) && self.env.contains_key(v))
        };
        let is_j = |e: &Expr| matches!(e.kind, ExprKind::Var(v, _) if v == j);
        let i64t = Type::Int(IntKind::I64);
        let mut out = Vec::new();
        fn walk<'a>(e: &'a Expr, f: &mut dyn FnMut(&'a Expr), stop: &dyn Fn(&Expr) -> bool) {
            f(e);
            if stop(e) {
                return;
            }
            for c in e.children() {
                walk(c, f, stop);
            }
        }
        // not inside another function (its own loop, its own index)
        let nested = |e: &Expr| matches!(e.kind, ExprKind::Map(..) | ExprKind::Reduce(..) | ExprKind::Fold(..) | ExprKind::Scan(..) | ExprKind::Filter(..) | ExprKind::Tasks(..));
        walk(
            &l.body,
            &mut |e: &'a Expr| {
                if let ExprKind::Index(a, i) = &e.kind {
                    let affine = is_j(i)
                        || matches!(&i.kind, ExprKind::Binary(BinOp::Add, u, v) if (is_j(u) && outer(v)) || (is_j(v) && outer(u)));
                    if affine && i.ty == i64t && matches!(a.kind, ExprKind::Var(..)) && outer(a) && !out.iter().any(|(_, k): &(&Expr, &Expr)| std::ptr::eq(*k, &**i)) {
                        out.push((&**a, &**i));
                    }
                }
            },
            &nested,
        );
        out
    }

    /// `fold_index_reads` in bounds at `j = 0` and `j = n - 1`, every lane
    /// (an `i1`).
    fn fold_precheck(&mut self, reads: &[(&Expr, &Expr)], j: VarId, n: &str, w: u32) -> R<String> {
        let last = self.ins(format!("sub i64 {}, 1", n));
        let mut ok = "true".to_string();
        for (a, i) in reads {
            let (_, len) = self.prepare_src(a)?;
            for at in ["0".to_string(), last.clone()] {
                self.bind(j, v(at, "i64"));
                let x = self.ex(i, w)?;
                let fine = if uni(&x) {
                    self.ins(format!("icmp ult i64 {}, {}", x.r, len))
                } else {
                    let nv = self.widen(&v(len.clone(), "i64"), w);
                    let lanes = self.ins(format!("icmp ult <{} x i64> {}, {}", w, x.r, nv.r));
                    let name = format!("llvm.vector.reduce.and.v{}i1", w);
                    self.decls.insert(format!("declare i1 @{}(<{} x i1>)", name, w));
                    self.ins(format!("call i1 @{}(<{} x i1> {})", name, w, lanes))
                };
                ok = self.ins(format!("and i1 {}, {}", ok, fine));
            }
        }
        Ok(ok)
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

    /// A `fold` of rule L (`last_where`, D346): its reads `x[i + p]` checked
    /// at `i = 0` and `i = n - 1` (`fold_precheck`), then `c` from `i = n - 1`
    /// down, the first `i` where it holds the result, `init` when none does;
    /// the other cases (a read out of bounds, `n = 0`) by the loop as written
    /// (its checks, its panic). `c` evaluated for fewer `i` gives the same
    /// result: it reads neither the accumulator nor anything that can stop
    /// the program.
    fn fold_last(&mut self, x: &Expr, init: &Expr, l: &Lambda, c: &Expr) -> R<V> {
        let (src, n) = self.prepare_src(x)?;
        let iv = self.ex(init, 1)?;
        let reads = self.fold_index_reads(l);
        let mut indexes = 0;
        fn count(e: &Expr, k: &mut usize) {
            if matches!(e.kind, ExprKind::Index(..)) {
                *k += 1;
            }
            for c in e.children() {
                count(c, k);
            }
        }
        count(c, &mut indexes);
        if indexes != reads.len() {
            return self.fold_loop(&src, &n, &iv, &init.ty, l);
        }
        let acc = self.alloca("i64");
        self.emit(format!("store i64 {}, ptr {}", iv.r, acc));
        let some = self.ins(format!("icmp sgt i64 {}, 0", n));
        let ok = if reads.is_empty() {
            some
        } else {
            let fine = self.fold_precheck(&reads, l.params[1], &n, 1)?;
            self.ins(format!("and i1 {}, {}", some, fine))
        };
        let (lhead, lbody, lfound, lnext, lslow, ljoin) = (self.label(), self.label(), self.label(), self.label(), self.label(), self.label());
        let at = self.alloca("i64");
        let last = self.ins(format!("sub i64 {}, 1", n));
        self.emit(format!("store i64 {}, ptr {}", last, at));
        self.cbr(&ok, &lhead, &lslow);
        self.block(&lhead);
        let i = self.ins(format!("load i64, ptr {}", at));
        let inside = self.ins(format!("icmp sge i64 {}, 0", i));
        self.cbr(&inside, &lbody, &ljoin);
        self.block(&lbody);
        for (_, k) in &reads {
            self.prechecked.insert(*k as *const Expr);
        }
        self.bind(l.params[0], iv.clone());
        self.bind(l.params[1], v(i.clone(), "i64"));
        let cv = self.ex(c, 1);
        for (_, k) in &reads {
            self.prechecked.remove(&(*k as *const Expr));
        }
        let cv = cv?;
        self.cbr(&cv.r, &lfound, &lnext);
        self.block(&lfound);
        self.emit(format!("store i64 {}, ptr {}", i, acc));
        self.br(&ljoin);
        self.block(&lnext);
        let down = self.ins(format!("sub i64 {}, 1", i));
        self.emit(format!("store i64 {}, ptr {}", down, at));
        self.br(&lhead);
        self.block(&lslow);
        let slow = self.fold_loop(&src, &n, &iv, &init.ty, l)?;
        self.emit(format!("store i64 {}, ptr {}", slow.r, acc));
        self.br(&ljoin);
        self.block(&ljoin);
        Ok(v(self.ins(format!("load i64, ptr {}", acc)), "i64"))
    }

    /// `c` of a search of rule F on the `w` indices from `i`: whether it
    /// holds at one of them, and the mask of those (`fold_first_lane` gives
    /// the first).
    fn fold_first_vector(&mut self, c: &Expr, (a, j): (VarId, VarId), iv: &V, reads: &[(&Expr, &Expr)], i: &str, w: u32) -> R<(String, String)> {
        for (_, k) in reads {
            self.prechecked.insert(*k as *const Expr);
        }
        self.bind(a, iv.clone());
        let lanes = self.elem(&Src::Iota, i, w)?;
        self.bind(j, lanes);
        let cv = self.ex(c, w);
        for (_, k) in reads {
            self.prechecked.remove(&(*k as *const Expr));
        }
        let cv = self.widen(&cv?, w);
        let bits = self.ins(format!("bitcast <{} x i1> {} to i{}", w, cv.r, w));
        let any = self.ins(format!("icmp ne i{} {}, 0", w, bits));
        Ok((any, bits))
    }

    /// The index of the first lane set of `bits` (not 0), the mask of the
    /// `w` indices from `i`.
    fn fold_first_lane(&mut self, bits: &str, i: &str, w: u32) -> String {
        self.decls.insert(format!("declare i{w} @llvm.cttz.i{w}(i{w}, i1)", w = w));
        let k = self.ins(format!("call i{w} @llvm.cttz.i{w}(i{w} {b}, i1 true)", w = w, b = bits));
        let k64 = if w == 64 { k } else { self.ins(format!("zext i{} {} to i64", w, k)) };
        self.ins(format!("add i64 {}, {}", i, k64))
    }

    /// A `fold` of rule F (`first_where`, D505): `c` from `i = 0` up, the
    /// first `i` where it holds the result, `init` when none does or when
    /// `init` is not negative; `c` evaluated only up to that `i`, as
    /// written. When `c` cannot stop the program (`safe_cond_in`) and its
    /// reads `x[i + p]` are in bounds at `i = 0` and `i = n - 1`
    /// (`fold_precheck`), `w` indices at a time first: `c` on `w` lanes, a
    /// mask, its first lane set the result (64 bytes of the elements read a
    /// vector where the target has masks of 64 bytes, 32 otherwise); the
    /// indices left (fewer than `w`), or all of them otherwise, one at a
    /// time with their checks (a masked vector for those left was slower:
    /// `parse` -4.7 % instead of -8.0 %, the searches of a few bytes of the
    /// keys, D505).
    fn fold_first(&mut self, x: &Expr, init: &Expr, l: &Lambda, c: &Expr) -> R<V> {
        let (_, n) = self.prepare_src(x)?;
        let iv = self.ex(init, 1)?;
        let (a, j) = (l.params[0], l.params[1]);
        let acc = self.alloca("i64");
        self.emit(format!("store i64 {}, ptr {}", iv.r, acc));
        let at = self.alloca("i64");
        self.emit(format!("store i64 0, ptr {}", at));
        let (lsearch, lscalar, ljoin) = (self.label(), self.label(), self.label());
        let neg = self.ins(format!("icmp slt i64 {}, 0", iv.r));
        self.cbr(&neg, &lsearch, &ljoin);
        self.block(&lsearch);
        let reads = self.fold_index_reads(l);
        let mut indexes = 0;
        fn count(e: &Expr, k: &mut usize) {
            if matches!(e.kind, ExprKind::Index(..)) {
                *k += 1;
            }
            for c in e.children() {
                count(c, k);
            }
        }
        count(c, &mut indexes);
        let size = reads.iter().map(|(a, _)| if a.ty.is_array() { esize(a.ty.elem()) } else { 8 }).max().unwrap_or(8).max(1);
        let bytes = if BYTE_MASKS.with(|b| b.get()) { 64 } else { 32 };
        let w = (bytes / size) as u32;
        if indexes == reads.len() && safe_cond_in(c, true) && w > 1 && !self.opts.isa.is_wasm() {
            // fewer indices than a vector: one at a time at once, without
            // the checks of the reads at both ends (D512: the searches of a
            // backslash in the keys of a few bytes of `json.decode`)
            let lcheck = self.label();
            let least = if size == 1 { NARROW_SEARCH.min(w) } else { w };
            let wide = self.ins(format!("icmp sge i64 {}, {}", n, least));
            self.cbr(&wide, &lcheck, &lscalar);
            self.block(&lcheck);
            let some = self.ins(format!("icmp sgt i64 {}, 0", n));
            let ok = if reads.is_empty() {
                some
            } else {
                let fine = self.fold_precheck(&reads, j, &n, 1)?;
                self.ins(format!("and i1 {}, {}", some, fine))
            };
            let (lvec, lhead, ltail) = (self.label(), self.label(), self.label());
            self.cbr(&ok, &lvec, &lscalar);
            self.block(&lvec);
            // the first FIRST_SCALAR indices one at a time, in bounds (their
            // reads checked at both ends): a short search ends there, before
            // the latency of a vector, its mask and its first lane (D512:
            // the end of a key or a text of a few bytes of `json.decode`, a
            // vector then the byte found read again, 30 % of the building
            // of its members)
            let (lpre, lpbody, lphit, lpnext) = (self.label(), self.label(), self.label(), self.label());
            let first = if size == 1 { FIRST_SCALAR } else { 0 };
            let pre_end = self.ins(format!("call i64 @llvm.smin.i64(i64 {}, i64 {})", n, first));
            self.decls.insert("declare i64 @llvm.smin.i64(i64, i64)".to_string());
            self.br(&lpre);
            self.block(&lpre);
            let pi = self.ins(format!("load i64, ptr {}", at));
            let pmore = self.ins(format!("icmp slt i64 {}, {}", pi, pre_end));
            self.cbr(&pmore, &lpbody, &lhead);
            self.block(&lpbody);
            for (_, k) in &reads {
                self.prechecked.insert(*k as *const Expr);
            }
            self.bind(a, iv.clone());
            self.bind(j, v(pi.clone(), "i64"));
            let pv = self.ex(c, 1);
            for (_, k) in &reads {
                self.prechecked.remove(&(*k as *const Expr));
            }
            let pv = pv?;
            self.cbr(&pv.r, &lphit, &lpnext);
            self.block(&lphit);
            self.emit(format!("store i64 {}, ptr {}", pi, acc));
            self.br(&ljoin);
            self.block(&lpnext);
            let pn = self.ins(format!("add i64 {}, 1", pi));
            self.emit(format!("store i64 {}, ptr {}", pn, at));
            self.br(&lpre);
            // then whole vectors from there, while one fits before `n`; for
            // bytes, vectors of 16 after them, and the indices still left
            // (fewer than 16) by one vector more, over the end of the one
            // before: its lanes already tested hold no `c`, so its first
            // lane set is the first index (D595: an array of 16 to 63
            // bytes, the end of a longer one, were tested one byte at a
            // time, 5 instructions each)
            let narrow: u32 = if size == 1 { NARROW_SEARCH } else { w };
            self.block(&lhead);
            let mut widths = vec![w];
            if narrow < w {
                widths.push(narrow);
            }
            for (t, &width) in widths.iter().enumerate() {
                let (lloop, lbody, lhit, lnext, ldone) = (self.label(), self.label(), self.label(), self.label(), self.label());
                let last = self.ins(format!("sub i64 {}, {}", n, width));
                // one array of bytes read: after a first whole vector where
                // the search is, the next ones from an address that is a
                // multiple of the vector (D596: the vectors of 64 bytes of a
                // long search were each across two lines of the cache; the
                // lanes tested twice hold no hit)
                if t == 0 && size == 1 && !reads.is_empty() {
                    if let (Src::Mat { ptr, .. }, _) = self.prepare_src(reads[0].0)? {
                        let (lfirst, lhit0, lalign) = (self.label(), self.label(), self.label());
                        let i0 = self.ins(format!("load i64, ptr {}", at));
                        let more0 = self.ins(format!("icmp sle i64 {}, {}", i0, last));
                        self.cbr(&more0, &lfirst, &ldone);
                        self.block(&lfirst);
                        let (any, bits) = self.fold_first_vector(c, (a, j), &iv, &reads, &i0, width)?;
                        self.cbr(&any, &lhit0, &lalign);
                        self.block(&lhit0);
                        let found = self.fold_first_lane(&bits, &i0, width);
                        self.emit(format!("store i64 {}, ptr {}", found, acc));
                        self.br(&ljoin);
                        self.block(&lalign);
                        self.bind(j, v(i0.clone(), "i64"));
                        self.prechecked.insert(reads[0].1 as *const Expr);
                        let ix = self.ex(reads[0].1, 1);
                        self.prechecked.remove(&(reads[0].1 as *const Expr));
                        let ix = ix?;
                        let base = self.ins(format!("ptrtoint ptr {} to i64", ptr));
                        let addr = self.ins(format!("add i64 {}, {}", base, ix.r));
                        let mis = self.ins(format!("and i64 {}, {}", addr, width - 1));
                        let adv = self.ins(format!("sub i64 {}, {}", width, mis));
                        let next = self.ins(format!("add i64 {}, {}", i0, adv));
                        self.emit(format!("store i64 {}, ptr {}", next, at));
                    }
                }
                self.br(&lloop);
                self.block(&lloop);
                let i = self.ins(format!("load i64, ptr {}", at));
                let more = self.ins(format!("icmp sle i64 {}, {}", i, last));
                self.cbr(&more, &lbody, &ldone);
                self.block(&lbody);
                let (any, bits) = self.fold_first_vector(c, (a, j), &iv, &reads, &i, width)?;
                self.cbr(&any, &lhit, &lnext);
                self.block(&lhit);
                let found = self.fold_first_lane(&bits, &i, width);
                self.emit(format!("store i64 {}, ptr {}", found, acc));
                self.br(&ljoin);
                self.block(&lnext);
                let next = self.ins(format!("add i64 {}, {}", i, width));
                self.emit(format!("store i64 {}, ptr {}", next, at));
                self.br(&lloop);
                self.block(&ldone);
                let _ = t;
            }
            if size == 1 {
                let (lover, lhit) = (self.label(), self.label());
                let i = self.ins(format!("load i64, ptr {}", at));
                let left = self.ins(format!("icmp slt i64 {}, {}", i, n));
                self.cbr(&left, &lover, &ljoin);
                self.block(&lover);
                let from = self.ins(format!("sub i64 {}, {}", n, narrow));
                let (any, bits) = self.fold_first_vector(c, (a, j), &iv, &reads, &from, narrow)?;
                self.cbr(&any, &lhit, &ljoin);
                self.block(&lhit);
                let found = self.fold_first_lane(&bits, &from, narrow);
                self.emit(format!("store i64 {}, ptr {}", found, acc));
                self.br(&ljoin);
            } else {
                self.br(&ltail);
            }
            self.block(&ltail);
            self.br(&lscalar);
        } else {
            self.br(&lscalar);
        }
        // one index at a time, from where the vectors stopped
        let (lhead, lbody, lhit, lnext) = (self.label(), self.label(), self.label(), self.label());
        self.block(&lscalar);
        self.br(&lhead);
        self.block(&lhead);
        let i = self.ins(format!("load i64, ptr {}", at));
        let more = self.ins(format!("icmp slt i64 {}, {}", i, n));
        self.cbr(&more, &lbody, &ljoin);
        self.block(&lbody);
        self.bind(a, iv.clone());
        self.bind(j, v(i.clone(), "i64"));
        let cv = self.ex(c, 1)?;
        self.cbr(&cv.r, &lhit, &lnext);
        self.block(&lhit);
        self.emit(format!("store i64 {}, ptr {}", i, acc));
        self.br(&ljoin);
        self.block(&lnext);
        let next = self.ins(format!("add i64 {}, 1", i));
        self.emit(format!("store i64 {}, ptr {}", next, at));
        self.br(&lhead);
        self.block(&ljoin);
        Ok(v(self.ins(format!("load i64, ptr {}", acc)), "i64"))
    }

    fn fold(&mut self, x: &Expr, init: &Expr, l: &Lambda) -> R<V> {
        let (src, n) = self.prepare_src(x)?;
        let iv = self.ex(init, 1)?;
        self.fold_loop(&src, &n, &iv, &init.ty, l)
    }

    /// The loop of a `fold` from its initial value `iv` (of type `at`).
    fn fold_loop(&mut self, src: &Src, n: &str, iv: &V, at: &Type, l: &Lambda) -> R<V> {
        let at = at.clone();
        let alt = lt(&at);
        let acc = self.alloca(&alt);
        self.emit(format!("store {} {}, ptr {}", alt, iv.r, acc));
        self.for_range("0", n, 1, &mut |g, i| {
            let a = g.ins(format!("load {}, ptr {}", alt, acc));
            let av = v(a, alt.clone());
            let el = g.elem(src, i, 1)?;
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

    /// A `fold` of rule R (`affine_recurrence`, D292), `v[t + 1] = f(v[t])`
    /// for `t` in `0 .. n`: `v[1]` computed as written, then, when it is in
    /// `0 .. m` and `v` has the `n + 1` elements written, `v[s] =
    /// f^(s - 1)(v[1])` for `s` in `2 .. n + 1` by `affine_fill` (each part
    /// from its first value, a power of `f` applied to `v[1]`). The same
    /// integers as the loop: from `0 <= y < m` no step and no power wraps
    /// (`affine_step`). Otherwise (fewer than RECUR_MIN steps, an update out
    /// of bounds and its panic, `v[1]` outside `0 .. m`) the loop as written.
    fn fold_affine(&mut self, x: &Expr, init: &Expr, l: &Lambda, f: Affine, value: &Expr) -> R<V> {
        const RECUR_MIN: i64 = 64;
        let (src, n) = self.prepare_src(x)?;
        let iv = self.ex(init, 1)?;
        let p = self.ins(format!("extractvalue {{ ptr, i64 }} {}, 0", iv.r));
        let len = self.ins(format!("extractvalue {{ ptr, i64 }} {}, 1", iv.r));
        let out = self.alloca("{ ptr, i64 }");
        let (lfirst, lfast, lslow, ljoin) = (self.label(), self.label(), self.label(), self.label());
        let many = self.ins(format!("icmp sge i64 {}, {}", n, RECUR_MIN));
        let inside = self.ins(format!("icmp slt i64 {}, {}", n, len));
        let both = self.ins(format!("and i1 {}, {}", many, inside));
        self.cbr(&both, &lfirst, &lslow);
        self.block(&lfirst);
        self.bind(l.params[0], iv.clone());
        self.bind(l.params[1], v("0", "i64"));
        let y1 = self.ex(value, 1)?;
        let ok = self.ins(format!("icmp ult i64 {}, {}", y1.r, f.m));
        self.cbr(&ok, &lfast, &lslow);
        self.block(&lfast);
        let at1 = self.ins(format!("getelementptr inbounds i64, ptr {}, i64 1", p));
        self.emit(format!("store i64 {}, ptr {}", y1.r, at1));
        let end = self.ins(format!("add i64 {}, 1", n));
        self.affine_fill(&p, "1", &y1.r, &end, f)?;
        self.emit(format!("store {{ ptr, i64 }} {}, ptr {}", iv.r, out));
        self.br(&ljoin);
        self.block(&lslow);
        let r = self.fold_loop(&src, &n, &iv, &init.ty, l)?;
        self.emit(format!("store {{ ptr, i64 }} {}, ptr {}", r.r, out));
        self.br(&ljoin);
        self.block(&ljoin);
        Ok(v(self.ins(format!("load {{ ptr, i64 }}, ptr {}", out)), "{ ptr, i64 }"))
    }

    /// `p[s] = f^(s - from)(y)` for `s` in `from + 1 .. end` (`p[from]` is
    /// `y`, in `0 .. m`): in parts of consecutive indices on the pool, one
    /// per thread with at least RECUR_PART values each, or here in a task or
    /// with one part (`affine_range`).
    fn affine_fill(&mut self, p: &str, from: &str, y: &str, end: &str, f: Affine) -> R<()> {
        const RECUR_PART: i64 = 8192;
        let lo = self.ins(format!("add i64 {}, 1", from));
        if self.in_task {
            return self.affine_range(p, from, y, &lo, end, f);
        }
        self.decls.insert("declare i64 @llvm.smax.i64(i64, i64)".into());
        self.decls.insert("declare i64 @llvm.smin.i64(i64, i64)".into());
        let count = self.ins(format!("sub i64 {}, {}", end, lo));
        let thr = self.ins("call i64 @isc_thread_count()".into());
        let most = self.ins(format!("sdiv i64 {}, {}", count, RECUR_PART));
        let most = self.ins(format!("call i64 @llvm.smax.i64(i64 {}, i64 1)", most));
        let parts = self.ins(format!("call i64 @llvm.smin.i64(i64 {}, i64 {})", thr, most));
        let several = self.ins(format!("icmp sgt i64 {}, 1", parts));
        let (lpar, lone, ljoin) = (self.label(), self.label(), self.label());
        self.cbr(&several, &lpar, &lone);
        self.block(&lpar);
        // the part k: lo + count * k / parts .. lo + count * (k + 1) / parts
        let extra = [v(p.to_string(), "ptr"), v(from.to_string(), "i64"), v(y.to_string(), "i64"), v(lo.clone(), "i64"), v(count.clone(), "i64"), v(parts.clone(), "i64")];
        let (task, ctx, ctx_bytes) = self.outline_task(&[], &extra, &mut |g, x, k| {
            let a = g.ins(format!("mul i64 {}, {}", x[4].r, k));
            let a = g.ins(format!("sdiv i64 {}, {}", a, x[5].r));
            let k1 = g.ins(format!("add i64 {}, 1", k));
            let b = g.ins(format!("mul i64 {}, {}", x[4].r, k1));
            let b = g.ins(format!("sdiv i64 {}, {}", b, x[5].r));
            let start = g.ins(format!("add i64 {}, {}", x[3].r, a));
            let stop = g.ins(format!("add i64 {}, {}", x[3].r, b));
            g.affine_range(&x[0].r, &x[1].r, &x[2].r, &start, &stop, f)
        })?;
        self.par_for(0, &parts, &task, &ctx, &ctx_bytes);
        self.br(&ljoin);
        self.block(&lone);
        self.affine_range(p, from, y, &lo, end, f)?;
        self.br(&ljoin);
        self.block(&ljoin);
        Ok(())
    }

    /// `p[s] = f^(s - from)(y)` for `s` in `lo .. hi` (`from < lo`, `y` in
    /// `0 .. m`): RECUR_CHAINS chains of `q = (hi - lo) / RECUR_CHAINS`
    /// consecutive values, advanced together so that their steps overlap
    /// (one step is a chain of multiplications, about 14 cycles), each from
    /// its first value (a power of `f`: the first chain from `y`, each next
    /// one `q` steps after it); then the last chain goes on to `hi`.
    fn affine_range(&mut self, p: &str, from: &str, y: &str, lo: &str, hi: &str, f: Affine) -> R<()> {
        const RECUR_CHAINS: i64 = 8;
        let len = self.ins(format!("sub i64 {}, {}", hi, lo));
        let q = self.ins(format!("sdiv i64 {}, {}", len, RECUR_CHAINS));
        let s0 = self.ins(format!("sub i64 {}, {}", lo, from));
        let (a0, c0) = self.affine_power(&s0, f);
        let mut first = self.affine_apply(&a0, &c0, y, f);
        let (aq, cq) = self.affine_power(&q, f);
        let mut chains = Vec::new();
        for j in 0..RECUR_CHAINS {
            if j > 0 {
                first = self.affine_apply(&aq, &cq, &first, f);
            }
            let slot = self.alloca("i64");
            self.emit(format!("store i64 {}, ptr {}", first, slot));
            let jq = self.ins(format!("mul i64 {}, {}", q, j));
            let at = self.ins(format!("add i64 {}, {}", lo, jq));
            let base = self.ins(format!("getelementptr inbounds i64, ptr {}, i64 {}", p, at));
            chains.push((slot, base));
        }
        self.for_range_scalar("0", &q, 1, &mut |g, i| {
            for (slot, base) in &chains {
                let yv = g.ins(format!("load i64, ptr {}", slot));
                let at = g.ins(format!("getelementptr inbounds i64, ptr {}, i64 {}", base, i));
                g.emit(format!("store i64 {}, ptr {}", yv, at));
                let next = g.affine_once(&yv, f);
                g.emit(format!("store i64 {}, ptr {}", next, slot));
            }
            Ok(())
        })?;
        let (last, _) = chains.last().unwrap().clone();
        let cq8 = self.ins(format!("mul i64 {}, {}", q, RECUR_CHAINS));
        let rest = self.ins(format!("add i64 {}, {}", lo, cq8));
        self.for_range_scalar(&rest, hi, 1, &mut |g, s| {
            let yv = g.ins(format!("load i64, ptr {}", last));
            let at = g.ins(format!("getelementptr inbounds i64, ptr {}, i64 {}", p, s));
            g.emit(format!("store i64 {}, ptr {}", yv, at));
            let next = g.affine_once(&yv, f);
            g.emit(format!("store i64 {}, ptr {}", next, last));
            Ok(())
        })?;
        Ok(())
    }

    /// `f(y)`, `y` in `0 .. m`: `(a * y + c) % m`, without a wrap (`affine_step`).
    fn affine_once(&mut self, y: &str, f: Affine) -> String {
        let t = self.ins(format!("mul nuw nsw i64 {}, {}", y, f.a));
        let t = self.ins(format!("add nuw nsw i64 {}, {}", t, f.c));
        self.ins(format!("urem i64 {}, {}", t, f.m))
    }

    /// `(a * y + c) % m`, `a`, `c` and `y` in `0 .. m` (a power of `f`).
    fn affine_apply(&mut self, a: &str, c: &str, y: &str, f: Affine) -> String {
        let t = self.ins(format!("mul nuw nsw i64 {}, {}", a, y));
        let t = self.ins(format!("add nuw nsw i64 {}, {}", t, c));
        self.ins(format!("urem i64 {}, {}", t, f.m))
    }

    /// `f^s` as `(A, C)`, `f^s(y) = (A * y + C) % m`, by squarings: the
    /// powers of two of `f` (`f^(2k) = f^k . f^k`), those of the bits of `s`
    /// composed; every product of two values below `m` (`affine_step`).
    fn affine_power(&mut self, s: &str, f: Affine) -> (String, String) {
        let m = f.m;
        let slots: Vec<String> = (0..5).map(|_| self.alloca("i64")).collect();
        let (ra, rc, ba, bc, ss) = (&slots[0], &slots[1], &slots[2], &slots[3], &slots[4]);
        for (slot, init) in [(ra, 1), (rc, 0), (ba, f.a % m), (bc, f.c % m)] {
            self.emit(format!("store i64 {}, ptr {}", init, slot));
        }
        self.emit(format!("store i64 {}, ptr {}", s, ss));
        let (head, body, done) = (self.label(), self.label(), self.label());
        self.br(&head);
        self.block(&head);
        let sv = self.ins(format!("load i64, ptr {}", ss));
        let more = self.ins(format!("icmp sgt i64 {}, 0", sv));
        self.cbr(&more, &body, &done);
        self.block(&body);
        let [rav, rcv, bav, bcv] = [ra, rc, ba, bc].map(|x| self.ins(format!("load i64, ptr {}", x)));
        // the bit: f^bit . R
        let bit = self.ins(format!("and i64 {}, 1", sv));
        let on = self.ins(format!("icmp ne i64 {}, 0", bit));
        let na = self.affine_apply(&bav, "0", &rav, f);
        let nc = self.affine_apply(&bav, &bcv, &rcv, f);
        let ra2 = self.ins(format!("select i1 {}, i64 {}, i64 {}", on, na, rav));
        let rc2 = self.ins(format!("select i1 {}, i64 {}, i64 {}", on, nc, rcv));
        // B . B
        let ba2 = self.affine_apply(&bav, "0", &bav, f);
        let bc2 = self.affine_apply(&bav, &bcv, &bcv, f);
        let s2 = self.ins(format!("lshr i64 {}, 1", sv));
        for (slot, val) in [(ra, &ra2), (rc, &rc2), (ba, &ba2), (bc, &bc2), (ss, &s2)] {
            self.emit(format!("store i64 {}, ptr {}", val, slot));
        }
        self.scalar_loops += 1;
        self.emit(format!("br label %{}, !llvm.loop !{}", head, self.scalar_loops + 1));
        self.block(&done);
        let a = self.ins(format!("load i64, ptr {}", ra));
        let c = self.ins(format!("load i64, ptr {}", rc));
        (a, c)
    }

    /// A `fold` of `split_plan` (D260): the `let`s that do not depend on the
    /// accumulator computed first for every element, into slots on the
    /// stack; then the loop of the accumulator, which reads them. Those that
    /// use the divider (roots, divisions of floats) or a value that does are
    /// computed for two elements at once, in vectors of 2 lanes, the others
    /// one element at a time: in the loop, LLVM pairs some roots and
    /// divisions and leaves the others scalar, while a vector of 2 lanes
    /// holds the divider as long as a scalar (4.5 cycles a root, 4 a
    /// division on the Xeon E-2388G); n / 2 of them instead of up to n. The
    /// parts of these values that do not use the divider stay scalar, put
    /// into vectors just before it. The same operations on the same values:
    /// the same bits. A check of an index that fails before the loop jumps to
    /// the `fold` as written, whose panic is then the one of the program;
    /// with constant indices, LLVM removes that path.
    fn fold_split(&mut self, x: &Expr, init: &Expr, l: &Lambda, n: i64, vars: &[VarId]) -> R<V> {
        const W: u32 = 2;
        // the `let`s in their order: (variable, value, in vectors: the value
        // with its leaves replaced, and the leaves)
        let mut parts: Vec<(VarId, &Expr, Option<(Expr, Vec<(VarId, Expr)>)>)> = Vec::new();
        let mut vec_vars: HashSet<VarId> = HashSet::new();
        let mut e = &*l.body;
        while let ExprKind::Let { var, init, body, .. } = &e.kind {
            if vars.contains(var) {
                if divides(init) || mentions(init, &vec_vars) {
                    let mut leaves = Vec::new();
                    let mut fresh = self.split_fresh;
                    let packed = pack_leaves(init, &vec_vars, &mut leaves, &mut fresh);
                    self.split_fresh = fresh;
                    parts.push((*var, &**init, Some((packed, leaves))));
                    vec_vars.insert(*var);
                } else {
                    parts.push((*var, &**init, None));
                }
            }
            e = body;
        }
        let slots: Vec<String> = parts.iter().map(|(_, e, _)| self.alloca(&format!("[{} x {}]", n, lt(&e.ty)))).collect();
        let lslow = self.label();
        let lfast = self.label();
        let ljoin = self.label();
        let saved = self.split_fail.replace(lslow.clone());
        let r = self.for_range("0", &n.to_string(), W as i64, &mut |g, i| {
            // each lane: the scalar values, and the leaves of the others
            let mut lanes: Vec<Vec<Vec<V>>> = parts.iter().map(|(_, _, p)| vec![Vec::new(); p.as_ref().map_or(0, |p| p.1.len())]).collect();
            for j in 0..W {
                let k = if j == 0 { i.to_string() } else { g.ins(format!("add i64 {}, {}", i, j)) };
                g.bind(l.params[1], V { r: k.clone(), t: "i64".into(), uni: false });
                for (q, (var, e, p)) in parts.iter().enumerate() {
                    match p {
                        None => {
                            let xv = g.ex(e, 1)?;
                            g.bind(*var, xv.clone());
                            let at = g.ins(format!("getelementptr inbounds {}, ptr {}, i64 {}", lt(&e.ty), slots[q], k));
                            g.emit(format!("store {} {}, ptr {}", xv.t, xv.r, at));
                        }
                        Some((_, leaves)) => {
                            for (f, (_, le)) in leaves.iter().enumerate() {
                                let xv = g.ex(le, 1)?;
                                lanes[q][f].push(xv);
                            }
                        }
                    }
                }
            }
            // the values in vectors, from their leaves
            for (q, (var, e, p)) in parts.iter().enumerate() {
                let Some((packed, leaves)) = p else { continue };
                for (f, (fv, le)) in leaves.iter().enumerate() {
                    let st = lt(&le.ty);
                    let vt = format!("<{} x {}>", W, st);
                    let mut r = "poison".to_string();
                    for (j, xv) in lanes[q][f].iter().enumerate() {
                        r = g.ins(format!("insertelement {} {}, {} {}, i32 {}", vt, r, st, xv.r, j));
                    }
                    g.bind(*fv, V { r, t: vt, uni: false });
                }
                let xv = g.ex(packed, W)?;
                let xv = g.widen(&xv, W);
                g.bind(*var, xv.clone());
                let at = g.ins(format!("getelementptr inbounds {}, ptr {}, i64 {}", lt(&e.ty), slots[q], i));
                g.emit(format!("store {} {}, ptr {}", xv.t, xv.r, at));
            }
            Ok(())
        });
        self.split_fail = saved;
        r?;
        self.br(&lfast);
        // the loop of the accumulator, the values of the `let`s read
        self.block(&lfast);
        for (q, (var, e, _)) in parts.iter().enumerate() {
            self.split_vars.insert(*var, (slots[q].clone(), lt(&e.ty), l.params[1]));
        }
        let fast = self.fold(x, init, l);
        for (var, _, _) in &parts {
            self.split_vars.remove(var);
        }
        let fast = fast?;
        let fend = self.current_label(&lfast);
        self.br(&ljoin);
        // a check failed: the `fold` as written
        self.block(&lslow);
        let slow = self.fold(x, init, l)?;
        let send = self.current_label(&lslow);
        self.br(&ljoin);
        self.block(&ljoin);
        let r = self.ins(format!("phi {} [ {}, %{} ], [ {}, %{} ]", fast.t, fast.r, fend, slow.r, send));
        Ok(v(r, fast.t))
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
            self.par_for(0, &c0, &task, &ctx, &ctx_bytes);
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
        self.par_for(0, &m, &task, &ctx, &ctx_bytes);
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
            g.par_for(0, &m, &task, &ctx, &ctx_bytes);
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
            ExprKind::ConstStrs(ss) => Some(ss.len() as i64),
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
        self.par_for(0, &tasks, &task, &ctx, &ctx_bytes);
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
    /// The text of `e` when it is a string literal, or a variable bound to
    /// one (D301).
    fn literal_of(&self, e: &Expr) -> Option<String> {
        match &e.kind {
            ExprKind::StrLit(t) if e.ty == Type::Str => Some(t.clone()),
            ExprKind::Var(v, _) => self.str_lits.get(v).cloned(),
            _ => None,
        }
    }

    /// A concatenation whose parts are texts and texts of integers not made
    /// (`int_texts`, D359), written by the compiled code (D363): the length
    /// of each part computed here (the digits of an integer from its bits),
    /// one block, each part written where it goes (D359 gave the runtime a
    /// table of three words a part, `isc_str_build`, read twice: about 15
    /// cycles a part).
    fn concat_parts_built(&mut self, es: &[Expr], slot: Option<String>) -> R<V> {
        // each part and its length, in order: a literal (a constant), a
        // text, the text of an integer (its sign, its digits counted from its
        // bits and the powers of 10, its width); a left text given (D351)
        enum P {
            Lit(String, usize),
            Text(String, String),
            Int { mag: String, neg: String, n: String, width: String, fs: i64, zp: u32 },
        }
        let mut parts = Vec::new();
        let mut given: Option<(String, String)> = None;
        let mut total = "0".to_string();
        for (i, x) in es.iter().enumerate() {
            match &x.kind {
                ExprKind::Var(t, _) if self.int_texts.contains_key(t) => {
                    let it = self.int_texts[t].clone();
                    let (neg, mag) = if it.signed {
                        let neg = self.ins(format!("icmp slt i64 {}, 0", it.x));
                        let m = self.ins(format!("sub i64 0, {}", it.x));
                        let mag = self.ins(format!("select i1 {}, i64 {}, i64 {}", neg, m, it.x));
                        (self.ins(format!("zext i1 {} to i64", neg)), mag)
                    } else {
                        ("0".to_string(), it.x.clone())
                    };
                    if !self.pow10 {
                        self.pow10 = true;
                        let ps = (0..20).map(|k| format!("i64 {}", 10u64.pow(k))).collect::<Vec<_>>().join(", ");
                        self.globals.push(format!("@.pow10 = private unnamed_addr constant [20 x i64] [{}]", ps));
                    }
                    self.decls.insert("declare i64 @llvm.ctlz.i64(i64, i1)".to_string());
                    let w1 = self.ins(format!("or i64 {}, 1", mag));
                    let lz = self.ins(format!("call i64 @llvm.ctlz.i64(i64 {}, i1 true)", w1));
                    let bits = self.ins(format!("sub i64 64, {}", lz));
                    let m = self.ins(format!("mul i64 {}, 1233", bits));
                    let t = self.ins(format!("lshr i64 {}, 12", m));
                    let pp = self.ins(format!("getelementptr inbounds [20 x i64], ptr @.pow10, i64 0, i64 {}", t));
                    let p10 = self.ins(format!("load i64, ptr {}", pp));
                    let lt = self.ins(format!("icmp ult i64 {}, {}", w1, p10));
                    let ltz = self.ins(format!("zext i1 {} to i64", lt));
                    let d = self.ins(format!("sub i64 {}, {}", t, ltz));
                    let d1 = self.ins(format!("add i64 {}, 1", d));
                    let n = self.ins(format!("add i64 {}, {}", d1, neg));
                    let (width, fs) = match &it.pad {
                        Some((len, fill, start)) => {
                            self.decls.insert("declare i64 @llvm.smax.i64(i64, i64)".to_string());
                            (self.ins(format!("call i64 @llvm.smax.i64(i64 {}, i64 {})", len, n)), (*fill as i64) | (*start as i64) << 8)
                        }
                        None => (n.clone(), 0),
                    };
                    total = self.ins(format!("add i64 {}, {}", total, width));
                    // padded to 2, 3 or 4 by zeros at its start (`pad(x, 2)`
                    // of a date, D446; its year and milliseconds, D477): its
                    // digits from the table of pairs when it fits
                    let zp = match &it.pad {
                        Some((len, b'0', true)) if len == "2" => 2,
                        Some((len, b'0', true)) if len == "3" => 3,
                        Some((len, b'0', true)) if len == "4" => 4,
                        _ => 0,
                    };
                    parts.push(P::Int { mag, neg, n, width, fs, zp });
                }
                // a left text given (its last use, D351): continued in its
                // block when it has room
                ExprKind::Var(_, Use::Move) if i == 0 => {
                    let xv = self.ex(x, 1)?;
                    let (p, n) = self.str_parts(&xv);
                    total = self.ins(format!("add i64 {}, {}", total, n));
                    given = Some((p, n));
                }
                ExprKind::Var(t, _) if self.lit_vars.contains_key(t) => {
                    let lit = self.lit_vars[t].clone();
                    let (g, n) = self.str_const(&lit);
                    total = self.ins(format!("add i64 {}, {}", total, n));
                    parts.push(P::Lit(g, n));
                }
                ExprKind::StrLit(lit) if x.ty == Type::Str => {
                    let (g, n) = self.str_const(lit);
                    total = self.ins(format!("add i64 {}, {}", total, n));
                    parts.push(P::Lit(g, n));
                }
                _ => {
                    let xv = self.ex(x, 1)?;
                    let (p, n) = self.str_parts(&xv);
                    total = self.ins(format!("add i64 {}, {}", total, n));
                    parts.push(P::Text(p, n));
                }
            }
        }
        // the one block (at least the byte of an integer), each part written
        // where it goes: the literals by moves of constants (`memcpy` of a
        // constant length, inlined), the texts and the integers by the
        // runtime (D363)
        let (base, mut at) = match (&given, slot) {
            (Some((p, n)), _) => (self.reserve_inline(p, n, &total), n.clone()),
            // a text that its scope only reads: in its buffer on the stack
            // when it fits, a block of the heap otherwise (D404)
            (None, Some(buf)) => {
                let fits = self.ins(format!("icmp ule i64 {}, {}", total, STACK_STR));
                let (lheap, ljoin) = (self.label(), self.label());
                let lfrom = self.current_label("entry");
                self.cbr(&fits, &ljoin, &lheap);
                self.block(&lheap);
                let h = self.ins(format!("call ptr @isc_alloc(i64 {})", total));
                self.br(&ljoin);
                self.block(&ljoin);
                (self.ins(format!("phi ptr [ {}, %{} ], [ {}, %{} ]", buf, lfrom, h, lheap)), "0".to_string())
            }
            (None, None) => (self.ins(format!("call ptr @isc_alloc(i64 {})", total)), "0".to_string()),
        };
        for part in &parts {
            let dst = self.ins(format!("getelementptr inbounds i8, ptr {}, i64 {}", base, at));
            let len = match part {
                P::Lit(g, n) => {
                    if *n > 0 {
                        self.emit(format!("call void @llvm.memcpy.p0.p0.i64(ptr {}, ptr {}, i64 {}, i1 false)", dst, g, n));
                    }
                    n.to_string()
                }
                // a text of at most 64 bytes copied here by moves that overlap,
                // longer, `memcpy` (`copy_short`, D478)
                P::Text(p, n) => {
                    self.copy_short(&dst, p, n);
                    n.clone()
                }
                P::Int { mag, neg, n, width, fs, zp } if *zp > 0 => {
                    if !self.digits2 {
                        self.digits2 = true;
                        let pairs: String = (0..100).map(|i| format!("{:02}", i)).collect();
                        self.globals.push(format!("@.digits2 = private unnamed_addr constant [200 x i8] c\"{}\"", pairs));
                    }
                    let limit = 10u64.pow(*zp);
                    let small = self.ins(format!("icmp ult i64 {}, {}", mag, limit));
                    let pos = self.ins(format!("icmp eq i64 {}, 0", neg));
                    let few = self.ins(format!("and i1 {}, {}", small, pos));
                    let (ltwo, lcall, lj) = (self.label(), self.label(), self.label());
                    self.cbr(&few, &ltwo, &lcall);
                    self.block(&ltwo);
                    // the pair of the table of `x` (below 100) at `at`
                    let put_pair = |g: &mut Self, x: &str, at: &str| {
                        let at2 = g.ins(format!("shl i64 {}, 1", x));
                        let src2 = g.ins(format!("getelementptr inbounds [200 x i8], ptr @.digits2, i64 0, i64 {}", at2));
                        let pair = g.ins(format!("load i16, ptr {}, align 1", src2));
                        g.emit(format!("store i16 {}, ptr {}, align 1", pair, at));
                    };
                    match zp {
                        2 => put_pair(self, mag, &dst),
                        3 => {
                            let hi = self.ins(format!("udiv i64 {}, 100", mag));
                            let lo = self.ins(format!("urem i64 {}, 100", mag));
                            let d = self.ins(format!("trunc i64 {} to i8", hi));
                            let c = self.ins(format!("add i8 {}, 48", d));
                            self.emit(format!("store i8 {}, ptr {}", c, dst));
                            let at = self.ins(format!("getelementptr inbounds i8, ptr {}, i64 1", dst));
                            put_pair(self, &lo, &at);
                        }
                        _ => {
                            let hi = self.ins(format!("udiv i64 {}, 100", mag));
                            let lo = self.ins(format!("urem i64 {}, 100", mag));
                            put_pair(self, &hi, &dst);
                            let at = self.ins(format!("getelementptr inbounds i8, ptr {}, i64 2", dst));
                            put_pair(self, &lo, &at);
                        }
                    }
                    self.br(&lj);
                    self.block(&lcall);
                    self.emit(format!("call void @isc_put_int(ptr {}, i64 {}, i64 {}, i64 {}, i64 {}, i64 {})", dst, mag, neg, n, width, fs));
                    self.br(&lj);
                    self.block(&lj);
                    width.clone()
                }
                P::Int { mag, neg, n, width, fs, .. } if *fs == 0 => {
                    // one or two digits, not negative: their bytes, without
                    // a call (D441: the fields of a date written digit by
                    // digit, nine calls a date; a length, an hour)
                    if !self.digits2 {
                        self.digits2 = true;
                        let pairs: String = (0..100).map(|i| format!("{:02}", i)).collect();
                        self.globals.push(format!("@.digits2 = private unnamed_addr constant [200 x i8] c\"{}\"", pairs));
                    }
                    let small = self.ins(format!("icmp ult i64 {}, 100", mag));
                    let pos = self.ins(format!("icmp eq i64 {}, 0", neg));
                    let few = self.ins(format!("and i1 {}, {}", small, pos));
                    let (lfew, lone, ltwo, lcall, lj) = (self.label(), self.label(), self.label(), self.label(), self.label());
                    self.cbr(&few, &lfew, &lcall);
                    self.block(&lfew);
                    let single = self.ins(format!("icmp ult i64 {}, 10", mag));
                    self.cbr(&single, &lone, &ltwo);
                    self.block(&lone);
                    let d = self.ins(format!("trunc i64 {} to i8", mag));
                    let c = self.ins(format!("add i8 {}, 48", d));
                    self.emit(format!("store i8 {}, ptr {}", c, dst));
                    self.br(&lj);
                    self.block(&ltwo);
                    let at2 = self.ins(format!("shl i64 {}, 1", mag));
                    let src2 = self.ins(format!("getelementptr inbounds [200 x i8], ptr @.digits2, i64 0, i64 {}", at2));
                    let pair = self.ins(format!("load i16, ptr {}, align 1", src2));
                    self.emit(format!("store i16 {}, ptr {}, align 1", pair, dst));
                    self.br(&lj);
                    self.block(&lcall);
                    self.emit(format!("call void @isc_put_int(ptr {}, i64 {}, i64 {}, i64 {}, i64 {}, i64 0)", dst, mag, neg, n, width));
                    self.br(&lj);
                    self.block(&lj);
                    width.clone()
                }
                P::Int { mag, neg, n, width, fs, .. } => {
                    self.emit(format!("call void @isc_put_int(ptr {}, i64 {}, i64 {}, i64 {}, i64 {}, i64 {})", dst, mag, neg, n, width, fs));
                    width.clone()
                }
            };
            at = self.ins(format!("add i64 {}, {}", at, len));
        }
        let r0 = self.ins(format!("insertvalue {{ ptr, i64 }} poison, ptr {}, 0", base));
        Ok(v(self.ins(format!("insertvalue {{ ptr, i64 }} {}, i64 {}, 1", r0, total)), "{ ptr, i64 }"))
    }

    /// The `n` bytes at `p` copied to `dst`, apart: at most 64 by moves that
    /// overlap, as `copy_short` of the runtime (D478: the call of the copy,
    /// and the `vzeroupper` before it, a third of `json.quote` of a name; a
    /// masked store of AVX-512 instead could not forward its bytes to the
    /// loads that read them back, `parseUrl` +5.7 %); longer, `memcpy`.
    fn copy_short(&mut self, dst: &str, p: &str, n: &str) {
        let small = self.ins(format!("icmp ule i64 {}, 64", n));
        let (lsmall, l32, lrest16, l16, l8, l4, l1, lrest8, lrest4, lrest1, lcall, lj) = (
            self.label(),
            self.label(),
            self.label(),
            self.label(),
            self.label(),
            self.label(),
            self.label(),
            self.label(),
            self.label(),
            self.label(),
            self.label(),
            self.label(),
        );
        self.cbr(&small, &lsmall, &lcall);
        // two moves of `w` bytes, at the start and at the end
        let pair = |g: &mut Self, ty: &str, w: i64| {
            let a = g.ins(format!("load {}, ptr {}, align 1", ty, p));
            let off = g.ins(format!("sub i64 {}, {}", n, w));
            let src2 = g.ins(format!("getelementptr inbounds i8, ptr {}, i64 {}", p, off));
            let b = g.ins(format!("load {}, ptr {}, align 1", ty, src2));
            g.emit(format!("store {} {}, ptr {}, align 1", ty, a, dst));
            let dst2 = g.ins(format!("getelementptr inbounds i8, ptr {}, i64 {}", dst, off));
            g.emit(format!("store {} {}, ptr {}, align 1", ty, b, dst2));
        };
        self.block(&lsmall);
        let ge32 = self.ins(format!("icmp uge i64 {}, 32", n));
        self.cbr(&ge32, &l32, &lrest16);
        self.block(&l32);
        pair(self, "<32 x i8>", 32);
        self.br(&lj);
        self.block(&lrest16);
        let ge16 = self.ins(format!("icmp uge i64 {}, 16", n));
        self.cbr(&ge16, &l16, &lrest8);
        self.block(&l16);
        pair(self, "i128", 16);
        self.br(&lj);
        self.block(&lrest8);
        let ge8 = self.ins(format!("icmp uge i64 {}, 8", n));
        self.cbr(&ge8, &l8, &lrest4);
        self.block(&l8);
        pair(self, "i64", 8);
        self.br(&lj);
        self.block(&lrest4);
        let ge4 = self.ins(format!("icmp uge i64 {}, 4", n));
        self.cbr(&ge4, &l4, &lrest1);
        self.block(&l4);
        pair(self, "i32", 4);
        self.br(&lj);
        self.block(&lrest1);
        let ge1 = self.ins(format!("icmp ne i64 {}, 0", n));
        self.cbr(&ge1, &l1, &lj);
        self.block(&l1);
        // 1 to 3 bytes: the first, the middle and the last
        let half = self.ins(format!("lshr i64 {}, 1", n));
        let last = self.ins(format!("sub i64 {}, 1", n));
        for k in ["0".to_string(), half.clone(), last.clone()] {
            let sp = self.ins(format!("getelementptr inbounds i8, ptr {}, i64 {}", p, k));
            let c = self.ins(format!("load i8, ptr {}, align 1", sp));
            let dp = self.ins(format!("getelementptr inbounds i8, ptr {}, i64 {}", dst, k));
            self.emit(format!("store i8 {}, ptr {}, align 1", c, dp));
        }
        self.br(&lj);
        // longer: `memcpy` called here (D509: through `isc_put_text`,
        // which called it, two calls a part)
        self.block(&lcall);
        self.emit(format!("call void @llvm.memcpy.p0.p0.i64(ptr {}, ptr {}, i64 {}, i1 false)", dst, p, n));
        self.br(&lj);
        self.block(&lj);
    }

    /// The last `n` bytes (1 to 24) of the three words `w0`, `w1`, `w2`
    /// stored at `d` (D588, `$textTail`; the digits of a padded integer,
    /// D606): the words whole (8 bytes), the first shifted, or two stores of
    /// 4 or three of 1 that overlap; no byte written outside the `n`. `nm1`
    /// is `n - 1`.
    fn put_tail(&mut self, d: &str, w0: &str, w1: &str, w2: &str, n: &str, nm1: &str) {
        // `x` stored (`width` bits) at byte `at` of the digits
        let put = |g: &mut Self, at: &str, x: &str, width: u32| {
            let p = g.ins(format!("getelementptr inbounds i8, ptr {}, i64 {}", d, at));
            let y = if width == 64 { x.to_string() } else { g.ins(format!("trunc i64 {} to i{}", x, width)) };
            g.emit(format!("store i{} {}, ptr {}, align 1", width, y, p));
        };
        // `w` shifted right by `bytes` bytes
        let shr = |g: &mut Self, w: &str, bytes: &str| {
            let b = g.ins(format!("shl i64 {}, 3", bytes));
            g.ins(format!("lshr i64 {}, {}", w, b))
        };
        let (lbig, lsmall, l16, l9, lpair, l4, l1, ljoin) =
            (self.label(), self.label(), self.label(), self.label(), self.label(), self.label(), self.label(), self.label());
        let gt8 = self.ins(format!("icmp sgt i64 {}, 8", n));
        self.cbr(&gt8, &lbig, &lsmall);
        // 9 to 24 digits: the first part shifted, the words after it whole
        self.block(&lbig);
        let gt16 = self.ins(format!("icmp sgt i64 {}, 16", n));
        self.cbr(&gt16, &l16, &l9);
        self.block(&l16);
        let s24 = self.ins(format!("sub i64 24, {}", n));
        let h = shr(self, w0, &s24);
        put(self, "0", &h, 64);
        let at16 = self.ins(format!("sub i64 {}, 16", n));
        put(self, &at16, w1, 64);
        self.br(&lpair);
        self.block(&l9);
        let s16 = self.ins(format!("sub i64 16, {}", n));
        let m = shr(self, w1, &s16);
        put(self, "0", &m, 64);
        self.br(&lpair);
        self.block(&lpair);
        let at8 = self.ins(format!("sub i64 {}, 8", n));
        put(self, &at8, w2, 64);
        self.br(&ljoin);
        // 1 to 8 digits: the last word shifted, two stores of 4 or
        // (below 4) the first, middle and last byte
        self.block(&lsmall);
        let s8 = self.ins(format!("sub i64 8, {}", n));
        let t = shr(self, w2, &s8);
        let ge4 = self.ins(format!("icmp sge i64 {}, 4", n));
        self.cbr(&ge4, &l4, &l1);
        self.block(&l4);
        put(self, "0", &t, 32);
        let at4 = self.ins(format!("sub i64 {}, 4", n));
        let t4 = shr(self, &t, &at4);
        put(self, &at4, &t4, 32);
        self.br(&ljoin);
        self.block(&l1);
        put(self, "0", &t, 8);
        let half = self.ins(format!("lshr i64 {}, 1", n));
        let th = shr(self, &t, &half);
        put(self, &half, &th, 8);
        let tl = shr(self, &t, nm1);
        put(self, nm1, &tl, 8);
        self.br(&ljoin);
        self.block(&ljoin);
    }

    /// `n` bytes `c` (an `i8`) from `dst` (D606), as `fill_short` of the
    /// runtime: at most 64 by stores that overlap (two of 32, 16, 8 or 4
    /// bytes, or the first, the middle and the last byte), `memset` beyond.
    fn fill_short(&mut self, dst: &str, c: &str, n: &str) {
        let small = self.ins(format!("icmp ule i64 {}, 64", n));
        let (lsmall, l32, lrest16, l16, l8, l4, l1, lrest8, lrest4, lrest1, lcall, lj) = (
            self.label(),
            self.label(),
            self.label(),
            self.label(),
            self.label(),
            self.label(),
            self.label(),
            self.label(),
            self.label(),
            self.label(),
            self.label(),
            self.label(),
        );
        self.cbr(&small, &lsmall, &lcall);
        // two stores of `x`, `w` bytes, at the start and at the end
        let pair = |g: &mut Self, ty: &str, x: &str, w: i64| {
            g.emit(format!("store {} {}, ptr {}, align 1", ty, x, dst));
            let off = g.ins(format!("sub i64 {}, {}", n, w));
            let dst2 = g.ins(format!("getelementptr inbounds i8, ptr {}, i64 {}", dst, off));
            g.emit(format!("store {} {}, ptr {}, align 1", ty, x, dst2));
        };
        // the byte in each of the `w` bytes of a vector
        let lanes = |g: &mut Self, w: i64| {
            let x = g.ins(format!("insertelement <{} x i8> poison, i8 {}, i64 0", w, c));
            g.ins(format!("shufflevector <{} x i8> {}, <{} x i8> poison, <{} x i32> zeroinitializer", w, x, w, w))
        };
        self.block(&lsmall);
        let ge32 = self.ins(format!("icmp uge i64 {}, 32", n));
        self.cbr(&ge32, &l32, &lrest16);
        self.block(&l32);
        let x = lanes(self, 32);
        pair(self, "<32 x i8>", &x, 32);
        self.br(&lj);
        self.block(&lrest16);
        let ge16 = self.ins(format!("icmp uge i64 {}, 16", n));
        self.cbr(&ge16, &l16, &lrest8);
        self.block(&l16);
        let x = lanes(self, 16);
        pair(self, "<16 x i8>", &x, 16);
        self.br(&lj);
        self.block(&lrest8);
        // in each byte of a word
        let c64 = self.ins(format!("zext i8 {} to i64", c));
        let word = self.ins(format!("mul i64 {}, 72340172838076673", c64));
        let ge8 = self.ins(format!("icmp uge i64 {}, 8", n));
        self.cbr(&ge8, &l8, &lrest4);
        self.block(&l8);
        pair(self, "i64", &word, 8);
        self.br(&lj);
        self.block(&lrest4);
        let ge4 = self.ins(format!("icmp uge i64 {}, 4", n));
        self.cbr(&ge4, &l4, &lrest1);
        self.block(&l4);
        let half_word = self.ins(format!("trunc i64 {} to i32", word));
        pair(self, "i32", &half_word, 4);
        self.br(&lj);
        self.block(&lrest1);
        let ge1 = self.ins(format!("icmp ne i64 {}, 0", n));
        self.cbr(&ge1, &l1, &lj);
        self.block(&l1);
        // 1 to 3 bytes: the first, the middle and the last
        let half = self.ins(format!("lshr i64 {}, 1", n));
        let last = self.ins(format!("sub i64 {}, 1", n));
        for k in ["0".to_string(), half.clone(), last.clone()] {
            let dp = self.ins(format!("getelementptr inbounds i8, ptr {}, i64 {}", dst, k));
            self.emit(format!("store i8 {}, ptr {}, align 1", c, dp));
        }
        self.br(&lj);
        self.block(&lcall);
        self.decls.insert("declare void @llvm.memset.p0.i64(ptr, i8, i64, i1)".into());
        self.emit(format!("call void @llvm.memset.p0.i64(ptr {}, i8 {}, i64 {}, i1 false)", dst, c, n));
        self.br(&lj);
        self.block(&lj);
    }

    /// The block of a new text of `total` bytes: the buffer `buf` on the
    /// stack of the `let` that only reads the text when it fits there
    /// (STACK_STR bytes, D404), a block of the heap otherwise.
    fn text_block(&mut self, total: &str, buf: Option<&str>) -> String {
        let Some(buf) = buf else {
            return self.ins(format!("call noalias ptr @isc_alloc(i64 {})", total));
        };
        let fits = self.ins(format!("icmp ule i64 {}, {}", total, STACK_STR));
        let (lheap, ljoin) = (self.label(), self.label());
        let lfrom = self.current_label("entry");
        self.cbr(&fits, &ljoin, &lheap);
        self.block(&lheap);
        let h = self.ins(format!("call ptr @isc_alloc(i64 {})", total));
        self.br(&ljoin);
        self.block(&ljoin);
        self.ins(format!("phi ptr [ {}, %{} ], [ {}, %{} ]", buf, lfrom, h, lheap))
    }

    /// `s.padStart(len, fill)` (`start`) or `s.padEnd(len, fill)` of the
    /// text `p`, `n` (D606), made here as `pad` of the runtime core: the
    /// text itself when it has `len` bytes or more or the fill is empty;
    /// else copies of the fill before or after it up to `len` bytes, the
    /// last one cut before the character that would pass `len` (so, for a
    /// fill of several bytes, `len - n` bytes less those of its remainder by
    /// the length of the fill that are past the start of a character). One
    /// block of that size (the buffer `buf` of a text only read when it
    /// fits, D458), the text copied (`copy_short`), then the fill: one
    /// byte by stores (`fill_short`), several bytes once, then the bytes
    /// written copied after themselves, twice as many each turn.
    fn pad_text(&mut self, p: &str, n: &str, len: &str, fill: &Fill, start: bool, buf: Option<&str>) -> V {
        // (a mark for the tests)
        self.emit(if buf.is_some() { "; text pad, on the stack when it fits" } else { "; text pad" }.to_string());
        // the bytes the text lacks, read only where it is shorter than `len`
        // (compared, not subtracted first: the difference of a `len` near
        // the least `i64` is not an `i64`)
        let short = self.ins(format!("icmp sgt i64 {}, {}", len, n));
        let missing = self.ins(format!("sub i64 {}, {}", len, n));
        // the bytes of the fill
        let gap = match fill {
            Fill::Byte(_) => self.ins(format!("select i1 {}, i64 {}, i64 0", short, missing)),
            Fill::Text(q, m) => {
                let slot = self.alloca("i64");
                let rest = self.alloca("i64");
                self.emit(format!("store i64 0, ptr {}", slot));
                let some = self.ins(format!("icmp ne i64 {}, 0", m));
                let pads = self.ins(format!("and i1 {}, {}", short, some));
                let (lpad, lmany, lwalk, lback, lstep, lset, ldone) =
                    (self.label(), self.label(), self.label(), self.label(), self.label(), self.label(), self.label());
                self.cbr(&pads, &lpad, &ldone);
                self.block(&lpad);
                self.emit(format!("store i64 {}, ptr {}", missing, slot));
                let one = self.ins(format!("icmp eq i64 {}, 1", m));
                self.cbr(&one, &ldone, &lmany);
                // several bytes: the last copy cut at the start of a character
                // (a byte `10xxxxxx` continues one)
                self.block(&lmany);
                let r0 = self.ins(format!("urem i64 {}, {}", missing, m));
                self.emit(format!("store i64 {}, ptr {}", r0, rest));
                self.br(&lwalk);
                self.block(&lwalk);
                let r = self.ins(format!("load i64, ptr {}", rest));
                let inside = self.ins(format!("icmp sgt i64 {}, 0", r));
                self.cbr(&inside, &lback, &lset);
                self.block(&lback);
                let at = self.ins(format!("getelementptr inbounds i8, ptr {}, i64 {}", q, r));
                let b = self.ins(format!("load i8, ptr {}, align 1", at));
                let top = self.ins(format!("and i8 {}, -64", b));
                let cont = self.ins(format!("icmp eq i8 {}, -128", top));
                self.cbr(&cont, &lstep, &lset);
                self.block(&lstep);
                let r1 = self.ins(format!("sub i64 {}, 1", r));
                self.emit(format!("store i64 {}, ptr {}", r1, rest));
                self.br(&lwalk);
                self.block(&lset);
                let cut = self.ins(format!("sub i64 {}, {}", r0, r));
                let g = self.ins(format!("sub i64 {}, {}", missing, cut));
                self.emit(format!("store i64 {}, ptr {}", g, slot));
                self.br(&ldone);
                self.block(&ldone);
                self.ins(format!("load i64, ptr {}", slot))
            }
        };
        let total = self.ins(format!("add i64 {}, {}", n, gap));
        let dst = self.text_block(&total, buf);
        // the text after the fill (`padStart`) or before it
        let (tp, fp) = if start {
            (self.ins(format!("getelementptr inbounds i8, ptr {}, i64 {}", dst, gap)), dst.clone())
        } else {
            (dst.clone(), self.ins(format!("getelementptr inbounds i8, ptr {}, i64 {}", dst, n)))
        };
        self.copy_short(&tp, p, n);
        match fill {
            Fill::Byte(c) => self.fill_short(&fp, &(*c as i8).to_string(), &gap),
            Fill::Text(q, m) => {
                let (lone, lmany, lfirst, lloop, lbody, lend) = (self.label(), self.label(), self.label(), self.label(), self.label(), self.label());
                let one = self.ins(format!("icmp eq i64 {}, 1", m));
                self.cbr(&one, &lone, &lmany);
                self.block(&lone);
                let c = self.ins(format!("load i8, ptr {}, align 1", q));
                self.fill_short(&fp, &c, &gap);
                self.br(&lend);
                // the fill once (its start when there is less to fill), then
                // the bytes written copied after themselves
                self.block(&lmany);
                let some = self.ins(format!("icmp sgt i64 {}, 0", gap));
                self.cbr(&some, &lfirst, &lend);
                self.block(&lfirst);
                let less = self.ins(format!("icmp slt i64 {}, {}", gap, m));
                let first = self.ins(format!("select i1 {}, i64 {}, i64 {}", less, gap, m));
                self.copy_short(&fp, q, &first);
                let done = self.alloca("i64");
                self.emit(format!("store i64 {}, ptr {}", first, done));
                self.br(&lloop);
                self.block(&lloop);
                let filled = self.ins(format!("load i64, ptr {}", done));
                let more = self.ins(format!("icmp slt i64 {}, {}", filled, gap));
                self.cbr(&more, &lbody, &lend);
                self.block(&lbody);
                let left = self.ins(format!("sub i64 {}, {}", gap, filled));
                let fewer = self.ins(format!("icmp slt i64 {}, {}", left, filled));
                let k = self.ins(format!("select i1 {}, i64 {}, i64 {}", fewer, left, filled));
                let to = self.ins(format!("getelementptr inbounds i8, ptr {}, i64 {}", fp, filled));
                // (apart: at most as many bytes as are written before them)
                self.copy_short(&to, &fp, &k);
                let next = self.ins(format!("add i64 {}, {}", filled, k));
                self.emit(format!("store i64 {}, ptr {}", next, done));
                self.br(&lloop);
                self.block(&lend);
            }
        }
        self.arr(&dst, &total)
    }

    /// `toString(x).padStart(len, c)` (`start`) or `.padEnd(len, c)` of the
    /// integer `x` (an `i64`, or a `u64` when not `signed`), `c` one byte,
    /// in one block (D348), made here (D606): its digits in three words and
    /// their number by the function of the prelude lowered for it
    /// (`$rtDigitWords`), a block of `len` bytes (at least 25: its size does
    /// not wait for the count of the digits when `len` is a constant), the
    /// `-` and the digits stored from the registers (`put_tail`), `c` before
    /// or after them; the text alone when it has `len` bytes or more.
    fn pad_int(&mut self, x: &str, signed: bool, len: &str, c: u8, start: bool) -> R<V> {
        let Some(words) = self.p.digit_words else {
            return Err("codegen: the padded text of an integer, its function of the prelude not lowered".into());
        };
        // (a mark for the tests)
        self.emit("; int pad".to_string());
        let (neg, mag) = if signed {
            let neg = self.ins(format!("icmp slt i64 {}, 0", x));
            let m = self.ins(format!("sub i64 0, {}", x));
            let mag = self.ins(format!("select i1 {}, i64 {}, i64 {}", neg, m, x));
            (self.ins(format!("zext i1 {} to i64", neg)), mag)
        } else {
            ("0".to_string(), x.to_string())
        };
        let ws = self.ins(format!("call {}{{ i64, i64, i64, i64 }} @isc_fn_{}(i64 {})", self.fn_cc(), words, mag));
        let part = |g: &mut Self, k: u32| g.ins(format!("extractvalue {{ i64, i64, i64, i64 }} {}, {}", ws, k));
        let (w0, w1, w2, n) = (part(self, 0), part(self, 1), part(self, 2), part(self, 3));
        self.decls.insert("declare i64 @llvm.smax.i64(i64, i64)".to_string());
        let size = self.ins(format!("call i64 @llvm.smax.i64(i64 {}, i64 25)", len));
        let dst = self.ins(format!("call noalias ptr @isc_alloc(i64 {})", size));
        let tn = self.ins(format!("add i64 {}, {}", n, neg));
        let total = self.ins(format!("call i64 @llvm.smax.i64(i64 {}, i64 {})", len, tn));
        let gap = self.ins(format!("sub i64 {}, {}", total, tn));
        let (tp, fp) = if start {
            (self.ins(format!("getelementptr inbounds i8, ptr {}, i64 {}", dst, gap)), dst.clone())
        } else {
            (dst.clone(), self.ins(format!("getelementptr inbounds i8, ptr {}, i64 {}", dst, tn)))
        };
        // the `-` first, the digits over it when there is none
        self.emit(format!("store i8 45, ptr {}, align 1", tp));
        let d = self.ins(format!("getelementptr inbounds i8, ptr {}, i64 {}", tp, neg));
        let nm1 = self.ins(format!("sub i64 {}, 1", n));
        self.put_tail(&d, &w0, &w1, &w2, &n, &nm1);
        self.fill_short(&fp, &(c as i8).to_string(), &gap);
        Ok(self.arr(&dst, &total))
    }

    /// The bytes of the join of the `n` strings at `xs` by a separator of
    /// `m` bytes (D607): their lengths added, and the separators between
    /// them.
    fn join_total(&mut self, xs: &str, n: &str, m: &str) -> R<String> {
        let sum = self.alloca("i64");
        self.emit(format!("store i64 0, ptr {}", sum));
        self.for_range("0", n, 1, &mut |g, i| {
            let lp = g.ins(format!("getelementptr inbounds {{ ptr, i64 }}, ptr {}, i64 {}, i32 1", xs, i));
            let len = g.ins(format!("load i64, ptr {}", lp));
            let s = g.ins(format!("load i64, ptr {}", sum));
            let s2 = g.ins(format!("add i64 {}, {}", s, len));
            g.emit(format!("store i64 {}, ptr {}", s2, sum));
            Ok(())
        })?;
        let s = self.ins(format!("load i64, ptr {}", sum));
        let several = self.ins(format!("icmp sgt i64 {}, 1", n));
        let n1 = self.ins(format!("sub i64 {}, 1", n));
        let between = self.ins(format!("select i1 {}, i64 {}, i64 0", several, n1));
        let seps = self.ins(format!("mul i64 {}, {}", between, m));
        Ok(self.ins(format!("add i64 {}, {}", s, seps)))
    }

    /// The strings `from .. to` of the array `xs` written at `dst` one after
    /// the other, each after the separator (`q`, `m`; `lit` its text when
    /// the compiler knows it) but the first of all (D607): a text of at
    /// most 64 bytes by moves, a longer one by `memcpy` (`copy_short`); a
    /// separator known here by a copy of a constant length (a store), none
    /// for an empty one.
    #[allow(clippy::too_many_arguments)]
    fn join_write(&mut self, dst: &str, xs: &str, from: &str, to: &str, q: &str, m: &str, lit: Option<&str>) -> R<()> {
        let at = self.alloca("ptr");
        self.emit(format!("store ptr {}, ptr {}", dst, at));
        self.for_range_scalar(from, to, 1, &mut |g, i| {
            if lit != Some("") {
                let (lsep, ltext) = (g.label(), g.label());
                let later = g.ins(format!("icmp sgt i64 {}, 0", i));
                g.cbr(&later, &lsep, &ltext);
                g.block(&lsep);
                let a = g.ins(format!("load ptr, ptr {}", at));
                match lit {
                    Some(t) => {
                        let (c, k) = g.str_const(t);
                        g.emit(format!("call void @llvm.memcpy.p0.p0.i64(ptr {}, ptr {}, i64 {}, i1 false)", a, c, k));
                    }
                    None => g.copy_short(&a, q, m),
                }
                let a2 = g.ins(format!("getelementptr inbounds i8, ptr {}, i64 {}", a, m));
                g.emit(format!("store ptr {}, ptr {}", a2, at));
                g.br(&ltext);
                g.block(&ltext);
            }
            let a = g.ins(format!("load ptr, ptr {}", at));
            let sp = g.ins(format!("getelementptr inbounds {{ ptr, i64 }}, ptr {}, i64 {}, i32 0", xs, i));
            let p = g.ins(format!("load ptr, ptr {}", sp));
            let lp = g.ins(format!("getelementptr inbounds {{ ptr, i64 }}, ptr {}, i64 {}, i32 1", xs, i));
            let len = g.ins(format!("load i64, ptr {}", lp));
            g.copy_short(&a, &p, &len);
            let a3 = g.ins(format!("getelementptr inbounds i8, ptr {}, i64 {}", a, len));
            g.emit(format!("store ptr {}, ptr {}", a3, at));
            Ok(())
        })
    }

    /// `join(xs, sep)` (D607), made here as the runtime made it: one block
    /// of the lengths of the `n` strings at `xs` and of the separators
    /// between them (`q`, `m`; `lit` its text when the compiler knows it),
    /// the strings written one after the other, each after its separator
    /// (`join_write`); a result of 1 MiB or more, of two strings or more, by
    /// ranges of its strings in parallel (`isc_join_large`, D427).
    fn join_text(&mut self, xs: &str, n: &str, q: &str, m: &str, lit: Option<&str>) -> R<V> {
        // (a mark for the tests)
        self.emit("; text join".to_string());
        let total = self.join_total(xs, n, m)?;
        let dst = self.ins(format!("call noalias ptr @isc_alloc(i64 {})", total));
        let large = self.ins(format!("icmp sge i64 {}, {}", total, JOIN_PARALLEL_MIN));
        let several = self.ins(format!("icmp sge i64 {}, 2", n));
        let by_ranges = self.ins(format!("and i1 {}, {}", large, several));
        let (lranges, lwhole, ldone) = (self.label(), self.label(), self.label());
        self.cbr(&by_ranges, &lranges, &lwhole);
        self.block(&lranges);
        self.join_fns()?;
        self.emit(format!("call void @isc_join_large(ptr {}, ptr {}, i64 {}, ptr {}, i64 {}, i64 {})", dst, xs, n, q, m, total));
        self.br(&ldone);
        self.block(&lwhole);
        self.join_write(&dst, xs, "0", n, q, m, lit)?;
        self.br(&ldone);
        self.block(&ldone);
        Ok(self.arr(&dst, &total))
    }

    /// A function of the module written by `body` with the emission
    /// helpers, apart from the function being written (kept aside
    /// meanwhile); `body` reads the parameters of `header` by their names.
    fn helper_fn(&mut self, header: &str, body: &mut dyn FnMut(&mut Self) -> R<()>) -> R<()> {
        let saved = std::mem::take(&mut self.f);
        let r = body(self);
        let text = self.finish(header);
        self.f = saved;
        r?;
        self.funcs.push(text);
        Ok(())
    }

    /// The functions of the joins, emitted once in a program that needs
    /// them (D607), as the runtime had them (D427, D428):
    /// - `isc_join_write(dst, xs, from, to, q, m)`: `join_write`, called;
    /// - `isc_join_ranges(xs, n, m, total, first, place)`: the ranges of the
    ///   join of the `n` strings `xs` by a separator of `m` bytes (`total`
    ///   bytes) made in parallel: the first string of each into `first` and
    ///   the place of the separator before it into `place` (257 slots
    ///   each), a range each `total / k` bytes about (`k` four a thread, at
    ///   most 256); their count, `first[count]` the number of strings and
    ///   `place[count]` the length of the join;
    /// - `isc_join_task(ctx, k)`: the range `k` written at its place;
    /// - `isc_join_large(dst, xs, n, q, m, total)`: a large join, written by
    ///   its ranges on the pool (each thread its own pages), or whole on one
    ///   thread;
    /// - `isc_join_next(cur, xs, q, m, buf, cap)`: the next bytes of the
    ///   join from the cursor `cur` (the string `i`, the byte `off` of it
    ///   with the separator before it, but the first string, the string
    ///   `end` where its range ends) to the end of its range, at most `cap`
    ///   of them, written into `buf`: their count (0 at the end); the cursor
    ///   moved past them.
    fn join_fns(&mut self) -> R<()> {
        if self.join_fns {
            return Ok(());
        }
        self.join_fns = true;
        let attrs = self.attrs();
        self.helper_fn(&format!("define internal void @isc_join_write(ptr %dst, ptr %xs, i64 %from, i64 %to, ptr %q, i64 %m){}", attrs), &mut |g| {
            g.join_write("%dst", "%xs", "%from", "%to", "%q", "%m", None)?;
            g.emit("ret void");
            Ok(())
        })?;
        self.helper_fn(&format!("define internal i64 @isc_join_ranges(ptr %xs, i64 %n, i64 %m, i64 %total, ptr %first, ptr %place){}", attrs), &mut |g| {
            g.decls.insert("declare i64 @llvm.smax.i64(i64, i64)".to_string());
            g.decls.insert("declare i64 @llvm.smin.i64(i64, i64)".to_string());
            let t = g.ins("call i64 @isc_thread_count()".to_string());
            let t4 = g.ins(format!("shl i64 {}, 2", t));
            let k1 = g.ins(format!("call i64 @llvm.smax.i64(i64 {}, i64 1)", t4));
            let k = g.ins(format!("call i64 @llvm.smin.i64(i64 {}, i64 256)", k1));
            let step = g.ins(format!("udiv i64 %total, {}", k));
            g.emit("store i64 0, ptr %first");
            g.emit("store i64 0, ptr %place");
            let cut = g.alloca("i64");
            // where the bytes of the string `i` start
            let at = g.alloca("i64");
            g.emit(format!("store i64 1, ptr {}", cut));
            g.emit(format!("store i64 0, ptr {}", at));
            g.for_range_scalar("0", "%n", 1, &mut |g, i| {
                let c = g.ins(format!("load i64, ptr {}", cut));
                let a = g.ins(format!("load i64, ptr {}", at));
                let later = g.ins(format!("icmp sgt i64 {}, 0", i));
                let room = g.ins(format!("icmp slt i64 {}, {}", c, k));
                let bound = g.ins(format!("mul i64 {}, {}", step, c));
                let reached = g.ins(format!("icmp sge i64 {}, {}", a, bound));
                let c0 = g.ins(format!("and i1 {}, {}", later, room));
                let cuts = g.ins(format!("and i1 {}, {}", c0, reached));
                let (lcut, lnext) = (g.label(), g.label());
                g.cbr(&cuts, &lcut, &lnext);
                // a range starts at the string `i`, its place the separator
                // before it
                g.block(&lcut);
                let fs = g.ins(format!("getelementptr inbounds i64, ptr %first, i64 {}", c));
                g.emit(format!("store i64 {}, ptr {}", i, fs));
                let ps = g.ins(format!("getelementptr inbounds i64, ptr %place, i64 {}", c));
                let before = g.ins(format!("sub i64 {}, %m", a));
                g.emit(format!("store i64 {}, ptr {}", before, ps));
                let c1 = g.ins(format!("add i64 {}, 1", c));
                g.emit(format!("store i64 {}, ptr {}", c1, cut));
                g.br(&lnext);
                g.block(&lnext);
                let lp = g.ins(format!("getelementptr inbounds {{ ptr, i64 }}, ptr %xs, i64 {}, i32 1", i));
                let len = g.ins(format!("load i64, ptr {}", lp));
                let i1 = g.ins(format!("add i64 {}, 1", i));
                let more = g.ins(format!("icmp slt i64 {}, %n", i1));
                let sep = g.ins(format!("select i1 {}, i64 %m, i64 0", more));
                let a1 = g.ins(format!("add i64 {}, {}", a, len));
                let a2 = g.ins(format!("add i64 {}, {}", a1, sep));
                g.emit(format!("store i64 {}, ptr {}", a2, at));
                Ok(())
            })?;
            let c = g.ins(format!("load i64, ptr {}", cut));
            let fs = g.ins(format!("getelementptr inbounds i64, ptr %first, i64 {}", c));
            g.emit(format!("store i64 %n, ptr {}", fs));
            let ps = g.ins(format!("getelementptr inbounds i64, ptr %place, i64 {}", c));
            g.emit(format!("store i64 %total, ptr {}", ps));
            g.emit(format!("ret i64 {}", c));
            Ok(())
        })?;
        // the context of the tasks of a large join: the block, the strings,
        // the separator, the ranges
        let ctx_ty = "{ ptr, ptr, ptr, i64, ptr, ptr }";
        self.helper_fn(&format!("define internal void @isc_join_task(ptr %ctx, i64 %k){}", attrs), &mut |g| {
            let field = |g: &mut Self, i: u32, t: &str| {
                let p = g.ins(format!("getelementptr inbounds {}, ptr %ctx, i32 0, i32 {}", ctx_ty, i));
                g.ins(format!("load {}, ptr {}", t, p))
            };
            let (dst, xs, q, m) = (field(g, 0, "ptr"), field(g, 1, "ptr"), field(g, 2, "ptr"), field(g, 3, "i64"));
            let (first, place) = (field(g, 4, "ptr"), field(g, 5, "ptr"));
            let f0 = g.ins(format!("getelementptr inbounds i64, ptr {}, i64 %k", first));
            let from = g.ins(format!("load i64, ptr {}", f0));
            let k1 = g.ins("add i64 %k, 1".to_string());
            let f1 = g.ins(format!("getelementptr inbounds i64, ptr {}, i64 {}", first, k1));
            let to = g.ins(format!("load i64, ptr {}", f1));
            let p0 = g.ins(format!("getelementptr inbounds i64, ptr {}, i64 %k", place));
            let at = g.ins(format!("load i64, ptr {}", p0));
            let d = g.ins(format!("getelementptr inbounds i8, ptr {}, i64 {}", dst, at));
            g.emit(format!("call void @isc_join_write(ptr {}, ptr {}, i64 {}, i64 {}, ptr {}, i64 {})", d, xs, from, to, q, m));
            g.emit("ret void");
            Ok(())
        })?;
        self.helper_fn(&format!("define internal void @isc_join_large(ptr %dst, ptr %xs, i64 %n, ptr %q, i64 %m, i64 %total){} cold noinline", attrs), &mut |g| {
            let t = g.ins("call i64 @isc_thread_count()".to_string());
            let one = g.ins(format!("icmp sle i64 {}, 1", t));
            let (lwhole, lranges) = (g.label(), g.label());
            g.cbr(&one, &lwhole, &lranges);
            g.block(&lwhole);
            g.emit("call void @isc_join_write(ptr %dst, ptr %xs, i64 0, i64 %n, ptr %q, i64 %m)");
            g.emit("ret void");
            g.block(&lranges);
            let first = g.alloca("[257 x i64]");
            let place = g.alloca("[257 x i64]");
            let ranges = g.ins(format!("call i64 @isc_join_ranges(ptr %xs, i64 %n, i64 %m, i64 %total, ptr {}, ptr {})", first, place));
            let ctx = g.alloca(ctx_ty);
            for (i, (t, x)) in [("ptr", "%dst"), ("ptr", "%xs"), ("ptr", "%q"), ("i64", "%m"), ("ptr", first.as_str()), ("ptr", place.as_str())].into_iter().enumerate() {
                let p = g.ins(format!("getelementptr inbounds {}, ptr {}, i32 0, i32 {}", ctx_ty, ctx, i));
                g.emit(format!("store {} {}, ptr {}", t, x, p));
            }
            let end = g.ins(format!("getelementptr {}, ptr null, i32 1", ctx_ty));
            let bytes = g.ins(format!("ptrtoint ptr {} to i64", end));
            g.emit(format!("call void @isc_parallel_for_fill(i64 {}, ptr @isc_join_task, ptr {}, i64 {})", ranges, ctx, bytes));
            g.emit("ret void");
            Ok(())
        })?;
        let cur_ty = "{ i64, i64, i64 }";
        self.helper_fn(&format!("define internal i64 @isc_join_next(ptr %cur, ptr %xs, ptr %q, i64 %m, ptr %buf, i64 %cap){}", attrs), &mut |g| {
            g.decls.insert("declare i64 @llvm.smin.i64(i64, i64)".to_string());
            let ip = g.ins(format!("getelementptr inbounds {}, ptr %cur, i32 0, i32 0", cur_ty));
            let op = g.ins(format!("getelementptr inbounds {}, ptr %cur, i32 0, i32 1", cur_ty));
            let ep = g.ins(format!("getelementptr inbounds {}, ptr %cur, i32 0, i32 2", cur_ty));
            let end = g.ins(format!("load i64, ptr {}", ep));
            let got = g.alloca("i64");
            g.emit(format!("store i64 0, ptr {}", got));
            let (head, lbody, lnext, lstay, ldone) = (g.label(), g.label(), g.label(), g.label(), g.label());
            g.br(&head);
            g.block(&head);
            let have = g.ins(format!("load i64, ptr {}", got));
            let i = g.ins(format!("load i64, ptr {}", ip));
            let room = g.ins(format!("icmp slt i64 {}, %cap", have));
            let left = g.ins(format!("icmp slt i64 {}, {}", i, end));
            let more = g.ins(format!("and i1 {}, {}", room, left));
            g.cbr(&more, &lbody, &ldone);
            g.block(&lbody);
            let sp = g.ins(format!("getelementptr inbounds {{ ptr, i64 }}, ptr %xs, i64 {}, i32 0", i));
            let p = g.ins(format!("load ptr, ptr {}", sp));
            let lp = g.ins(format!("getelementptr inbounds {{ ptr, i64 }}, ptr %xs, i64 {}, i32 1", i));
            let len = g.ins(format!("load i64, ptr {}", lp));
            // the separator before every string but the first, then the string
            let later = g.ins(format!("icmp sgt i64 {}, 0", i));
            let sl = g.ins(format!("select i1 {}, i64 %m, i64 0", later));
            let off = g.ins(format!("load i64, ptr {}", op));
            let in_sep = g.ins(format!("icmp slt i64 {}, {}", off, sl));
            let from = g.ins(format!("select i1 {}, ptr %q, ptr {}", in_sep, p));
            let past = g.ins(format!("sub i64 {}, {}", off, sl));
            let a = g.ins(format!("select i1 {}, i64 {}, i64 {}", in_sep, off, past));
            let flen = g.ins(format!("select i1 {}, i64 {}, i64 {}", in_sep, sl, len));
            let rest = g.ins(format!("sub i64 {}, {}", flen, a));
            let space = g.ins(format!("sub i64 %cap, {}", have));
            let k = g.ins(format!("call i64 @llvm.smin.i64(i64 {}, i64 {})", rest, space));
            let src = g.ins(format!("getelementptr inbounds i8, ptr {}, i64 {}", from, a));
            let to = g.ins(format!("getelementptr inbounds i8, ptr %buf, i64 {}", have));
            g.copy_short(&to, &src, &k);
            let have2 = g.ins(format!("add i64 {}, {}", have, k));
            g.emit(format!("store i64 {}, ptr {}", have2, got));
            let off2 = g.ins(format!("add i64 {}, {}", off, k));
            let whole = g.ins(format!("add i64 {}, {}", sl, len));
            let done = g.ins(format!("icmp eq i64 {}, {}", off2, whole));
            g.cbr(&done, &lnext, &lstay);
            g.block(&lnext);
            let i1 = g.ins(format!("add i64 {}, 1", i));
            g.emit(format!("store i64 {}, ptr {}", i1, ip));
            g.emit(format!("store i64 0, ptr {}", op));
            g.br(&head);
            g.block(&lstay);
            g.emit(format!("store i64 {}, ptr {}", off2, op));
            g.br(&head);
            g.block(&ldone);
            g.emit(format!("ret i64 {}", have));
            Ok(())
        })
    }

    /// The string `p`, `n` starts (`start`) or ends with the bytes `t` (at
    /// most 16): at least as long, and its first or last bytes, read as
    /// one integer (up to 8) or two that overlap, equal to those of `t`.
    /// With `exact`, the string is `t`: as long as `t`, and those bytes.
    fn literal_affix(&mut self, p: &str, n: &str, t: &[u8], start: bool, exact: bool) -> V {
        if t.is_empty() {
            return if exact { v(self.ins(format!("icmp eq i64 {}, 0", n)), "i1") } else { v("true", "i1") };
        }
        let l = t.len();
        let word = |b: &[u8]| -> String {
            let mut x: u128 = 0;
            for (k, &c) in b.iter().enumerate() {
                x |= (c as u128) << (8 * k);
            }
            x.to_string()
        };
        let long = self.ins(format!("icmp {} i64 {}, {}", if exact { "eq" } else { "sge" }, n, l));
        let (lcheck, ljoin) = (self.label(), self.label());
        let lfrom = self.current_label("entry");
        self.cbr(&long, &lcheck, &ljoin);
        self.block(&lcheck);
        let base = if start {
            p.to_string()
        } else {
            let off = self.ins(format!("sub i64 {}, {}", n, l));
            self.ins(format!("getelementptr inbounds i8, ptr {}, i64 {}", p, off))
        };
        let same = if l <= 8 {
            let ty = format!("i{}", 8 * l);
            let x = self.ins(format!("load {}, ptr {}, align 1", ty, base));
            self.ins(format!("icmp eq {} {}, {}", ty, x, word(t)))
        } else {
            let x0 = self.ins(format!("load i64, ptr {}, align 1", base));
            let at = self.ins(format!("getelementptr inbounds i8, ptr {}, i64 {}", base, l - 8));
            let x1 = self.ins(format!("load i64, ptr {}, align 1", at));
            let c0 = self.ins(format!("icmp eq i64 {}, {}", x0, word(&t[..8])));
            let c1 = self.ins(format!("icmp eq i64 {}, {}", x1, word(&t[l - 8..])));
            self.ins(format!("and i1 {}, {}", c0, c1))
        };
        let lend = self.current_label(&lcheck);
        self.br(&ljoin);
        self.block(&ljoin);
        v(self.ins(format!("phi i1 [ false, %{} ], [ {}, %{} ]", lfrom, same, lend)), "i1")
    }

    /// `s.slice(a, b)` of a string read where it is (D303): `a` and `b`
    /// checked as the runtime does (`0 <= a <= b <= n`, each at the start of
    /// a character or at the end), then the string at `p + a` of `b - a`
    /// bytes, its pointer inside `s`; a failed check calls the slice of the
    /// runtime, which stops the program with its message.
    fn str_slice_view(&mut self, p: &str, n: &str, a: &str, b: &str, span: Span) -> V {
        let c0 = self.ins(format!("icmp sge i64 {}, 0", a));
        let c1 = self.ins(format!("icmp sle i64 {}, {}", a, b));
        let c2 = self.ins(format!("icmp sle i64 {}, {}", b, n));
        let c01 = self.ins(format!("and i1 {}, {}", c0, c1));
        let ok = self.ins(format!("and i1 {}, {}", c01, c2));
        let (la, lload_a, lb, lload_b, lbad, lview) =
            (self.label(), self.label(), self.label(), self.label(), self.label(), self.label());
        self.cbr(&ok, &la, &lbad);
        // a byte of continuation (0x80 to 0xBF) is inside a character
        for (i, here, load, next) in [(a, &la, &lload_a, &lb), (b, &lb, &lload_b, &lview)] {
            self.block(here);
            let inside = self.ins(format!("icmp slt i64 {}, {}", i, n));
            self.cbr(&inside, load, next);
            self.block(load);
            let at = self.ins(format!("getelementptr inbounds i8, ptr {}, i64 {}", p, i));
            let byte = self.ins(format!("load i8, ptr {}, align 1", at));
            let starts = self.ins(format!("icmp sge i8 {}, -64", byte));
            self.cbr(&starts, next, &lbad);
        }
        self.block(&lbad);
        self.emit(format!("call void @isc_panic_text_slice(ptr {}, i64 {}, i64 {}, i64 {}, {})", p, n, a, b, Self::pos(span)));
        self.emit("unreachable");
        self.block(&lview);
        let q = self.ins(format!("getelementptr inbounds i8, ptr {}, i64 {}", p, a));
        let len = self.ins(format!("sub i64 {}, {}", b, a));
        let s0 = self.ins(format!("insertvalue {{ ptr, i64 }} poison, ptr {}, 0", q));
        v(self.ins(format!("insertvalue {{ ptr, i64 }} {}, i64 {}, 1", s0, len)), "{ ptr, i64 }")
    }

    /// A new block holding the `n` bytes at `p`, the copy of a text (D592):
    /// `isc_alloc` (the empty block of the runtime for none), then the
    /// bytes copied.
    fn text_copy(&mut self, p: &str, n: &str) -> V {
        // (a mark for the tests that count the copies of texts)
        self.emit("; text copy".to_string());
        let dst = self.ins(format!("call noalias ptr @isc_alloc(i64 {})", n));
        self.emit(format!("call void @llvm.memcpy.p0.p0.i64(ptr {}, ptr {}, i64 {}, i1 false)", dst, p, n));
        self.arr(&dst, n)
    }

    /// The texts `parts` (pointer, length) one after the other in a new
    /// block of the sum of their lengths (D592).
    fn text_concat(&mut self, parts: &[(String, String)]) -> V {
        // (a mark for the tests: the texts of a literal are written
        // otherwise, D368)
        self.emit("; text concat".to_string());
        let mut total = parts[0].1.clone();
        for (_, n) in &parts[1..] {
            total = self.ins(format!("add i64 {}, {}", total, n));
        }
        let dst = self.ins(format!("call noalias ptr @isc_alloc(i64 {})", total));
        let mut at = dst.clone();
        for (k, (p, n)) in parts.iter().enumerate() {
            self.emit(format!("call void @llvm.memcpy.p0.p0.i64(ptr {}, ptr {}, i64 {}, i1 false)", at, p, n));
            if k + 1 < parts.len() {
                at = self.ins(format!("getelementptr inbounds i8, ptr {}, i64 {}", at, n));
            }
        }
        self.arr(&dst, &total)
    }

    /// The `memcmp` of the `k` bytes at `p` and at `q` (D592): its count
    /// in the word of the machine (32 bits in WebAssembly, where a call of
    /// another signature than the function's is a trap).
    fn memcmp(&mut self, p: &str, q: &str, k: &str) -> String {
        if self.opts.isa.is_wasm() {
            self.decls.insert("declare i32 @memcmp(ptr, ptr, i32)".into());
            let k32 = self.ins(format!("trunc i64 {} to i32", k));
            self.ins(format!("call i32 @memcmp(ptr {}, ptr {}, i32 {})", p, q, k32))
        } else {
            self.decls.insert("declare i32 @memcmp(ptr, ptr, i64)".into());
            self.ins(format!("call i32 @memcmp(ptr {}, ptr {}, i64 {})", p, q, k))
        }
    }

    /// -1, 0 or 1: the order of the bytes of two texts (D592): `memcmp` of
    /// the bytes both have, then the lengths.
    fn text_order(&mut self, p: &str, n: &str, q: &str, m: &str) -> String {
        let shorter = self.ins(format!("icmp slt i64 {}, {}", n, m));
        let k = self.ins(format!("select i1 {}, i64 {}, i64 {}", shorter, n, m));
        let c = self.memcmp(p, q, &k);
        let neg = self.ins(format!("icmp slt i32 {}, 0", c));
        let pos = self.ins(format!("icmp sgt i32 {}, 0", c));
        let longer = self.ins(format!("icmp sgt i64 {}, {}", n, m));
        // by the lengths when the common bytes are the same
        let l1 = self.ins(format!("select i1 {}, i32 1, i32 0", longer));
        let by_len = self.ins(format!("select i1 {}, i32 -1, i32 {}", shorter, l1));
        let s1 = self.ins(format!("select i1 {}, i32 1, i32 {}", pos, by_len));
        self.ins(format!("select i1 {}, i32 -1, i32 {}", neg, s1))
    }

    /// An `i1`: the `m` bytes at `q` are those at `p` (D592).
    fn text_same(&mut self, p: &str, q: &str, m: &str) -> String {
        let c = self.memcmp(p, q, m);
        self.ins(format!("icmp eq i32 {}, 0", c))
    }

    /// `s.toUpperCase()` (`upper`) or `s.toLowerCase()` of the text `p`,
    /// `n`: the call of the function of this module that makes it
    /// (`text_case_fn`), with the buffer `slot` of the stack for a text
    /// only read in its scope (D501). The call is always inlined: its
    /// function then has no frame of its own to make for a short text
    /// (called, 64 instructions a line of 60 bytes and 142 a word of 8 bytes
    /// kept; inlined, 53 and 125).
    fn text_case(&mut self, p: &str, n: &str, upper: bool, slot: Option<String>) -> R<V> {
        let name = self.text_case_fn(upper, slot.is_some())?;
        let buf = slot.map(|b| format!(", ptr {}", b)).unwrap_or_default();
        Ok(v(self.ins(format!("call {}{{ ptr, i64 }} @{}(ptr {}, i64 {}{}) alwaysinline", self.fn_cc(), name, p, n, buf)), "{ ptr, i64 }"))
    }

    /// The function of the module that makes a text in upper (`upper`) or
    /// lower case, written once: `{ ptr, i64 } (ptr %p, i64 %n)`, and with
    /// `into` a third parameter `%buf`, a buffer of `STACK_STR` bytes of
    /// the stack of its caller, where a result of 1 to `STACK_STR` bytes is
    /// written (another one: a block, by the first function).
    ///
    /// Its result for a text of ASCII: a copy whose letters are in the
    /// other case, by vectors, no byte one at a time past the first 3: a
    /// text of a vector of the target or more (32 bytes, 16 without AVX2)
    /// by whole vectors from its start, then one over its end; a shorter
    /// one by two vectors of 16, 8 or 4 bytes, at its start and over its end
    /// (the text and the result are two places: a byte written twice is
    /// written the same). The bytes read are joined by `or`: one test says
    /// whether one was not ASCII. Such a text is finished by the prelude
    /// (`$caseText`, lowered for it: the characters of several bytes, by
    /// the tables of Unicode) from the text and that copy, which it only
    /// reads: the buffer, the block (freed after), or, for a text shorter
    /// than a vector without a buffer, a place of the stack: its bytes are
    /// tested before a block is taken for it. (The runtime's `isc_to_case`
    /// and `isc_to_case_into` tested the whole text, then wrote it,
    /// character by character through a formatter, twice, when it was not
    /// ASCII.)
    fn text_case_fn(&mut self, upper: bool, into: bool) -> R<String> {
        let Some(rest) = self.p.case_text else {
            return Err("codegen: a text in another case, its function of the prelude not lowered".into());
        };
        let name = format!("isc_text_{}{}", if upper { "upper" } else { "lower" }, if into { "_into" } else { "" });
        if !self.case_fns.insert((upper, into)) {
            return Ok(name);
        }
        // (the function without a buffer, for the texts that do not fit)
        let plain = if into { self.text_case_fn(upper, false)? } else { String::new() };
        let cc = self.fn_cc();
        let saved_f = std::mem::take(&mut self.f);
        let wide: u32 = if self.opts.isa.vector_registers().0 >= 32 && self.opts.isa != Isa::Avx { 32 } else { 16 };
        // the first letter of the case that changes
        let first = if upper { "97" } else { "65" };
        let (p, n) = ("%p", "%n");
        let vty = |lanes: u32| if lanes == 1 { "i8".to_string() } else { format!("<{} x i8>", lanes) };
        // the `lanes` bytes of the text from `at`
        let read = |g: &mut Self, at: &str, lanes: u32| -> String {
            let sp = g.ins(format!("getelementptr inbounds i8, ptr {}, i64 {}", p, at));
            g.ins(format!("load {}, ptr {}, align 1", vty(lanes), sp))
        };
        // the bytes `x` in the other case, written at `at` of `dst`
        let write = |g: &mut Self, x: &str, dst: &str, at: &str, lanes: u32| {
            let ty = vty(lanes);
            let (bt, zero) = if lanes == 1 { ("i1".to_string(), "0") } else { (format!("<{} x i1>", lanes), "zeroinitializer") };
            let d = g.ins(format!("sub {} {}, {}", ty, x, g.cst("i8", first, lanes)));
            let c = g.ins(format!("icmp ult {} {}, {}", ty, d, g.cst("i8", "26", lanes)));
            let m = g.ins(format!("select {} {}, {} {}, {} {}", bt, c, ty, g.cst("i8", "32", lanes), ty, zero));
            let y = g.ins(format!("xor {} {}, {}", ty, x, m));
            let dp = g.ins(format!("getelementptr inbounds i8, ptr {}, i64 {}", dst, at));
            g.emit(format!("store {} {}, ptr {}, align 1", ty, y, dp));
        };
        // a byte of the `lanes` bytes `x` is not ASCII
        let any_high = |g: &mut Self, x: &str, lanes: u32| -> String {
            if lanes == 1 {
                return g.ins(format!("icmp slt i8 {}, 0", x));
            }
            let neg = g.ins(format!("icmp slt <{} x i8> {}, zeroinitializer", lanes, x));
            let bits = g.ins(format!("bitcast <{} x i1> {} to i{}", lanes, neg, lanes));
            g.ins(format!("icmp ne i{} {}, 0", lanes, bits))
        };
        // the result by the prelude, from the text and its copy at `copy`
        let finish = |g: &mut Self, copy: &str| -> String {
            let text = g.arr(p, n);
            let t = g.arr(copy, n);
            g.ins(format!(
                "call {}{{ ptr, i64 }} @isc_fn_{}({{ ptr, i64 }} {}, {{ ptr, i64 }} {}, i64 0, i64 0, i64 0, i1 {})",
                cc, rest, text.r, t.r, upper
            ))
        };
        let ret = |g: &mut Self, ptr: &str| {
            let r = g.arr(ptr, n);
            g.emit(format!("ret {{ ptr, i64 }} {}", r.r));
        };
        // the place of the copy of a short text that is not ASCII, when no
        // buffer is given
        let scratch = if into { "%buf".to_string() } else { self.alloca(&format!("[{} x i8], align 16", wide)) };
        if into {
            // a result that does not fit: a block, by the other function
            let nm1 = self.ins(format!("sub i64 {}, 1", n));
            let fits = self.ins(format!("icmp ult i64 {}, {}", nm1, STACK_STR));
            let (lfits, lblock) = (self.label(), self.label());
            self.cbr(&fits, &lfits, &lblock);
            self.block(&lblock);
            let r = self.ins(format!("tail call {}{{ ptr, i64 }} @{}(ptr {}, i64 {})", cc, plain, p, n));
            self.emit(format!("ret {{ ptr, i64 }} {}", r));
            self.block(&lfits);
        }
        let (lvec, lloop, ltail, lshort, lother, lrest) = (self.label(), self.label(), self.label(), self.label(), self.label(), self.label());
        let long = self.ins(format!("icmp uge i64 {}, {}", n, wide));
        self.cbr(&long, &lvec, &lshort);
        // a vector or more: whole vectors, then one over the end
        self.block(&lvec);
        let dst = if into { "%buf".to_string() } else { self.ins(format!("call noalias ptr @isc_alloc(i64 {})", n)) };
        let vt = vty(wide);
        let (at, seen) = (self.alloca("i64"), self.alloca(&vt));
        self.emit(format!("store i64 0, ptr {}", at));
        self.emit(format!("store {} zeroinitializer, ptr {}", vt, seen));
        let last = self.ins(format!("sub i64 {}, {}", n, wide));
        self.br(&lloop);
        self.block(&lloop);
        let i = self.ins(format!("load i64, ptr {}", at));
        let x = read(self, &i, wide);
        write(self, &x, &dst, &i, wide);
        let so_far = self.ins(format!("load {}, ptr {}", vt, seen));
        let joined = self.ins(format!("or {} {}, {}", vt, so_far, x));
        self.emit(format!("store {} {}, ptr {}", vt, joined, seen));
        let next = self.ins(format!("add nuw nsw i64 {}, {}", i, wide));
        self.emit(format!("store i64 {}, ptr {}", next, at));
        let more = self.ins(format!("icmp sle i64 {}, {}", next, last));
        self.cbr(&more, &lloop, &ltail);
        self.block(&ltail);
        let x = read(self, &last, wide);
        write(self, &x, &dst, &last, wide);
        let so_far = self.ins(format!("load {}, ptr {}", vt, seen));
        let joined = self.ins(format!("or {} {}, {}", vt, so_far, x));
        let h = any_high(self, &joined, wide);
        let lascii = self.label();
        self.cbr(&h, &lother, &lascii);
        self.block(&lascii);
        ret(self, &dst);
        // not ASCII: by the prelude, the block of the copy then freed
        self.block(&lother);
        let r = finish(self, &dst);
        if !into {
            self.emit(format!("call void @isc_free(ptr {})", dst));
        }
        self.emit(format!("ret {{ ptr, i64 }} {}", r));
        // shorter: two vectors of half a vector, or a quarter, down to 4
        // bytes, the first that the text fills; without a buffer, its bytes
        // tested before a block is taken
        self.block(&lshort);
        let mut lanes = wide / 2;
        loop {
            let (lthis, lnext, lplain, lcopy) = (self.label(), self.label(), self.label(), self.label());
            let fills = if lanes >= 4 { self.ins(format!("icmp uge i64 {}, {}", n, lanes)) } else { self.ins(format!("icmp ne i64 {}, 0", n)) };
            self.cbr(&fills, &lthis, &lnext);
            self.block(&lthis);
            // (below 4 bytes: the first, the middle and the last)
            let (places, width) = if lanes >= 4 {
                let off = self.ins(format!("sub i64 {}, {}", n, lanes));
                (vec!["0".to_string(), off], lanes)
            } else {
                let half = self.ins(format!("lshr i64 {}, 1", n));
                let end = self.ins(format!("sub i64 {}, 1", n));
                (vec!["0".to_string(), half, end], 1)
            };
            let xs: Vec<String> = places.iter().map(|at| read(self, at, width)).collect();
            let mut all = xs[0].clone();
            for x in &xs[1..] {
                all = self.ins(format!("or {} {}, {}", vty(width), all, x));
            }
            let h = any_high(self, &all, width);
            self.cbr(&h, &lcopy, &lplain);
            self.block(&lplain);
            // (after the call that takes a block, its bytes read again
            // rather than kept through it)
            let (dst, xs2) = if into {
                ("%buf".to_string(), xs.clone())
            } else {
                let block = self.ins(format!("call noalias ptr @isc_alloc(i64 {})", n));
                let again: Vec<String> = places.iter().map(|at| read(self, at, width)).collect();
                (block, again)
            };
            for (x, at) in xs2.iter().zip(&places) {
                write(self, x, &dst, at, width);
            }
            ret(self, &dst);
            self.block(&lcopy);
            for (x, at) in xs.iter().zip(&places) {
                write(self, x, &scratch, at, width);
            }
            self.br(&lrest);
            self.block(&lnext);
            if lanes < 4 {
                break;
            }
            lanes /= 2;
        }
        // no byte: the empty text (never with a buffer: it has a byte)
        let none = self.ins("call noalias ptr @isc_alloc(i64 0)".to_string());
        ret(self, &none);
        self.block(&lrest);
        let r = finish(self, &scratch);
        self.emit(format!("ret {{ ptr, i64 }} {}", r));
        let params = if into { "ptr %p, i64 %n, ptr %buf" } else { "ptr %p, i64 %n" };
        let attrs = self.attrs();
        let text = self.finish(&format!("define internal {}{{ ptr, i64 }} @{}({}){}", cc, name, params, attrs));
        self.f = saved_f;
        self.funcs.push(text);
        Ok(name)
    }

    /// The text of the integer `x` (an `i64`, or a `u64` when not `signed`)
    /// by the function of the prelude lowered for it (D588).
    fn int_text(&mut self, x: &str, signed: bool) -> R<V> {
        let Some((s, u)) = self.p.int_text else {
            return Err("codegen: the text of an integer, its function of the prelude not lowered".into());
        };
        self.text_called.insert(if signed { s } else { u });
        let call = format!("call {}{{ ptr, i64 }} @isc_fn_{}(i64 {})", self.fn_cc(), if signed { s } else { u }, x);
        Ok(v(self.ins(call), "{ ptr, i64 }"))
    }

    /// The last `n` bytes (1 to 24) of the 24 bytes of three words written
    /// at `at` by whole words (`$fixedText`): the first word that holds
    /// some of them shifted to its bytes, the words after it whole. Up to
    /// 8 bytes, the word shifted writes 8: those past `n` are zeros.
    fn words_tail(&mut self, at: &str, w: [&str; 3], n: &str) {
        // `x` stored `off` bytes after `at`
        let put = |g: &mut Self, off: &str, x: &str| {
            let p = g.ins(format!("getelementptr inbounds i8, ptr {}, i64 {}", at, off));
            g.emit(format!("store i64 {}, ptr {}, align 1", x, p));
        };
        // `x` without its first `bytes - n` bytes
        let shifted = |g: &mut Self, x: &str, bytes: u32| {
            let s = g.ins(format!("sub i64 {}, {}", bytes, n));
            let b = g.ins(format!("shl i64 {}, 3", s));
            g.ins(format!("lshr i64 {}, {}", x, b))
        };
        let (lbig, lsmall, l16, l9, lpair, ljoin) = (self.label(), self.label(), self.label(), self.label(), self.label(), self.label());
        let gt8 = self.ins(format!("icmp sgt i64 {}, 8", n));
        self.cbr(&gt8, &lbig, &lsmall);
        self.block(&lbig);
        let gt16 = self.ins(format!("icmp sgt i64 {}, 16", n));
        self.cbr(&gt16, &l16, &l9);
        self.block(&l16);
        let h = shifted(self, w[0], 24);
        put(self, "0", &h);
        let at16 = self.ins(format!("sub i64 {}, 16", n));
        put(self, &at16, w[1]);
        self.br(&lpair);
        self.block(&l9);
        let m = shifted(self, w[1], 16);
        put(self, "0", &m);
        self.br(&lpair);
        self.block(&lpair);
        let at8 = self.ins(format!("sub i64 {}, 8", n));
        put(self, &at8, w[2]);
        self.br(&ljoin);
        self.block(&lsmall);
        let t = shifted(self, w[2], 8);
        put(self, "0", &t);
        self.br(&ljoin);
        self.block(&ljoin);
    }

    fn str_op(&mut self, e: &Expr, op: isc_ir::StrOp, es: &[Expr]) -> R<V> {
        use isc_ir::StrOp as S;
        if matches!(op, S::Group | S::GroupCounts) && group_stream_ok(&es[0]) {
            return self.group_stream(&es[0], op == S::GroupCounts);
        }
        // the text of a boolean (D591): `true` or `false` stored as one word
        // into a block of 8 bytes, its first 4 or 5 the text
        if let (S::ToString, Type::Bool) = (op, &es[0].ty) {
            let x = self.ex(&es[0], 1)?;
            let dst = self.ins("call noalias ptr @isc_alloc(i64 8)".to_string());
            let w = self.ins(format!("select i1 {}, i64 1702195828, i64 435728179558", x.r));
            self.emit(format!("store i64 {}, ptr {}, align 8", w, dst));
            let n = self.ins(format!("select i1 {}, i64 4, i64 5", x.r));
            return Ok(self.arr(&dst, &n));
        }
        // the text of an integer that no fusion writes in place: the
        // function of the prelude that writes it (D588), its argument made
        // an `i64` or a `u64`
        if let (S::ToString, Type::Int(k), Some((signed, unsigned))) = (op, &es[0].ty, self.p.int_text) {
            let wide = Type::Int(if k.signed() { IntKind::I64 } else { IntKind::U64 });
            let x = if k.bits() < 64 { Expr::new(ExprKind::Convert(Box::new(es[0].clone())), wide, es[0].span) } else { es[0].clone() };
            let f = if k.signed() { signed } else { unsigned };
            let call = Expr::new(ExprKind::Call(f, vec![x]), Type::Str, e.span);
            return self.ex(&call, 1);
        }
        // the text of an integer padded (`int_texts`, D348): with a fill of
        // one byte, written in one block (`pad_int`); otherwise the text
        // made here, padded, then freed
        if let (S::PadStart | S::PadEnd, ExprKind::Var(t, _)) = (op, &es[0].kind) {
            if let Some(IntText { x, signed, .. }) = self.int_texts.get(t).cloned() {
                let len = self.ex(&es[1], 1)?;
                let fill = self.ex(&es[2], 1)?;
                let start = (op == S::PadStart) as i32;
                // described for the concatenation that reads it (D359)
                if self.defer_pad == Some(e as *const Expr) {
                    if let Some(f) = self.literal_of(&es[2]).filter(|f| f.len() == 1) {
                        self.padded = Some(IntText { x, signed, pad: Some((len.r.clone(), f.as_bytes()[0], start == 1)) });
                        return Ok(v("zeroinitializer".to_string(), "{ ptr, i64 }"));
                    }
                }
                return match self.literal_of(&es[2]) {
                    Some(f) if f.len() == 1 => self.pad_int(&x, signed, &len.r, f.as_bytes()[0], start == 1),
                    _ => {
                        let text = self.int_text(&x, signed)?;
                        let (p, n) = self.str_parts(&text);
                        let (q, m) = self.str_parts(&fill);
                        let r = self.pad_text(&p, &n, &len.r, &Fill::Text(q, m), start == 1, None);
                        self.drop_value(&text, &Type::Str);
                        Ok(r)
                    }
                };
            }
        }
        // a concatenation with the text of an integer (D359) or a literal
        // (D368): written by the compiled code, its literals as constants;
        // texts after a left text given (D351) too (D509: `isc_str_append_n`
        // summed its parts and copied each by a call, 36 % of `json.quote`
        // of escaped texts)
        if op == S::Concat
            && (es.iter().any(|x| match &x.kind {
                ExprKind::Var(t, _) => self.int_texts.contains_key(t) || self.lit_vars.contains_key(t),
                ExprKind::StrLit(_) => x.ty == Type::Str,
                _ => false,
            }) || (es.len() >= 3 && matches!(es[0].kind, ExprKind::Var(_, Use::Move))))
        {
            let slot = self.stack_concats.get(&(e as *const Expr)).cloned();
            return self.concat_parts_built(es, slot);
        }
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
            S::Concat if vals.len() == 2 => {
                let (p, n) = self.str_parts(&vals[0]);
                let (q, m) = self.str_parts(&vals[1]);
                // the left text given (its last use, D351): the rest written
                // after it in its block when it has room
                if matches!(es[0].kind, ExprKind::Var(_, Use::Move)) {
                    new_str(self, format!("call void @isc_str_append(ptr OUT, ptr {}, i64 {}, ptr {}, i64 {})", p, n, q, m))
                } else {
                    self.text_concat(&[(p, n), (q, m)])
                }
            }
            // the parts of `a + b + c`, of a template literal: their texts
            // in a table on the stack, one block made for all (D342)
            // (not its last use: the texts copied one after the other into a
            // block of their lengths, here, D592)
            S::Concat if !matches!(es[0].kind, ExprKind::Var(_, Use::Move)) => {
                let parts: Vec<(String, String)> = vals.iter().map(|x| self.str_parts(x)).collect();
                self.text_concat(&parts)
            }
            S::Concat => {
                let k = vals.len();
                let table = self.alloca(&format!("[{} x {{ ptr, i64 }}]", k));
                for (i, x) in vals.iter().enumerate() {
                    let slot = self.ins(format!("getelementptr inbounds [{} x {{ ptr, i64 }}], ptr {}, i64 0, i64 {}", k, table, i));
                    self.emit(format!("store {{ ptr, i64 }} {}, ptr {}", x.r, slot));
                }
                new_str(self, format!("call void @isc_str_append_n(ptr OUT, ptr {}, i64 {})", table, k))
            }
            S::Len => {
                let (_, n) = self.str_parts(&vals[0]);
                v(n, "i64")
            }
            // the bytes: an array of `u8`, the same layout
            // the bytes of a string read where it is: the same pointer and
            // length (D303)
            S::Bytes if isc_ir::passes::str_view(op, es, &e.ty) => v(vals[0].r.clone(), "{ ptr, i64 }"),
            S::Bytes => {
                let (p, n) = self.str_parts(&vals[0]);
                self.text_copy(&p, &n)
            }
            // bytes at their last use all below 0x80 (D450): their block is
            // the text, ASCII and so UTF-8, nothing to check
            S::FromBytes if matches!(es[0].kind, ExprKind::Var(x, Use::Move) if self.byte_bounds.get(&x).is_some_and(|&b| b < 0x80)) => v(vals[0].r.clone(), "{ ptr, i64 }"),
            S::FromBytes => {
                let (p, n) = self.str_parts(&vals[0]);
                let pos = Self::pos(e.span);
                // bytes at their last use: their block taken (D399)
                let f = if matches!(es[0].kind, ExprKind::Var(_, Use::Move)) { "isc_str_adopt_bytes" } else { "isc_str_from_bytes" };
                new_str(self, format!("call void @{}(ptr OUT, ptr {}, i64 {}, {})", f, p, n, pos))
            }
            S::ToString => {
                let x = &vals[0];
                match &es[0].ty {
                    Type::Int(k) => {
                        let w = if k.bits() < 64 {
                            let op = if k.signed() { "sext" } else { "zext" };
                            self.ins(format!("{} i{} {} to i64", op, k.bits(), x.r))
                        } else {
                            x.r.clone()
                        };
                        return self.int_text(&w, k.signed());
                    }
                    // the text of a float: the function of the prelude
                    // that writes it
                    t @ (Type::F64 | Type::F32) => {
                        let Some((d, f)) = self.p.float_text else {
                            return Err("codegen: the text of a float, its function of the prelude not lowered".into());
                        };
                        let (i, lty) = if *t == Type::F64 { (d, "double") } else { (f, "float") };
                        self.text_called.insert(i);
                        let call = format!("call {}{{ ptr, i64 }} @isc_fn_{}({} {})", self.fn_cc(), i, lty, x.r);
                        return Ok(v(self.ins(call), "{ ptr, i64 }"));
                    }
                    // (a boolean: made above, D591)
                    t => return Err(format!("codegen: toString of {:?}", t)),
                }
            }
            // text (D135): an array of strings has the layout of a string
            // views of the text that lives longer than the array (D555)
            S::Lines if self.view_parts.contains(&(e as *const Expr)) => {
                let (p, n) = self.str_parts(&vals[0]);
                new_str(self, format!("call void @isc_lines_view(ptr OUT, ptr {}, i64 {})", p, n))
            }
            S::Lines => {
                let (p, n) = self.str_parts(&vals[0]);
                new_str(self, format!("call void @isc_lines(ptr OUT, ptr {}, i64 {})", p, n))
            }
            S::Split => {
                let (p, n) = self.str_parts(&vals[0]);
                let (q, m) = self.str_parts(&vals[1]);
                let pos = Self::pos(e.span);
                let f = if self.view_parts.contains(&(e as *const Expr)) { "isc_split_view" } else { "isc_split" };
                new_str(self, format!("call void @{}(ptr OUT, ptr {}, i64 {}, ptr {}, i64 {}, {})", f, p, n, q, m, pos))
            }
            // the functions of TypeScript (D155)
            S::LastIndexOf => {
                let (p, n) = self.str_parts(&vals[0]);
                let (q, m) = self.str_parts(&vals[1]);
                v(self.ins(format!("call i64 @isc_last_index_of(ptr {}, i64 {}, ptr {}, i64 {})", p, n, q, m)), "i64")
            }
            // against a literal of at most 16 bytes: its length, then its
            // bytes compared as one or two integers, without a call (D301)
            S::StartsWith | S::EndsWith if self.literal_of(&es[1]).is_some_and(|t| t.len() <= 16) => {
                let t = self.literal_of(&es[1]).unwrap();
                let (p, n) = self.str_parts(&vals[0]);
                self.literal_affix(&p, &n, t.as_bytes(), op == S::StartsWith, false)
            }
            // the start or the end of the text has the bytes of the other
            // (D592): its length at most that of the text, then `memcmp`
            S::StartsWith | S::EndsWith => {
                let (p, n) = self.str_parts(&vals[0]);
                let (q, m) = self.str_parts(&vals[1]);
                let fits = self.ins(format!("icmp sle i64 {}, {}", m, n));
                let k = self.ins(format!("select i1 {}, i64 {}, i64 0", fits, m));
                let at = if op == S::StartsWith {
                    p
                } else {
                    let off = self.ins(format!("sub i64 {}, {}", n, k));
                    self.ins(format!("getelementptr inbounds i8, ptr {}, i64 {}", p, off))
                };
                let same = self.text_same(&at, &q, &k);
                v(self.ins(format!("and i1 {}, {}", fits, same)), "i1")
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
                        // its text moved into it: the runtime owns it (D315)
                        let given = matches!(es[0].kind, ExprKind::Var(_, isc_ir::Use::Move)) as i32;
                        new_str(self, format!("call void @isc_rx_replace(ptr OUT, ptr {}, i64 {}, ptr {}, i64 {}, ptr {}, i64 {}, i32 {}, i32 {}, {})", p, n, q, m, r, k, all, given, pos))
                    }
                }
            }
            // made here for a text of ASCII, finished by the prelude for the
            // others (`text_case`); a text only read in its scope: into its
            // buffer on the stack when it fits (D501, as the padded texts of
            // D458)
            S::ToUpper | S::ToLower => {
                let (p, n) = self.str_parts(&vals[0]);
                let slot = self.stack_concats.get(&(e as *const Expr)).cloned();
                self.text_case(&p, &n, op == S::ToUpper, slot)?
            }
            // made here (`pad_text`, D606)
            S::PadStart | S::PadEnd => {
                let (p, n) = self.str_parts(&vals[0]);
                // a fill of one byte known here, or a text
                let fill = match self.literal_of(&es[2]).filter(|f| f.len() == 1) {
                    Some(f) => Fill::Byte(f.as_bytes()[0]),
                    None => {
                        let (q, m) = self.str_parts(&vals[2]);
                        Fill::Text(q, m)
                    }
                };
                // a text only read in its scope: into its buffer on the stack
                // when it fits (D458, as the concatenations of D404)
                let buf = self.stack_concats.get(&(e as *const Expr)).cloned();
                self.pad_text(&p, &n, &vals[1].r, &fill, op == S::PadStart, buf.as_deref())
            }
            // `s.repeat(count)` (D591), made here: a negative count a panic,
            // a block of `len * count` bytes (a product past `i64` asks for
            // the most, refused by the allocator), the text copied once, then
            // the bytes written copied after themselves, twice as many each
            // turn
            S::Repeat => {
                let (p, len) = self.str_parts(&vals[0]);
                let count = vals[1].r.clone();
                let pos = Self::pos(e.span);
                let ok = self.ins(format!("icmp sge i64 {}, 0", count));
                self.guard(&ok, format!("call void @isc_panic_repeat(i64 {}, {})", count, pos));
                self.decls.insert("declare { i64, i1 } @llvm.smul.with.overflow.i64(i64, i64)".into());
                let m = self.ins(format!("call {{ i64, i1 }} @llvm.smul.with.overflow.i64(i64 {}, i64 {})", len, count));
                let prod = self.ins(format!("extractvalue {{ i64, i1 }} {}, 0", m));
                let over = self.ins(format!("extractvalue {{ i64, i1 }} {}, 1", m));
                let total = self.ins(format!("select i1 {}, i64 9223372036854775807, i64 {}", over, prod));
                let dst = self.ins(format!("call noalias ptr @isc_alloc(i64 {})", total));
                let some = self.ins(format!("icmp sgt i64 {}, 0", total));
                let (lfirst, lloop, lbody, ldone) = (self.label(), self.label(), self.label(), self.label());
                // the bytes written so far (a slot of the stack, a register
                // once compiled)
                let slot = self.alloca("i64");
                self.cbr(&some, &lfirst, &ldone);
                self.block(&lfirst);
                self.emit(format!("call void @llvm.memcpy.p0.p0.i64(ptr {}, ptr {}, i64 {}, i1 false)", dst, p, len));
                self.emit(format!("store i64 {}, ptr {}", len, slot));
                self.br(&lloop);
                self.block(&lloop);
                let filled = self.ins(format!("load i64, ptr {}", slot));
                let more = self.ins(format!("icmp slt i64 {}, {}", filled, total));
                self.cbr(&more, &lbody, &ldone);
                self.block(&lbody);
                let left = self.ins(format!("sub i64 {}, {}", total, filled));
                let small = self.ins(format!("icmp slt i64 {}, {}", left, filled));
                let c = self.ins(format!("select i1 {}, i64 {}, i64 {}", small, left, filled));
                let to = self.ins(format!("getelementptr inbounds i8, ptr {}, i64 {}", dst, filled));
                self.emit(format!("call void @llvm.memcpy.p0.p0.i64(ptr {}, ptr {}, i64 {}, i1 false)", to, dst, c));
                let next = self.ins(format!("add i64 {}, {}", filled, c));
                self.emit(format!("store i64 {}, ptr {}", next, slot));
                self.br(&lloop);
                self.block(&ldone);
                self.arr(&dst, &total)
            }
            S::Slice if isc_ir::passes::str_view(op, es, &e.ty) => {
                let (p, n) = self.str_parts(&vals[0]);
                self.str_slice_view(&p, &n, &vals[1].r, &vals[2].r, e.span)
            }
            // `s.slice(a, b)` (D590), as `slice_str` of the runtime core, made
            // here: the bounds (`0 <= a <= b <= len`), then `a` and `b` each at
            // the start of a character (its byte not `10xxxxxx`, or the end),
            // then the bytes copied into a new block (the empty block of the
            // runtime for none)
            S::Slice => {
                let (p, len) = self.str_parts(&vals[0]);
                let (a, b) = (vals[1].r.clone(), vals[2].r.clone());
                let pos = Self::pos(e.span);
                let c0 = self.ins(format!("icmp sge i64 {}, 0", a));
                let c1 = self.ins(format!("icmp sle i64 {}, {}", a, b));
                let c2 = self.ins(format!("icmp sle i64 {}, {}", b, len));
                let c01 = self.ins(format!("and i1 {}, {}", c0, c1));
                let ok = self.ins(format!("and i1 {}, {}", c01, c2));
                self.guard(&ok, format!("call void @isc_panic_str_slice(i64 {}, i64 {}, i64 {}, {})", a, b, len, pos));
                for i in [&a, &b] {
                    let inside = self.ins(format!("icmp slt i64 {}, {}", i, len));
                    let (lin, lout) = (self.label(), self.label());
                    self.cbr(&inside, &lin, &lout);
                    self.block(&lin);
                    let q = self.ins(format!("getelementptr inbounds i8, ptr {}, i64 {}", p, i));
                    let c = self.ins(format!("load i8, ptr {}, align 1", q));
                    let top = self.ins(format!("and i8 {}, -64", c));
                    let start = self.ins(format!("icmp ne i8 {}, -128", top));
                    self.guard(&start, format!("call void @isc_panic_boundary(i64 {}, {})", i, pos));
                    self.br(&lout);
                    self.block(&lout);
                }
                let n = self.ins(format!("sub i64 {}, {}", b, a));
                let dst = self.ins(format!("call noalias ptr @isc_alloc(i64 {})", n));
                let src = self.ins(format!("getelementptr inbounds i8, ptr {}, i64 {}", p, a));
                self.emit(format!("call void @llvm.memcpy.p0.p0.i64(ptr {}, ptr {}, i64 {}, i1 false)", dst, src, n));
                self.arr(&dst, &n)
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
            S::Group | S::GroupCounts => {
                let (p, n) = self.str_parts(&vals[0]);
                // the last indices not made for `$groupCounts` (D317)
                let kind = group_kind(es[0].ty.elem()) | if op == S::GroupCounts { GROUP_COUNTS } else { 0 };
                new_str(self, format!("call void @isc_group(ptr OUT, ptr {}, i64 {}, i32 {})", p, n, kind))
            }
            // the index of hashes of the keys of a map, the position of a key
            // in it (D492)
            S::HashIndex => {
                let (p, n) = self.str_parts(&vals[0]);
                let kind = group_kind(es[0].ty.elem());
                new_str(self, format!("call void @isc_hash_index(ptr OUT, ptr {}, i64 {}, i32 {})", p, n, kind))
            }
            // integer keys probed in the generated code: `hash_word` of the key
            // widened to 64 bits, then the slots from its own up to an empty
            // one, each key read with its type (no call, no choice of its
            // kind at each probe: `get` of an integer 7.5 ns, 4 times its
            // floor, with the call)
            S::HashFind if matches!(es[0].ty.elem(), Type::Int(_)) => {
                let (p, _) = self.str_parts(&vals[0]);
                let (q, m) = self.str_parts(&vals[1]);
                let t = vals[2].t.clone();
                let signed = matches!(&es[2].ty, Type::Int(k) if k.signed());
                let kw = if t == "i64" { vals[2].r.clone() } else { self.ins(format!("{} {} {} to i64", if signed { "sext" } else { "zext" }, t, vals[2].r)) };
                let h0 = self.ins(format!("mul i64 {}, -7046029254386353131", kw));
                let h1 = self.ins(format!("lshr i64 {}, 29", h0));
                let h = self.ins(format!("xor i64 {}, {}", h0, h1));
                let mask = self.ins(format!("sub i64 {}, 1", m));
                let at0 = self.ins(format!("and i64 {}, {}", h, mask));
                let res = self.alloca("i64");
                let at = self.alloca("i64");
                self.emit(format!("store i64 -1, ptr {}", res));
                self.emit(format!("store i64 {}, ptr {}", at0, at));
                let (lloop, lcheck, lnext, lfound, ldone) = (self.label(), self.label(), self.label(), self.label(), self.label());
                // an index has at least 2 slots (`hash_index`); none: absent
                let none = self.ins(format!("icmp eq i64 {}, 0", m));
                self.cbr(&none, &ldone, &lloop);
                self.block(&lloop);
                let a = self.ins(format!("load i64, ptr {}", at));
                let sp = self.ins(format!("getelementptr inbounds i64, ptr {}, i64 {}", q, a));
                let slot = self.ins(format!("load i64, ptr {}", sp));
                let empty = self.ins(format!("icmp eq i64 {}, 0", slot));
                self.cbr(&empty, &ldone, &lcheck);
                self.block(&lcheck);
                let j = self.ins(format!("sub i64 {}, 1", slot));
                let kp = self.ins(format!("getelementptr inbounds {}, ptr {}, i64 {}", t, p, j));
                let kj = self.ins(format!("load {}, ptr {}", t, kp));
                let eq = self.ins(format!("icmp eq {} {}, {}", t, kj, vals[2].r));
                self.cbr(&eq, &lfound, &lnext);
                self.block(&lfound);
                self.emit(format!("store i64 {}, ptr {}", j, res));
                self.br(&ldone);
                self.block(&lnext);
                let a1 = self.ins(format!("add i64 {}, 1", a));
                let a2 = self.ins(format!("and i64 {}, {}", a1, mask));
                self.emit(format!("store i64 {}, ptr {}", a2, at));
                self.br(&lloop);
                self.block(&ldone);
                v(self.ins(format!("load i64, ptr {}", res)), "i64")
            }
            // `leadingZeros(x)` (D588): `llvm.ctlz` of its width, 0 defined
            S::LeadingZeros => {
                let t = lt(&es[0].ty);
                self.decls.insert(format!("declare {} @llvm.ctlz.{}({}, i1)", t, t, t));
                let c = self.ins(format!("call {} @llvm.ctlz.{}({} {}, i1 false)", t, t, t, vals[0].r));
                let r = if t == "i64" { c } else { self.ins(format!("zext {} {} to i64", t, c)) };
                v(r, "i64")
            }
            // `$textTail(w0, w1, w2, n, minus)` (D588), as `text_tail` of the
            // runtime core: one check (`n` in 1 to 24, every byte ASCII), a
            // block of `n` bytes and the `-`, the `-` written first (the
            // digits over it when there is none), then the digits from the
            // registers: the words whole (8 bytes), the first shifted, or
            // two stores of 4 or three of 1 that overlap
            S::TextTail => {
                let (w0, w1, w2) = (vals[0].r.clone(), vals[1].r.clone(), vals[2].r.clone());
                let (n, minus) = (vals[3].r.clone(), vals[4].r.clone());
                let o1 = self.ins(format!("or i64 {}, {}", w0, w1));
                let o2 = self.ins(format!("or i64 {}, {}", o1, w2));
                let hi = self.ins(format!("and i64 {}, -9187201950435737472", o2));
                let ascii = self.ins(format!("icmp eq i64 {}, 0", hi));
                let nm1 = self.ins(format!("sub i64 {}, 1", n));
                let inside = self.ins(format!("icmp ult i64 {}, 24", nm1));
                let ok = self.ins(format!("and i1 {}, {}", ascii, inside));
                self.guard(&ok, format!("call void @isc_panic_tail(i64 {}, {})", n, Self::pos(e.span)));
                let off = self.ins(format!("zext i1 {} to i64", minus));
                let total = self.ins(format!("add i64 {}, {}", n, off));
                // a block of 25 bytes whatever `n` (the class of 3 as well):
                // the allocation does not wait for the count of the digits;
                // or the buffer of the caller (`buffered_texts`)
                let dst = match self.text_buffer(e) {
                    Some(tb) => tb,
                    None => self.ins("call noalias ptr @isc_alloc(i64 25)".to_string()),
                };
                self.emit(format!("store i8 45, ptr {}, align 1", dst));
                let d = self.ins(format!("getelementptr inbounds i8, ptr {}, i64 {}", dst, off));
                self.put_tail(&d, &w0, &w1, &w2, &n, &nm1);
                self.arr(&dst, &total)
            }
            // `$floatBits(x)`: the bits of the float, those of a `f32` in
            // the low half
            S::FloatBits => {
                let r = if es[0].ty == Type::F32 {
                    let b = self.ins(format!("bitcast float {} to i32", vals[0].r));
                    self.ins(format!("zext i32 {} to i64", b))
                } else {
                    self.ins(format!("bitcast double {} to i64", vals[0].r))
                };
                v(r, "i64")
            }
            // `$bitsFloat(w)`: the float of these bits
            S::BitsFloat => v(self.ins(format!("bitcast i64 {} to double", vals[0].r)), "double"),
            // `$mulHigh(a, b)`: the high half of the product of 128 bits,
            // one instruction of x86-64; WebAssembly has no such product
            // (LLVM would call a function of a C library there): the four
            // products of the halves of 32 bits and their carries
            S::MulHigh => {
                let (a, b) = (vals[0].r.clone(), vals[1].r.clone());
                let r = if self.opts.isa.is_wasm() {
                    let a0 = self.ins(format!("and i64 {}, 4294967295", a));
                    let a1 = self.ins(format!("lshr i64 {}, 32", a));
                    let b0 = self.ins(format!("and i64 {}, 4294967295", b));
                    let b1 = self.ins(format!("lshr i64 {}, 32", b));
                    let p00 = self.ins(format!("mul i64 {}, {}", a0, b0));
                    let p01 = self.ins(format!("mul i64 {}, {}", a0, b1));
                    let p10 = self.ins(format!("mul i64 {}, {}", a1, b0));
                    let p11 = self.ins(format!("mul i64 {}, {}", a1, b1));
                    let c0 = self.ins(format!("lshr i64 {}, 32", p00));
                    let m0 = self.ins(format!("and i64 {}, 4294967295", p01));
                    let m1 = self.ins(format!("and i64 {}, 4294967295", p10));
                    let s0 = self.ins(format!("add i64 {}, {}", c0, m0));
                    let mid = self.ins(format!("add i64 {}, {}", s0, m1));
                    let h0 = self.ins(format!("lshr i64 {}, 32", p01));
                    let h1 = self.ins(format!("lshr i64 {}, 32", p10));
                    let c1 = self.ins(format!("lshr i64 {}, 32", mid));
                    let s1 = self.ins(format!("add i64 {}, {}", p11, h0));
                    let s2 = self.ins(format!("add i64 {}, {}", s1, h1));
                    self.ins(format!("add i64 {}, {}", s2, c1))
                } else {
                    let x = self.ins(format!("zext i64 {} to i128", a));
                    let y = self.ins(format!("zext i64 {} to i128", b));
                    let m = self.ins(format!("mul nuw i128 {}, {}", x, y));
                    let h = self.ins(format!("lshr i128 {}, 64", m));
                    self.ins(format!("trunc i128 {} to i64", h))
                };
                v(r, "i64")
            }
            // `$decimalText(d, w1, w2, n, point, minus)`: one check (the
            // digit, the bytes ASCII, `n` in 1 to 17, `point` within 400), a
            // block of 64 bytes or more (one class of the allocator up to
            // 44 zeros), the `-` written first (the digits over it when
            // there is none), then the digits from the registers by whole
            // words, past the end of the text where that is shorter:
            // - the point inside the digits: the 17 digits, then those of
            //   the fraction again one byte further (a word shifted, and
            //   the last word when the fraction starts in the first), the
            //   point between them;
            // - the point after them: the 17 digits and 16 zeros, the
            //   other zeros by `memset`;
            // - the point before them: `0.` and 16 zeros, the other zeros
            //   by `memset`, then the 17 digits.
            S::DecimalText => {
                let (d, w1, w2) = (vals[0].r.clone(), vals[1].r.clone(), vals[2].r.clone());
                let (n, point, minus) = (vals[3].r.clone(), vals[4].r.clone(), vals[5].r.clone());
                let o = self.ins(format!("or i64 {}, {}", w1, w2));
                let hi = self.ins(format!("and i64 {}, -9187201950435737472", o));
                let od = self.ins(format!("or i64 {}, {}", hi, d));
                let dok = self.ins(format!("icmp ult i64 {}, 10", od));
                let nm1 = self.ins(format!("sub i64 {}, 1", n));
                let nok = self.ins(format!("icmp ult i64 {}, 17", nm1));
                let pp = self.ins(format!("add i64 {}, 400", point));
                let pok = self.ins(format!("icmp ult i64 {}, 801", pp));
                let ok1 = self.ins(format!("and i1 {}, {}", dok, nok));
                let ok = self.ins(format!("and i1 {}, {}", ok1, pok));
                let (msg, mlen) = self.str_const("decimalText: a digit and 16 bytes of ASCII, 1 to 17 of them written, the point within 400 digits");
                let (none, _) = self.str_const("");
                self.guard(&ok, format!("call void @isc_panic_text(ptr {}, i64 {}, ptr {}, i64 0, ptr {}, i64 0, {})", msg, mlen, none, none, Self::pos(e.span)));
                let off = self.ins(format!("zext i1 {} to i64", minus));
                let before = self.ins(format!("icmp sgt i64 {}, 0", point));
                let z = self.ins(format!("sub i64 0, {}", point));
                let mag = self.ins(format!("select i1 {}, i64 {}, i64 {}", before, point, z));
                // (or the buffer of the caller, `buffered_texts`: the point
                // within 400 digits, so at most 420 bytes of `TEXT_BUF`)
                let dst = match self.text_buffer(e) {
                    Some(tb) => tb,
                    None => {
                        let need = self.ins(format!("add i64 {}, 20", mag));
                        let big = self.ins(format!("icmp ugt i64 {}, 64", need));
                        let size = self.ins(format!("select i1 {}, i64 {}, i64 64", big, need));
                        self.ins(format!("call noalias ptr @isc_alloc(i64 {})", size))
                    }
                };
                self.emit(format!("store i8 45, ptr {}, align 1", dst));
                let p = self.ins(format!("getelementptr inbounds i8, ptr {}, i64 {}", dst, off));
                let d8 = self.ins(format!("trunc i64 {} to i8", d));
                let dch = self.ins(format!("add i8 {}, 48", d8));
                // `x` (of the type `t`) stored `at` bytes after `base`
                let put = |g: &mut Self, base: &str, at: &str, t: &str, x: &str| {
                    let q = g.ins(format!("getelementptr inbounds i8, ptr {}, i64 {}", base, at));
                    g.emit(format!("store {} {}, ptr {}, align 1", t, x, q));
                };
                let zeros = "3472328296227680304";
                self.decls.insert("declare void @llvm.memset.p0.i64(ptr, i8, i64, i1)".into());
                let (lpos, lsmall, lpoint, lzeros, lfill, lfill2, ldigits, ljoin) =
                    (self.label(), self.label(), self.label(), self.label(), self.label(), self.label(), self.label(), self.label());
                self.cbr(&before, &lpos, &lsmall);
                // digits before the point: the 17 digits first
                self.block(&lpos);
                self.emit(format!("store i8 {}, ptr {}, align 1", dch, p));
                put(self, &p, "1", "i64", &w1);
                put(self, &p, "9", "i64", &w2);
                let inside = self.ins(format!("icmp slt i64 {}, {}", point, n));
                self.cbr(&inside, &lpoint, &lzeros);
                // the point inside: the digits `point` to 16 one byte further
                self.block(&lpoint);
                let j = self.ins(format!("sub i64 {}, 1", point));
                let j7 = self.ins(format!("and i64 {}, 7", j));
                let sh = self.ins(format!("shl i64 {}, 3", j7));
                let low = self.ins(format!("icmp ult i64 {}, 8", j));
                let src = self.ins(format!("select i1 {}, i64 {}, i64 {}", low, w1, w2));
                let x = self.ins(format!("lshr i64 {}, {}", src, sh));
                let fp = self.ins(format!("getelementptr inbounds i8, ptr {}, i64 {}", p, point));
                put(self, &fp, "1", "i64", &x);
                // the last word after a fraction that starts in the first;
                // else written past the text, where it changes nothing
                let near = self.ins(format!("getelementptr inbounds i8, ptr {}, i64 10", p));
                let far = self.ins(format!("getelementptr inbounds i8, ptr {}, i64 48", dst));
                let q = self.ins(format!("select i1 {}, ptr {}, ptr {}", low, near, far));
                self.emit(format!("store i64 {}, ptr {}, align 1", w2, q));
                self.emit(format!("store i8 46, ptr {}, align 1", fp));
                let len_a = self.ins(format!("add i64 {}, 1", n));
                self.br(&ljoin);
                // the point after the digits: zeros up to it
                self.block(&lzeros);
                put(self, &p, "17", "i64", zeros);
                put(self, &p, "25", "i64", zeros);
                let more = self.ins(format!("icmp sgt i64 {}, 33", point));
                self.cbr(&more, &lfill, &ljoin);
                self.block(&lfill);
                let rest = self.ins(format!("sub i64 {}, 33", point));
                let at = self.ins(format!("getelementptr inbounds i8, ptr {}, i64 33", p));
                self.emit(format!("call void @llvm.memset.p0.i64(ptr {}, i8 48, i64 {}, i1 false)", at, rest));
                self.br(&ljoin);
                // the point before the digits: `0.`, zeros, the digits
                self.block(&lsmall);
                put(self, &p, "0", "i16", "11824");
                put(self, &p, "2", "i64", zeros);
                put(self, &p, "10", "i64", zeros);
                let more = self.ins(format!("icmp sgt i64 {}, 16", z));
                self.cbr(&more, &lfill2, &ldigits);
                self.block(&lfill2);
                let rest = self.ins(format!("sub i64 {}, 16", z));
                let at = self.ins(format!("getelementptr inbounds i8, ptr {}, i64 18", p));
                self.emit(format!("call void @llvm.memset.p0.i64(ptr {}, i8 48, i64 {}, i1 false)", at, rest));
                self.br(&ldigits);
                self.block(&ldigits);
                let r0 = self.ins(format!("getelementptr inbounds i8, ptr {}, i64 {}", p, z));
                put(self, &r0, "2", "i8", &dch);
                put(self, &r0, "3", "i64", &w1);
                put(self, &r0, "11", "i64", &w2);
                let n2 = self.ins(format!("add i64 {}, 2", n));
                let len_c = self.ins(format!("add i64 {}, {}", n2, z));
                self.br(&ljoin);
                self.block(&ljoin);
                let len = self.ins(format!(
                    "phi i64 [ {}, %{} ], [ {}, %{} ], [ {}, %{} ], [ {}, %{} ]",
                    len_a, lpoint, point, lzeros, point, lfill, len_c, ldigits
                ));
                let total = self.ins(format!("add i64 {}, {}", len, off));
                self.arr(&dst, &total)
            }
            // `$withText(t, at, piece)`: one check (the bytes `at .. at +
            // len(piece)` inside `t`; each of their two ends at the start of
            // a character of `t` or at its end: no byte `10xxxxxx` there),
            // then the bytes of `piece` copied there: into `t` itself when it
            // is given (a new text of the program at its last use: a block of
            // the heap, never a literal, a view or a buffer of the stack),
            // into a copy otherwise. Whole characters of a text replaced by a
            // text: the result is UTF-8 whatever the prelude asks.
            S::WithText => {
                let (p, n) = self.str_parts(&vals[0]);
                let at = vals[1].r.clone();
                let (q, m) = self.str_parts(&vals[2]);
                // (`at` up to the length of `t` less that of `piece`: no sum
                // that an `at` near the largest integer would wrap)
                let room = self.ins(format!("sub i64 {}, {}", n, m));
                let c0 = self.ins(format!("icmp sge i64 {}, 0", at));
                let c1 = self.ins(format!("icmp sle i64 {}, {}", at, room));
                let inside = self.ins(format!("and i1 {}, {}", c0, c1));
                let end = self.ins(format!("add i64 {}, {}", at, m));
                let (la, lload_a, lb, lload_b, lbad, lok) = (self.label(), self.label(), self.label(), self.label(), self.label(), self.label());
                self.cbr(&inside, &la, &lbad);
                for (i, here, load, next) in [(&at, &la, &lload_a, &lb), (&end, &lb, &lload_b, &lok)] {
                    self.block(here);
                    let before = self.ins(format!("icmp slt i64 {}, {}", i, n));
                    self.cbr(&before, load, next);
                    self.block(load);
                    let bp = self.ins(format!("getelementptr inbounds i8, ptr {}, i64 {}", p, i));
                    let byte = self.ins(format!("load i8, ptr {}, align 1", bp));
                    let starts = self.ins(format!("icmp sge i8 {}, -64", byte));
                    self.cbr(&starts, next, &lbad);
                }
                self.block(&lbad);
                let (msg, mlen) = self.str_const("withText: a text written outside another or inside one of its characters");
                let (none, _) = self.str_const("");
                self.emit(format!("call void @isc_panic_text(ptr {}, i64 {}, ptr {}, i64 0, ptr {}, i64 0, {})", msg, mlen, none, none, Self::pos(e.span)));
                self.emit("unreachable");
                self.block(&lok);
                let base = if matches!(es[0].kind, ExprKind::Var(_, Use::Move)) {
                    p
                } else {
                    let c = self.ins(format!("call noalias ptr @isc_alloc(i64 {})", n));
                    self.emit(format!("call void @llvm.memcpy.p0.p0.i64(ptr {}, ptr {}, i64 {}, i1 false)", c, p, n));
                    c
                };
                let dst = self.ins(format!("getelementptr inbounds i8, ptr {}, i64 {}", base, at));
                // a text of 2 or 3 bytes (a character, most often) by two
                // moves of 2 that overlap, of 4 to 8 by two of 4, of one byte
                // by one; the longer ones as the texts of a concatenation
                let (lfew, l2, l1, lmore, l4, lany, ldone) = (self.label(), self.label(), self.label(), self.label(), self.label(), self.label(), self.label());
                let pair = |g: &mut Self, ty: &str, w: i64| {
                    let a = g.ins(format!("load {}, ptr {}, align 1", ty, q));
                    let off = g.ins(format!("sub i64 {}, {}", m, w));
                    let src2 = g.ins(format!("getelementptr inbounds i8, ptr {}, i64 {}", q, off));
                    let b = g.ins(format!("load {}, ptr {}, align 1", ty, src2));
                    g.emit(format!("store {} {}, ptr {}, align 1", ty, a, dst));
                    let dst2 = g.ins(format!("getelementptr inbounds i8, ptr {}, i64 {}", dst, off));
                    g.emit(format!("store {} {}, ptr {}, align 1", ty, b, dst2));
                };
                let few = self.ins(format!("icmp ult i64 {}, 4", m));
                self.cbr(&few, &lfew, &lmore);
                self.block(&lfew);
                let two = self.ins(format!("icmp uge i64 {}, 2", m));
                self.cbr(&two, &l2, &l1);
                self.block(&l2);
                pair(self, "i16", 2);
                self.br(&ldone);
                self.block(&l1);
                let one = self.ins(format!("icmp ne i64 {}, 0", m));
                let lbyte = self.label();
                self.cbr(&one, &lbyte, &ldone);
                self.block(&lbyte);
                let c = self.ins(format!("load i8, ptr {}, align 1", q));
                self.emit(format!("store i8 {}, ptr {}, align 1", c, dst));
                self.br(&ldone);
                self.block(&lmore);
                let mid = self.ins(format!("icmp ule i64 {}, 8", m));
                self.cbr(&mid, &l4, &lany);
                self.block(&l4);
                pair(self, "i32", 4);
                self.br(&ldone);
                self.block(&lany);
                self.copy_short(&dst, &q, &m);
                self.br(&ldone);
                self.block(&ldone);
                self.arr(&base, &n)
            }
            // `$fixedText(i0, i1, i2, ni, f0, f1, f2, nf, minus)`: one check
            // (`ni` in 1 to 24, `nf` in 0 to 24, every byte ASCII), a block
            // of 64 bytes (a `-`, 24 bytes, a point and 24 bytes are 50),
            // the `-` written first (the digits over it when there is
            // none), then the bytes of each part from the registers by
            // whole words (`words_tail`): what the first word of the whole
            // part writes past its bytes is written again by the point and
            // the fraction, what the first word of the fraction writes
            // past its bytes lies past the text and inside the block
            S::FixedText => {
                let (ni, nf, minus) = (vals[3].r.clone(), vals[7].r.clone(), vals[8].r.clone());
                let mut all = vals[0].r.clone();
                for k in [1, 2, 4, 5, 6] {
                    all = self.ins(format!("or i64 {}, {}", all, vals[k].r));
                }
                let hi = self.ins(format!("and i64 {}, -9187201950435737472", all));
                let ascii = self.ins(format!("icmp eq i64 {}, 0", hi));
                let nim1 = self.ins(format!("sub i64 {}, 1", ni));
                let iok = self.ins(format!("icmp ult i64 {}, 24", nim1));
                let fok = self.ins(format!("icmp ult i64 {}, 25", nf));
                let ok1 = self.ins(format!("and i1 {}, {}", ascii, iok));
                let ok = self.ins(format!("and i1 {}, {}", ok1, fok));
                let (msg, mlen) = self.str_const("fixedText: twice 24 bytes of ASCII, the last 1 to 24 of the first written, the last 0 to 24 of the others");
                let (none, _) = self.str_const("");
                self.guard(&ok, format!("call void @isc_panic_text(ptr {}, i64 {}, ptr {}, i64 0, ptr {}, i64 0, {})", msg, mlen, none, none, Self::pos(e.span)));
                let off = self.ins(format!("zext i1 {} to i64", minus));
                let dst = self.ins("call noalias ptr @isc_alloc(i64 64)".to_string());
                self.emit(format!("store i8 45, ptr {}, align 1", dst));
                let p = self.ins(format!("getelementptr inbounds i8, ptr {}, i64 {}", dst, off));
                self.words_tail(&p, [&vals[0].r, &vals[1].r, &vals[2].r], &ni);
                let (lfrac, ljoin) = (self.label(), self.label());
                let lfrom = self.current_label("entry");
                let whole = self.ins(format!("icmp eq i64 {}, 0", nf));
                self.cbr(&whole, &ljoin, &lfrac);
                self.block(&lfrac);
                let q = self.ins(format!("getelementptr inbounds i8, ptr {}, i64 {}", p, ni));
                self.emit(format!("store i8 46, ptr {}, align 1", q));
                let r = self.ins(format!("getelementptr inbounds i8, ptr {}, i64 1", q));
                self.words_tail(&r, [&vals[4].r, &vals[5].r, &vals[6].r], &nf);
                let nf1 = self.ins(format!("add i64 {}, 1", nf));
                let lend = self.current_label(&lfrac);
                self.br(&ljoin);
                self.block(&ljoin);
                let after = self.ins(format!("phi i64 [ 0, %{} ], [ {}, %{} ]", lfrom, nf1, lend));
                let len = self.ins(format!("add i64 {}, {}", ni, off));
                let total = self.ins(format!("add i64 {}, {}", len, after));
                self.arr(&dst, &total)
            }
            // `$panic(a, b, c)` (D582): the call that does not return, then
            // a block no path reaches, where the value of its type is zero
            S::Panic => {
                let (p, n) = self.str_parts(&vals[0]);
                let (q, m) = self.str_parts(&vals[1]);
                let (r, l) = self.str_parts(&vals[2]);
                self.emit(format!("call void @isc_panic_text(ptr {}, i64 {}, ptr {}, i64 {}, ptr {}, i64 {}, {})", p, n, q, m, r, l, Self::pos(e.span)));
                self.emit("unreachable");
                let dead = self.label();
                self.block(&dead);
                v(Self::zero(&e.ty), lt(&e.ty))
            }
            // `readWord(b, at, n)` (D580), as `read_word` of the runtime core:
            // `n` in 1 ..= 8, then one check of the bytes `at .. at + n`, then
            // two loads of 4 bytes that overlap (4 to 8 bytes) or the first,
            // middle and last byte (1 to 3), as `digits8` reads its digits
            S::ReadWord => {
                let (p, len) = self.str_parts(&vals[0]);
                let (at, n) = (vals[1].r.clone(), vals[2].r.clone());
                let nm1 = self.ins(format!("sub i64 {}, 1", n));
                let nok = self.ins(format!("icmp ult i64 {}, 8", nm1));
                self.guard(&nok, format!("call void @isc_panic_word(i64 {}, {})", n, Self::pos(e.span)));
                let last = self.ins(format!("add i64 {}, {}", at, nm1));
                let c0 = self.ins(format!("icmp sge i64 {}, 0", at));
                let c1 = self.ins(format!("icmp slt i64 {}, {}", last, len));
                let ok = self.ins(format!("and i1 {}, {}", c0, c1));
                let idx = self.ins(format!("select i1 {}, i64 {}, i64 {}", c0, last, at));
                self.guard(&ok, format!("call void @isc_panic_bounds(i64 {}, i64 {}, i32 0, {})", idx, len, Self::pos(e.span)));
                let base = self.ins(format!("getelementptr inbounds i8, ptr {}, i64 {}", p, at));
                // a whole word asked by a literal: one load (two loads of 4
                // bytes are not always made one again)
                if n == "8" {
                    return Ok(v(self.ins(format!("load i64, ptr {}, align 1", base)), "i64"));
                }
                let big = self.ins(format!("icmp uge i64 {}, 4", n));
                let (lbig, lsmall, ljoin) = (self.label(), self.label(), self.label());
                self.cbr(&big, &lbig, &lsmall);
                self.block(&lbig);
                let lo = self.ins(format!("load i32, ptr {}, align 1", base));
                let off = self.ins(format!("sub i64 {}, 4", n));
                let hp = self.ins(format!("getelementptr inbounds i8, ptr {}, i64 {}", base, off));
                let hi = self.ins(format!("load i32, ptr {}, align 1", hp));
                let lo64 = self.ins(format!("zext i32 {} to i64", lo));
                let hi64 = self.ins(format!("zext i32 {} to i64", hi));
                let sh = self.ins(format!("shl i64 {}, 3", off));
                let his = self.ins(format!("shl i64 {}, {}", hi64, sh));
                let w1 = self.ins(format!("or i64 {}, {}", lo64, his));
                self.br(&ljoin);
                self.block(&lsmall);
                let b0 = self.ins(format!("load i8, ptr {}, align 1", base));
                let half = self.ins(format!("lshr i64 {}, 1", n));
                let mp = self.ins(format!("getelementptr inbounds i8, ptr {}, i64 {}", base, half));
                let bm = self.ins(format!("load i8, ptr {}, align 1", mp));
                let lp = self.ins(format!("getelementptr inbounds i8, ptr {}, i64 {}", base, nm1));
                let bl = self.ins(format!("load i8, ptr {}, align 1", lp));
                let b064 = self.ins(format!("zext i8 {} to i64", b0));
                let bm64 = self.ins(format!("zext i8 {} to i64", bm));
                let bl64 = self.ins(format!("zext i8 {} to i64", bl));
                let shm = self.ins(format!("shl i64 {}, 3", half));
                let shl_ = self.ins(format!("shl i64 {}, 3", nm1));
                let bms = self.ins(format!("shl i64 {}, {}", bm64, shm));
                let bls = self.ins(format!("shl i64 {}, {}", bl64, shl_));
                let w2a = self.ins(format!("or i64 {}, {}", b064, bms));
                let w2 = self.ins(format!("or i64 {}, {}", w2a, bls));
                self.br(&ljoin);
                self.block(&ljoin);
                v(self.ins(format!("phi i64 [ {}, %{} ], [ {}, %{} ]", w1, lbig, w2, lsmall)), "i64")
            }
            S::HashFind => {
                let (p, n) = self.str_parts(&vals[0]);
                let (q, m) = self.str_parts(&vals[1]);
                let kind = group_kind(es[0].ty.elem());
                let (kw, kp, kn) = if kind == 11 {
                    let (kp, kn) = self.str_parts(&vals[2]);
                    ("0".to_string(), kp, kn)
                } else {
                    let t = vals[2].t.clone();
                    let signed = matches!(&es[2].ty, Type::Int(k) if k.signed());
                    let kw = if t == "i64" { vals[2].r.clone() } else { self.ins(format!("{} {} {} to i64", if signed { "sext" } else { "zext" }, t, vals[2].r)) };
                    (kw, "null".to_string(), "0".to_string())
                };
                v(self.ins(format!("call i64 @isc_hash_find(ptr {}, i64 {}, ptr {}, i64 {}, i32 {}, i64 {}, ptr {}, i64 {})", p, n, q, m, kind, kw, kp, kn)), "i64")
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
            // `x.toFixed(d)`: `d` in 0 to 100 (the panic of the runtime
            // otherwise, at the place of the call), then the function of
            // the prelude that writes the text
            S::ToFixed => {
                let Some(f) = self.p.fixed_text else {
                    return Err("codegen: toFixed, its function of the prelude not lowered".into());
                };
                let d = vals[1].r.clone();
                let ok = self.ins(format!("icmp ule i64 {}, 100", d));
                self.guard(&ok, format!("call void @isc_panic_fixed(i64 {}, {})", d, Self::pos(e.span)));
                self.text_called.insert(f);
                let call = format!("call {}{{ ptr, i64 }} @isc_fn_{}(double {}, i64 {})", self.fn_cc(), f, vals[0].r, d);
                v(self.ins(call), "{ ptr, i64 }")
            }
            // made here (`join_text`, D607)
            S::Join => {
                let (p, n) = self.str_parts(&vals[0]);
                let (q, m) = self.str_parts(&vals[1]);
                let lit = self.literal_of(&es[1]);
                self.join_text(&p, &n, &q, &m, lit.as_deref())?
            }
            // equal to a literal of at most 16 bytes: its length, then its
            // bytes compared as one or two integers, without a call
            S::Cmp(b @ (BinOp::Eq | BinOp::Ne)) if (0..2).any(|k| self.literal_of(&es[k]).is_some_and(|t| t.len() <= 16)) => {
                let k = (0..2).find(|&k| self.literal_of(&es[k]).is_some_and(|t| t.len() <= 16)).unwrap();
                let t = self.literal_of(&es[k]).unwrap();
                let (p, n) = self.str_parts(&vals[1 - k]);
                let same = self.literal_affix(&p, &n, t.as_bytes(), true, true);
                if b == BinOp::Eq {
                    same
                } else {
                    v(self.ins(format!("xor i1 {}, true", same.r)), "i1")
                }
            }
            S::Cmp(b) => {
                let (p, n) = self.str_parts(&vals[0]);
                let (q, m) = self.str_parts(&vals[1]);
                // decided without a call when it can be (D532): two lengths
                // that differ for an equality; the first 8 bytes of two texts
                // of 8 bytes or more when they differ (their order by the
                // same bytes read in the order of the text)
                let eq = matches!(b, BinOp::Eq | BinOp::Ne);
                let (lw, lload, ldiff, lcall, ljoin) = (self.label(), self.label(), self.label(), self.label(), self.label());
                let lfrom = self.current_label("entry");
                let mut ins = Vec::new();
                if eq {
                    let lenne = self.ins(format!("icmp ne i64 {}, {}", n, m));
                    self.cbr(&lenne, &ljoin, &lw);
                    ins.push(("1".to_string(), lfrom.clone()));
                } else {
                    self.br(&lw);
                }
                self.block(&lw);
                let a8 = self.ins(format!("icmp uge i64 {}, 8", n));
                let b8 = self.ins(format!("icmp uge i64 {}, 8", m));
                let both = self.ins(format!("and i1 {}, {}", a8, b8));
                self.cbr(&both, &lload, &lcall);
                self.block(&lload);
                let x = self.ins(format!("load i64, ptr {}, align 1", p));
                let y = self.ins(format!("load i64, ptr {}, align 1", q));
                let differ = self.ins(format!("icmp ne i64 {}, {}", x, y));
                self.cbr(&differ, &ldiff, &lcall);
                self.block(&ldiff);
                let cd = if eq {
                    "1".to_string()
                } else {
                    self.decls.insert("declare i64 @llvm.bswap.i64(i64)".into());
                    let sx = self.ins(format!("call i64 @llvm.bswap.i64(i64 {})", x));
                    let sy = self.ins(format!("call i64 @llvm.bswap.i64(i64 {})", y));
                    let below = self.ins(format!("icmp ult i64 {}, {}", sx, sy));
                    self.ins(format!("select i1 {}, i32 -1, i32 1", below))
                };
                ins.push((cd, ldiff.clone()));
                self.br(&ljoin);
                self.block(&lcall);
                let cc = self.text_order(&p, &n, &q, &m);
                ins.push((cc, self.current_label(&lcall)));
                self.br(&ljoin);
                self.block(&ljoin);
                let c = self.ins(format!("phi i32 {}", ins.iter().map(|(x, l)| format!("[ {}, %{} ]", x, l)).collect::<Vec<_>>().join(", ")));
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

    /// `print` of a float: the text of the function of the prelude, written
    /// in a buffer of the stack (`emit_text_print`), after `pre` bytes of
    /// the separator `, ` (0 or 2).
    fn print_float(&mut self, x: &V, t: &Type, pre: &str) -> R<()> {
        let Some((d, f)) = self.p.float_text else {
            return Err("codegen: print of a float, the function of the prelude of its text not lowered".into());
        };
        let i = if *t == Type::F64 { d } else { f };
        self.text_prints.insert(i);
        self.emit(format!("call {}void @isc_fn_{}_print({} {}, i64 {})", self.fn_cc(), i, lt(t), x.r, pre));
        Ok(())
    }

    fn print_value(&mut self, x: &V, t: &Type, shape: &Shape) -> R<()> {
        match t {
            // refused by the checker (D339)
            Type::Handle => return Err("a resource of the system cannot be printed".into()),
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
            Type::F64 | Type::F32 => self.print_float(x, t, "0")?,
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
                            let f = self.ins(format!("extractvalue {} {}, {}", x.t, x.r, slot));
                            let ct = ts[slot as usize].clone();
                            // a float after a separator: both written at once
                            if i > 0 && matches!(ct, Type::F64 | Type::F32) {
                                self.print_float(&v(f, lt(&ct)), &ct, "2")?;
                                continue;
                            }
                            if i > 0 {
                                self.print_lit(", ");
                            }
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
                    let f = self.ins(format!("extractvalue {} {}, {}", x.t, x.r, i));
                    // a float after a separator (not the name of a field):
                    // both written at once
                    if i > 0 && names.is_none() && matches!(c, Type::F64 | Type::F32) {
                        self.print_float(&v(f, lt(c)), c, "2")?;
                        continue;
                    }
                    if i > 0 {
                        self.print_lit(", ");
                    }
                    if let Some(ns) = &names {
                        self.print_lit(&format!("{}: ", ns[i]));
                    }
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
                    // a float: its separator written with it
                    if matches!(et, Type::F64 | Type::F32) {
                        let pre = g.ins(format!("select i1 {}, i64 0, i64 2", first));
                        let el = g.load_elem(&p, &n, i, &et, 1);
                        return g.print_float(&el, &et, &pre);
                    }
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

    /// A primitive of the kernel (D601, D605). The arena: its base in a
    /// word of the module, written once by `$arenaReserve`, its size a
    /// constant; an address is tested to be inside it (`at - base` and the
    /// width below the size: one comparison, false while nothing is
    /// reserved) before its bytes are read or written. The zone of the
    /// thread: 4096 bytes of its thread-local storage, an offset tested
    /// against its end.
    /// `at`, for a primitive that takes an address: the address is `x + c`,
    /// `c` a constant; its offset in the arena is computed as the offset of
    /// `x` plus `c`, so that the tests of the words of a range tested once
    /// from `x` (`$arenaHas`) are comparisons of that one offset, which
    /// LLVM folds.
    fn kernel(&mut self, op: isc_ir::KernelOp, vals: &[V], at: Option<(String, i64)>, pos: &str) -> V {
        use isc_ir::KernelOp as K;
        self.kernel_used = true;
        // The base of the arena: a word of the module that only its code
        // reads and writes (`internal`: LLVM knows that no store through an
        // address changes it), `NO_ARENA` until the arena is reserved. Its
        // size is a constant (`KERNEL_ARENA`): the test of a range is one
        // comparison of an offset with a constant, and LLVM folds the tests
        // of the words inside a range already tested (D605).
        self.decls.insert(format!("@isc_arena_base = internal global i64 {}", NO_ARENA));
        let arena = isc_ir::KERNEL_ARENA;
        let fail = |g: &mut Self, ok: &str, msg: &str| {
            let (m, n) = g.str_const(msg);
            let (none, _) = g.str_const("");
            g.guard(ok, format!("call void @isc_panic_text(ptr {}, i64 {}, ptr {}, i64 0, ptr {}, i64 0, {})", m, n, none, none, pos));
        };
        // the offset of `a` in the arena (not one of it when it is not below
        // the size: before the reservation, no address of the program is)
        let offset = |g: &mut Self, a: &str| -> String {
            let base = g.ins(format!("load i64, ptr @isc_arena_base, !tbaa !{}", TBAA_BASE));
            match &at {
                Some((x, c)) if *c != 0 => {
                    let off = g.ins(format!("sub i64 {}, {}", x, base));
                    g.ins(format!("add i64 {}, {}", off, c))
                }
                _ => g.ins(format!("sub i64 {}, {}", a, base)),
            }
        };
        // the address of the `w` bytes at `a`, tested inside the arena
        let place = |g: &mut Self, a: &str, w: u8| -> String {
            let off = offset(g, a);
            let ok = g.ins(format!("icmp ult i64 {}, {}", off, arena - (w as u64 - 1)));
            fail(g, &ok, "the kernel reached outside its arena");
            g.ins(format!("inttoptr i64 {} to ptr", a))
        };
        // the base and the size of the arena for the program: 0 before it is
        // reserved
        let reserved = |g: &mut Self, size: bool| -> String {
            let base = g.ins(format!("load i64, ptr @isc_arena_base, !tbaa !{}", TBAA_BASE));
            let none = g.ins(format!("icmp eq i64 {}, {}", base, NO_ARENA));
            g.ins(format!("select i1 {}, i64 0, i64 {}", none, if size { arena.to_string() } else { base }))
        };
        let zone = |g: &mut Self, at: &str, w: u8| -> String {
            let model = if g.opts.windows { "" } else { "(localexec)" };
            g.decls.insert(format!("@isc_kernel_zone = internal thread_local{} global [{} x i8] zeroinitializer, align 64", model, isc_ir::KERNEL_ZONE));
            let ok = g.ins(format!("icmp ule i64 {}, {}", at, isc_ir::KERNEL_ZONE - w as u64));
            fail(g, &ok, "the kernel reached outside the zone of its thread");
            g.ins(format!("getelementptr inbounds i8, ptr @isc_kernel_zone, i64 {}", at))
        };
        let read = |g: &mut Self, p: &str, w: u8, kind: u64| -> V {
            if w == 8 {
                return v(g.ins(format!("load i64, ptr {}, align 1, !tbaa !{}", p, kind)), "i64");
            }
            let x = g.ins(format!("load i{}, ptr {}, align 1, !tbaa !{}", w * 8, p, kind));
            v(g.ins(format!("zext i{} {} to i64", w * 8, x)), "i64")
        };
        let write = |g: &mut Self, p: &str, w: u8, x: &str, kind: u64| -> V {
            if w == 8 {
                g.emit(format!("store i64 {}, ptr {}, align 1, !tbaa !{}", x, p, kind));
            } else {
                let t = g.ins(format!("trunc i64 {} to i{}", x, w * 8));
                g.emit(format!("store i{} {}, ptr {}, align 1, !tbaa !{}", w * 8, t, p, kind));
            }
            v("0".to_string(), "i8")
        };
        match op {
            K::ArenaReserve => {
                // (with the entry that frees a block of the arena, for the
                // functions of the runtime that free one: where the entries
                // of the allocator are emitted, `kernel_text`)
                self.decls.insert("declare i64 @isc_arena_reserve(i64, ptr)".into());
                let entry = if self.p.kernel.is_some() && !self.opts.isa.is_wasm() && !self.opts.windows { "@isc.kernel.free" } else { "null" };
                let base = self.ins(format!("call i64 @isc_arena_reserve(i64 {}, ptr {})", arena, entry));
                let got = self.ins(format!("icmp ne i64 {}, 0", base));
                let word = self.ins(format!("select i1 {}, i64 {}, i64 {}", got, base, NO_ARENA));
                self.emit(format!("store i64 {}, ptr @isc_arena_base, !tbaa !{}", word, TBAA_BASE));
                v(base, "i64")
            }
            K::ArenaBase => v(reserved(self, false), "i64"),
            K::ArenaSize => v(reserved(self, true), "i64"),
            K::ArenaCommit => {
                self.decls.insert("declare i64 @isc_arena_commit(i64, i64, i64, i64)".into());
                let base = reserved(self, false);
                let size = reserved(self, true);
                v(self.ins(format!("call i64 @isc_arena_commit(i64 {}, i64 {}, i64 {}, i64 {})", vals[0].r, vals[1].r, base, size)), "i64")
            }
            // `bytes` at most the arena, and the offset at most what is left
            // after them
            K::ArenaHas => {
                let off = offset(self, &vals[0].r);
                let fits = self.ins(format!("icmp ule i64 {}, {}", vals[1].r, arena));
                let left = self.ins(format!("sub i64 {}, {}", arena, vals[1].r));
                let within = self.ins(format!("icmp ule i64 {}, {}", off, left));
                v(self.ins(format!("and i1 {}, {}", fits, within)), "i1")
            }
            // (a function of the module, which the allocations of the
            // generated code, redirected to the kernel, are not: `late`)
            K::RuntimeAlloc => {
                self.decls.insert(
                    "define internal i64 @isc.runtime.alloc(i64 %n) nounwind {\nentry:\n  %p = call noalias ptr @isc_alloc(i64 %n)\n  %a = ptrtoint ptr %p to i64\n  ret i64 %a\n}"
                        .into(),
                );
                v(self.ins(format!("call i64 @isc.runtime.alloc(i64 {})", vals[0].r)), "i64")
            }
            K::Load(w) => {
                let p = place(self, &vals[0].r, w);
                read(self, &p, w, TBAA_ARENA)
            }
            K::Store(w) => {
                let p = place(self, &vals[0].r, w);
                write(self, &p, w, &vals[1].r, TBAA_ARENA)
            }
            K::ZoneLoad(w) => {
                let p = zone(self, &vals[0].r, w);
                read(self, &p, w, TBAA_ZONE)
            }
            K::ZoneStore(w) => {
                let p = zone(self, &vals[0].r, w);
                write(self, &p, w, &vals[1].r, TBAA_ZONE)
            }
            // a word of 8 bytes of the arena, on 8 bytes (an atomic
            // operation of LLVM needs its alignment): tested with the place
            K::Cas | K::AtomicLoad | K::AtomicStore => {
                let p = place(self, &vals[0].r, 8);
                let low = self.ins(format!("and i64 {}, 7", vals[0].r));
                let ok = self.ins(format!("icmp eq i64 {}, 0", low));
                fail(self, &ok, "the kernel reached outside its arena");
                match op {
                    K::Cas => {
                        let r = self.ins(format!("cmpxchg ptr {}, i64 {}, i64 {} seq_cst seq_cst, !tbaa !{}", p, vals[1].r, vals[2].r, TBAA_ARENA));
                        v(self.ins(format!("extractvalue {{ i64, i1 }} {}, 0", r)), "i64")
                    }
                    K::AtomicLoad => v(self.ins(format!("load atomic i64, ptr {} seq_cst, align 8, !tbaa !{}", p, TBAA_ARENA)), "i64"),
                    _ => {
                        self.emit(format!("store atomic i64 {}, ptr {} seq_cst, align 8, !tbaa !{}", vals[1].r, p, TBAA_ARENA));
                        v("0".to_string(), "i8")
                    }
                }
            }
            K::SpinHint => {
                if !self.opts.isa.is_wasm() {
                    self.decls.insert("declare void @llvm.x86.sse2.pause()".into());
                    self.emit("call void @llvm.x86.sse2.pause()".to_string());
                }
                v("0".to_string(), "i8")
            }
        }
    }

    fn action(&mut self, op: ActionOp, args: &[Expr], e: &Expr) -> R<V> {
        // a word of the arena at `x + c` or `x - c`, `c` a constant (D605)
        if let ActionOp::Kernel(o) = op {
            use isc_ir::KernelOp as K;
            if matches!(o, K::Load(_) | K::Store(_) | K::Cas | K::AtomicLoad | K::AtomicStore | K::ArenaHas) {
                let lit = |k: &Expr| match k.kind {
                    ExprKind::Lit(isc_ir::Lit::Int(c)) => Some(c),
                    _ => None,
                };
                let (x, c) = match &args[0].kind {
                    ExprKind::Binary(BinOp::Add, x, k) if lit(k).is_some() => (&**x, lit(k).unwrap()),
                    ExprKind::Binary(BinOp::Sub, x, k) if lit(k).is_some() => (&**x, lit(k).unwrap().wrapping_neg()),
                    _ => (&args[0], 0),
                };
                let xv = self.ex(x, 1)?;
                let a = if c == 0 { xv.r.clone() } else { self.ins(format!("add i64 {}, {}", xv.r, c)) };
                let mut vals = vec![v(a, "i64")];
                for a in &args[1..] {
                    vals.push(self.ex(a, 1)?);
                }
                let pos = Self::pos(e.span);
                return Ok(self.kernel(o, &vals, Some((xv.r, c)), &pos));
            }
        }
        let mut vals = Vec::new();
        for a in args {
            vals.push(self.ex(a, 1)?);
        }
        let pos = Self::pos(e.span);
        Ok(match op {
            ActionOp::Kernel(o) => self.kernel(o, &vals, None, &pos),
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
            // an integer: its digits and the newline in one call (D498)
            ActionOp::Print(_) if matches!(args[0].ty, Type::Int(_)) => {
                let Type::Int(k) = &args[0].ty else { unreachable!() };
                let w = if k.bits() < 64 {
                    let op = if k.signed() { "sext" } else { "zext" };
                    self.ins(format!("{} i{} {} to i64", op, k.bits(), vals[0].r))
                } else {
                    vals[0].r.clone()
                };
                let f = if k.signed() { "isc_print_i64_line" } else { "isc_print_u64_line" };
                self.emit(format!("call void @{}(i64 {})", f, w));
                v("false", "i1")
            }
            ActionOp::Print(k) => {
                let shape = self.p.shapes[k as usize].clone();
                self.print_value(&vals[0], &args[0].ty, &shape)?;
                self.emit("call void @isc_print_newline()");
                v("false", "i1")
            }
            ActionOp::Clock => v(self.ins("call i64 @isc_clock_ns()".into()), "i64"),
            ActionOp::PrintBytes => {
                let (p, n) = self.str_parts(&vals[0]);
                self.emit(format!("call void @isc_print_bytes(ptr {}, i64 {})", p, n));
                v("false", "i1")
            }
            // the fallible forms (D138): the value and the message through
            // pointers, then the pair
            // a primitive of the system (D322): its arguments by their
            // parts, its tuple written by the runtime at OUT
            ActionOp::Sys(o) => {
                let tt = lt(&e.ty);
                let out = self.alloca(&tt);
                let mut parts = vec![format!("ptr {}", out)];
                let mut types = vec!["ptr".to_string()];
                for (a, x) in args.iter().zip(&vals) {
                    match &a.ty {
                        Type::Str | Type::Array(..) => {
                            let (p, n) = self.str_parts(x);
                            parts.push(format!("ptr {}", p));
                            parts.push(format!("i64 {}", n));
                            types.extend(["ptr".to_string(), "i64".to_string()]);
                        }
                        Type::Bool => {
                            let b = self.ins(format!("zext i1 {} to i32", x.r));
                            parts.push(format!("i32 {}", b));
                            types.push("i32".to_string());
                        }
                        _ => {
                            parts.push(format!("i64 {}", x.r));
                            types.push("i64".to_string());
                        }
                    }
                }
                let f = sys_symbol(o);
                self.decls.insert(format!("declare void @{}({})", f, types.join(", ")));
                self.emit(format!("call void @{}({})", f, parts.join(", ")));
                v(self.ins(format!("load {}, ptr {}", tt, out)), &tt)
            }
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
        // what the program wrote is out before the function writes its own
        // output (D299)
        self.emit("call void @isc_flush()");
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
        // the names of registers of this function alone (D518)
        let saved_parts = std::mem::take(&mut self.part_values);
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
        self.part_values = saved_parts;
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
