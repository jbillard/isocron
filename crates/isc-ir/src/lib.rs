//! Isocron high-level IR (SPEC section 13).
//!
//! The IR is produced from the typed AST by `lower`: every non-recursive
//! function is inlined and every higher-order function is specialized, so a
//! program is a single expression tree (the body of `main`). Types are
//! monomorphic and sizes are erased: array lengths are run-time values, and
//! the size tests required by section 4 are explicit `CheckSize` nodes.
//!
//! `passes` then applies, in order, the fusion preparation (rule F1), the
//! binding of temporaries, and the ownership analysis (section 8) whose
//! results (`Use::Move`, `drop` flags, `Copy` nodes) drive the code
//! generator. The interpreter ignores them.

pub use isc_types::tast::{Fallible, KernelOp, MathFn, SysOp, KERNEL_ARENA, KERNEL_ZONE};

pub mod blocks;
pub mod lower;
pub mod passes;
pub mod print;
pub mod report;
pub mod requirements;
pub mod verify;

pub use isc_syntax::ast::{BinOp, UnOp};
use isc_syntax::Span;
pub use isc_types::types::IntKind;

pub type VarId = u32;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Type {
    Int(IntKind),
    F64,
    F32,
    Bool,
    Unit,
    Str,
    /// An integer of any size (D156): laid out, owned and freed as a
    /// string (its bytes: `isc_runtime_core::big`).
    Big,
    Tuple(Vec<Type>),
    /// An array; its length when the program fixes it (D204).
    Array(Box<Type>, Fixed),
    /// A value of the recursive type `Program::recs[i]`: a pointer to a
    /// node on the heap, with a reference count (D103).
    Rec(u32),
    /// A resource of the system (D339): its number (a descriptor, a
    /// socket); closed exactly once, which the checker proves (a linear
    /// value of the typed tree), so a scalar here.
    Handle,
}

/// The length of an array known when the program is compiled (`f64[5]`,
/// D204), or not (`f64[n]`): read by the generated code only. Two array
/// types are the same type whatever it says: it takes no part in equality
/// or hashing.
#[derive(Clone, Copy, Debug, Default)]
pub struct Fixed(pub Option<u32>);

impl Fixed {
    pub const ANY: Fixed = Fixed(None);
}

impl PartialEq for Fixed {
    fn eq(&self, _: &Fixed) -> bool {
        true
    }
}

impl Eq for Fixed {}

impl std::hash::Hash for Fixed {
    fn hash<H: std::hash::Hasher>(&self, _: &mut H) {}
}

impl Type {
    pub fn is_array(&self) -> bool {
        matches!(self, Type::Array(_, _))
    }
    pub fn elem(&self) -> &Type {
        match self {
            Type::Array(e, _) => e,
            _ => panic!("elem of non-array type {:?}", self),
        }
    }
    pub fn is_scalar(&self) -> bool {
        matches!(self, Type::Int(_) | Type::F64 | Type::F32 | Type::Bool)
    }
    pub fn is_float(&self) -> bool {
        matches!(self, Type::F64 | Type::F32)
    }
    /// True when a value of this type owns heap memory.
    /// A slice of an array of this type is a view of its source (rule F4,
    /// D97): its elements are contiguous. The columns of an array of tuples
    /// are placed according to its length, so a slice of one is a copy.
    pub fn slice_is_view(&self) -> bool {
        matches!(self, Type::Array(e, _) if !matches!(**e, Type::Tuple(_) | Type::Array(_, _)))
    }

    /// Holds a node of a recursive type (counted owners, D129, D175).
    pub fn has_nodes(&self) -> bool {
        match self {
            Type::Rec(_) => true,
            Type::Array(e, _) => e.has_nodes(),
            Type::Tuple(ts) => ts.iter().any(|t| t.has_nodes()),
            _ => false,
        }
    }

    pub fn has_heap(&self) -> bool {
        match self {
            // a string is owned and freed as an array of bytes (D134)
            Type::Array(_, _) | Type::Rec(_) | Type::Str | Type::Big => true,
            Type::Tuple(ts) => ts.iter().any(|t| t.has_heap()),
            _ => false,
        }
    }
}

/// Integer literals are stored canonically: the value truncated to the width
/// of its type, then sign-extended (signed types) or zero-extended (unsigned
/// types) to 64 bits.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Lit {
    Int(i64),
    F64(f64),
    F32(f32),
    Bool(bool),
    Unit,
}

