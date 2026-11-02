//! Reference interpreter of the IR (SPEC section 13: "oracle bit à bit").
//!
//! It implements the semantics directly and simply: `reduce` uses the fixed
//! form of section 10, `fold` and `scan` are sequential, integer arithmetic
//! is modular, floats are IEEE 754 binary64 without contraction. It runs on
//! the IR after the middle-end passes, which never change a value; it only
//! uses their `Use::Move` marks to update arrays in place.

use std::io::Write;
use std::collections::HashSet;
use std::rc::Rc;

use isc_ir::*;
pub mod ffi;
pub mod rt;

/// Reduction parameters of SPEC section 10.
pub const LANES: usize = 32;
pub const BLOCK: usize = 4096;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Int(i64),
    F64(f64),
    F32(f32),
    Bool(bool),
    Unit,
    Str(Rc<str>),
    Tuple(Rc<Vec<Value>>),
    Array(Rc<Array>),
    /// A node of a recursive type: the tuple it holds (D103).
    Node(Rc<Value>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Array {
    Int(Vec<i64>),
    F64(Vec<f64>),
    F32(Vec<f32>),
    Bool(Vec<bool>),
    Str(Vec<Rc<str>>),
    /// An array of tuples, one column per component (SPEC section 4); the
    /// columns have the same length.
    Tuple(Vec<Array>),
    /// An array of arrays (SPEC section 4): its rows, of the same length.
    Rows(Vec<Rc<Array>>),
    /// An array of values of a recursive type: its nodes (D130).
    Nodes(Vec<Rc<Value>>),
}

impl Array {
    pub fn len(&self) -> usize {
        match self {
            Array::Int(v) => v.len(),
            Array::F64(v) => v.len(),
            Array::F32(v) => v.len(),
            Array::Bool(v) => v.len(),
            Array::Str(v) => v.len(),
            Array::Tuple(cols) => cols[0].len(),
            Array::Rows(rows) => rows.len(),
            Array::Nodes(v) => v.len(),
        }
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn get(&self, i: usize) -> Value {
        match self {
            Array::Int(v) => Value::Int(v[i]),
            Array::F64(v) => Value::F64(v[i]),
            Array::F32(v) => Value::F32(v[i]),
            Array::Bool(v) => Value::Bool(v[i]),
            Array::Str(v) => Value::Str(v[i].clone()),
            Array::Tuple(cols) => Value::Tuple(Rc::new(cols.iter().map(|c| c.get(i)).collect())),
            Array::Rows(rows) => Value::Array(rows[i].clone()),
            Array::Nodes(v) => Value::Node(v[i].clone()),
        }
    }
    fn set(&mut self, i: usize, x: Value) {
        match (self, x) {
            (Array::Int(v), Value::Int(x)) => v[i] = x,
            (Array::F64(v), Value::F64(x)) => v[i] = x,
            (Array::F32(v), Value::F32(x)) => v[i] = x,
            (Array::Bool(v), Value::Bool(x)) => v[i] = x,
            (Array::Str(v), Value::Str(x)) => v[i] = x,
            (Array::Tuple(cols), Value::Tuple(xs)) => {
                for (c, x) in cols.iter_mut().zip(xs.iter()) {
                    c.set(i, x.clone());
                }
            }
            (Array::Rows(rows), Value::Array(r)) => rows[i] = r,
            (Array::Nodes(v), Value::Node(x)) => v[i] = x,
            _ => panic!("ill-typed array store"),
        }
    }
    fn with_capacity(elem: &Type, n: usize) -> Array {
        match elem {
            Type::Int(_) => Array::Int(Vec::with_capacity(n)),
            Type::F64 => Array::F64(Vec::with_capacity(n)),
            Type::F32 => Array::F32(Vec::with_capacity(n)),
            Type::Bool => Array::Bool(Vec::with_capacity(n)),
            Type::Str => Array::Str(Vec::with_capacity(n)),
            Type::Tuple(ts) => Array::Tuple(ts.iter().map(|t| Array::with_capacity(t, n)).collect()),
            Type::Array(_) => Array::Rows(Vec::with_capacity(n)),
            Type::Rec(_) => Array::Nodes(Vec::with_capacity(n)),
            t => panic!("arrays of {:?} are not supported", t),
        }
    }
    fn push(&mut self, x: Value) {
        match (self, x) {
            (Array::Int(v), Value::Int(x)) => v.push(x),
            (Array::F64(v), Value::F64(x)) => v.push(x),
            (Array::F32(v), Value::F32(x)) => v.push(x),
            (Array::Bool(v), Value::Bool(x)) => v.push(x),
            (Array::Str(v), Value::Str(x)) => v.push(x),
            (Array::Tuple(cols), Value::Tuple(xs)) => {
                for (c, x) in cols.iter_mut().zip(xs.iter()) {
                    c.push(x.clone());
                }
            }
            (Array::Rows(rows), Value::Array(r)) => rows.push(r),
            (Array::Nodes(v), Value::Node(x)) => v.push(x),
            _ => panic!("ill-typed array push"),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Panic {
    pub line: u32,
    pub col: u32,
    pub msg: String,
}

impl std::fmt::Display for Panic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", rt::panic_text(self.line, self.col, &self.msg))
    }
}

type R<T> = Result<T, Panic>;

fn panic_at<T>(e: &Expr, msg: String) -> R<T> {
    Err(Panic { line: e.span.line, col: e.span.col, msg })
}

impl Interp<'_> {
    /// The operations on strings (D134).
    fn str_op(&mut self, e: &Expr, op: isc_ir::StrOp, es: &[Expr]) -> R<Value> {
        use isc_ir::StrOp as S;
        let mut vs = Vec::new();
        for x in es {
            vs.push(self.eval(x)?);
        }
        let s = |v: &Value| match v {
            Value::Str(s) => s.clone(),
            v => panic!("string expected, got {:?}", v),
        };
        Ok(match op {
            S::Concat => {
                let (a, b) = (s(&vs[0]), s(&vs[1]));
                let mut t = String::with_capacity(a.len() + b.len());
                t.push_str(&a);
                t.push_str(&b);
                Value::Str(Rc::from(t))
            }
            S::Len => Value::Int(s(&vs[0]).len() as i64),
            S::Bytes => Value::Array(Rc::new(Array::Int(s(&vs[0]).bytes().map(|b| b as i64).collect()))),
            S::FromBytes => {
                let b: Vec<u8> = match &vs[0] {
                    Value::Array(a) => (0..a.len())
                        .map(|i| match a.get(i) {
                            Value::Int(v) => v as u8,
                            v => panic!("byte expected, got {:?}", v),
                        })
                        .collect(),
                    v => panic!("array expected, got {:?}", v),
                };
                match String::from_utf8(b) {
                    Ok(t) => Value::Str(Rc::from(t)),
                    Err(err) => return panic_at(e, rt::msg_not_utf8(err.utf8_error().valid_up_to() as i64)),
                }
            }
            S::ToString => {
                let t = match (&vs[0], &es[0].ty) {
                    (Value::Int(i), Type::Int(k)) if !k.signed() => (*i as u64).to_string(),
                    (Value::Int(i), _) => i.to_string(),
                    (Value::F64(f), _) => rt::format_f64(*f),
                    (Value::F32(f), _) => rt::format_f32(*f),
                    (Value::Bool(b), _) => b.to_string(),
                    (v, _) => panic!("scalar expected, got {:?}", v),
                };
                Value::Str(Rc::from(t))
            }
            S::Lines => {
                let t = s(&vs[0]);
                Value::Array(Rc::new(Array::Str(isc_runtime_core::text_lines(&t).map(Rc::from).collect())))
            }
            S::Split => {
                let (t, sep) = (s(&vs[0]), s(&vs[1]));
                if sep.is_empty() {
                    return panic_at(e, rt::msg_split_empty());
                }
                Value::Array(Rc::new(Array::Str(t.split(&*sep).map(Rc::from).collect())))
            }
            S::Trim => Value::Str(Rc::from(s(&vs[0]).trim())),
            S::Join => {
                let parts: Vec<Rc<str>> = match &vs[0] {
                    Value::Array(a) => match &**a {
                        Array::Str(v) => v.clone(),
                        a => panic!("strings expected, got {:?}", a),
                    },
                    v => panic!("array expected, got {:?}", v),
                };
                let sep = s(&vs[1]);
                let refs: Vec<&str> = parts.iter().map(|x| &**x).collect();
                Value::Str(Rc::from(refs.join(&*sep)))
            }
            S::Cmp(b) => {
                let o = s(&vs[0]).as_bytes().cmp(s(&vs[1]).as_bytes());
                use std::cmp::Ordering::*;
                Value::Bool(match b {
                    BinOp::Eq => o == Equal,
                    BinOp::Ne => o != Equal,
                    BinOp::Lt => o == Less,
                    BinOp::Le => o != Greater,
                    BinOp::Gt => o == Greater,
                    BinOp::Ge => o != Less,
                    _ => unreachable!(),
                })
            }
        })
    }
}

pub struct Interp<'a> {
    /// V of the fixed reduction form (32 in the language).
    lanes: usize,
    shapes: Vec<isc_ir::Shape>,
    env: Vec<Option<Value>>,
    out: &'a mut dyn Write,
    /// The recursive functions, and the variables each one binds (saved
    /// around a call: a call of itself binds them again).
    fns: Rc<Vec<isc_ir::Function>>,
    recs: Rc<Vec<RecType>>,
    locals: Rc<Vec<Vec<VarId>>>,
    /// An address near the top of the interpreter's stack.
    stack_top: usize,
    /// The direct tail calls of each function (SPEC section 11: a loop),
    /// the functions being run, and the arguments of a tail call that ends
    /// the current body.
    tail_sites: Rc<Vec<HashSet<*const Expr>>>,
    running: Vec<u32>,
    pending_tail: Option<(u32, Vec<Value>)>,
    /// The `extern` functions and their addresses (D125).
    externs: Rc<Vec<(isc_ir::ExternFn, usize)>>,
}

/// The calls of function `fi` to itself in tail position: through the body
/// of a `let`, the branches of an `if` and the body of a size test.
fn tail_sites(fi: u32, e: &Expr, out: &mut HashSet<*const Expr>) {
    match &e.kind {
        ExprKind::Let { body, .. } | ExprKind::CheckSize(_, _, body) | ExprKind::CheckNonNegative(_, body) | ExprKind::Free(_, body) => {
            tail_sites(fi, body, out)
        }
        ExprKind::If(_, t, f) => {
            tail_sites(fi, t, out);
            tail_sites(fi, f, out);
        }
        // a tail call of any function: run by the loop of the call in
        // progress, the frame reused (D102, D113)
        ExprKind::Call(..) => {
            let _ = fi;
            out.insert(e as *const Expr);
        }
        _ => {}
    }
}

/// With recursive functions, the interpreter runs in a thread of this stack
/// size, and stops on a stack overflow when a call finds less than
/// `STACK_MARGIN` bytes left (D102), as the compiled code does with the
/// stack of its thread.
const INTERP_STACK: usize = 4 << 30;
const STACK_MARGIN: usize = 16 << 20;

fn stack_address() -> usize {
    let x = 0u8;
    std::hint::black_box(&x) as *const u8 as usize
}

fn bound_vars(e: &Expr, out: &mut Vec<VarId>) {
    match &e.kind {
        ExprKind::Let { var, .. } => out.push(*var),
        ExprKind::Map(_, l) | ExprKind::Reduce(_, l, _) | ExprKind::Fold(_, _, l) | ExprKind::Scan(_, l, _) | ExprKind::Filter(_, l) => {
            out.extend(&l.params)
        }
        _ => {}
    }
    for c in e.children() {
        bound_vars(c, out);
    }
}

/// Runs `main`; returns its exit code.
pub fn run(p: &Program, args: &[String], out: &mut (dyn Write + Send)) -> R<i32> {
    run_with(p, args, out, LANES)
}

/// `run` with another V for the fixed reduction form (an experiment: the
/// language fixes V = 32).
pub fn run_with(p: &Program, args: &[String], out: &mut (dyn Write + Send), lanes: usize) -> R<i32> {
    assert!(lanes.is_power_of_two() && lanes >= 2 && BLOCK % lanes == 0, "invalid V");
    if p.fns.is_empty() {
        return run_in(p, args, out, lanes);
    }
    // recursion: a thread with a stack for MAX_DEPTH calls
    std::thread::scope(|s| {
        std::thread::Builder::new()
            .stack_size(INTERP_STACK)
            .spawn_scoped(s, || run_in(p, args, out, lanes))
            .expect("cannot start the interpreter's thread")
            .join()
            .expect("the interpreter's thread failed")
    })
}

fn run_in(p: &Program, args: &[String], out: &mut dyn Write, lanes: usize) -> R<i32> {
    let locals = p
        .fns
        .iter()
        .map(|f| {
            let mut v = f.params.clone();
            bound_vars(&f.body, &mut v);
            v
        })
        .collect();
    // the tail calls by address: in the copy that is run
    let fns = Rc::new(p.fns.clone());
    let mut it = Interp {
        lanes,
        shapes: p.shapes.clone(),
        env: vec![None; p.vars.len()],
        out,
        fns: fns.clone(),
        recs: Rc::new(p.recs.clone()),
        locals: Rc::new(locals),
        stack_top: stack_address(),
        tail_sites: Rc::new(
            fns.iter()
                .enumerate()
                .map(|(i, f)| {
                    let mut s = HashSet::new();
                    tail_sites(i as u32, &f.body, &mut s);
                    s
                })
                .collect(),
        ),
        running: Vec::new(),
        pending_tail: None,
        externs: Rc::new(Vec::new()),
    };
    // the C functions, found before the program starts (a compiled program
    // does not link without them)
    let mut externs = Vec::new();
    for f in &p.externs {
        match ffi::lookup(&f.name) {
            Some(a) => externs.push((f.clone(), a)),
            None => {
                return Err(Panic {
                    line: 0,
                    col: 0,
                    msg: format!("the extern function `{}` is not found (a library to link: `iscc run --link name`)", f.name),
                })
            }
        }
    }
    it.externs = Rc::new(externs);
    it.env[p.args as usize] =
        Some(Value::Array(Rc::new(Array::Str(args.iter().map(|s| Rc::from(s.as_str())).collect()))));
    let v = it.eval(&p.body)?;
    let _ = it.out.flush();
    match v {
        Value::Int(c) => Ok(c as i32),
        _ => Ok(0),
    }
}

fn int_kind(t: &Type) -> IntKind {
    match t {
        Type::Int(k) => *k,
        _ => panic!("not an integer type: {:?}", t),
    }
}

fn as_u(k: IntKind, v: i64) -> u128 {
    if k.signed() {
        v as i128 as u128
    } else {
        v as u64 as u128
    }
}

fn wide(k: IntKind, v: i64) -> i128 {
    if k.signed() {
        v as i128
    } else {
        v as u64 as i128
    }
}

pub fn int_mul(k: IntKind, a: i64, b: i64) -> i64 {
    canon(k, a.wrapping_mul(b) as i128)
}

/// `x ^ k` by binary exponentiation, most significant bit first (section 10).
pub fn pow_steps<T: Copy>(x: T, e: u64, one: T, mul: &dyn Fn(T, T) -> T) -> T {
    if e == 0 {
        return one;
    }
    let top = 63 - e.leading_zeros();
    let mut r = x;
    for b in (0..top).rev() {
        r = mul(r, r);
        if (e >> b) & 1 == 1 {
            r = mul(r, x);
        }
    }
    r
}

pub fn convert(v: &Value, from: &Type, to: &Type) -> Value {
    match (v, from, to) {
        (Value::Int(x), Type::Int(k1), Type::Int(k2)) => Value::Int(canon(*k2, wide(*k1, *x))),
        (Value::Int(x), Type::Int(k1), Type::F64) => {
            Value::F64(if k1.signed() { *x as f64 } else { *x as u64 as f64 })
        }
        (Value::F64(f), Type::F64, Type::Int(k)) => {
            let f = *f;
            Value::Int(match k {
                IntKind::I8 => f as i8 as i64,
                IntKind::I16 => f as i16 as i64,
                IntKind::I32 => f as i32 as i64,
                IntKind::I64 => f as i64,
                IntKind::U8 => f as u8 as i64,
                IntKind::U16 => f as u16 as i64,
                IntKind::U32 => f as u32 as i64,
                IntKind::U64 => f as u64 as i64,
            })
        }
        (Value::F64(f), Type::F64, Type::F64) => Value::F64(*f),
        (Value::Int(x), Type::Int(k1), Type::F32) => {
            Value::F32(if k1.signed() { *x as f32 } else { *x as u64 as f32 })
        }
        (Value::F32(f), Type::F32, Type::Int(_)) => convert(&Value::F64(*f as f64), &Type::F64, to),
        (Value::F32(f), Type::F32, Type::F64) => Value::F64(*f as f64),
        (Value::F64(f), Type::F64, Type::F32) => Value::F32(*f as f32),
        (Value::F32(f), Type::F32, Type::F32) => Value::F32(*f),
        _ => panic!("bad conversion {:?} {:?} -> {:?}", v, from, to),
    }
}

impl<'a> Interp<'a> {
    fn get(&mut self, v: VarId, u: Use) -> Value {
        let slot = &mut self.env[v as usize];
        match u {
            Use::Move => slot.take().expect("interpreter: variable used after move"),
            Use::Borrow => slot.clone().expect("interpreter: unbound variable"),
        }
    }

    fn set(&mut self, v: VarId, x: Value) {
        self.env[v as usize] = Some(x);
    }

    fn array(&mut self, e: &Expr) -> R<Rc<Array>> {
        match self.eval(e)? {
            Value::Array(a) => Ok(a),
            v => panic!("expected an array, got {:?}", v),
        }
    }

    fn int(&mut self, e: &Expr) -> R<i64> {
        match self.eval(e)? {
            Value::Int(i) => Ok(i),
            v => panic!("expected an integer, got {:?}", v),
        }
    }

    fn call(&mut self, l: &Lambda, args: Vec<Value>) -> R<Value> {
        for (p, a) in l.params.iter().zip(args) {
            self.set(*p, a);
        }
        self.eval(&l.body)
    }

    fn index_of(&self, e: &Expr, idx: i64, it: &Type, len: usize) -> R<usize> {
        let k = int_kind(it);
        let w = wide(k, idx);
        if w < 0 || w >= len as i128 {
            return panic_at(e, rt::msg_bounds(w, len as i64));
        }
        Ok(w as usize)
    }

    pub fn eval(&mut self, e: &Expr) -> R<Value> {
        use ExprKind::*;
        Ok(match &e.kind {
            StrLit(s) => Value::Str(Rc::from(s.as_str())),
            StrOp(op, es) => return self.str_op(e, *op, es),
            Lit(l) => match l {
                isc_ir::Lit::Int(v) => Value::Int(*v),
                isc_ir::Lit::F64(f) => Value::F64(*f),
                isc_ir::Lit::F32(f) => Value::F32(*f),
                isc_ir::Lit::Bool(b) => Value::Bool(*b),
                isc_ir::Lit::Unit => Value::Unit,
            },
            Var(v, u) => self.get(*v, *u),
            Let { var, init, body, .. } => {
                let x = self.eval(init)?;
                self.set(*var, x);
                self.eval(body)?
            }
            Tuple(es) => {
                let mut vs = Vec::new();
                for x in es {
                    vs.push(self.eval(x)?);
                }
                Value::Tuple(Rc::new(vs))
            }
            Proj(x, i) | Take(x, i) => match self.eval(x)? {
                Value::Tuple(t) => t[*i as usize].clone(),
                v => panic!("proj of {:?}", v),
            },
            Unary(op, x) => {
                let v = self.eval(x)?;
                match (op, v, &x.ty) {
                    (UnOp::Not, Value::Bool(b), _) => Value::Bool(!b),
                    (UnOp::Neg, Value::F64(f), _) => Value::F64(-f),
                    (UnOp::Neg, Value::F32(f), _) => Value::F32(-f),
                    (UnOp::Neg, Value::Int(i), t) => {
                        let k = int_kind(t);
                        Value::Int(canon(k, -wide(k, i)))
                    }
                    (o, v, _) => panic!("bad unary {:?} {:?}", o, v),
                }
            }
            Binary(op, a, b) => {
                let x = self.eval(a)?;
                let y = self.eval(b)?;
                self.binary(e, *op, x, y, &a.ty)?
            }
            Pow(b, k) => {
                let x = self.eval(b)?;
                let n = self.int(k)?;
                let kk = int_kind(&k.ty);
                if kk.signed() && n < 0 {
                    return panic_at(e, rt::msg_negexp(n));
                }
                let n = as_u(kk, n) as u64;
                match x {
                    Value::F64(f) => Value::F64(pow_steps(f, n, 1.0, &|a, b| a * b)),
                    Value::F32(f) => Value::F32(pow_steps(f, n, 1.0, &|a, b| a * b)),
                    Value::Int(i) => {
                        let bk = int_kind(&b.ty);
                        Value::Int(pow_steps(i, n, canon(bk, 1), &|a, c| int_mul(bk, a, c)))
                    }
                    v => panic!("bad pow base {:?}", v),
                }
            }
            Convert(x) => {
                let v = self.eval(x)?;
                convert(&v, &x.ty, &e.ty)
            }
            Math(f, xs) => {
                let mut vs = Vec::new();
                for x in xs {
                    vs.push(self.eval(x)?);
                }
                math(*f, &vs)
            }
            Sqrt(x) => match self.eval(x)? {
                Value::F64(f) => Value::F64(f.sqrt()),
                Value::F32(f) => Value::F32(f.sqrt()),
                v => panic!("sqrt of {:?}", v),
            },
            If(c, t, f) => match self.eval(c)? {
                Value::Bool(true) => self.eval(t)?,
                Value::Bool(false) => self.eval(f)?,
                v => panic!("if on {:?}", v),
            },
            Len(x) => Value::Int(self.array(x)?.len() as i64),
            ArrayLit(es) => {
                let mut a = Array::with_capacity(e.ty.elem(), es.len());
                for x in es {
                    let v = self.eval(x)?;
                    a.push(v);
                }
                Value::Array(Rc::new(a))
            }
            Index(a, i) => {
                let arr = self.array(a)?;
                let idx = self.int(i)?;
                let j = self.index_of(e, idx, &i.ty, arr.len())?;
                arr.get(j)
            }
            Window(x, k) => {
                // the rows copied: values are never mutated, so that a copy
                // cannot be told from the view of SPEC section 6
                let arr = self.array(x)?;
                let k = self.int(k)?;
                let n = arr.len() as i64;
                if k > n {
                    return panic_at(e, rt::msg_window(k, n));
                }
                let elem = x.ty.elem();
                let mut rows = Vec::with_capacity((n - k + 1) as usize);
                for i in 0..(n - k + 1) {
                    let mut row = Array::with_capacity(elem, k as usize);
                    for j in i..i + k {
                        row.push(arr.get(j as usize));
                    }
                    rows.push(Rc::new(row));
                }
                Value::Array(Rc::new(Array::Rows(rows)))
            }
            Slice(x, a, b) => {
                let arr = self.array(x)?;
                let lo = self.int(a)?;
                let hi = self.int(b)?;
                let n = arr.len() as i64;
                if lo < 0 || lo > hi || hi > n {
                    return panic_at(e, rt::msg_slice(lo, hi, n));
                }
                let mut out = Array::with_capacity(e.ty.elem(), (hi - lo) as usize);
                for j in lo..hi {
                    out.push(arr.get(j as usize));
                }
                Value::Array(Rc::new(out))
            }
            Update { target, index, value, .. } => {
                // section 8, rule 2: index and value first
                let idx = self.int(index)?;
                let v = self.eval(value)?;
                let mut arr = self.array(target)?;
                let j = self.index_of(e, idx, &index.ty, arr.len())?;
                Rc::make_mut(&mut arr).set(j, v);
                Value::Array(arr)
            }
            Map(ins, l) => {
                let mut arrs = Vec::new();
                for x in ins {
                    arrs.push(self.array(x)?);
                }
                let n = arrs[0].len();
                let mut out = Array::with_capacity(e.ty.elem(), n);
                for i in 0..n {
                    let args = arrs.iter().map(|a| a.get(i)).collect();
                    let v = self.call(l, args)?;
                    out.push(v);
                }
                Value::Array(Rc::new(out))
            }
            Reduce(x, l, z) => {
                let arr = self.array(x)?;
                let z = self.eval(z)?;
                let items: Vec<Value> = (0..arr.len()).map(|i| arr.get(i)).collect();
                self.reduce(&items, l, &z)?
            }
            Fold(x, init, l) => {
                let arr = self.array(x)?;
                let mut acc = self.eval(init)?;
                for i in 0..arr.len() {
                    acc = self.call(l, vec![acc, arr.get(i)])?;
                }
                acc
            }
            Filter(x, l) => {
                let arr = self.array(x)?;
                let mut out = Array::with_capacity(e.ty.elem(), 0);
                for i in 0..arr.len() {
                    let v = arr.get(i);
                    if let Value::Bool(true) = self.call(l, vec![v.clone()])? {
                        out.push(v);
                    }
                }
                Value::Array(Rc::new(out))
            }
            Scan(x, l, z) => {
                // from left to right, whatever the operator (SPEC section 6);
                // a scan of floats in its fixed form (D112): the first block
                // of B elements from `z`, each other block from its own
                // first element, `op(carry, ...)` with the carry the last
                // value of the block before
                let arr = self.array(x)?;
                let mut acc = self.eval(z)?;
                let mut out = Array::with_capacity(e.ty.elem(), arr.len());
                let blocked = e.ty.elem().is_float();
                let mut carry = acc.clone();
                for i in 0..arr.len() {
                    if blocked && i >= BLOCK && i % BLOCK == 0 {
                        carry = out.get(i - 1);
                        acc = arr.get(i);
                    } else {
                        acc = self.call(l, vec![acc, arr.get(i)])?;
                    }
                    let v = if blocked && i >= BLOCK { self.call(l, vec![carry.clone(), acc.clone()])? } else { acc.clone() };
                    out.push(v);
                }
                Value::Array(Rc::new(out))
            }
            Zeros(n) => {
                let n = self.int(n)?.max(0) as usize;
                Value::Array(Rc::new(match e.ty.elem() {
                    Type::Int(_) => Array::Int(vec![0; n]),
                    Type::F64 => Array::F64(vec![0.0; n]),
                    Type::F32 => Array::F32(vec![0.0; n]),
                    t => panic!("zeros of {:?}", t),
                }))
            }
            Iota(n) => {
                let n = self.int(n)?.max(0);
                Value::Array(Rc::new(Array::Int((0..n).collect())))
            }
            Fill(n, v) => {
                let n = self.int(n)?.max(0) as usize;
                let v = self.eval(v)?;
                let mut a = Array::with_capacity(e.ty.elem(), n);
                for _ in 0..n {
                    a.push(v.clone());
                }
                Value::Array(Rc::new(a))
            }
            CheckSize(a, b, body) => {
                let x = self.int(a)?;
                let y = self.int(b)?;
                if x != y {
                    return panic_at(e, rt::msg_size(x, y));
                }
                self.eval(body)?
            }
            CheckNonNegative(k, body) => {
                let x = self.int(k)?;
                if x < 0 {
                    return panic_at(e, rt::msg_negsize(x));
                }
                self.eval(body)?
            }
            Copy(x) => deep_copy(self.eval(x)?),
            Zero => zero_value(&e.ty),
            Action(op, args) => self.action(e, *op, args)?,
            Box(x) => Value::Node(Rc::new(self.eval(x)?)),
            // memory is the code generator's: the value of the body
            Free(_, body) => self.eval(body)?,
            Unbox(x) => match self.eval(x)? {
                Value::Node(inner) => (*inner).clone(),
                v => panic!("unbox of {:?}", v),
            },
            Call(f, args) => {
                let mut vals = Vec::new();
                for a in args {
                    vals.push(self.eval(a)?);
                }
                let fi = *f as usize;
                // a direct tail call: its arguments for the loop of the call
                // in progress, which runs the body again
                if let Some(&cur) = self.running.last() {
                    if self.tail_sites[cur as usize].contains(&(e as *const Expr)) {
                        self.pending_tail = Some((*f, vals));
                        return Ok(Value::Unit);
                    }
                }
                let fns = self.fns.clone();
                let func = &fns[fi];
                if self.stack_top.saturating_sub(stack_address()) > INTERP_STACK - STACK_MARGIN {
                    return Err(Panic { line: func.span.line, col: func.span.col, msg: rt::msg_stack(&func.name) });
                }
                // the variables of each function run by this loop, saved once
                // (a call of itself, or of one that calls back, binds them)
                let locals = self.locals.clone();
                let mut saved: Vec<(VarId, Option<Value>)> = Vec::new();
                let mut saved_fns: Vec<usize> = Vec::new();
                let mut cur = fi;
                let mut vals = vals;
                let r = loop {
                    if !saved_fns.contains(&cur) {
                        saved_fns.push(cur);
                        for v in &locals[cur] {
                            saved.push((*v, self.env[*v as usize].take()));
                        }
                    }
                    let func = &fns[cur];
                    for (p, x) in func.params.iter().zip(vals) {
                        self.env[*p as usize] = Some(x);
                    }
                    self.running.push(cur as u32);
                    let res = self.eval(&func.body);
                    self.running.pop();
                    match res {
                        Ok(v) => match self.pending_tail.take() {
                            Some((g, next)) => {
                                cur = g as usize;
                                vals = next;
                            }
                            None => break Ok(v),
                        },
                        Err(p) => break Err(p),
                    }
                };
                for (v, s) in saved.into_iter().rev() {
                    self.env[v as usize] = s;
                }
                r?
            }
            Bench(r, body) => {
                let reps = self.int(r)?;
                if reps < 1 {
                    return panic_at(e, rt::msg_bench(reps));
                }
                let t0 = std::time::Instant::now();
                let mut last = Value::Unit;
                for _ in 0..reps {
                    last = self.eval(body)?;
                }
                let ns = t0.elapsed().as_nanos() as u64;
                Value::Tuple(Rc::new(vec![last, Value::Int(ns as i64)]))
            }
        })
    }

    fn binary(&mut self, e: &Expr, op: BinOp, x: Value, y: Value, t: &Type) -> R<Value> {
        use BinOp::*;
        Ok(match (x, y) {
            // values of a recursive type: structural, IEEE equality of the
            // floats (D129)
            (Value::Node(a), Value::Node(b)) if op == Eq => Value::Bool(a == b),
            (Value::F64(a), Value::F64(b)) => match op {
                Add => Value::F64(a + b),
                Sub => Value::F64(a - b),
                Mul => Value::F64(a * b),
                Div => Value::F64(a / b),
                Rem => Value::F64(a % b),
                Eq => Value::Bool(a == b),
                Ne => Value::Bool(a != b),
                Lt => Value::Bool(a < b),
                Le => Value::Bool(a <= b),
                Gt => Value::Bool(a > b),
                Ge => Value::Bool(a >= b),
                Min => Value::F64(rt::minimum_f64(a, b)),
                Max => Value::F64(rt::maximum_f64(a, b)),
                And | Or | BitAnd | BitOr | BitXor | Shl | Shr => panic!("logic on floats"),
            },
            (Value::F32(a), Value::F32(b)) => match op {
                Add => Value::F32(a + b),
                Sub => Value::F32(a - b),
                Mul => Value::F32(a * b),
                Div => Value::F32(a / b),
                Rem => Value::F32(a % b),
                Eq => Value::Bool(a == b),
                Ne => Value::Bool(a != b),
                Lt => Value::Bool(a < b),
                Le => Value::Bool(a <= b),
                Gt => Value::Bool(a > b),
                Ge => Value::Bool(a >= b),
                Min => Value::F32(rt::minimum_f32(a, b)),
                Max => Value::F32(rt::maximum_f32(a, b)),
                And | Or | BitAnd | BitOr | BitXor | Shl | Shr => panic!("logic on floats"),
            },
            (Value::Int(a), Value::Int(b)) => {
                let k = int_kind(t);
                let (wa, wb) = (wide(k, a), wide(k, b));
                match op {
                    Add => Value::Int(canon(k, wa + wb)),
                    Sub => Value::Int(canon(k, wa - wb)),
                    Mul => Value::Int(int_mul(k, a, b)),
                    Div | Rem => {
                        if wb == 0 {
                            return panic_at(e, rt::msg_div0());
                        }
                        // i128 division truncates toward zero; MIN / -1 wraps
                        Value::Int(canon(k, if op == Div { wa / wb } else { wa % wb }))
                    }
                    Eq => Value::Bool(wa == wb),
                    Ne => Value::Bool(wa != wb),
                    Lt => Value::Bool(wa < wb),
                    Le => Value::Bool(wa <= wb),
                    Gt => Value::Bool(wa > wb),
                    Ge => Value::Bool(wa >= wb),
                    Min => Value::Int(if wa <= wb { a } else { b }),
                    Max => Value::Int(if wa >= wb { a } else { b }),
                    // canonical values stay canonical (D136)
                    BitAnd => Value::Int(a & b),
                    BitOr => Value::Int(a | b),
                    BitXor => Value::Int(a ^ b),
                    // the amount modulo the number of bits
                    Shl | Shr => {
                        let n = (b as u64 & (k.bits() as u64 - 1)) as u32;
                        Value::Int(match op {
                            Shl => canon(k, wa << n),
                            _ if k.signed() => a >> n,
                            _ => ((a as u64) >> n) as i64,
                        })
                    }
                    And | Or => panic!("logic on integers"),
                }
            }
            (Value::Bool(a), Value::Bool(b)) => match op {
                Eq => Value::Bool(a == b),
                Ne => Value::Bool(a != b),
                Lt => Value::Bool(!a & b),
                Le => Value::Bool(a <= b),
                Gt => Value::Bool(a & !b),
                Ge => Value::Bool(a >= b),
                _ => panic!("bad bool op"),
            },
            (x, y) => panic!("bad binary operands {:?} {:?}", x, y),
        })
    }

    /// `reduce_block` of section 10.
    fn reduce_block(&mut self, xs: &[Value], l: &Lambda, z: &Value) -> R<Value> {
        let lanes = self.lanes;
        let mut acc: Vec<Value> = vec![z.clone(); lanes];
        for (i, x) in xs.iter().enumerate() {
            let a = std::mem::replace(&mut acc[i % lanes], Value::Unit);
            acc[i % lanes] = self.call(l, vec![a, x.clone()])?;
        }
        let mut d = lanes / 2;
        while d >= 1 {
            for j in 0..d {
                let a = std::mem::replace(&mut acc[j], Value::Unit);
                let b = acc[j + d].clone();
                acc[j] = self.call(l, vec![a, b])?;
            }
            d /= 2;
        }
        Ok(acc.swap_remove(0))
    }

    /// `reduce` of section 10.
    fn reduce(&mut self, xs: &[Value], l: &Lambda, z: &Value) -> R<Value> {
        if xs.len() <= BLOCK {
            return self.reduce_block(xs, l, z);
        }
        let mut partials = Vec::new();
        for b in xs.chunks(BLOCK) {
            partials.push(self.reduce_block(b, l, z)?);
        }
        self.reduce(&partials, l, z)
    }

    fn string(&mut self, e: &Expr) -> R<Rc<str>> {
        match self.eval(e)? {
            Value::Str(s) => Ok(s),
            v => panic!("expected a string, got {:?}", v),
        }
    }

    fn action(&mut self, e: &Expr, op: ActionOp, args: &[Expr]) -> R<Value> {
        Ok(match op {
            ActionOp::ReadF64 => {
                let p = self.string(&args[0])?;
                match rt::read_f64_file(&p) {
                    Ok(v) => Value::Array(Rc::new(Array::F64(v))),
                    Err(m) => return panic_at(e, m),
                }
            }
            ActionOp::ReadF32 => {
                let p = self.string(&args[0])?;
                match rt::read_f32_file(&p) {
                    Ok(v) => Value::Array(Rc::new(Array::F32(v))),
                    Err(m) => return panic_at(e, m),
                }
            }
            ActionOp::WriteF32 => {
                let p = self.string(&args[0])?;
                let a = self.array(&args[1])?;
                let xs = match &*a {
                    Array::F32(v) => v.clone(),
                    _ => panic!("write_f32 of non-f32"),
                };
                if let Err(m) = rt::write_f32_file(&p, &xs) {
                    return panic_at(e, m);
                }
                Value::Unit
            }
            ActionOp::ReadBytes => {
                let p = self.string(&args[0])?;
                match rt::read_bytes_file(&p) {
                    Ok(v) => Value::Array(Rc::new(Array::Int(v.into_iter().map(|b| b as i64).collect()))),
                    Err(m) => return panic_at(e, m),
                }
            }
            ActionOp::WriteF64 => {
                let p = self.string(&args[0])?;
                let a = self.array(&args[1])?;
                let xs = match &*a {
                    Array::F64(v) => v.clone(),
                    _ => panic!("write_f64 of non-f64"),
                };
                if let Err(m) = rt::write_f64_file(&p, &xs) {
                    return panic_at(e, m);
                }
                Value::Unit
            }
            ActionOp::WriteBytes => {
                let p = self.string(&args[0])?;
                let a = self.array(&args[1])?;
                let xs: Vec<u8> = match &*a {
                    Array::Int(v) => v.iter().map(|b| *b as u8).collect(),
                    _ => panic!("write_bytes of non-u8"),
                };
                if let Err(m) = rt::write_bytes_file(&p, &xs) {
                    return panic_at(e, m);
                }
                Value::Unit
            }
            // the fallible forms (D138): the value and a message
            ActionOp::OrError(f) => {
                use isc_ir::Fallible as F;
                let pair = |v: Value, m: Result<(), String>| {
                    Value::Tuple(Rc::new(vec![v, Value::Str(Rc::from(m.err().unwrap_or_default()))]))
                };
                let empty = |t: &Type| Value::Array(Rc::new(Array::with_capacity(t.elem(), 0)));
                let vt = match &e.ty {
                    Type::Tuple(ts) => ts[0].clone(),
                    t => t.clone(),
                };
                let a = self.string(&args[0])?;
                match f {
                    F::ParseI64 => match rt::parse_i64(&a) {
                        Ok(v) => pair(Value::Int(v), Ok(())),
                        Err(m) => pair(Value::Int(0), Err(m)),
                    },
                    F::ParseF64 => match rt::parse_f64(&a) {
                        Ok(v) => pair(Value::F64(v), Ok(())),
                        Err(m) => pair(Value::F64(0.0), Err(m)),
                    },
                    F::ReadBytes => match rt::read_bytes_file(&a) {
                        Ok(v) => pair(Value::Array(Rc::new(Array::Int(v.into_iter().map(|b| b as i64).collect()))), Ok(())),
                        Err(m) => pair(empty(&vt), Err(m)),
                    },
                    F::ReadF64 => match rt::read_f64_file(&a) {
                        Ok(v) => pair(Value::Array(Rc::new(Array::F64(v))), Ok(())),
                        Err(m) => pair(empty(&vt), Err(m)),
                    },
                    F::ReadText => match rt::read_text_file(&a) {
                        Ok(t) => pair(Value::Str(Rc::from(t)), Ok(())),
                        Err(m) => pair(Value::Str(Rc::from("")), Err(m)),
                    },
                    F::ReadCsvF64 => {
                        let skip = self.int(&args[1])?;
                        match rt::read_text_file(&a).and_then(|t| rt::parse_csv(&t, skip)) {
                            Ok(rows) => pair(
                                Value::Array(Rc::new(Array::Rows(rows.into_iter().map(|r| Rc::new(Array::F64(r))).collect()))),
                                Ok(()),
                            ),
                            Err(m) => pair(empty(&vt), Err(m)),
                        }
                    }
                    F::WriteBytes | F::WriteText => {
                        let data: Vec<u8> = if f == F::WriteText {
                            self.string(&args[1])?.as_bytes().to_vec()
                        } else {
                            match &*self.array(&args[1])? {
                                Array::Int(v) => v.iter().map(|b| *b as u8).collect(),
                                _ => panic!("write_bytes of non-u8"),
                            }
                        };
                        Value::Str(Rc::from(rt::write_bytes_file(&a, &data).err().unwrap_or_default()))
                    }
                }
            }
            ActionOp::PrintBytes => {
                let a = self.array(&args[0])?;
                let xs: Vec<u8> = match &*a {
                    Array::Int(v) => v.iter().map(|b| *b as u8).collect(),
                    _ => panic!("print_bytes of non-u8"),
                };
                let _ = self.out.write_all(&xs);
                Value::Unit
            }
            ActionOp::Print(k) => {
                let v = self.eval(&args[0])?;
                let mut s = String::new();
                let shape = self.shapes[k as usize].clone();
                print_value(&v, &args[0].ty, &shape, &self.recs, &mut s);
                s.push('\n');
                let _ = self.out.write_all(s.as_bytes());
                Value::Unit
            }
            ActionOp::Clock => {
                thread_local! { static T0: std::time::Instant = std::time::Instant::now(); }
                Value::Int(T0.with(|t| t.elapsed().as_nanos() as i64))
            }
            ActionOp::ParseI64 => {
                let s = self.string(&args[0])?;
                match rt::parse_i64(&s) {
                    Ok(v) => Value::Int(v),
                    Err(m) => return panic_at(e, m),
                }
            }
            ActionOp::ParseF64 => {
                let s = self.string(&args[0])?;
                match rt::parse_f64(&s) {
                    Ok(v) => Value::F64(v),
                    Err(m) => return panic_at(e, m),
                }
            }
            // text (D135)
            ActionOp::ReadText => {
                let p = self.string(&args[0])?;
                match rt::read_text_file(&p) {
                    Ok(t) => Value::Str(Rc::from(t)),
                    Err(m) => return panic_at(e, m),
                }
            }
            ActionOp::WriteText => {
                let p = self.string(&args[0])?;
                let t = self.string(&args[1])?;
                if let Err(m) = rt::write_bytes_file(&p, t.as_bytes()) {
                    return panic_at(e, m);
                }
                Value::Unit
            }
            ActionOp::ParseCsvF64 | ActionOp::ReadCsvF64 => {
                let a = self.string(&args[0])?;
                let skip = self.int(&args[1])?;
                let t = if op == ActionOp::ReadCsvF64 {
                    match rt::read_text_file(&a) {
                        Ok(t) => Rc::from(t),
                        Err(m) => return panic_at(e, m),
                    }
                } else {
                    a
                };
                match rt::parse_csv(&t, skip) {
                    Ok(rows) => Value::Array(Rc::new(Array::Rows(rows.into_iter().map(|r| Rc::new(Array::F64(r))).collect()))),
                    Err(m) => return panic_at(e, m),
                }
            }
            ActionOp::Extern(i) => return self.call_extern(e, i, args),
        })
    }

    /// A call of a C function (D125): scalars in their registers, a
    /// `string` as a NUL-terminated copy, an array of scalars as a pointer
    /// to its elements (in their C width) and its length.
    fn call_extern(&mut self, e: &Expr, i: u32, args: &[Expr]) -> R<Value> {
        let externs = self.externs.clone();
        let (f, addr) = &externs[i as usize];
        let mut keep_strings: Vec<std::ffi::CString> = Vec::new();
        let mut keep_bytes: Vec<Vec<u8>> = Vec::new();
        let mut keep_arrays: Vec<Rc<Array>> = Vec::new();
        let mut cargs = Vec::new();
        for (a, t) in args.iter().zip(f.params.iter()) {
            let v = self.eval(a)?;
            match (t, v) {
                (Type::Int(_), Value::Int(x)) => cargs.push(ffi::Arg::Int(x)),
                (Type::Bool, Value::Bool(b)) => cargs.push(ffi::Arg::Int(b as i64)),
                (Type::F64, Value::F64(x)) => cargs.push(ffi::Arg::Float(x)),
                (Type::F32, Value::F32(x)) => cargs.push(ffi::Arg::Float(f64::from_bits(x.to_bits() as u64))),
                (Type::Str, Value::Str(s)) => match std::ffi::CString::new(s.as_bytes()) {
                    Ok(c) => {
                        cargs.push(ffi::Arg::Int(c.as_ptr() as i64));
                        keep_strings.push(c);
                    }
                    Err(_) => return panic_at(e, isc_runtime_core::Panic::ExternNul(&f.name).to_string()),
                },
                (Type::Array(el), Value::Array(arr)) => {
                    let (ptr, len) = match (&**el, &*arr) {
                        (Type::F64, Array::F64(x)) => (x.as_ptr() as i64, x.len()),
                        (Type::F32, Array::F32(x)) => (x.as_ptr() as i64, x.len()),
                        (Type::Bool, Array::Bool(x)) => (x.as_ptr() as i64, x.len()),
                        (Type::Int(k), Array::Int(x)) => {
                            let w = (k.bits() / 8) as usize;
                            let mut b = Vec::with_capacity(x.len() * w);
                            for v in x {
                                b.extend_from_slice(&v.to_le_bytes()[..w]);
                            }
                            let p = b.as_ptr() as i64;
                            keep_bytes.push(b);
                            (p, x.len())
                        }
                        (t, a) => panic!("extern argument: array {:?} for {:?}", a, t),
                    };
                    keep_arrays.push(arr);
                    cargs.push(ffi::Arg::Int(ptr));
                    cargs.push(ffi::Arg::Int(len as i64));
                }
                (t, v) => panic!("extern argument: {:?} for {:?}", v, t),
            }
        }
        let float = matches!(f.ret, Type::F64 | Type::F32);
        let (ri, rf) = unsafe { ffi::call(*addr, &cargs, float) };
        drop((keep_strings, keep_bytes, keep_arrays));
        Ok(match &f.ret {
            Type::F64 => Value::F64(rf),
            Type::F32 => Value::F32(f32::from_bits(rf.to_bits() as u32)),
            Type::Bool => Value::Bool(ri as u8 != 0),
            Type::Int(k) => Value::Int(match (k.bits(), k.signed()) {
                (8, true) => ri as i8 as i64,
                (8, false) => ri as u8 as i64,
                (16, true) => ri as i16 as i64,
                (16, false) => ri as u16 as i64,
                (32, true) => ri as i32 as i64,
                (32, false) => ri as u32 as i64,
                _ => ri,
            }),
            _ => Value::Unit,
        })
    }
}

fn deep_copy(v: Value) -> Value {
    match v {
        Value::Array(a) => Value::Array(Rc::new((*a).clone())),
        Value::Tuple(t) => Value::Tuple(Rc::new(t.iter().cloned().map(deep_copy).collect())),
        v => v,
    }
}

/// `print` (SPEC section 12): the value as its type and shape say.
fn print_value(v: &Value, t: &Type, shape: &isc_ir::Shape, recs: &[RecType], s: &mut String) {
    use isc_ir::Shape;
    match (v, t) {
        (Value::Node(inner), Type::Rec(i)) => {
            let r = &recs[*i as usize];
            print_value(inner, &r.layout, &r.shape, recs, s)
        }
        (Value::Int(i), Type::Int(k)) => {
            if k.signed() {
                s.push_str(&i.to_string())
            } else {
                s.push_str(&(*i as u64).to_string())
            }
        }
        (Value::F64(f), _) => s.push_str(&rt::format_f64(*f)),
        (Value::F32(f), _) => s.push_str(&rt::format_f32(*f)),
        (Value::Bool(b), _) => s.push_str(if *b { "true" } else { "false" }),
        (Value::Unit, _) => s.push_str("()"),
        (Value::Str(x), _) => s.push_str(x),
        (Value::Tuple(vs), Type::Tuple(ts)) if matches!(shape, Shape::Sum(_)) => {
            let vars = match shape {
                Shape::Sum(v) => v,
                _ => unreachable!(),
            };
            let tag = match &vs[0] {
                Value::Int(k) => *k as usize,
                v => panic!("sum tag {:?}", v),
            };
            let (name, slots, shapes) = &vars[tag];
            s.push_str(name);
            if !slots.is_empty() {
                s.push('(');
                for (i, (&slot, sh)) in slots.iter().zip(shapes.iter()).enumerate() {
                    if i > 0 {
                        s.push_str(", ");
                    }
                    print_value(&vs[slot as usize], &ts[slot as usize], sh, recs, s);
                }
                s.push(')');
            }
        }
        (Value::Tuple(vs), Type::Tuple(ts)) => {
            let (open, close, names, subs) = match shape {
                Shape::Record(ns, ss) => ('{', '}', Some(ns), Some(ss)),
                Shape::Tuple(ss) => ('(', ')', None, Some(ss)),
                _ => ('(', ')', None, None),
            };
            s.push(open);
            for (i, (v, t)) in vs.iter().zip(ts.iter()).enumerate() {
                if i > 0 {
                    s.push_str(", ");
                }
                if let Some(ns) = names {
                    s.push_str(&ns[i]);
                    s.push_str(": ");
                }
                print_value(v, t, subs.map(|ss| &ss[i]).unwrap_or(&Shape::Plain), recs, s);
            }
            s.push(close);
        }
        (Value::Array(a), Type::Array(et)) => {
            let sub = match shape {
                Shape::Array(e) => &**e,
                _ => &Shape::Plain,
            };
            s.push('[');
            for i in 0..a.len() {
                if i > 0 {
                    s.push_str(", ");
                }
                print_value(&a.get(i), et, sub, recs, s);
            }
            s.push(']');
        }
        (v, t) => panic!("cannot print {:?} as {:?}", v, t),
    }
}

/// The zero value of a type (`ExprKind::Zero`).
fn zero_value(t: &Type) -> Value {
    match t {
        Type::Int(_) => Value::Int(0),
        Type::F64 => Value::F64(0.0),
        Type::F32 => Value::F32(0.0),
        Type::Bool => Value::Bool(false),
        Type::Unit => Value::Unit,
        Type::Str => Value::Str(Rc::from("")),
        Type::Tuple(ts) => Value::Tuple(Rc::new(ts.iter().map(zero_value).collect())),
        Type::Array(e) => Value::Array(Rc::new(Array::with_capacity(e, 0))),
        // the slot of a variant not taken: never read
        Type::Rec(_) => Value::Unit,
    }
}

/// A mathematical function: the code of the runtime (D108).
fn math(f: MathFn, vs: &[Value]) -> Value {
    use isc_runtime_core as m;
    match (f, vs) {
        (MathFn::Exp, [Value::F64(x)]) => Value::F64(m::exp_f64(*x)),
        (MathFn::Log, [Value::F64(x)]) => Value::F64(m::log_f64(*x)),
        (MathFn::Sin, [Value::F64(x)]) => Value::F64(m::sin_f64(*x)),
        (MathFn::Cos, [Value::F64(x)]) => Value::F64(m::cos_f64(*x)),
        (MathFn::Pow, [Value::F64(x), Value::F64(y)]) => Value::F64(m::pow_f64(*x, *y)),
        (MathFn::Fma, [Value::F64(a), Value::F64(b), Value::F64(c)]) => Value::F64(m::fma_f64(*a, *b, *c)),
        (MathFn::Exp, [Value::F32(x)]) => Value::F32(m::exp_f32(*x)),
        (MathFn::Log, [Value::F32(x)]) => Value::F32(m::log_f32(*x)),
        (MathFn::Sin, [Value::F32(x)]) => Value::F32(m::sin_f32(*x)),
        (MathFn::Cos, [Value::F32(x)]) => Value::F32(m::cos_f32(*x)),
        (MathFn::Pow, [Value::F32(x), Value::F32(y)]) => Value::F32(m::pow_f32(*x, *y)),
        (MathFn::Fma, [Value::F32(a), Value::F32(b), Value::F32(c)]) => Value::F32(m::fma_f32(*a, *b, *c)),
        (f, vs) => panic!("{} of {:?}", f.name(), vs),
    }
}