/// Canonical representation of an integer of kind `k`.
pub fn canon(k: IntKind, v: i128) -> i64 {
    let bits = k.bits();
    let mask: u128 = if bits == 64 { u64::MAX as u128 } else { (1u128 << bits) - 1 };
    let t = (v as u128) & mask;
    if k.signed() && bits < 64 && (t >> (bits - 1)) & 1 == 1 {
        (t | !mask) as i64
    } else {
        t as u64 as i64
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Use {
    Borrow,
    /// Last use of an owned variable: ownership is transferred.
    Move,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ActionOp {
    ReadF64,
    ReadF32,
    ReadBytes,
    WriteF64,
    WriteF32,
    WriteBytes,
    /// `print`, with the index of its shape in `Program::shapes`.
    Print(u32),
    Clock,
    ParseF64,
    /// Text (D135).
    ReadText,
    WriteText,
    ParseCsvF64,
    ReadCsvF64,
    /// The bytes of a `[n]u8` to the standard output (D136).
    PrintBytes,
    /// A fallible form of an action (D138): `(value, message)`, or a
    /// message for a write.
    OrError(Fallible),
    /// A call of the C function `Program::externs[i]` (D125).
    Extern(u32),
    /// The standard input (D155): its bytes, its text.
    ReadStdin,
    ReadStdinText,
    /// A primitive of the system (D322): its value and the code of its
    /// error, a tuple.
    Sys(SysOp),
    /// A primitive of the kernel of the runtime (D601): the raw memory of
    /// the arena and the zone of the thread.
    Kernel(KernelOp),
}

/// The operations on strings (D134).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StrOp {
    /// `a ++ b`: a new string.
    Concat,
    /// The number of bytes.
    Len,
    /// The bytes, a new `[k]u8`.
    Bytes,
    /// A string of bytes (a panic if they are not UTF-8).
    FromBytes,
    /// The text of a scalar (that of `print`): a new string.
    ToString,
    /// `==`, `!=`, `<`, `<=`, `>`, `>=` by bytes.
    Cmp(BinOp),
    /// Text (D135): new arrays of new strings, or a new string.
    Lines,
    Split,
    Join,
    /// The functions of TypeScript (D155).
    /// The byte index of the last `t` in `s`, -1 if none (D482).
    LastIndexOf,
    StartsWith,
    EndsWith,
    ReplaceAll,
    ToUpper,
    ToLower,
    PadStart,
    PadEnd,
    Repeat,
    /// `toFixed(x, d)` of a `f64`.
    ToFixed,
    Slice,
    /// `argsort(x)` of an array of scalars or strings (D155): a new `[n]i64`.
    ArgSort,
    /// `$group(keys)` (D167): the first, last and count of each distinct
    /// key, in the order of first occurrence: a new `[3m]i64`.
    Group,
    /// `$groupCounts(keys)` (D317): the columns of `Group`, the last
    /// indices zeros (`countBy` reads the first indices and the counts).
    GroupCounts,
    /// `$hashIndex(keys)` (D492): the index of hashes of the distinct keys
    /// of a map, a new `[s]i64` (a power of two at least twice the keys).
    HashIndex,
    /// `$hashFind(keys, index, k)` (D492): the position of `k`, -1 if absent.
    HashFind,
    /// `readWord(b, at, n)` (D580): the `n` bytes of `b` from `at` as a word.
    ReadWord,
    /// `$panic(a, b, c)` (D582): a panic, its message the three texts, of
    /// the type its place expects.
    Panic,
    /// `leadingZeros(x)` (D588): the zero bits above the highest one.
    LeadingZeros,
    /// `$textTail(w0, w1, w2, n, minus)` (D588): the last `n` bytes of
    /// three words, a `-` before them when `minus`.
    TextTail,
    /// `$floatBits(x)`: the bits of a float as a `u64`.
    FloatBits,
    /// `$mulHigh(a, b)`: the high half of the product of two `u64`.
    MulHigh,
    /// `$decimalText(d, w1, w2, n, point, minus)`: a decimal number of 17
    /// digits as a string, `n` of them written, `point` before the point.
    DecimalText,
    /// `$withText(t, at, piece)`: `t` with the bytes of `piece` from `at`,
    /// written in `t` itself when it is given (a variable at its last use).
    WithText,
    /// An operation of `bigint` (D156).
    Big(BigOp),
    /// A regular expression (D157); a `RegExp` is the string `flags/source`.
    Rx(RxOp),
    /// `s.replace(a, b)` of a text: its first `a` (D157).
    ReplaceFirst,
    /// ECMAScript's `Number(s)`, `parseFloat(s)`, `parseInt(s, radix)` (D156).
    JsNumber,
    JsParseFloat,
    JsParseInt,
}

impl StrOp {
    /// The operation panics for some values of its operands (D581): bytes
    /// that are not UTF-8 (`stringOf`), an empty separator (`split`), a
    /// slice past the end or inside a character, a negative count
    /// (`repeat`), a number of digits out of 0 to 100 (`toFixed`), a word
    /// out of its bytes (`readWord`), a `bigint` from a text that is no
    /// integer, divided by zero or raised to a negative or huge exponent, a
    /// regular expression made of a pattern that is not one or replacing
    /// all without the flag `g`, a text written outside another or inside
    /// one of its characters (`$withText`). The passes never compute it
    /// where the program does not (`passes::cannot_panic`). A memory
    /// exhausted is no panic of a value: not counted, as for `iota(n)`.
    pub fn may_panic(&self) -> bool {
        matches!(
            self,
            StrOp::FromBytes
                | StrOp::Split
                | StrOp::Slice
                | StrOp::Repeat
                | StrOp::ToFixed
                | StrOp::ReadWord
                | StrOp::Panic
                | StrOp::TextTail
                | StrOp::DecimalText
                | StrOp::WithText
                | StrOp::Big(BigOp::Div | BigOp::Rem | BigOp::Pow | BigOp::Parse)
                | StrOp::Rx(RxOp::New | RxOp::Replace | RxOp::ReplaceAll)
        )
    }
}

/// The operations of `bigint` (D156): `/` truncated toward zero, `%` of the
/// sign of the dividend, as ECMAScript.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BigOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Pow,
    Neg,
    /// `<`, `<=`, `>`, `>=` (equality is by bytes: the bytes are canonical).
    Cmp(BinOp),
    /// From an `i64`, a `u64`, a text (a panic if it is not an integer).
    FromI64,
    FromU64,
    Parse,
    /// To an `i64` (modulo 2^64), the nearest `f64`, its decimal text.
    ToI64,
    ToF64,
    ToString,
    /// `argsort` of an array of `bigint`.
    ArgSort,
}

/// The operations of regular expressions (D157).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RxOp {
    /// `RegExp(source, flags)`: checked (a panic when it cannot compile).
    New,
    Test,
    Search,
    Count,
    Matches,
    /// `s.replace(re, to)`, `s.replaceAll(re, to)`.
    Replace,
    ReplaceAll,
}

/// A C function declared with `extern` (D125): its parameters (scalars,
/// `string`, arrays of scalars) and its result (a scalar or `()`).
#[derive(Clone, Debug, PartialEq)]
pub struct ExternFn {
    pub name: String,
    pub params: Vec<Type>,
    pub ret: Type,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Lambda {
    pub params: Vec<VarId>,
    pub body: Box<Expr>,
    /// Set by the ownership pass: the parameter owns its value and must be
    /// freed at the end of the body (fold accumulator not consumed).
    pub drop_params: Vec<bool>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Expr {
    pub kind: ExprKind,
    pub ty: Type,
    pub span: Span,
    /// Inside a function with `allowReassoc` (SPEC section 10, D109): its
    /// floating-point operations may be reassociated and contracted.
    pub reassoc: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ExprKind {
    Lit(Lit),
    Var(VarId, Use),
    /// `stmt` marks a statement of an action: it is evaluated in order and
    /// never removed (section 12). `drop` (set by the ownership pass) frees
    /// the variable after the body.
    Let { var: VarId, init: Box<Expr>, body: Box<Expr>, stmt: bool, drop: bool },
    Tuple(Vec<Expr>),
    Proj(Box<Expr>, u32),
    /// Scalar operations.
    Unary(UnOp, Box<Expr>),
    /// Scalar operations; `and`/`or` are lowered to `If`.
    Binary(BinOp, Box<Expr>, Box<Expr>),
    /// Scalar base, integer exponent (section 10).
    Pow(Box<Expr>, Box<Expr>),
    /// Scalar conversion to `ty`.
    Convert(Box<Expr>),
    Sqrt(Box<Expr>),
    If(Box<Expr>, Box<Expr>, Box<Expr>),
    /// Length of an array, as `i64`.
    Len(Box<Expr>),
    ArrayLit(Vec<Expr>),
    /// An array literal of scalar constants (D173): data of the program,
    /// borrowed and never freed (a consumer gets a copy).
    ConstArray(Vec<Lit>),
    /// An array literal of string literals (D401): data of the program, the
    /// array and the bytes of its strings, borrowed and never freed (a
    /// consumer gets a copy, each string copied).
    ConstStrs(Vec<String>),
    Index(Box<Expr>, Box<Expr>),
    /// `x[a..b]`, the bounds as `i64`: a new array of `b - a` elements, a
    /// panic unless `0 <= a <= b <= len(x)` (SPEC section 6; a view that is
    /// not copied comes with rule F4).
    Slice(Box<Expr>, Box<Expr>, Box<Expr>),
    /// `window(x, k)`, `k` as `i64`: the `len(x) - k + 1` windows of `k`
    /// elements, a view of `x` (SPEC section 6); a panic when `k > len(x)`.
    Window(Box<Expr>, Box<Expr>),
    Update { target: Box<Expr>, index: Box<Expr>, value: Box<Expr>, require_unique: bool },
    /// Element-wise map over one or more arrays of equal length.
    Map(Vec<Expr>, Lambda),
    /// Fixed-form reduction (section 10).
    Reduce(Box<Expr>, Lambda, Box<Expr>),
    Fold(Box<Expr>, Box<Expr>, Lambda),
    /// The elements of `x` where the lambda is true, in order: a new array
    /// (SPEC section 6; rule F2 fuses it with its consumer, phase 2).
    Filter(Box<Expr>, Lambda),
    /// `mapParallel(x, f)` (D373): the action of the lambda applied to each
    /// element of `x` (a variable), each application a task of the pool,
    /// at the same time; a new array of the results, in the order of `x`.
    /// Never fused, never removed (an action).
    Tasks(Box<Expr>, Lambda),
    /// Inclusive cumulation from left to right (SPEC section 6): a new array,
    /// out[i] = op(out[i - 1], x[i]) with out[-1] = e (rule M2).
    Scan(Box<Expr>, Lambda, Box<Expr>),
    Zeros(Box<Expr>),
    Iota(Box<Expr>),
    Fill(Box<Expr>, Box<Expr>),
    /// Panics unless both `i64` values are equal, then evaluates the body.
    CheckSize(Box<Expr>, Box<Expr>, Box<Expr>),
    /// Panics if the `i64` value is negative (a `size` argument), then
    /// evaluates the body.
    CheckNonNegative(Box<Expr>, Box<Expr>),
    /// Deep copy (inserted by the ownership pass).
    Copy(Box<Expr>),
    /// The zero value of the type: 0, `false`, empty string, empty array,
    /// tuple of zeros (the unused slots of a value of a sum type).
    Zero,
    Action(ActionOp, Vec<Expr>),
    /// Evaluates the body `reps` times; the result is `(last value, ns)`.
    Bench(Box<Expr>, Box<Expr>),
    /// A call of the recursive function `Program::fns[f]` (SPEC section 11:
    /// not inlined). The arguments are borrowed; the result is new.
    Call(u32, Vec<Expr>),
    /// A new node of a recursive type holding the tuple (its parts are taken
    /// by the node), and the tuple a node holds (borrowed from the node).
    Box(Box<Expr>),
    Unbox(Box<Expr>),
    /// A mathematical function of the runtime (D108).
    Math(MathFn, Vec<Expr>),
    /// A string literal: a new string each time it is evaluated (D134).
    StrLit(String),
    /// Component `i` of a tuple variable that owns its value, taken out of
    /// it (D138): every component that owns memory is taken once, by a
    /// `let` right after the tuple's own; the tuple is then not freed as a
    /// whole, each component is (`let (a, b) = f(x)` without copies).
    Take(Box<Expr>, u32),
    /// An operation on strings (D134); its operands are only read.
    StrOp(StrOp, Vec<Expr>),
    /// Evaluates the body, then frees the variable: set by the ownership
    /// pass in the branch of an `if` that does not consume a variable the
    /// other branch moves (D105).
    Free(VarId, Box<Expr>),
}

/// A recursive sum type, for its type arguments: the tuple a node holds (the
/// layout of a sum: the tag, then the slots of every variant).
#[derive(Clone, Debug, PartialEq)]
pub struct RecType {
    pub name: String,
    pub layout: Type,
    pub shape: Shape,
}

/// A recursive function, specialized for its type arguments.
#[derive(Clone, Debug, PartialEq)]
pub struct Function {
    pub name: String,
    pub params: Vec<VarId>,
    pub body: Expr,
    pub span: isc_syntax::Span,
    /// `requireTail` (SPEC section 9), with its position (D106).
    pub require_tail: Option<isc_syntax::Span>,
    /// `@cold` (D584): compiled `cold` and `noinline`, its calls unlikely.
    pub cold: bool,
    /// Per parameter: owned by the function and taken apart on entry (the
    /// state of a loop, D206); its caller lends it, so the generated code
    /// copies it once on entry.
    pub consumed: Vec<bool>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VarInfo {
    pub name: String,
    pub ty: Type,
    /// Bound by code of the prelude inlined into the program (D598): its
    /// checks are not in the report of the program.
    pub prelude: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Program {
    /// The `args` parameter of `main`.
    pub args: VarId,
    /// `main` frees the array `args` (it owns it and does not give it away:
    /// set by the ownership pass, D96).
    pub drop_args: bool,
    /// Why each `Copy` was inserted, in evaluation order, for the memory
    /// report (D98).
    pub copy_reasons: Vec<(isc_syntax::Span, String)>,
    /// The recursive functions (D102).
    pub fns: Vec<Function>,
    /// The recursive types (D103).
    pub recs: Vec<RecType>,
    /// Where a function is chosen at run time (defunctionalized, D107).
    pub choices: Vec<isc_syntax::Span>,
    /// The functions with `allowReassoc` (D109), for the memory report.
    pub reassoc_fns: Vec<(String, isc_syntax::Span)>,
    /// The C functions declared with `extern` (D125).
    pub externs: Vec<ExternFn>,
    pub body: Expr,
    pub vars: Vec<VarInfo>,
    /// The shapes of the printed values (`ActionOp::Print`).
    pub shapes: Vec<Shape>,
    /// The primitives of the system that `main` reaches, each with a path of
    /// calls to it and the place of the call in the program (D324): a target
    /// that lacks one refuses the program.
    pub sys_paths: Vec<(SysOp, Vec<String>, isc_syntax::Span)>,
    /// The tuples of sum types told apart by their type alone, with the
    /// slots of each variant (D448): a value of one frees the fields of its
    /// variant only.
    pub sum_slots: Vec<(Type, Vec<Vec<u32>>)>,
    /// The arrays of texts whose texts are views of the texts of an array
    /// or a text that lives longer (D555): each owns its block alone, freed
    /// without its texts, and is never moved (`passes::borrow_elements`).
    pub views: std::collections::BTreeSet<VarId>,
    /// The functions of the prelude that write the text of an `i64` and of
    /// a `u64` (`$rtIntText`, `$rtUintText`, D588), lowered when the program
    /// writes one: the code generator calls them for `toString` of an
    /// integer that none of its fusions writes in place.
    pub int_text: Option<(u32, u32)>,
    /// The functions of the prelude that write the text of a `f64` and of
    /// a `f32` (`$rtF64Text`, `$rtF32Text`), lowered when the program
    /// writes one: the code generator calls them for `toString` of a float.
    pub float_text: Option<(u32, u32)>,
    /// The function of the prelude that gives the digits of a `u64` in three
    /// words and their number (`$rtDigitWords`, D606), lowered when the
    /// program writes the text of an integer and pads a text: the code
    /// generator writes the padded text of an integer from them, in one
    /// block (D348).
    pub digit_words: Option<u32>,
    /// The function of the prelude that finishes `toUpperCase` and
    /// `toLowerCase` of a text that is not ASCII (`$caseText`), lowered
    /// when the program has one: the code generator calls it where a byte
    /// of the text is not ASCII.
    pub case_text: Option<u32>,
    /// The entries of the allocator written in Isocron (D605), lowered for
    /// every program: `$kStart`, `$kAlloc`, `$kFree`.
    pub kernel: Option<(u32, u32, u32)>,
}

/// What `print` shows beyond the IR type of a value: the names of the
/// fields of records, which are tuples in the IR.
#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    /// As the type says: no record inside.
    Plain,
    Tuple(Vec<Shape>),
    /// Field names, in the order of the IR tuple, and their shapes.
    Record(Vec<String>, Vec<Shape>),
    Array(Box<Shape>),
    /// A sum type (an IR tuple: the tag, then the slots of every variant):
    /// for each variant, its constructor, the indices of its slots, and
    /// their shapes.
    Sum(Vec<(String, Vec<u32>, Vec<Shape>)>),
    /// A recursive type: the shape of its layout is `Program::recs[i].shape`.
    Rec(u32),
}

impl Expr {
    pub fn new(kind: ExprKind, ty: Type, span: Span) -> Expr {
        Expr { kind, ty, span, reassoc: false }
    }

    /// Marks this expression and all inside it `reassoc`.
    pub fn mark_reassoc(&mut self) {
        self.reassoc = true;
        for c in self.children_mut() {
            c.mark_reassoc();
        }
    }
    pub fn lit(l: Lit, ty: Type, span: Span) -> Expr {
        Expr::new(ExprKind::Lit(l), ty, span)
    }
    pub fn i64(v: i64, span: Span) -> Expr {
        Expr::new(ExprKind::Lit(Lit::Int(v)), Type::Int(IntKind::I64), span)
    }
    pub fn var(v: VarId, ty: Type, span: Span) -> Expr {
        Expr::new(ExprKind::Var(v, Use::Borrow), ty, span)
    }

    /// Calls `f` on each direct child expression (lambda bodies included).
    pub fn children(&self) -> Vec<&Expr> {
        use ExprKind::*;
        match &self.kind {
            Lit(_) | Var(..) | Zero | StrLit(_) | ConstArray(_) | ConstStrs(_) => vec![],
            Let { init, body, .. } => vec![init, body],
            Tuple(es) | ArrayLit(es) | Action(_, es) | Call(_, es) | Math(_, es) | StrOp(_, es) => es.iter().collect(),
            Proj(e, _) | Take(e, _) | Unary(_, e) | Convert(e) | Sqrt(e) | Len(e) | Zeros(e) | Iota(e) | Copy(e) | Box(e) | Unbox(e) | Free(_, e) => vec![e],
            Binary(_, a, b) | Pow(a, b) | Index(a, b) | Fill(a, b) | Bench(a, b) | CheckNonNegative(a, b) | Window(a, b) => vec![a, b],
            If(a, b, c) | CheckSize(a, b, c) | Slice(a, b, c) => vec![a, b, c],
            Update { target, index, value, .. } => vec![index, value, target],
            Map(ins, l) => {
                let mut v: Vec<&Expr> = ins.iter().collect();
                v.push(&l.body);
                v
            }
            Filter(x, l) | Tasks(x, l) => vec![x, &l.body],
            Reduce(x, l, e) | Scan(x, l, e) => vec![x, e, &l.body],
            Fold(x, i, l) => vec![x, i, &l.body],
        }
    }

    pub fn children_mut(&mut self) -> Vec<&mut Expr> {
        use ExprKind::*;
        match &mut self.kind {
            Lit(_) | Var(..) | Zero | StrLit(_) | ConstArray(_) | ConstStrs(_) => vec![],
            Let { init, body, .. } => vec![init, body],
            Tuple(es) | ArrayLit(es) | Action(_, es) | Call(_, es) | Math(_, es) | StrOp(_, es) => es.iter_mut().collect(),
            Proj(e, _) | Take(e, _) | Unary(_, e) | Convert(e) | Sqrt(e) | Len(e) | Zeros(e) | Iota(e) | Copy(e) | Box(e) | Unbox(e) | Free(_, e) => vec![e],
            Binary(_, a, b) | Pow(a, b) | Index(a, b) | Fill(a, b) | Bench(a, b) | CheckNonNegative(a, b) | Window(a, b) => vec![a, b],
            If(a, b, c) | CheckSize(a, b, c) | Slice(a, b, c) => vec![a, b, c],
            Update { target, index, value, .. } => vec![index, value, target],
            Map(ins, l) => {
                let mut v: Vec<&mut Expr> = ins.iter_mut().collect();
                v.push(&mut l.body);
                v
            }
            Filter(x, l) | Tasks(x, l) => vec![x, &mut l.body],
            Reduce(x, l, e) | Scan(x, l, e) => vec![x, e, &mut l.body],
            Fold(x, i, l) => vec![x, i, &mut l.body],
        }
    }
}
